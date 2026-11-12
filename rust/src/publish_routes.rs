use std::collections::{BTreeSet, HashMap, HashSet};

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::db::{self, ActivityRow};
use crate::routes::{ActionResult, ActivityId, ApiError, ApiResult, internal, is_publishable, require_activity, require_publish_connections};
use crate::schedule::{self, ScheduleError};
use crate::strava_client::StravaError;
use crate::sync::{self, Services, SyncError};
use crate::{AppState, mock};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishScheduled {
    batch_id: String,
    publish_scheduled_at: String,
    remaining_ms: i64,
    undo_window_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityIdsRequest {
    activity_ids: Vec<i64>,
}

fn unique_ids(payload: ActivityIdsRequest) -> Result<Vec<i64>, ApiError> {
    if payload.activity_ids.is_empty() {
        return Err(ApiError::invalid("activityIds must list at least one activity"));
    }
    Ok(payload.activity_ids.into_iter().collect::<BTreeSet<_>>().into_iter().collect())
}

fn schedule_publish(state: &AppState, ids: &[i64], kind: &str) -> ApiResult<PublishScheduled> {
    let activities = publishable_activities(state, ids, kind == "single")?;
    let batch = state.scheduler.schedule(&activities, kind).map_err(|err| match err {
        ScheduleError::AlreadyScheduled => {
            ApiError::conflict("Already publishing to Strava. Refresh to see where it stands.")
        }
        ScheduleError::Db(err) => err.into(),
    })?;
    let remaining = batch.deadline - state.scheduler.now();
    Ok(Json(PublishScheduled {
        publish_scheduled_at: schedule::deadline_iso(batch.deadline),
        batch_id: batch.batch_id,
        remaining_ms: (remaining * 1000.0).round().max(0.0) as i64,
        undo_window_ms: (batch.window_seconds * 1000.0).round() as i64,
    }))
}

pub async fn schedule_publish_activity(
    State(state): State<AppState>,
    ActivityId(id): ActivityId,
) -> ApiResult<PublishScheduled> {
    schedule_publish(&state, &[id], "single")
}

pub async fn schedule_publish_activities(
    State(state): State<AppState>,
    payload: Result<Json<ActivityIdsRequest>, JsonRejection>,
) -> ApiResult<PublishScheduled> {
    let ids = unique_ids(payload?.0)?;
    schedule_publish(&state, &ids, "bulk")
}

fn publishable_activities(state: &AppState, ids: &[i64], single: bool) -> Result<Vec<ActivityRow>, ApiError> {
    let conn = state.db.lock();
    let activities = ids.iter().map(|id| require_activity(&conn, *id)).collect::<Result<Vec<_>, _>>()?;
    if !activities.iter().all(is_publishable) {
        return Err(ApiError::conflict(if single {
            "Only pending, held, or missing activities can be published"
        } else {
            "Some selected activities are no longer publishable. Refresh and try again."
        }));
    }
    require_publish_connections(&conn)?;
    Ok(activities)
}

pub async fn publish_activity(State(state): State<AppState>, ActivityId(id): ActivityId) -> ApiResult<ActionResult> {
    let activity = publishable_activities(&state, &[id], true)?.remove(0);
    let sent = match state.clients.clone() {
        Some(clients) => match clients.garmin().await {
            Ok(garmin) => {
                let strava = clients.strava();
                sync::publish_now(&Services { db: &state.db, garmin: &garmin, strava: &strava }, id, Utc::now()).await
            }
            Err(err) => Err(err.into()),
        },
        None => mock::publish_now(&state.db, id, Utc::now()),
    };
    sent.map_err(|err| match err {
        SyncError::Blocked(reason) => ApiError::conflict(format!("Activity was blocked by Hevy sync: {reason}.")),
        SyncError::Strava(StravaError::Auth(message)) => ApiError::conflict(message),
        err => ApiError::new(StatusCode::BAD_GATEWAY, format!("Could not publish activity: {err}")),
    })?;
    state.events.publish("refresh");
    let verb = if activity.publish_status == "missing" { "Republished" } else { "Published" };
    Ok(Json(ActionResult::success(format!("{verb} {} to Strava", activity.title))))
}

pub async fn publish_activities(
    State(state): State<AppState>,
    payload: Result<Json<ActivityIdsRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let ids = unique_ids(payload?.0)?;
    publishable_activities(&state, &ids, false)?;
    let selected: HashSet<i64> = ids.into_iter().collect();
    let stats = match state.clients.clone() {
        Some(clients) => {
            let garmin = clients.garmin().await.map_err(|err| internal("publish", err))?;
            let strava = clients.strava();
            sync::publish_pending(&Services { db: &state.db, garmin: &garmin, strava: &strava }, Utc::now(), Some(&selected)).await
        }
        None => mock::publish_selected(&state.db, &selected, Utc::now()),
    }
    .map_err(|err| match err {
        SyncError::Strava(StravaError::Auth(message)) => ApiError::conflict(message),
        err => internal("publish", err),
    })?;
    state.events.publish("refresh");
    if stats.failed + stats.blocked > 0 {
        let mut parts = vec![format!("Published {}", stats.published)];
        if stats.failed > 0 {
            parts.push(format!("{} failed and stayed ready to retry", stats.failed));
        }
        if stats.blocked > 0 {
            parts.push(format!("{} blocked by Hevy sync", stats.blocked));
        }
        return Ok(Json(ActionResult {
            message: parts.join("; "),
            severity: "warning",
            published_count: stats.published,
            failed_count: stats.failed,
            blocked_count: stats.blocked,
        }));
    }
    let noun = if stats.published == 1 { "activity" } else { "activities" };
    Ok(Json(ActionResult {
        published_count: stats.published,
        ..ActionResult::success(format!("Published {} {noun} to Strava", stats.published))
    }))
}

fn cancel_publish(state: &AppState, ids: &[i64]) -> ApiResult<ActionResult> {
    if state.scheduler.cancel(ids)? > 0 {
        return Ok(Json(ActionResult::success("Nothing was sent to Strava.")));
    }
    let conn = state.db.lock();
    for id in ids {
        if schedule::get(&conn, *id)?.is_some() {
            return Err(ApiError::conflict("Too late to undo: it is already being sent to Strava."));
        }
    }
    Err(ApiError::conflict("Nothing is waiting to publish."))
}

pub async fn cancel_publish_activity(State(state): State<AppState>, ActivityId(id): ActivityId) -> ApiResult<ActionResult> {
    cancel_publish(&state, &[id])
}

pub async fn cancel_publish_activities(
    State(state): State<AppState>,
    payload: Result<Json<ActivityIdsRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let ids = unique_ids(payload?.0)?;
    cancel_publish(&state, &ids)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishBatch {
    batch_id: String,
    state: &'static str,
    remaining_ms: i64,
    result: Option<ActionResult>,
    error: Option<String>,
}

/// The same toast an immediate publish would return, built from what the
/// scheduler recorded.
fn batch_outcome(rows: &[schedule::ScheduleRow], titles: &HashMap<i64, String>) -> (Option<ActionResult>, Option<String>) {
    if rows[0].batch_kind == "single" {
        let row = &rows[0];
        let title = titles.get(&row.garmin_activity_id).map_or("Activity", String::as_str);
        return match row.state.as_str() {
            schedule::PUBLISHED => {
                let verb = if row.prior_status == "missing" { "Republished" } else { "Published" };
                (Some(ActionResult::success(format!("{verb} {title} to Strava"))), None)
            }
            schedule::SKIPPED => (None, Some(format!("{title} was no longer publishable, so nothing was sent."))),
            _ => (None, Some(row.message.clone().unwrap_or_else(|| "Could not publish activity".into()))),
        };
    }
    let count = |state: &str| rows.iter().filter(|r| r.state == state).count() as i64;
    let (published, failed, blocked, skipped) =
        (count(schedule::PUBLISHED), count(schedule::FAILED), count(schedule::BLOCKED), count(schedule::SKIPPED));
    if failed + blocked + skipped > 0 {
        let mut parts = vec![format!("Published {published}")];
        if failed > 0 {
            parts.push(format!("{failed} failed and stayed ready to retry"));
        }
        if blocked > 0 {
            parts.push(format!("{blocked} blocked by Hevy sync"));
        }
        if skipped > 0 {
            parts.push(format!("{skipped} no longer publishable"));
        }
        return (
            Some(ActionResult {
                message: parts.join("; "),
                severity: "warning",
                published_count: published,
                failed_count: failed,
                blocked_count: blocked,
            }),
            None,
        );
    }
    let noun = if published == 1 { "activity" } else { "activities" };
    (
        Some(ActionResult {
            published_count: published,
            ..ActionResult::success(format!("Published {published} {noun} to Strava"))
        }),
        None,
    )
}

pub async fn publish_batch(State(state): State<AppState>, Path(batch_id): Path<String>) -> ApiResult<PublishBatch> {
    let conn = state.db.lock();
    let rows = schedule::batch_rows(&conn, &batch_id)?;
    if rows.is_empty() {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "Nothing is publishing under that id."));
    }
    let remaining_ms = ((rows[0].deadline - state.scheduler.now()) * 1000.0).round().max(0.0) as i64;
    let pending = |s: &str| rows.iter().any(|r| r.state == s);
    let (batch_state, remaining_ms) = if pending(schedule::WAITING) {
        ("waiting", remaining_ms)
    } else if pending(schedule::SENDING) {
        ("sending", 0)
    } else {
        ("done", 0)
    };
    let (result, error) = if batch_state == "done" {
        let mut titles = HashMap::new();
        for row in &rows {
            if let Some(activity) = db::get_activity(&conn, row.garmin_activity_id)? {
                titles.insert(row.garmin_activity_id, activity.title);
            }
        }
        batch_outcome(&rows, &titles)
    } else {
        (None, None)
    };
    Ok(Json(PublishBatch { batch_id, state: batch_state, remaining_ms, result, error }))
}
