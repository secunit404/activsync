use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use super::{PublishStats, Services, SyncError, SyncResult, as_python_float, parse_start};
use crate::config;
use crate::db::{self, ActivityRow, Db};
use crate::strava_client::{self, StravaError};

/// A Hevy-eligible activity waits until a Hevy poll has run late enough
/// after it ended to have seen the workout, so it is not published bare and
/// then replaced.
fn under_hevy_settlement_hold(conn: &rusqlite::Connection, row: &ActivityRow) -> SyncResult<bool> {
    let cfg = config::load_config(conn, false)?;
    if !db::truthy(cfg.get("hevy_enabled")) {
        return Ok(false);
    }
    if row.activity_type != "strength_training" && row.activity_type != "other" {
        return Ok(false);
    }
    if db::has_linked_workout(conn, row.garmin_activity_id)? {
        return Ok(false);
    }
    let Ok(garmin_data) = serde_json::from_str::<Value>(&row.garmin_data) else {
        return Ok(true);
    };
    let duration = garmin_data
        .get("duration")
        .filter(|d| d.is_number())
        .and_then(Value::as_f64);
    let Some(duration) = duration.filter(|d| *d >= 0.0) else {
        return Ok(true);
    };
    let default_interval = config::defaults()["hevy_poll_interval_minutes"]
        .as_f64()
        .unwrap_or(5.0);
    let interval = cfg
        .get("hevy_poll_interval_minutes")
        .and_then(as_python_float)
        .unwrap_or(default_interval);
    let end = parse_start(row)? + Duration::milliseconds((duration * 1000.0) as i64);
    let release_after = end + Duration::milliseconds(((interval.max(0.0) + 5.0) * 60_000.0) as i64);
    let last_success = db::get_config_value(conn, "hevy_last_success_at")?
        .and_then(|v| v.as_str().and_then(crate::timefmt::parse_timestamp));
    Ok(last_success.is_none_or(|at| at < release_after))
}

/// Two callers uploading the same activity at once would both miss the
/// other in Strava's search (the first is still processing) and post a
/// duplicate, so one activity publishes at a time.
static PUBLISH_LOCKS: LazyLock<Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(Mutex::default);

fn publish_lock(garmin_activity_id: i64) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = PUBLISH_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(garmin_activity_id).or_default().clone()
}

async fn publish_row(
    services: &Services<'_>,
    garmin_activity_id: i64,
    now: DateTime<Utc>,
    manual: bool,
) -> SyncResult<()> {
    let lock = publish_lock(garmin_activity_id);
    let _guard = lock.lock().await;
    let row = {
        let conn = services.db.lock();
        if let Some((hevy_id, status)) =
            db::blocking_workout_for_activity(&conn, garmin_activity_id)?
        {
            return Err(SyncError::Blocked(format!(
                "hevy workout {hevy_id} is {status}"
            )));
        }
        let row = db::get_activity(&conn, garmin_activity_id)?.ok_or_else(|| {
            SyncError::Invalid(format!("Activity {garmin_activity_id} was not found"))
        })?;
        if row.is_published() {
            return Ok(());
        }
        if !manual && under_hevy_settlement_hold(&conn, &row)? {
            return Err(SyncError::Blocked("settlement hold".into()));
        }
        row
    };
    let start = parse_start(&row)?;
    let existing = services
        .strava
        .find_existing_activity(start, strava_client::DEFAULT_MATCH_TOLERANCE_MINUTES)
        .await?;
    if let Some(strava_id) = existing {
        db::set_published(&services.db.lock(), garmin_activity_id, strava_id, now)?;
        return Ok(());
    }

    let fit = services.garmin.download_fit(garmin_activity_id).await?;
    let description = Some(row.description.as_str()).filter(|d| !d.is_empty());
    let strength_id = match strength_document(services.db, garmin_activity_id, &fit)? {
        None => None,
        Some(document) => match services
            .strava
            .publish_strength(garmin_activity_id, &document, Some(&row.title), description)
            .await
        {
            Ok(id) => Some(id),
            Err(StravaError::Rejected(reason)) => {
                crate::log_warn!(
                    "sync",
                    "strava upload {garmin_activity_id}: Strava rejected Hevy's sets, uploading the FIT instead: {reason}"
                );
                None
            }
            Err(err) => return Err(err.into()),
        },
    };
    let strava_id = match strength_id {
        Some(id) => id,
        None => {
            services
                .strava
                .publish(garmin_activity_id, &fit, Some(&row.title), description)
                .await?
        }
    };
    db::set_published(&services.db.lock(), garmin_activity_id, strava_id, now)?;
    Ok(())
}

