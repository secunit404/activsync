//! Applying a Hevy workout to Garmin, as the Python app's `hevy_apply`: the
//! exercise-sets payload, the four strategies, and the crash-safe operation
//! journal that uploads a rebuilt FIT and then deletes the watch original.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

use crate::auth_core::random_token;
use crate::config::Config;
use crate::db::{self, Db};
use crate::exercise_tables::{self, TABLES, UNKNOWN_CATEGORY};
use crate::fit_builder::{
    self, DeviceIdentity, MAX_SCALE, MIN_SCALE, Profile, REST_BETWEEN_EXERCISES_S,
    REST_BETWEEN_SETS_S, ResolvedExercise, WARMUP_SET_S, WORKING_SET_S,
};
use crate::garmin_client::{GarminClient, GarminError};
use crate::hevy_db::{self, OpField, Operation, Workout};
use crate::hevy_description::{generate_description, generate_title};
use crate::hr_sources;
use crate::timefmt::parse_timestamp;

/// The poller rechecks an unresolved upload every tick (about a minute), so
/// this is roughly how long Garmin gets to finish importing before parking.
pub const RESOLVE_MAX_CHECKS: i64 = 15;
const RESOLUTION_DRIFT_S: f64 = 10.0 * 60.0;
const RESOLUTION_TYPES: [&str; 2] = ["strength_training", "other"];
const GONE_ON_GARMIN: &str = "watch activity deleted on Garmin";

#[derive(Debug)]
pub enum ApplyError {
    /// An exercise with no Garmin mapping; routes to the mapping gate.
    MappingMiss {
        title: String,
        template_id: Option<String>,
    },
    Garmin(GarminError),
    Db(rusqlite::Error),
    Invalid(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MappingMiss { title, template_id } => {
                let template = template_id.as_deref().unwrap_or("None");
                write!(
                    f,
                    "no Garmin mapping for exercise '{title}' (template {template})"
                )
            }
            Self::Garmin(err) => err.fmt(f),
            Self::Db(err) => err.fmt(f),
            Self::Invalid(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<GarminError> for ApplyError {
    fn from(err: GarminError) -> Self {
        Self::Garmin(err)
    }
}

impl From<rusqlite::Error> for ApplyError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Db(err)
    }
}

/// Every exercise resolved, or the first unmapped one as a miss.
pub fn resolve_exercises(
    conn: &Connection,
    workout: &Value,
) -> Result<Vec<ResolvedExercise>, ApplyError> {
    let empty = Vec::new();
    let exercises = workout
        .get("exercises")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let mut resolved = Vec::new();
    for exercise in exercises {
        let title = exercise
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let template_id = exercise.get("exercise_template_id").and_then(Value::as_str);
        let Some((category, subcategory)) =
            exercise_tables::lookup_exercise(conn, title, template_id)?
        else {
            return Err(ApplyError::MappingMiss {
                title: title.into(),
                template_id: template_id.map(Into::into),
            });
        };
        let sets = exercise
            .get("sets")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        resolved.push(ResolvedExercise {
            title: title.into(),
            category,
            subcategory,
            sets,
        });
    }
    Ok(resolved)
}

/// Python's `timedelta(seconds=x)`, which rounds to the microsecond.
pub(crate) fn seconds(value: f64) -> Duration {
    Duration::microseconds((value * 1e6).round_ties_even() as i64)
}

/// Python's `round(x, 3)`: the exact decimal rounding of the binary value.
fn round3(value: f64) -> f64 {
    format!("{value:.3}").parse().unwrap_or(value)
}

pub(crate) fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64)
}

