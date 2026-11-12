//! The Hevy history import, reading the real Hevy account or the mock one.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

use crate::config::Config;
use crate::db::{self, ActivityRow, DbResult};
use crate::hevy_client::{HevyClient, HevyError};
use crate::hevy_db::Workout;
use crate::hevy_sync::{self, WatchMatch};
use crate::{exercise_tables, hevy_db, timefmt};

const PAGE_SIZE: i64 = 10;

pub struct Item {
    pub workout: Value,
    pub twin: Option<ActivityRow>,
    pub action: String,
    pub missing_template_ids: Vec<String>,
}

fn field<'a>(workout: &'a Value, key: &str) -> &'a str {
    workout.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn at(workout: &Value, key: &str) -> Option<DateTime<Utc>> {
    workout
        .get(key)
        .and_then(Value::as_str)
        .and_then(timefmt::parse_timestamp)
}

pub fn workout_id(workout: &Value) -> &str {
    field(workout, "id")
}

/// The mock account's workouts from `since` on; it serves them all at once.
pub fn workouts_since(account: Vec<Value>, since: DateTime<Utc>) -> Vec<Value> {
    account
        .into_iter()
        .filter(|w| at(w, "start_time").is_some_and(|start| start >= since))
        .collect()
}

/// Total workouts on the Hevy account, or `None` if it cannot be read. It
/// only bounds the scan and labels the preview, so an outage degrades.
pub async fn account_workout_count(client: &HevyClient) -> Option<i64> {
    client.get_workout_count().await.unwrap_or_else(|err| {
        crate::log_warn!("hevy_backfill", "hevy workout count unavailable: {err}");
        None
    })
}

/// Pages through Hevy's newest-first workouts back to `since`. The account
/// total bounds the scan when a page omits `page_count`, which would
/// otherwise read as the last page and cut history short.
pub async fn fetch_since(
    client: &HevyClient,
    since: DateTime<Utc>,
    total_count: Option<i64>,
) -> Result<Vec<Value>, HevyError> {
    let max_pages = total_count.map(|total| ((total.max(0) + PAGE_SIZE - 1) / PAGE_SIZE).max(1));
    let mut collected = Vec::new();
    let mut page = 1;
    loop {
        let mut data = client.get_workouts_page(page, PAGE_SIZE).await?;
        let workouts = match data.get_mut("workouts").map(Value::take) {
            Some(Value::Array(workouts)) => workouts,
            _ => Vec::new(),
        };
        let any = !workouts.is_empty();
        let mut older_seen = false;
        for workout in workouts {
            match at(&workout, "start_time") {
                None => continue,
                Some(start) if start < since => older_seen = true,
                Some(_) => collected.push(workout),
            }
        }
        if older_seen || !any {
            return Ok(collected);
        }
        let bound = data
            .get("page_count")
            .filter(|c| c.is_i64() || c.is_u64())
            .and_then(Value::as_i64)
            .or(max_pages);
        if bound.is_none_or(|bound| page >= bound) {
            return Ok(collected);
        }
        page += 1;
    }
}

fn is_strength(activity: &ActivityRow) -> bool {
    matches!(
        activity.activity_type.as_str(),
        "strength_training" | "other"
    )
}

fn twin_activity(activities: &[ActivityRow], workout: &Value) -> Option<ActivityRow> {
    let start = at(workout, "start_time")?;
    activities
        .iter()
        .filter(|a| is_strength(a))
        .find(|a| timefmt::parse_timestamp(&a.start_time) == Some(start))
        .cloned()
}

/// The sync path's watch match for a workout that is not stored yet. Its id
/// is one no stored workout has, so every existing claim counts as another's.
fn find_watch_match(conn: &Connection, workout: &Value) -> DbResult<WatchMatch> {
    let row = Workout {
        hevy_id: format!("__preview_{}__", workout_id(workout)),
        title: String::new(),
        start_time: field(workout, "start_time").into(),
        end_time: field(workout, "end_time").into(),
        source_updated_at: None,
        payload: String::new(),
        status: String::new(),
        applied_strategy: None,
        source_garmin_activity_id: None,
        garmin_activity_id: None,
        error: None,
        garmin_applied_updated_at: None,
        strava_applied_updated_at: None,
    };
    hevy_sync::find_watch_match(conn, &row)
}

/// Whether any exercise failed to resolve, and the distinct template ids
/// involved; an exercise without a template id misses with no id to list.
fn mapping_misses(conn: &Connection, workout: &Value) -> DbResult<(bool, Vec<String>)> {
    let (mut any, mut missing) = (false, Vec::new());
    for exercise in workout
        .get("exercises")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let template = exercise.get("exercise_template_id").and_then(Value::as_str);
        if exercise_tables::lookup_exercise(conn, field(exercise, "title"), template)?.is_none() {
            any = true;
            if let Some(id) = template.filter(|id| !id.is_empty())
                && !missing.iter().any(|m| m == id)
            {
                missing.push(id.to_string());
            }
        }
    }
    Ok((any, missing))
}

