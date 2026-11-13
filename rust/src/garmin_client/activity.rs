use std::collections::HashSet;
use std::io::Read;

use serde_json::{Number, Value, json};

use super::GarminError;
use crate::garmin_errors::py_str;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActivityRecord {
    pub garmin_activity_id: i64,
    pub activity_type: String,
    pub title: String,
    pub description: String,
    pub start_time: String,
    pub distance: Option<Number>,
    pub duration: Option<Number>,
    pub moving_duration: Option<Number>,
    pub elapsed_duration: Option<Number>,
    pub elevation_gain: Option<Number>,
    pub elevation_loss: Option<Number>,
    pub calories: Option<Number>,
    pub avg_hr: Option<Number>,
    pub max_hr: Option<Number>,
    pub avg_power: Option<Number>,
    pub max_power: Option<Number>,
    pub norm_power: Option<Number>,
    pub aerobic_te: Option<Number>,
    pub anaerobic_te: Option<Number>,
    pub training_load: Option<Number>,
    pub avg_cadence: Option<Number>,
    pub max_cadence: Option<Number>,
    pub total_sets: Option<Number>,
    pub total_reps: Option<Number>,
    pub total_volume: Option<Number>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UploadResult {
    pub upload_id: Option<Value>,
    /// `None` while Garmin is still processing the import; resolving it is
    /// the operation journal's job, never a retry loop here.
    pub activity_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserProfile {
    pub weight_kg: Option<f64>,
    pub birth_year: Option<i64>,
    pub sex: Option<String>,
    pub vo2max: Option<f64>,
}

pub(super) fn activity_id_of(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

/// Garmin sometimes returns `internalId` as a quoted string; stored as is,
/// every later call with it 404s.
pub(super) fn sanitize_activity_id(raw: Option<&Value>) -> Option<i64> {
    match raw? {
        Value::Number(n) => n.as_i64(),
        Value::Null => None,
        other => py_str(other)
            .trim()
            .trim_matches(['\'', '"'])
            .trim()
            .parse()
            .ok(),
    }
}

/// Kept as Garmin sent it, int or float, since the stored `garmin_data`
/// (and its content hash) must read exactly as the Python app wrote it.
fn number(act: &Value, key: &str) -> Option<Number> {
    act.get(key).and_then(Value::as_number).cloned()
}

pub(super) fn record_of(act: &Value) -> Result<ActivityRecord, GarminError> {
    let text = |key: &str| {
        act.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Ok(ActivityRecord {
        garmin_activity_id: act
            .get("activityId")
            .and_then(activity_id_of)
            .ok_or_else(|| GarminError::Malformed("activity without activityId".into()))?,
        activity_type: act["activityType"]["typeKey"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        title: text("activityName"),
        description: text("description"),
        start_time: text("startTimeGMT"),
        distance: number(act, "distance"),
        duration: number(act, "duration"),
        moving_duration: number(act, "movingDuration"),
        elapsed_duration: number(act, "elapsedDuration"),
        elevation_gain: number(act, "elevationGain"),
        elevation_loss: number(act, "elevationLoss"),
        calories: number(act, "calories"),
        avg_hr: number(act, "averageHR"),
        max_hr: number(act, "maxHR"),
        avg_power: number(act, "avgPower"),
        max_power: number(act, "maxPower"),
        norm_power: number(act, "normPower"),
        aerobic_te: number(act, "aerobicTrainingEffect"),
        anaerobic_te: number(act, "anaerobicTrainingEffect"),
        training_load: number(act, "activityTrainingLoad"),
        avg_cadence: number(act, "averageRunningCadenceInStepsPerMinute"),
        max_cadence: number(act, "maxRunningCadenceInStepsPerMinute"),
        total_sets: number(act, "totalSets"),
        total_reps: number(act, "totalReps"),
        total_volume: number(act, "totalVolume"),
    })
}

pub(super) fn fit_from_zip(zip_bytes: &[u8], activity_id: i64) -> Result<Vec<u8>, GarminError> {
    let malformed = |e: zip::result::ZipError| GarminError::Malformed(e.to_string());
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(malformed)?;
    for index in 0..archive.len() {
        let mut member = archive.by_index(index).map_err(malformed)?;
        if member
            .name()
            .map_err(malformed)?
            .to_lowercase()
            .ends_with(".fit")
        {
            let mut fit = Vec::new();
            member
                .read_to_end(&mut fit)
                .map_err(|e| GarminError::Malformed(e.to_string()))?;
            return Ok(fit);
        }
    }
    Err(GarminError::Malformed(format!(
        "no .fit file in original download for activity {activity_id}"
    )))
}

pub(super) fn activity_types_of(raw: &Value) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut types: Vec<(&str, String)> = raw
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("typeKey")?.as_str())
        .filter(|key| !key.is_empty() && *key != "all" && seen.insert(*key))
        .map(|key| (key, crate::mock::python_title(&key.replace('_', " "))))
        .collect();
    types.sort_by(|a, b| a.1.cmp(&b.1));
    types
        .into_iter()
        .map(|(key, label)| json!({"type_key": key, "label": label}))
        .collect()
}
