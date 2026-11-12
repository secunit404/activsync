use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hevy_apply::float;
use crate::hevy_db::{self, Workout};
use crate::hevy_description::generate_description;
use crate::hevy_match::{self, MatchError};
use crate::routes::{ApiError, ApiResult};
use crate::{AppState, config, db, schedule, timefmt, view};

const IN_FLIGHT: [&str; 3] = ["waiting_watch", "awaiting_match", "syncing"];
const PROBLEMS: [&str; 3] = ["needs_mapping", "failed", "needs_review"];
const TERMINAL_STATUSES: [&str; 5] = [
    "merged",
    "described",
    "replaced",
    "uploaded_passive",
    "linked_existing",
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HevyQueueItem {
    hevy_id: String,
    title: String,
    status: String,
    error: Option<String>,
    start_display: String,
    needs_mapping: bool,
    has_open_operation: bool,
    resyncable: bool,
    awaiting_match: bool,
    matched_garmin_activity_id: Option<i64>,
    matched_garmin_title: Option<String>,
    matched_strava_activity_id: Option<i64>,
    matched_strava_url: Option<String>,
    match_scheduled_at: Option<String>,
    match_phase: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HevyQueueCounts {
    in_flight: usize,
    problems: usize,
    skipped: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HevyQueueState {
    enabled: bool,
    in_flight: Vec<HevyQueueItem>,
    problems: Vec<HevyQueueItem>,
    skipped: Vec<HevyQueueItem>,
    counts: HevyQueueCounts,
}

/// Hevy rows carry ISO timestamps; normalised to Garmin's form so both render
/// through the same formatter, with the raw value when that fails.
fn start_display(start_time: &str, tz_name: &str) -> String {
    let normalised = start_time.replace('T', " ");
    let cleaned = normalised
        .split('+')
        .next()
        .unwrap_or_default()
        .trim_end_matches('Z');
    match (
        timefmt::parse_garmin(cleaned),
        tz_name.parse::<chrono_tz::Tz>(),
    ) {
        (Some(utc), Ok(tz)) => utc.with_timezone(&tz).format("%Y-%m-%d %H:%M").to_string(),
        _ => start_time.to_string(),
    }
}

fn queue_items(
    conn: &Connection,
    statuses: &[&str],
    tz_name: &str,
) -> Result<Vec<HevyQueueItem>, ApiError> {
    let decisions = hevy_match::active_by_workout(conn)?;
    let mut items = Vec::new();
    for status in statuses {
        for row in hevy_db::list_workouts(conn, status)? {
            let decision = decisions.get(&row.hevy_id);
            let matched = match row.source_garmin_activity_id {
                Some(id) => db::get_activity(conn, id)?,
                None => None,
            };
            let error = row.error.clone().unwrap_or_default();
            items.push(HevyQueueItem {
                title: row.display_title().to_string(),
                status: status.to_string(),
                start_display: start_display(&row.start_time, tz_name),
                needs_mapping: *status == "needs_mapping",
                has_open_operation: hevy_db::get_open_operation(conn, &row.hevy_id)?.is_some(),
                resyncable: error.contains("deleted on Garmin"),
                awaiting_match: *status == "awaiting_match",
                matched_garmin_activity_id: row.source_garmin_activity_id,
                matched_garmin_title: matched.as_ref().map(|a| a.title.clone()),
                matched_strava_activity_id: matched
                    .as_ref()
                    .filter(|a| a.publish_status == "published")
                    .and_then(|a| a.strava_activity_id),
                matched_strava_url: matched.as_ref().and_then(view::strava_url_for),
                match_scheduled_at: decision.map(|(deadline, _)| schedule::deadline_iso(*deadline)),
                match_phase: decision.map(|(_, state)| state.clone()),
                hevy_id: row.hevy_id,
                error: row.error,
            });
        }
    }
    Ok(items)
}

pub async fn queue_state(State(state): State<AppState>) -> ApiResult<HevyQueueState> {
    let conn = state.db.lock();
    let cfg = config::load_config(&conn, state.development)?;
    let tz_name = cfg.display_timezone();
    let in_flight = queue_items(&conn, &IN_FLIGHT, &tz_name)?;
    let problems = queue_items(&conn, &PROBLEMS, &tz_name)?;
    let skipped = queue_items(&conn, &["skipped"], &tz_name)?;
    Ok(Json(HevyQueueState {
        enabled: db::truthy(cfg.get("hevy_enabled")),
        counts: HevyQueueCounts {
            in_flight: in_flight.len(),
            problems: problems.len(),
            skipped: skipped.len(),
        },
        in_flight,
        problems,
        skipped,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HevyWorkoutSetDetail {
    number: usize,
    set_type: String,
    reps: Option<f64>,
    weight_kg: Option<f64>,
    distance_meters: Option<f64>,
    duration_seconds: Option<f64>,
    rpe: Option<f64>,
    custom_metric: Option<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HevyWorkoutExerciseDetail {
    title: String,
    notes: Option<String>,
    template_id: Option<String>,
    sets: Vec<HevyWorkoutSetDetail>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HevyWorkoutDetail {
    hevy_id: String,
    title: String,
    start_time: String,
    end_time: String,
    notes: Option<String>,
    exercises: Vec<HevyWorkoutExerciseDetail>,
    description_preview: String,
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn custom_metric(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Null => None,
        Value::String(s) => Some(Value::String(s.clone())),
        Value::Number(n) => n.as_f64().map(Value::from),
        Value::Bool(b) => Some(Value::from(f64::from(u8::from(*b)))),
        other => Some(Value::String(other.to_string())),
    }
}

/// The row a workout's queue entry or import preview row shows, with its
/// description as it would be written.
pub fn workout_detail(
    payload: &Value,
    row: (&str, &str, &str, &str),
    template: Option<&str>,
) -> Result<HevyWorkoutDetail, ApiError> {
    let (hevy_id, title, start_time, end_time) = row;
    let empty = Vec::new();
    let exercises = payload
        .get("exercises")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let exercises = exercises
        .iter()
        .map(|exercise| {
            let sets = exercise
                .get("sets")
                .and_then(Value::as_array)
                .unwrap_or(&empty);
            HevyWorkoutExerciseDetail {
                title: text(exercise.get("title"))
                    .or_else(|| text(exercise.get("name")))
                    .unwrap_or_else(|| "Unknown exercise".into()),
                notes: text(exercise.get("notes")),
                template_id: text(exercise.get("exercise_template_id")),
                sets: sets
                    .iter()
                    .enumerate()
                    .map(|(index, set)| HevyWorkoutSetDetail {
                        number: index + 1,
                        set_type: text(set.get("type")).unwrap_or_else(|| "normal".into()),
                        reps: float(set.get("reps")),
                        weight_kg: float(set.get("weight_kg").or_else(|| set.get("weight"))),
                        distance_meters: float(set.get("distance_meters")),
                        duration_seconds: float(set.get("duration_seconds")),
                        rpe: float(set.get("rpe")),
                        custom_metric: custom_metric(set.get("custom_metric")),
                    })
                    .collect(),
            }
        })
        .collect();
    let preview = generate_description(payload, None, None, template).map_err(|err| {
        crate::log_warn!("hevy_queue", "Hevy description template is invalid: {err}");
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
    })?;
    Ok(HevyWorkoutDetail {
        hevy_id: hevy_id.to_string(),
        title: Some(title.to_string())
            .filter(|t| !t.is_empty())
            .or_else(|| text(payload.get("title")))
            .unwrap_or_else(|| hevy_id.to_string()),
        start_time: text(payload.get("start_time")).unwrap_or_else(|| start_time.to_string()),
        end_time: text(payload.get("end_time")).unwrap_or_else(|| end_time.to_string()),
        notes: text(payload.get("description")).or_else(|| text(payload.get("notes"))),
        exercises,
        description_preview: preview,
    })
}

fn require_workout(conn: &Connection, hevy_id: &str) -> Result<Workout, ApiError> {
    hevy_db::get_workout(conn, hevy_id)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Unknown Hevy workout."))
}

pub async fn workout(
    State(state): State<AppState>,
    Path(hevy_id): Path<String>,
) -> ApiResult<HevyWorkoutDetail> {
    let conn = state.db.lock();
    let workout = require_workout(&conn, &hevy_id)?;
    let cfg = config::load_config(&conn, state.development)?;
    let row = (
        workout.hevy_id.as_str(),
        workout.title.as_str(),
        workout.start_time.as_str(),
        workout.end_time.as_str(),
    );
    let template = cfg.get("hevy_description_template").and_then(Value::as_str);
    Ok(Json(workout_detail(
        &workout.payload_json(),
        row,
        template,
    )?))
}

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HevyStrategy {
    Replace,
    Merge,
    Describe,
}

impl HevyStrategy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Merge => "merge",
            Self::Describe => "describe",
        }
    }
}

#[derive(Deserialize)]
pub struct HevyMatchRequest {
    strategy: HevyStrategy,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HevyMatchScheduled {
    deadline: String,
    strategy: HevyStrategy,
    remaining_ms: i64,
    undo_window_ms: i64,
}

pub async fn choose_match(
    State(state): State<AppState>,
    Path(hevy_id): Path<String>,
    payload: Result<Json<HevyMatchRequest>, JsonRejection>,
) -> ApiResult<HevyMatchScheduled> {
    let strategy = payload?.0.strategy;
    {
        let conn = state.db.lock();
        let workout = require_workout(&conn, &hevy_id)?;
        if workout.status != "awaiting_match" {
            return Err(ApiError::conflict(
                "This workout is not waiting for a match decision.",
            ));
        }
        let source = match workout.source_garmin_activity_id {
            Some(id) => db::get_activity(&conn, id)?,
            None => None,
        };
        if strategy == HevyStrategy::Replace && source.is_some_and(|a| a.is_published()) {
            return Err(ApiError::conflict(
                "Replace is unavailable because this activity is already on Strava. \
                 Choose Merge or Description only.",
            ));
        }
    }
    let scheduler = &state.match_scheduler;
    let scheduled = scheduler
        .schedule(&hevy_id, strategy.as_str())
        .map_err(|err| match err {
            MatchError::AlreadyScheduled => {
                ApiError::conflict("This workout is already being matched.")
            }
            MatchError::Db(err) => err.into(),
        })?;
    Ok(Json(HevyMatchScheduled {
        deadline: schedule::deadline_iso(scheduled.deadline),
        strategy,
        remaining_ms: ((scheduled.deadline - scheduler.now()) * 1000.0)
            .round()
            .max(0.0) as i64,
        undo_window_ms: (scheduled.window_seconds * 1000.0).round() as i64,
    }))
}

#[derive(Serialize)]
pub struct HevyMatchCancelled {
    cancelled: bool,
}

pub async fn cancel_match(
    State(state): State<AppState>,
    Path(hevy_id): Path<String>,
) -> ApiResult<HevyMatchCancelled> {
    let conn = state.db.lock();
    if state.match_scheduler.cancel(&conn, &hevy_id)? > 0 {
        return Ok(Json(HevyMatchCancelled { cancelled: true }));
    }
    if hevy_match::get(&conn, &hevy_id)?.is_some_and(|d| d.state == hevy_match::APPLYING) {
        return Err(ApiError::conflict(
            "Too late to undo: Garmin is already being updated.",
        ));
    }
    Err(ApiError::new(
        StatusCode::NOT_FOUND,
        "No Hevy match is waiting to be undone.",
    ))
}

#[derive(Serialize)]
pub struct HevyMatchStatus {
    state: String,
    message: Option<String>,
}

pub async fn match_status(
    State(state): State<AppState>,
    Path(hevy_id): Path<String>,
) -> ApiResult<HevyMatchStatus> {
    let decision = hevy_match::get(&state.db.lock(), &hevy_id)?.ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "No Hevy match decision for that workout.",
        )
    })?;
    Ok(Json(HevyMatchStatus {
        state: decision.state,
        message: decision.message,
    }))
}

#[derive(Serialize)]
pub struct HevyQueueActionResult {
    message: String,
}

/// Runs a queue action under the workout's execution lease, so it cannot
/// interleave with a sync pass working on the same workout.
fn with_lease(
    state: &AppState,
    hevy_id: &str,
    mutate: impl FnOnce(&Connection, &Workout) -> Result<String, ApiError>,
) -> ApiResult<HevyQueueActionResult> {
    let result = {
        let conn = state.db.lock();
        let workout = require_workout(&conn, hevy_id)?;
        let Some(token) = hevy_db::acquire_lease(&conn, hevy_id, Utc::now())? else {
            return Err(ApiError::conflict(
                "This workout is being processed. Try again in a moment.",
            ));
        };
        let result = mutate(&conn, &workout);
        hevy_db::release_lease(&conn, hevy_id, &token)?;
        result?
    };
    state.events.publish("refresh");
    Ok(Json(HevyQueueActionResult { message: result }))
}

pub async fn retry(
    State(state): State<AppState>,
    Path(hevy_id): Path<String>,
) -> ApiResult<HevyQueueActionResult> {
    with_lease(&state, &hevy_id, |conn, row| {
        if !PROBLEMS.contains(&row.status.as_str()) {
            return Err(ApiError::conflict(format!(
                "Nothing to retry — workout is {}.",
                row.status
            )));
        }
        match hevy_db::get_open_operation(conn, &row.hevy_id)? {
            Some(operation) => {
                if operation.phase == "needs_review" {
                    let resume = if operation.target_activity_id.is_some_and(|id| id != 0) {
                        "finalizing"
                    } else if operation.next_step.as_deref() == Some("resolve")
                        || operation
                            .upload_id
                            .as_deref()
                            .is_some_and(|u| !u.is_empty())
                    {
                        "submission_unknown"
                    } else {
                        "preparing"
                    };
                    hevy_db::resume_operation(conn, operation.id, resume)?;
                }
                hevy_db::set_workout_status(conn, &row.hevy_id, "syncing", None)?;
            }
            None => hevy_db::set_workout_status(conn, &row.hevy_id, "waiting_watch", None)?,
        }
        Ok(format!("Retry queued for {}.", row.display_title()))
    })
}

pub async fn skip(
    State(state): State<AppState>,
    Path(hevy_id): Path<String>,
) -> ApiResult<HevyQueueActionResult> {
    let scheduler = state.match_scheduler.clone();
    with_lease(&state, &hevy_id, |conn, row| {
        if TERMINAL_STATUSES.contains(&row.status.as_str()) || row.status == "skipped" {
            return Err(ApiError::conflict(format!(
                "Cannot skip a {} workout.",
                row.status
            )));
        }
        if hevy_db::get_open_operation(conn, &row.hevy_id)?.is_some() {
            return Err(ApiError::conflict(
                "This workout has an operation in progress. Resolve or retry it instead of skipping it.",
            ));
        }
        scheduler.cancel(conn, &row.hevy_id)?;
        if hevy_match::is_active(conn, &row.hevy_id)? {
            return Err(ApiError::conflict(
                "Garmin is already being updated for this workout.",
            ));
        }
        if row.status == "awaiting_match" {
            hevy_db::reset_links(conn, &row.hevy_id)?;
        }
        hevy_db::set_workout_status(conn, &row.hevy_id, "skipped", None)?;
        Ok(format!("Skipped {}.", row.display_title()))
    })
}

pub async fn unskip(
    State(state): State<AppState>,
    Path(hevy_id): Path<String>,
) -> ApiResult<HevyQueueActionResult> {
    with_lease(&state, &hevy_id, |conn, row| {
        if row.status != "skipped" {
            return Err(ApiError::conflict(format!(
                "Workout is {}, not skipped.",
                row.status
            )));
        }
        hevy_db::set_workout_status(conn, &row.hevy_id, "waiting_watch", None)?;
        Ok(format!(
            "Restored {} to the sync queue.",
            row.display_title()
        ))
    })
}

pub async fn resync_fresh(
    State(state): State<AppState>,
    Path(hevy_id): Path<String>,
) -> ApiResult<HevyQueueActionResult> {
    with_lease(&state, &hevy_id, |conn, row| {
        if hevy_db::get_open_operation(conn, &row.hevy_id)?.is_some() {
            return Err(ApiError::conflict(
                "This workout has an operation in progress. Resolve it before you re-sync it fresh.",
            ));
        }
        let deleted = row
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("deleted on Garmin");
        if row.status != "needs_review" || !deleted {
            return Err(ApiError::conflict(
                "Re-sync as fresh is only available when the linked activity was deleted on Garmin.",
            ));
        }
        hevy_db::reset_links(conn, &row.hevy_id)?;
        hevy_db::set_workout_status(conn, &row.hevy_id, "waiting_watch", None)?;
        Ok(format!("Fresh sync queued for {}.", row.display_title()))
    })
}
