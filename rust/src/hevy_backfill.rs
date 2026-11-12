//! The Hevy history import, as in the Python app's `hevy_backfill.py`,
//! reading the real Hevy account or the mock one.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

use crate::config::Config;
use crate::db::{self, ActivityRow, DbResult};
use crate::hevy_client::{HevyClient, HevyError};
use crate::{exercise_tables, hevy_db, hevy_seed, timefmt};

const OVERLAP_MIN: f64 = 0.70;
const DRIFT_MAX_MINUTES: f64 = 20.0;

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

/// The mock account serves every workout on one page, so the scan never
/// pages; the boundary filter is the part that matters.
pub fn workouts_since(account: Vec<Value>, since: DateTime<Utc>) -> Vec<Value> {
    account
        .into_iter()
        .filter(|w| at(w, "start_time").is_some_and(|start| start >= since))
        .collect()
}

const PAGE_SIZE: i64 = 10;

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
    let max_pages = total_count
        .map(|total| (total.max(0) + PAGE_SIZE - 1) / PAGE_SIZE)
        .map(|pages| pages.max(1));
    let mut collected = Vec::new();
    let mut page = 1;
    loop {
        let data = client.get_workouts_page(page, PAGE_SIZE).await?;
        let workouts = data
            .get("workouts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut older_seen = false;
        for workout in &workouts {
            match at(workout, "start_time") {
                None => continue,
                Some(start) if start < since => older_seen = true,
                Some(_) => collected.push(workout.clone()),
            }
        }
        if older_seen || workouts.is_empty() {
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

pub fn account(now: DateTime<Utc>) -> Vec<Value> {
    hevy_seed::account_workouts(now)
}

fn is_strength(activity: &ActivityRow) -> bool {
    matches!(
        activity.activity_type.as_str(),
        "strength_training" | "other"
    )
}

fn duration_seconds(activity: &ActivityRow) -> f64 {
    serde_json::from_str::<Value>(&activity.garmin_data)
        .ok()
        .and_then(|data| data.get("duration").and_then(Value::as_f64))
        .unwrap_or(0.0)
}

fn twin_activity(activities: &[ActivityRow], workout: &Value) -> Option<ActivityRow> {
    let start = at(workout, "start_time")?;
    activities
        .iter()
        .filter(|a| is_strength(a))
        .find(|a| timefmt::parse_timestamp(&a.start_time) == Some(start))
        .cloned()
}

pub enum WatchMatch {
    One(i64),
    Multiple,
    Claimed,
    None,
}

/// A local watch activity overlapping most of the workout and starting
/// within 20 minutes of it. Activities other workouts own do not count.
fn find_watch_match(
    conn: &Connection,
    activities: &[ActivityRow],
    workout: &Value,
) -> DbResult<WatchMatch> {
    let (Some(start), Some(end)) = (at(workout, "start_time"), at(workout, "end_time")) else {
        return Ok(WatchMatch::None);
    };
    if end <= start {
        return Ok(WatchMatch::None);
    }
    let duration = (end - start).num_milliseconds() as f64 / 1000.0;
    let mut stmt =
        conn.prepare("SELECT source_garmin_activity_id, garmin_activity_id FROM hevy_workouts")?;
    let claims: Vec<i64> = stmt
        .query_map([], |row| {
            Ok([row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?])
        })?
        .collect::<DbResult<Vec<_>>>()?
        .into_iter()
        .flatten()
        .flatten()
        .collect();
    let (mut unclaimed, mut claimed_by_others) = (Vec::new(), 0);
    for activity in activities.iter().filter(|a| is_strength(a)) {
        let Some(act_start) = timefmt::parse_timestamp(&activity.start_time) else {
            continue;
        };
        let act_end =
            act_start + Duration::milliseconds((duration_seconds(activity) * 1000.0) as i64);
        let overlap = (end.min(act_end) - start.max(act_start)).num_milliseconds() as f64 / 1000.0;
        if overlap <= 0.0 || overlap / duration < OVERLAP_MIN {
            continue;
        }
        if (act_start - start).num_milliseconds().abs() as f64 / 60_000.0 > DRIFT_MAX_MINUTES {
            continue;
        }
        if claims.contains(&activity.garmin_activity_id) {
            claimed_by_others += 1;
        } else {
            unclaimed.push(activity.garmin_activity_id);
        }
    }
    Ok(match (unclaimed.as_slice(), claimed_by_others) {
        ([one], _) => WatchMatch::One(*one),
        ([], 0) => WatchMatch::None,
        ([], _) => WatchMatch::Claimed,
        _ => WatchMatch::Multiple,
    })
}

/// Whether any exercise failed to resolve, and the distinct template ids
/// involved; an exercise without a template id misses with no id to list.
fn mapping_misses(conn: &Connection, workout: &Value) -> DbResult<(bool, Vec<String>)> {
    let (mut any, mut missing) = (false, Vec::new());
    let empty = Vec::new();
    for exercise in workout
        .get("exercises")
        .and_then(Value::as_array)
        .unwrap_or(&empty)
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
        let action = if let Some(found) = &twin {
            if owned(conn, found.garmin_activity_id)? {
                "needs_review".to_string()
            } else if found.is_published() || review {
                "awaiting_match".to_string()
            } else {
                strategy.clone()
            }
        } else {
            match find_watch_match(conn, &activities, &workout)? {
                WatchMatch::One(id) => {
                    twin = db::get_activity(conn, id)?;
                    if twin.as_ref().is_some_and(ActivityRow::is_published) || review {
                        "awaiting_match".to_string()
                    } else {
                        let (miss, ids) = if strategy == "describe" {
                            (false, Vec::new())
                        } else {
                            mapping_misses(conn, &workout)?
                        };
                        missing = ids;
                        if miss {
                            "needs_mapping".to_string()
                        } else {
                            strategy.clone()
                        }
                    }
                }
                WatchMatch::Multiple | WatchMatch::Claimed => "needs_review".to_string(),
                WatchMatch::None => {
                    let (miss, ids) = if strategy == "describe" {
                        (false, Vec::new())
                    } else {
                        mapping_misses(conn, &workout)?
                    };
                    missing = ids;
                    let grace = Duration::minutes(cfg.int("hevy_grace_minutes"));
                    if miss {
                        "needs_mapping".to_string()
                    } else if at(&workout, "end_time").is_some_and(|end| now - end < grace) {
                        "waiting_watch".to_string()
                    } else if strategy == "describe" {
                        let (miss, ids) = mapping_misses(conn, &workout)?;
                        missing = ids;
                        if miss { "needs_mapping" } else { "passive" }.to_string()
                    } else {
                        "passive".to_string()
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