/// The exerciseSets PUT payload, the synthetic set timing spread across the
/// real activity window.
pub fn build_exercise_sets_payload(
    resolved: &[ResolvedExercise],
    start: &str,
    duration_s: f64,
) -> Result<Value, String> {
    let act_start =
        parse_timestamp(start).ok_or_else(|| format!("unparseable activity start: '{start}'"))?;
    if let Some(exercise) = resolved.iter().find(|e| e.category == UNKNOWN_CATEGORY) {
        return Err(format!("exercise '{}' resolved to UNKNOWN", exercise.title));
    }

    struct Slot<'a> {
        ex_idx: usize,
        set: &'a Value,
        set_dur: f64,
        rest_dur: f64,
    }
    let mut slots = Vec::new();
    for (ex_idx, exercise) in resolved.iter().enumerate() {
        for (s_idx, set) in exercise.sets.iter().enumerate() {
            let is_warmup = set.get("type").and_then(Value::as_str).unwrap_or("normal") == "warmup";
            let set_dur = match number(set.get("duration_seconds")) {
                Some(explicit) if explicit > 0.0 => explicit,
                _ if is_warmup => WARMUP_SET_S,
                _ => WORKING_SET_S,
            };
            let last_set = s_idx == exercise.sets.len() - 1;
            let rest_dur = match (last_set, ex_idx == resolved.len() - 1) {
                (true, true) => 0.0,
                (true, false) => REST_BETWEEN_EXERCISES_S,
                _ => REST_BETWEEN_SETS_S,
            };
            slots.push(Slot {
                ex_idx,
                set,
                set_dur,
                rest_dur,
            });
        }
    }

    let ideal: f64 = slots.iter().map(|s| s.set_dur + s.rest_dur).sum();
    let scale = if ideal > 0.0 { duration_s / ideal } else { 1.0 };
    let scale = scale.min(MAX_SCALE).max(MIN_SCALE);

    let stamp = |cursor: f64| {
        (act_start + seconds(cursor))
            .format("%Y-%m-%dT%H:%M:%S.0")
            .to_string()
    };
    let mut sets = Vec::new();
    let mut cursor = 0.0;
    for slot in &slots {
        let exercise = &resolved[slot.ex_idx];
        let category = TABLES
            .categories
            .get(&exercise.category)
            .ok_or_else(|| format!("no category name for id {}", exercise.category))?;
        // A null name under a valid category renders as the generic label;
        // an unrecognised name string would render as "Unknown".
        let name = TABLES.subcategory_name(exercise.category, exercise.subcategory);
        let scaled = slot.set_dur * scale;
        let reps = number(slot.set.get("reps")).map_or(0, |r| r.trunc() as i64);
        let weight = number(slot.set.get("weight_kg"))
            .filter(|w| *w != 0.0)
            .map_or(0.0, |kg| (kg * 1000.0).round_ties_even());
        sets.push(json!({
            "exercises": [{"category": category, "name": name, "probability": null}],
            "duration": round3(scaled),
            "repetitionCount": reps,
            "weight": weight,
            "setType": "ACTIVE",
            "startTime": stamp(cursor),
            "wktStepIndex": slot.ex_idx,
            "messageIndex": sets.len(),
        }));
        cursor += scaled;
        if slot.rest_dur > 0.0 {
            let rest = slot.rest_dur * scale;
            sets.push(json!({
                "exercises": [],
                "duration": round3(rest),
                "setType": "REST",
                "startTime": stamp(cursor),
                "wktStepIndex": slot.ex_idx,
                "messageIndex": sets.len(),
            }));
            cursor += rest;
        }
    }
    Ok(json!({"exerciseSets": sets}))
}

/// The duration in an activity's `garmin_data`; 0 when absent or unreadable.
pub(crate) fn activity_duration_seconds(activity: &db::ActivityRow) -> f64 {
    let data: Value = serde_json::from_str(&activity.garmin_data).unwrap_or(Value::Null);
    float(data.get("duration")).unwrap_or(0.0)
}

/// Python's `float()` over a decoded value.
pub(crate) fn float(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

pub(crate) fn workout_window(row: &Workout) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    Some((
        parse_timestamp(&row.start_time)?,
        parse_timestamp(&row.end_time)?,
    ))
}

pub(crate) fn total_seconds(span: Duration) -> f64 {
    span.num_microseconds()
        .map_or(span.num_seconds() as f64, |us| us as f64 / 1e6)
}

/// The linked activity's start and duration, else the workout's own window.
pub(crate) fn activity_window(
    conn: &Connection,
    activity_id: i64,
    row: &Workout,
) -> Result<(String, f64), ApplyError> {
    if let Some(activity) = db::get_activity(conn, activity_id)? {
        let duration = activity_duration_seconds(&activity);
        if duration > 0.0 {
            return Ok((activity.start_time, duration));
        }
    }
    let duration = workout_window(row).map_or(0.0, |(start, end)| total_seconds(end - start));
    Ok((row.start_time.clone(), duration))
}

/// Garmin's recorded calories and heart rate (the target, then the watch
/// original), then what the rebuilt FIT carries. A replace target is not in
/// the local table until polling sees it, so the original is the source.
pub(crate) fn activity_metrics(
    conn: &Connection,
    row: &Workout,
    target: i64,
    generated: (Option<f64>, Option<f64>),
) -> Result<(Option<i64>, Option<i64>), ApplyError> {
    let rounded = |value: Option<f64>| {
        value
            .filter(|v| v.is_finite())
            .map(|v| v.round_ties_even() as i64)
    };
    let (mut calories, mut avg_hr) = (None, None);
    for activity_id in [Some(target), row.source_garmin_activity_id]
        .into_iter()
        .flatten()
    {
        let Some(activity) = db::get_activity(conn, activity_id)? else {
            continue;
        };
        let metrics: Value = serde_json::from_str(&activity.garmin_data).unwrap_or(Value::Null);
        calories = calories.or_else(|| rounded(float(metrics.get("calories"))));
        avg_hr = avg_hr.or_else(|| rounded(float(metrics.get("avg_hr"))));
        if calories.is_some() && avg_hr.is_some() {
            return Ok((calories, avg_hr));
        }
    }
    Ok((
        calories.or_else(|| rounded(generated.0)),
        avg_hr.or_else(|| rounded(generated.1)),
    ))
}

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