fn owned(conn: &Connection, activity_id: i64) -> DbResult<bool> {
    let found: Option<String> = conn
        .query_row(
            "SELECT hevy_id FROM hevy_workouts WHERE source_garmin_activity_id = ?1 OR garmin_activity_id = ?1 LIMIT 1",
            [activity_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

/// The import plan, without writing anything. Workouts already in the queue
/// are left out unless the owner skipped them there.
pub fn preview_items(
    conn: &Connection,
    workouts: Vec<Value>,
    cfg: &Config,
    now: DateTime<Utc>,
) -> DbResult<Vec<Item>> {
    let activities = db::list_activities(conn, false)?;
    let review = cfg.get("hevy_match_mode").and_then(Value::as_str) == Some("review");
    let strategy = cfg.str("hevy_watch_strategy");
    // Describe uploads no sets, so a missing mapping only matters once the
    // workout would be uploaded on its own.
    let gated_misses = |workout: &Value| {
        if strategy == "describe" {
            Ok((false, Vec::new()))
        } else {
            mapping_misses(conn, workout)
        }
    };
    let mut items = Vec::new();
    for workout in workouts {
        let hevy_id = workout_id(&workout);
        if !hevy_id.is_empty()
            && hevy_db::get_workout(conn, hevy_id)?.is_some_and(|w| w.status != "skipped")
        {
            continue;
        }
        let mut missing = Vec::new();
        let mut twin = twin_activity(&activities, &workout);
        let action: String = if let Some(found) = &twin {
            if owned(conn, found.garmin_activity_id)? {
                "needs_review".into()
            } else if found.is_published() || review {
                "awaiting_match".into()
            } else {
                strategy.clone()
            }
        } else {
            match find_watch_match(conn, &workout)? {
                WatchMatch::One(id) => {
                    twin = db::get_activity(conn, id)?;
                    if twin.as_ref().is_some_and(ActivityRow::is_published) || review {
                        "awaiting_match".into()
                    } else {
                        let (miss, ids) = gated_misses(&workout)?;
                        missing = ids;
                        if miss {
                            "needs_mapping".into()
                        } else {
                            strategy.clone()
                        }
                    }
                }
                WatchMatch::Multiple | WatchMatch::Claimed => "needs_review".into(),
                WatchMatch::None => {
                    let (miss, ids) = gated_misses(&workout)?;
                    missing = ids;
                    let grace = Duration::minutes(cfg.int("hevy_grace_minutes"));
                    if miss {
                        "needs_mapping".into()
                    } else if at(&workout, "end_time").is_some_and(|end| now - end < grace) {
                        "waiting_watch".into()
                    } else if strategy == "describe" {
                        let (miss, ids) = mapping_misses(conn, &workout)?;
                        missing = ids;
                        if miss { "needs_mapping" } else { "passive" }.into()
                    } else {
                        "passive".into()
                    }
                }
            }
        };
        items.push(Item {
            workout,
            twin,
            action,
            missing_template_ids: missing,
        });
    }
    Ok(items)
}

/// Ingests the chosen items and claims their unique Garmin matches; returns
/// how many were matched. Nothing polls Hevy in mock mode, so matched
/// workouts stay waiting until a real server picks them up.
pub fn run_items(conn: &Connection, items: &[&Item]) -> DbResult<usize> {
    let mut matched = 0;
    for item in items {
        let workout = &item.workout;
        let hevy_id = workout_id(workout);
        if hevy_id.is_empty() {
            continue;
        }
        let existing = hevy_db::get_workout(conn, hevy_id)?;
        if existing.as_ref().is_some_and(|w| w.status != "skipped") {
            continue;
        }
        hevy_db::upsert_workout(
            conn,
            hevy_id,
            field(workout, "title"),
            field(workout, "start_time"),
            field(workout, "end_time"),
            field(workout, "updated_at"),
            workout,
        )?;
        let Some(twin) = &item.twin else {
            if existing.is_some() {
                hevy_db::set_workout_status(conn, hevy_id, "waiting_watch", None)?;
            }
            continue;
        };
        if item.action == "needs_review" {
            hevy_db::set_workout_status(
                conn,
                hevy_id,
                "needs_review",
                Some("backfill twin is already claimed by another workout"),
            )?;
            continue;
        }
        if !hevy_db::claim_source(conn, hevy_id, twin.garmin_activity_id)? {
            crate::log_warn!(
                "hevy_backfill",
                "backfill twin for {hevy_id} already claimed"
            );
            hevy_db::set_workout_status(
                conn,
                hevy_id,
                "needs_review",
                Some("backfill match was claimed by another workout"),
            )?;
            continue;
        }
        if item.action == "awaiting_match" {
            let error = if twin.is_published() {
                "Already published to Strava. Choose Merge or Description only."
            } else {
                "Choose how to apply this Hevy workout."
            };
            hevy_db::set_workout_status(conn, hevy_id, "awaiting_match", Some(error))?;
        } else {
            hevy_db::set_workout_status(conn, hevy_id, "waiting_watch", None)?;
        }
        matched += 1;
    }
    Ok(matched)
}
