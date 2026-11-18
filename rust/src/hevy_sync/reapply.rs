use chrono::{DateTime, Utc};

use super::{StepError, apply_mapping_gate, log_transition};
use crate::db;
use crate::garmin_client::GarminError;
use crate::hevy_apply::{
    Apply, ApplyError, activity_metrics, exercise_sets_for, resolve_exercises,
};
use crate::hevy_client::HevyClient;
use crate::hevy_db::{self, APPLIED_STATUSES, Workout};
use crate::hevy_description::{generate_description, generate_title};
use crate::poller::HevyLegError;
use crate::strava_client::StravaClient;

fn applied_strategy(row: &Workout) -> &str {
    row.applied_strategy.as_deref().unwrap_or("merge")
}

fn writes_summary(apply: &Apply<'_>, strategy: &str) -> bool {
    matches!(strategy, "describe" | "passive") || apply.summary_on_structured()
}

async fn redo(apply: &Apply<'_>, hevy: &HevyClient, row: &Workout) -> Result<(), StepError> {
    let strategy = applied_strategy(row);
    if !apply_mapping_gate(apply.db, row, strategy, hevy).await? {
        return Ok(());
    }
    let target = row.garmin_activity_id.ok_or_else(|| {
        ApplyError::Invalid(format!("workout {} has no linked activity", row.hevy_id))
    })?;
    if strategy != "describe" {
        let payload = {
            let conn = apply.db.lock();
            let resolved = match resolve_exercises(&conn, &row.payload_json()) {
                Ok(resolved) => resolved,
                Err(ApplyError::MappingMiss { title, .. }) => {
                    let error = format!("unmapped exercises: {title}");
                    hevy_db::set_workout_status(
                        &conn,
                        &row.hevy_id,
                        "needs_mapping",
                        Some(&error),
                    )?;
                    return Ok(());
                }
                Err(err) => return Err(err.into()),
            };
            exercise_sets_for(&conn, &resolved, target, row)?
        };
        apply.garmin.put_exercise_sets(target, &payload).await?;
    }
    apply
        .apply_metadata(target, row, writes_summary(apply, strategy), (None, None))
        .await?;
    hevy_db::set_applied_garmin(
        &apply.db.lock(),
        &row.hevy_id,
        row.source_updated_at.as_deref(),
    )?;
    Ok(())
}

/// A newer Hevy revision of an applied workout, re-applied with the
/// strategy it was first applied with, never the current setting.
pub(super) async fn reapply(
    apply: &Apply<'_>,
    hevy: &HevyClient,
    row: &Workout,
    now: DateTime<Utc>,
) -> rusqlite::Result<()> {
    let db = apply.db;
    let Some(token) = hevy_db::acquire_lease(&db.lock(), &row.hevy_id, now)? else {
        return Ok(());
    };
    let outcome = redo(apply, hevy, row).await;
    let conn = db.lock();
    match outcome {
        Ok(()) => {}
        Err(StepError::Apply(ApplyError::Garmin(GarminError::ActivityGone(_)))) => {
            let error = "linked activity deleted on Garmin — re-sync as fresh upload";
            hevy_db::set_workout_status(&conn, &row.hevy_id, "needs_review", Some(error))?;
        }
        Err(err) => {
            crate::log_warn!(
                "hevy_sync",
                "re-applying hevy workout {} failed: {err}",
                row.hevy_id
            );
            hevy_db::set_workout_status(&conn, &row.hevy_id, &row.status, Some(&err.to_string()))?;
        }
    }
    hevy_db::release_lease(&conn, &row.hevy_id, &token)?;
    log_transition(&conn, &row.hevy_id, &row.status);
    Ok(())
}

/// Pushes edited titles and descriptions to published Strava copies. The
/// Garmin side can succeed while Strava is down, so this retries each tick
/// until Strava has the applied Garmin revision.
pub(super) async fn strava_catchup(
    apply: &Apply<'_>,
    strava: &StravaClient,
) -> Result<bool, HevyLegError> {
    let db = apply.db;
    let mut changed = false;
    for status in APPLIED_STATUSES {
        let rows = hevy_db::list_workouts(&db.lock(), status)?;
        for row in rows {
            let garmin_applied = row.garmin_applied_updated_at.as_deref().unwrap_or_default();
            if garmin_applied.is_empty()
                || garmin_applied <= row.strava_applied_updated_at.as_deref().unwrap_or_default()
            {
                continue;
            }
            let (activity, metrics) = {
                let conn = db.lock();
                let activity = match row.garmin_activity_id.filter(|id| *id != 0) {
                    Some(id) => db::get_activity(&conn, id)?,
                    None => None,
                };
                // Not published yet: the publish carries the current text.
                let Some(activity) =
                    activity.filter(|a| a.strava_activity_id.is_some_and(|id| id != 0))
                else {
                    continue;
                };
                let metrics =
                    activity_metrics(&conn, &row, activity.garmin_activity_id, (None, None))
                        .map_err(|err| HevyLegError::Failed(err.to_string()))?;
                (activity, metrics)
            };
            let payload = row.payload_json();
            let write_summary = writes_summary(apply, applied_strategy(&row));
            let texts =
                generate_title(&payload, apply.template("hevy_title_template")).and_then(|title| {
                    let description = if write_summary {
                        generate_description(
                            &payload,
                            metrics.0,
                            metrics.1,
                            apply.template("hevy_description_template"),
                        )?
                    } else {
                        activity.description.clone()
                    };
                    Ok((title, description))
                });
            let strava_id = activity.strava_activity_id.unwrap_or_default();
            let pushed = match texts {
                Ok((title, description)) => strava
                    .update_activity_metadata(strava_id, &title, &description)
                    .await
                    .map_err(|e| e.to_string()),
                Err(err) => Err(err),
            };
            if let Err(err) = pushed {
                crate::log_warn!(
                    "hevy_sync",
                    "strava catch-up failed for {}: {err}",
                    row.hevy_id
                );
                continue;
            }
            hevy_db::set_applied_strava(&db.lock(), &row.hevy_id, garmin_applied)?;
            changed = true;
        }
    }
    Ok(changed)
}