/// The workout's window overlapping any local activity: the passive path's
/// guard, since a blind upload would duplicate a session recorded under
/// another type.
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
pub fn resolution_candidates(
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

/// A FIT written to its own temporary folder for the upload, removed after.
struct TempFit(std::path::PathBuf);

impl TempFit {
    fn write(hevy_id: &str, bytes: &[u8]) -> std::io::Result<Self> {
        let dir = std::env::temp_dir().join(format!("activsync-fit-{}", random_token(6)));
        std::fs::create_dir(&dir)?;
        let fit = Self(dir);
        std::fs::write(fit.path(hevy_id), bytes)?;
        Ok(fit)
    }

    fn path(&self, hevy_id: &str) -> std::path::PathBuf {
        self.0.join(format!("hevy_{hevy_id}.fit"))
    }
}

impl Drop for TempFit {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What applying needs: the database, a signed-in Garmin, a config snapshot
/// and the merged calorie profile (cached Garmin values plus the override).
pub struct Apply<'a> {
    pub db: &'a Db,
    pub garmin: &'a GarminClient,
    pub cfg: &'a Config,
    pub profile: &'a Profile,
}

enum Prepared {
    Built { fit: Vec<u8> },
    Closed { status: &'static str, error: String },
}

impl Apply<'_> {
    pub(crate) fn summary_on_structured(&self) -> bool {
        self.cfg
            .get("hevy_summary_on_structured")
            .is_none_or(|v| db::truthy(Some(v)))
    }

    pub(crate) fn template(&self, key: &str) -> Option<&str> {
        self.cfg.get(key).and_then(Value::as_str)
    }

    pub(crate) async fn apply_metadata(
        &self,
        activity_id: i64,
        row: &Workout,
        write_summary: bool,
        generated: (Option<f64>, Option<f64>),
    ) -> Result<(), ApplyError> {
        let payload = row.payload_json();
        let title = generate_title(&payload, self.template("hevy_title_template"))
            .map_err(ApplyError::Invalid)?;
        self.garmin.set_title(activity_id, &title).await?;
        if write_summary {
            let (calories, avg_hr) =
                activity_metrics(&self.db.lock(), row, activity_id, generated)?;
            let description = generate_description(
                &payload,
                calories,
                avg_hr,
                self.template("hevy_description_template"),
            )
            .map_err(ApplyError::Invalid)?;
            self.garmin
                .set_description(activity_id, &description)
                .await?;
        }
        Ok(())
    }

    fn source_of(row: &Workout) -> Result<i64, ApplyError> {
        row.source_garmin_activity_id.ok_or_else(|| {
            ApplyError::Invalid(format!("workout {} has no watch activity", row.hevy_id))
        })
    }

    /// Mark the applied revision so an unchanged workout is not reapplied.
    fn finish(
        &self,
        row: &Workout,
        target: i64,
        strategy: &str,
        status: &str,
    ) -> Result<(), ApplyError> {
        let conn = self.db.lock();
        hevy_db::link_target(&conn, &row.hevy_id, target, strategy)?;
        hevy_db::set_workout_status(&conn, &row.hevy_id, status, None)?;
        hevy_db::set_applied_garmin(&conn, &row.hevy_id, row.source_updated_at.as_deref())?;
        Ok(())
    }

    pub async fn execute_merge(&self, row: &Workout) -> Result<(), ApplyError> {
        let source = Self::source_of(row)?;
        let resolved = resolve_exercises(&self.db.lock(), &row.payload_json())?;
        let existing = self.garmin.get_exercise_sets(source).await?;
        let (start, duration) = {
            let conn = self.db.lock();
            hevy_db::save_backup(&conn, source, &row.hevy_id, &existing, None)?;
            activity_window(&conn, source, row)?
        };
        let mut payload = build_exercise_sets_payload(&resolved, &start, duration)
            .map_err(ApplyError::Invalid)?;
        payload["activityId"] = json!(source);
        match self.garmin.put_exercise_sets(source, &payload).await {
            Ok(()) => {}
            Err(GarminError::SubcategoryRejected(_)) => {
                // The 400 does not say which pair was refused: list the
                // specific ones as candidates, never blame one.
                let candidates: Vec<&str> = resolved
                    .iter()
                    .filter(|r| !matches!(r.subcategory, 0 | 65535))
                    .map(|r| r.title.as_str())
                    .collect();
                let error = format!(
                    "Garmin rejected an exercise pair; adjust one of: {}",
                    candidates.join(", ")
                );
                hevy_db::set_workout_status(
                    &self.db.lock(),
                    &row.hevy_id,
                    "needs_mapping",
                    Some(&error),
                )?;
                return Ok(());
            }
            Err(GarminError::ActivityGone(_)) => {
                hevy_db::set_workout_status(
                    &self.db.lock(),
                    &row.hevy_id,
                    "needs_review",
                    Some(GONE_ON_GARMIN),
                )?;
                return Ok(());
            }
            Err(err) => return Err(err.into()),
        }
        self.apply_metadata(source, row, self.summary_on_structured(), (None, None))
            .await?;
        self.finish(row, source, "merge", "merged")
    }

    pub async fn execute_describe(&self, row: &Workout) -> Result<(), ApplyError> {
        let source = Self::source_of(row)?;
        match self.apply_metadata(source, row, true, (None, None)).await {
            Err(ApplyError::Garmin(GarminError::ActivityGone(_))) => {
                hevy_db::set_workout_status(
                    &self.db.lock(),
                    &row.hevy_id,
                    "needs_review",
                    Some(GONE_ON_GARMIN),
                )?;
                Ok(())
            }
            Err(err) => Err(err),
            Ok(()) => self.finish(row, source, "describe", "described"),
        }
    }

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

    fn park(&self, op: &Operation, row: &Workout, error: &str) -> Result<(), ApplyError> {
        hevy_db::set_operation_outcome(
            &self.db.lock(),
            op.id,
            &row.hevy_id,
            "needs_review",
            "needs_review",
            Some(error),
        )?;
        Ok(())
    }

    fn update(&self, op: &Operation, fields: &[OpField]) -> Result<(), ApplyError> {
        hevy_db::update_operation(&self.db.lock(), op.id, fields)?;
        Ok(())
    }

    /// Drives one operation as far as it can go this tick. Every transition
    /// is saved before the next side effect, so a crash resumes where it
    /// stopped. `submission_unknown` is entered only from the upload call
    /// itself and is never resubmitted.
    pub async fn advance_operation(&self, row: &Workout) -> Result<(), ApplyError> {
        loop {
            let Some(op) = hevy_db::get_open_operation(&self.db.lock(), &row.hevy_id)? else {
                return Ok(());
            };
            match op.phase.as_str() {
                "preparing" => {
                    if !self.prepare(&op, row).await? {
                        return Ok(());
                    }
                }
                "uploading" => {
                    if !self.upload(&op, row).await? {
                        return Ok(());
                    }
                }
                "submission_unknown" => {
                    if !self.resolve(&op, row).await? {
                        return Ok(());
                    }
                }
                "finalizing" => return self.finalize(&op, row).await,
                _ => return Ok(()),
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
            if generic {
                if let Some(detected) = fit_builder::identity_from_fit(&fit) {
                    persist_identity(&conn, detected)?;
                }
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
        let prepared = match self.build(op, row).await {
            Ok(fit) => Prepared::Built { fit },
            Err(ApplyError::MappingMiss { title, .. }) => Prepared::Closed {
                status: "needs_mapping",
                error: format!("unmapped exercises: {title}"),
            },
            Err(err) => Prepared::Closed {
                status: "failed",
                error: format!("FIT build failed: {err}"),
            },
        };
        let fit = match prepared {
            Prepared::Built { fit } => fit,
            Prepared::Closed { status, error } => {
                hevy_db::set_operation_outcome(
                    &self.db.lock(),
                    op.id,
                    &row.hevy_id,
                    "failed",
                    status,
                    Some(&error),
                )?;
                return Ok(false);
            }
        };
        let uploaded = match TempFit::write(&row.hevy_id, &fit) {
            Ok(file) => self.garmin.upload_fit(&file.path(&row.hevy_id)).await,
            Err(err) => {
                let error = format!("FIT build failed: {err}");
                hevy_db::set_operation_outcome(
                    &self.db.lock(),
                    op.id,
                    &row.hevy_id,
                    "failed",
                    "failed",
                    Some(&error),
                )?;
                return Ok(false);
            }
        };
        let result = match uploaded {
            Ok(result) => result,
            Err(GarminError::UploadRejected(message)) => {
                hevy_db::set_operation_outcome(
                    &self.db.lock(),
                    op.id,
                    &row.hevy_id,
                    "failed",
                    "failed",
                    Some(&message),
                )?;
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
        let mut next_step = op.next_step.clone().unwrap_or_else(|| "metadata".into());
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
            next_step = "delete".into();
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