/// Strava's JSON strength upload for a Hevy-linked activity, or `None` to
/// upload the FIT. A "described" workout kept the watch's sets on Garmin, so
/// Strava should match those.
fn strength_document(db: &Db, garmin_activity_id: i64, fit: &[u8]) -> SyncResult<Option<Value>> {
    let conn = db.lock();
    let Some((status, payload)) = db::applied_workout_for_activity(&conn, garmin_activity_id)?
    else {
        return Ok(None);
    };
    if status == "described" {
        return Ok(None);
    }
    let payload = if payload.is_empty() { "{}" } else { &payload };
    let tz = config::load_config(&conn, false)?
        .get("display_timezone")
        .and_then(Value::as_str)
        .map(str::to_string);
    let built = serde_json::from_str::<Value>(payload)
        .map_err(|e| e.to_string())
        .and_then(|workout| {
            crate::strava_strength::build_strength_upload(&conn, &workout, fit, tz.as_deref())
        });
    match built {
        Ok(document) => Ok(Some(document)),
        Err(reason) => {
            crate::log_warn!(
                "sync",
                "strava upload {garmin_activity_id}: uploading the FIT instead of Hevy's sets: {reason}"
            );
            Ok(None)
        }
    }
}

/// The poller (`selected` is `None`) publishes only pending rows: missing
/// and held rows are the user's call. A manual bulk publish names its rows,
/// and every status the dashboard lets the user check is publishable.
pub async fn publish_pending(
    services: &Services<'_>,
    now: DateTime<Utc>,
    selected: Option<&HashSet<i64>>,
) -> SyncResult<PublishStats> {
    let mut stats = PublishStats::default();
    let manual = selected.is_some();
    let statuses: &[&str] = if manual {
        &["pending", "missing", "held"]
    } else {
        &["pending"]
    };
    for status in statuses {
        let rows = db::list_activities_with_status(&services.db.lock(), status)?;
        for row in rows {
            let id = row.garmin_activity_id;
            if selected.is_some_and(|ids| !ids.contains(&id)) {
                continue;
            }
            // Read per row: the owner can press Publish while this pass
            // uploads earlier rows, and a row in its undo window stays unsent.
            if !manual && db::scheduled_publish_ids(&services.db.lock())?.contains(&id) {
                continue;
            }
            match publish_row(services, id, now, manual).await {
                Ok(()) => stats.published += 1,
                Err(SyncError::Blocked(reason)) => {
                    if manual {
                        crate::log_warn!("sync", "publish blocked for activity {id}: {reason}");
                    }
                    stats.blocked += 1;
                    stats.blocked_reasons.push(reason);
                }
                Err(err) => {
                    crate::log_warn!("sync", "publish failed for activity {id}: {err}");
                    stats.failed += 1;
                }
            }
        }
    }
    if stats.published > 0 || stats.failed > 0 || (manual && stats.blocked > 0) {
        crate::log_info!(
            "sync",
            "strava publish: {} published, {} failed, {} blocked",
            stats.published,
            stats.failed,
            stats.blocked
        );
    }
    Ok(stats)
}

/// Publishes one activity at the owner's request: no settlement hold.
pub async fn publish_now(
    services: &Services<'_>,
    garmin_activity_id: i64,
    now: DateTime<Utc>,
) -> SyncResult<()> {
    publish_row(services, garmin_activity_id, now, true).await
}
