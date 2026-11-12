//! The replace and passive strategies: a journalled FIT upload, then the
//! watch original's deletion.

use std::collections::BTreeSet;
use std::path::PathBuf;

use rusqlite::Connection;
use serde_json::{Map, Value, json};

use super::{
    Apply, ApplyError, activity_duration_seconds, float, resolve_exercises, seconds, total_seconds,
    workout_window,
};
use crate::auth_core::random_token;
use crate::config::Config;
use crate::db;
use crate::fit_builder::{self, DeviceIdentity};
use crate::garmin_client::GarminError;
use crate::hevy_db::{self, OpField, Operation, Workout};
use crate::hr_sources;
use crate::timefmt::parse_timestamp;

/// The poller rechecks an unresolved upload every tick (about a minute), so
/// this is roughly how long Garmin gets to finish importing before parking.
const RESOLVE_MAX_CHECKS: i64 = 15;
const RESOLUTION_DRIFT_S: f64 = 10.0 * 60.0;
const RESOLUTION_TYPES: [&str; 2] = ["strength_training", "other"];

fn persist_identity(conn: &Connection, identity: DeviceIdentity) -> Result<(), ApplyError> {
    let mut stored = match db::get_config_value(conn, "settings")? {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    stored.insert(
        "hevy_device_identity".into(),
        json!({"manufacturer": identity.manufacturer, "product": identity.product, "serial": identity.serial}),
    );
    db::set_config_value(conn, "settings", &Value::Object(stored))?;
    Ok(())
}

/// The user's override, then the detected or stored identity, else a fresh
/// per-install fallback that is saved so it stays stable.
fn identity_for_build(conn: &Connection, cfg: &Config) -> Result<DeviceIdentity, ApplyError> {
    let settings = db::get_config_value(conn, "settings")?.unwrap_or(Value::Null);
    let present = |v: Option<&Value>| v.filter(|v| db::truthy(Some(v))).cloned();
    let stored = present(settings.get("hevy_device_identity_override"))
        .or_else(|| present(settings.get("hevy_device_identity")))
        .or_else(|| present(cfg.get("hevy_device_identity")));
    if let Some(stored) = stored {
        return Ok(fit_builder::identity_from_config(Some(&stored)));
    }
    let identity = fit_builder::new_fallback_identity();
    persist_identity(conn, identity)?;
    Ok(identity)
}

/// The passive path's guard: a blind upload would duplicate a session
/// recorded under another type.
fn overlapping_any_type(conn: &Connection, row: &Workout) -> Result<Option<String>, ApplyError> {
    let Some((start, end)) = workout_window(row) else {
        return Ok(None);
    };
    for activity in db::list_activities(conn, false)? {
        let Some(act_start) = parse_timestamp(&activity.start_time) else {
            continue;
        };
        let act_end = act_start + seconds(activity_duration_seconds(&activity));
        if act_start < end && act_end > start {
            return Ok(Some(activity.activity_type));
        }
    }
    Ok(None)
}

/// Strict matching for an upload whose outcome is unknown: only a new
/// strength or other activity starting near the workout with a plausible
/// duration may be adopted. A new run in the window must never be renamed
/// and have the watch activity deleted under it.
fn resolution_candidates(
    activities: &[Value],
    row: &Workout,
    pre_upload_ids: &[i64],
    source: Option<i64>,
) -> BTreeSet<i64> {
    let Some((hevy_start, hevy_end)) = workout_window(row) else {
        return BTreeSet::new();
    };
    let hevy_duration = total_seconds(hevy_end - hevy_start);
    if hevy_duration <= 0.0 {
        return BTreeSet::new();
    }
    let mut candidates = BTreeSet::new();
    for act in activities {
        let Some(activity_id) = act.get("activityId").and_then(hevy_db::id_of) else {
            continue;
        };
        if pre_upload_ids.contains(&activity_id) || Some(activity_id) == source {
            continue;
        }
        let kind = act
            .get("activityType")
            .and_then(|t| t.get("typeKey"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !RESOLUTION_TYPES.contains(&kind) {
            continue;
        }
        let Some(act_start) = act
            .get("startTimeGMT")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
        else {
            continue;
        };
        if total_seconds(act_start - hevy_start).abs() > RESOLUTION_DRIFT_S {
            continue;
        }
        let Some(duration) = float(act.get("duration")).filter(|d| *d > 0.0) else {
            continue;
        };
        let ratio = duration / hevy_duration;
        if (0.25..=4.0).contains(&ratio) {
            candidates.insert(activity_id);
        }
    }
    candidates
}

struct TempFit(PathBuf);

impl TempFit {
    fn write(hevy_id: &str, bytes: &[u8]) -> std::io::Result<Self> {
        let dir = std::env::temp_dir().join(format!("activsync-fit-{}", random_token(6)));
        std::fs::create_dir(&dir)?;
        let fit = Self(dir);
        std::fs::write(fit.path(hevy_id), bytes)?;
        Ok(fit)
    }

    fn path(&self, hevy_id: &str) -> PathBuf {
        self.0.join(format!("hevy_{hevy_id}.fit"))
    }
}

impl Drop for TempFit {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Apply<'_> {
    pub async fn execute_replace(&self, row: &Workout) -> Result<(), ApplyError> {
        self.open_and_advance(row, "replace", row.source_garmin_activity_id)
            .await
    }

    pub async fn execute_passive(&self, row: &Workout) -> Result<(), ApplyError> {
        self.open_and_advance(row, "upload_passive", None).await
    }

    async fn open_and_advance(
        &self,
        row: &Workout,
        kind: &str,
        source: Option<i64>,
    ) -> Result<(), ApplyError> {
        {
            let conn = self.db.lock();
            hevy_db::set_workout_status(&conn, &row.hevy_id, "syncing", None)?;
            if hevy_db::get_open_operation(&conn, &row.hevy_id)?.is_none()
                && hevy_db::open_operation(&conn, &row.hevy_id, kind, source, &[])?.is_none()
            {
                // Another open operation owns this workout or its source.
                return Ok(());
            }
        }
        self.advance_operation(row).await
    }

    fn close(
        &self,
        op: &Operation,
        row: &Workout,
        phase: &str,
        status: &str,
        error: &str,
    ) -> Result<(), ApplyError> {
        hevy_db::set_operation_outcome(
            &self.db.lock(),
            op.id,
            &row.hevy_id,
            phase,
            status,
            Some(error),
        )?;
        Ok(())
    }

    fn park(&self, op: &Operation, row: &Workout, error: &str) -> Result<(), ApplyError> {
        self.close(op, row, "needs_review", "needs_review", error)
    }

    fn update(&self, op: &Operation, fields: &[OpField]) -> Result<(), ApplyError> {
        hevy_db::update_operation(&self.db.lock(), op.id, fields)?;
        Ok(())
    }

    /// Every transition is saved before the next side effect, so a crash
    /// resumes where it stopped. `submission_unknown` is entered only from the
    /// upload call itself and is never resubmitted.
    pub async fn advance_operation(&self, row: &Workout) -> Result<(), ApplyError> {
        loop {
            let Some(op) = hevy_db::get_open_operation(&self.db.lock(), &row.hevy_id)? else {
                return Ok(());
            };
            let moved_on = match op.phase.as_str() {
                "preparing" => self.prepare(&op, row).await?,
                "uploading" => self.upload(&op, row).await?,
                "submission_unknown" => self.resolve(&op, row).await?,
                "finalizing" => return self.finalize(&op, row).await,
                _ => return Ok(()),
            };
            if !moved_on {
                return Ok(());
            }
        }
    }

    /// Returns whether the operation moved on to the next phase.
    async fn prepare(&self, op: &Operation, row: &Workout) -> Result<bool, ApplyError> {
        if op.kind == "replace" {
            let source = op
                .source_activity_id
                .ok_or_else(|| ApplyError::Invalid("replace without a source".into()))?;
            let fit = self.garmin.download_fit(source).await?;
            let existing = self.garmin.get_exercise_sets(source).await?;
            let conn = self.db.lock();
            hevy_db::save_backup(&conn, source, &row.hevy_id, &existing, Some(&fit))?;
            let settings = db::get_config_value(&conn, "settings")?.unwrap_or(Value::Null);
            let stored = settings
                .get("hevy_device_identity")
                .filter(|v| db::truthy(Some(v)));
            // Product 0 is the generic no-watch fallback: upgrade it as soon
            // as a real watch FIT makes per-user detection possible.
            let generic = stored.is_none_or(|s| {
                s.get("product")
                    .is_some_and(|p| p.as_f64() == Some(0.0) || p == "0")
            });
            if generic && let Some(detected) = fit_builder::identity_from_fit(&fit) {
                persist_identity(&conn, detected)?;
            }
            let published = db::get_activity(&conn, source)?
                .is_some_and(|a| a.strava_activity_id.is_some_and(|id| id != 0));
            drop(conn);
            if published {
                self.park(op, row, "source activity already published to Strava")?;
                return Ok(false);
            }
        } else {
            let wrong_type = overlapping_any_type(&self.db.lock(), row)?;
            if let Some(kind) = wrong_type.filter(|k| !k.is_empty()) {
                self.park(
                    op,
                    row,
                    &format!(
                        "overlapping {kind} activity exists — passive upload would duplicate it"
                    ),
                )?;
                return Ok(false);
            }
        }
        let snapshot = self.garmin.list_activity_ids_near(&row.start_time).await?;
        self.update(
            op,
            &[OpField::Phase("uploading"), OpField::PreUploadIds(snapshot)],
        )?;
        Ok(true)
    }

    /// Everything before the upload call is pre-submission: a failure there
    /// closes the operation rather than wedging it in `uploading`.
    async fn build(&self, op: &Operation, row: &Workout) -> Result<Vec<u8>, ApplyError> {
        let payload = row.payload_json();
        let resolved = resolve_exercises(&self.db.lock(), &payload)?;
        let window = workout_window(row);
        let hr = if op.kind == "replace" {
            let fit = match op.source_activity_id {
                Some(source) => hevy_db::backup_fit(&self.db.lock(), source)?,
                None => None,
            };
            match (fit, window) {
                (Some(fit), Some((start, _))) if !fit.is_empty() => {
                    let start_ms =
                        (start.timestamp_micros() as f64 / 1e6 * 1000.0).round_ties_even() as i64;
                    hr_sources::extract_fit_hr(&fit, start_ms)
                }
                _ => Vec::new(),
            }
        } else {
            match window {
                Some((start, end)) => {
                    hr_sources::passive_hr_for_window(self.garmin, start, end).await
                }
                None => Vec::new(),
            }
        };
        let identity = identity_for_build(&self.db.lock(), self.cfg)?;
        let (fit, summary) =
            fit_builder::build_fit(&payload, &resolved, &hr, self.profile, identity)
                .map_err(ApplyError::Invalid)?;
        self.update(
            op,
            &[
                OpField::GeneratedCalories(Some(summary.calories as f64)),
                OpField::GeneratedAvgHr(summary.avg_hr.map(|hr| hr as f64)),
            ],
        )?;
        Ok(fit)
    }

    async fn upload(&self, op: &Operation, row: &Workout) -> Result<bool, ApplyError> {
        let fit = match self.build(op, row).await {
            Ok(fit) => fit,
            Err(ApplyError::MappingMiss { title, .. }) => {
                let error = format!("unmapped exercises: {title}");
                self.close(op, row, "failed", "needs_mapping", &error)?;
                return Ok(false);
            }
            Err(err) => {
                let error = format!("FIT build failed: {err}");
                self.close(op, row, "failed", "failed", &error)?;
                return Ok(false);
            }
        };
        let uploaded = match TempFit::write(&row.hevy_id, &fit) {
            Ok(file) => self.garmin.upload_fit(&file.path(&row.hevy_id)).await,
            Err(err) => {
                let error = format!("FIT build failed: {err}");
                self.close(op, row, "failed", "failed", &error)?;
                return Ok(false);
            }
        };
        let result = match uploaded {
            Ok(result) => result,
            Err(GarminError::UploadRejected(message)) => {
                self.close(op, row, "failed", "failed", &message)?;
                return Ok(false);
            }
            Err(err) => {
                // The outcome is unknown: record it and wait, never resubmit.
                self.update(
                    op,
                    &[
                        OpField::Phase("submission_unknown"),
                        OpField::NextStep("resolve"),
                        OpField::LastError(err.to_string()),
                    ],
                )?;
                return Ok(false);
            }
        };
        let upload_id = result
            .upload_id
            .as_ref()
            .map(|id| id.as_str().map_or_else(|| id.to_string(), str::to_string));
        match result.activity_id.filter(|id| {
            *id != 0 && !op.pre_upload_ids.contains(id) && Some(*id) != op.source_activity_id
        }) {
            Some(new_id) => {
                self.update(
                    op,
                    &[
                        OpField::Phase("finalizing"),
                        OpField::NextStep("metadata"),
                        OpField::TargetActivityId(new_id),
                        OpField::UploadId(upload_id),
                    ],
                )?;
                Ok(true)
            }
            None => {
                // Accepted but not yet resolvable: let Garmin finish.
                self.update(
                    op,
                    &[
                        OpField::Phase("submission_unknown"),
                        OpField::NextStep("resolve"),
                        OpField::UploadId(upload_id),
                    ],
                )?;
                Ok(false)
            }
        }
    }

    async fn resolve(&self, op: &Operation, row: &Workout) -> Result<bool, ApplyError> {
        let activities = self.garmin.list_activities_near(&row.start_time).await?;
        let candidates =
            resolution_candidates(&activities, row, &op.pre_upload_ids, op.source_activity_id);
        if candidates.len() == 1 {
            let target = *candidates.first().expect("one candidate");
            self.update(
                op,
                &[
                    OpField::Phase("finalizing"),
                    OpField::NextStep("metadata"),
                    OpField::TargetActivityId(target),
                ],
            )?;
            return Ok(true);
        }
        if candidates.len() > 1 {
            let listed: Vec<String> = candidates.iter().map(i64::to_string).collect();
            self.park(
                op,
                row,
                &format!(
                    "ambiguous upload result: candidates [{}]",
                    listed.join(", ")
                ),
            )?;
            return Ok(false);
        }
        let attempts = op.attempt_count + 1;
        if attempts >= RESOLVE_MAX_CHECKS {
            self.park(
                op,
                row,
                &format!("upload outcome unresolved after {RESOLVE_MAX_CHECKS} checks"),
            )?;
            return Ok(false);
        }
        self.update(op, &[OpField::AttemptCount(attempts)])?;
        Ok(false)
    }

    /// metadata → delete → finalize, each step saved so a crash resumes at it.
    async fn finalize(&self, op: &Operation, row: &Workout) -> Result<(), ApplyError> {
        let target = op
            .target_activity_id
            .ok_or_else(|| ApplyError::Invalid("finalizing without a target".into()))?;
        let replace = op.kind == "replace";
        let mut next_step = op.next_step.as_deref().unwrap_or("metadata");
        if next_step == "metadata" {
            let write_summary = !replace || self.summary_on_structured();
            self.apply_metadata(
                target,
                row,
                write_summary,
                (op.generated_calories, op.generated_avg_hr),
            )
            .await?;
            self.update(op, &[OpField::NextStep("delete")])?;
            next_step = "delete";
        }
        if next_step == "delete" {
            if let Some(source) = op
                .source_activity_id
                .filter(|s| replace && *s != 0 && *s != target)
            {
                match self.garmin.delete_activity(source).await {
                    // A 404 is the success signal: a crash after a delete replays here.
                    Ok(()) | Err(GarminError::NotFound(_) | GarminError::ActivityGone(_)) => {}
                    Err(err) => {
                        let deletes = op.delete_attempt_count + 1;
                        if deletes >= 3 {
                            return self.park(
                                op,
                                row,
                                &format!("could not delete watch activity {source}: {err}"),
                            );
                        }
                        return self.update(
                            op,
                            &[
                                OpField::DeleteAttemptCount(deletes),
                                OpField::LastError(err.to_string()),
                            ],
                        );
                    }
                }
            }
            self.update(op, &[OpField::NextStep("finalize")])?;
        }
        let (strategy, status) = if replace {
            ("replace", "replaced")
        } else {
            ("passive", "uploaded_passive")
        };
        hevy_db::complete_operation(
            &self.db.lock(),
            op.id,
            &row.hevy_id,
            target,
            strategy,
            status,
            row.source_updated_at.as_deref(),
        )?;
        Ok(())
    }
}
