//! Applies a Hevy workout to Garmin by one of four strategies.

mod journal;

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::config::Config;
use crate::db::{self, Db};
use crate::exercise_tables::{self, TABLES, UNKNOWN_CATEGORY};
use crate::fit_builder::{
    MAX_SCALE, MIN_SCALE, Profile, REST_BETWEEN_EXERCISES_S, REST_BETWEEN_SETS_S, ResolvedExercise,
    WARMUP_SET_S, WORKING_SET_S,
};
use crate::garmin_client::{GarminClient, GarminError};
use crate::hevy_db::{self, Workout};
use crate::hevy_description::{generate_description, generate_title};
use crate::timefmt::parse_timestamp;

const GONE_ON_GARMIN: &str = "watch activity deleted on Garmin";

#[derive(Debug)]
pub enum ApplyError {
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

pub(crate) fn resolve_exercises(
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

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64)
}

/// Set timings are synthetic, spread across the real activity window.
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
    // Not `clamp`: a NaN scale must land on MAX_SCALE, as Python's
    // `max(MIN, min(MAX, scale))` makes it.
    #[allow(clippy::manual_clamp)]
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

fn activity_window(
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

pub(crate) fn exercise_sets_for(
    conn: &Connection,
    resolved: &[ResolvedExercise],
    activity_id: i64,
    row: &Workout,
) -> Result<Value, ApplyError> {
    let (start, duration) = activity_window(conn, activity_id, row)?;
    let mut payload =
        build_exercise_sets_payload(resolved, &start, duration).map_err(ApplyError::Invalid)?;
    payload["activityId"] = json!(activity_id);
    Ok(payload)
}

/// The target's Garmin metrics, then the watch original's, then the rebuilt
/// FIT's: a replace target is not in the local table until polling sees it.
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

pub struct Apply<'a> {
    pub db: &'a Db,
    pub garmin: &'a GarminClient,
    pub cfg: &'a Config,
    pub profile: &'a Profile,
}

impl Apply<'_> {
    pub(crate) fn summary_on_structured(&self) -> bool {
        self.cfg.summary_on_structured()
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

    fn set_status(&self, row: &Workout, status: &str, error: &str) -> Result<(), ApplyError> {
        hevy_db::set_workout_status(&self.db.lock(), &row.hevy_id, status, Some(error))?;
        Ok(())
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
        let payload = {
            let conn = self.db.lock();
            hevy_db::save_backup(&conn, source, &row.hevy_id, &existing, None)?;
            exercise_sets_for(&conn, &resolved, source, row)?
        };
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
                return self.set_status(row, "needs_mapping", &error);
            }
            Err(GarminError::ActivityGone(_)) => {
                return self.set_status(row, "needs_review", GONE_ON_GARMIN);
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
                self.set_status(row, "needs_review", GONE_ON_GARMIN)
            }
            Err(err) => Err(err),
            Ok(()) => self.finish(row, source, "describe", "described"),
        }
    }
}
