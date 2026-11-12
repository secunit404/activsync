//! Strava's JSON strength upload. Strava reads a FIT's sets through its own
//! partial table of Garmin exercise ids, so machine variants come out wrong
//! or "Unknown"; the JSON names each set's Strava exercise directly.

use chrono::{DateTime, Offset, TimeZone, Utc};
use fitparser::profile::MesgNum;
use fitparser::{FitDataRecord, Value as Fit};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

use crate::fit_encode::FIT_EPOCH_S;
use crate::garmin_errors::py_repr;

/// fitparser turns a `local_date_time` into an instant by counting from the
/// FIT epoch read in the server's own time zone; this undoes that, giving
/// back the raw wall-clock seconds whatever zone the server runs in.
fn local_reference() -> Option<i64> {
    let epoch = chrono::NaiveDate::from_ymd_opt(1989, 12, 31)?.and_hms_opt(0, 0, 0)?;
    Some(
        chrono::Local
            .from_local_datetime(&epoch)
            .single()?
            .timestamp(),
    )
}

struct Recording {
    start: DateTime<Utc>,
    elapsed_s: i64,
    active_s: Option<i64>,
    calories: Option<i64>,
    utc_offset_s: Option<i64>,
    heartrate: Vec<(i64, i64)>,
    set_timing: Vec<(DateTime<Utc>, f64)>,
    device: Option<String>,
}

/// Python's `round()`: halves go to the even neighbour.
fn py_round(value: f64) -> i64 {
    value.round_ties_even() as i64
}

/// The error is the reason to upload the FIT instead.
pub fn build_strength_upload(
    conn: &Connection,
    workout: &Value,
    fit: &[u8],
    tz_name: Option<&str>,
) -> Result<Value, String> {
    let mut hevy_sets: Vec<(String, &Value)> = Vec::new();
    for exercise in workout
        .get("exercises")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let title = exercise
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let template_id = exercise.get("exercise_template_id").and_then(Value::as_str);
        let found = crate::exercise_tables::lookup_strava_exercise(conn, title, template_id)
            .map_err(|e| e.to_string())?;
        let Some((exercise_type, _)) = found else {
            let template = template_id.map_or_else(|| "None".to_string(), str::to_string);
            return Err(format!(
                "no Strava exercise for {} (template {template})",
                py_repr(&Value::from(title))
            ));
        };
        for set in exercise
            .get("sets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            hevy_sets.push((exercise_type.clone(), set));
        }
    }
    if hevy_sets.is_empty() {
        return Err("workout has no sets".into());
    }

    let recording = match read_fit(fit) {
        Some(recording) => recording,
        None => from_workout(workout)?,
    };
    let timing = (recording.set_timing.len() == hevy_sets.len()).then_some(&recording.set_timing);

    let mut sets = Vec::new();
    for (index, (exercise_type, hevy_set)) in hevy_sets.iter().enumerate() {
        let mut entry = Map::new();
        entry.insert("exercise_type".into(), json!(exercise_type));
        if let Some(reps) = hevy_set
            .get("reps")
            .filter(|r| r.is_i64() || r.is_u64())
            .and_then(Value::as_i64)
            .filter(|r| *r >= 0)
        {
            entry.insert("repetitions".into(), json!(reps));
        }
        if let Some(weight) = hevy_set
            .get("weight_kg")
            .and_then(Value::as_f64)
            .filter(|w| *w > 0.0)
        {
            entry.insert("weight".into(), json!(weight));
        }
        if let Some(timing) = timing {
            let (set_start, duration) = timing[index];
            entry.insert("start_time".into(), json!(iso(set_start)));
            entry.insert("duration".into(), json!(py_round(duration)));
        } else if let Some(duration) = hevy_set.get("duration_seconds").and_then(Value::as_f64) {
            entry.insert("duration".into(), json!(py_round(duration)));
        }
        sets.push(Value::Object(entry));
    }

    let offset = recording
        .utc_offset_s
        .unwrap_or_else(|| zone_offset(recording.start, tz_name));
    let mut document = json!({
        "version": "1.0",
        "start_time": iso(recording.start),
        "utc_offset": offset,
        "elapsed_time": recording.elapsed_s,
        "sets": sets,
    });
    if let Some(active) = recording.active_s {
        document["active_time"] = json!(active);
    }
    if let Some(calories) = recording.calories.filter(|c| *c != 0) {
        document["total_calories"] = json!(calories);
    }
    if let Some(device) = recording.device {
        document["creator"] = json!({"name": device});
    }
    if !recording.heartrate.is_empty() {
        document["streams"] = json!({
            "time": recording.heartrate.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            "heartrate": recording.heartrate.iter().map(|(_, bpm)| *bpm).collect::<Vec<_>>(),
        });
    }
    Ok(document)
}

fn field<'a>(record: &'a FitDataRecord, name: &str) -> Option<&'a Fit> {
    record
        .fields()
        .iter()
        .find(|f| f.name() == name)
        .map(|f| f.value())
}

fn timestamp(value: Option<&Fit>) -> Option<DateTime<Utc>> {
    match value? {
        Fit::Timestamp(at) => Some(at.with_timezone(&Utc)),
        _ => None,
    }
}

fn number(value: Option<&Fit>) -> Option<f64> {
    Some(match value? {
        Fit::Byte(v) | Fit::UInt8(v) | Fit::UInt8z(v) => f64::from(*v),
        Fit::SInt8(v) => f64::from(*v),
        Fit::SInt16(v) => f64::from(*v),
        Fit::UInt16(v) | Fit::UInt16z(v) => f64::from(*v),
        Fit::SInt32(v) => f64::from(*v),
        Fit::UInt32(v) | Fit::UInt32z(v) => f64::from(*v),
        Fit::SInt64(v) => *v as f64,
        Fit::UInt64(v) | Fit::UInt64z(v) => *v as f64,
        Fit::Float32(v) => f64::from(*v),
        Fit::Float64(v) => *v,
        _ => return None,
    })
}

fn integer(value: Option<&Fit>) -> Option<i64> {
    match value? {
        Fit::Float32(_) | Fit::Float64(_) => None,
        other => number(Some(other)).map(|n| n as i64),
    }
}

fn text(value: Option<&Fit>) -> Option<&str> {
    match value? {
        Fit::String(s) => Some(s),
        _ => None,
    }
}

fn read_fit(fit: &[u8]) -> Option<Recording> {
    if fit.is_empty() {
        return None;
    }
    let records = fitparser::from_bytes(fit).ok()?;
    let of = |kind: MesgNum| records.iter().filter(move |r| r.kind() == kind);
    let session = of(MesgNum::Session).next()?;
    let start = timestamp(field(session, "start_time"))?;
    let elapsed = number(field(session, "total_elapsed_time")).filter(|e| *e > 0.0)?;
    let timer = number(field(session, "total_timer_time"));
    let calories = number(field(session, "total_calories"));

    // local_timestamp is wall-clock time, so its distance from the UTC
    // timestamp is the athlete's offset.
    let utc_offset_s = of(MesgNum::Activity).next().and_then(|activity| {
        let utc = timestamp(field(activity, "timestamp"))?;
        let local = match field(activity, "local_timestamp")? {
            Fit::Timestamp(at) => at.timestamp() - local_reference()?,
            other => integer(Some(other))?,
        };
        Some(FIT_EPOCH_S + local - utc.timestamp())
    });

    let mut heartrate: Vec<(i64, i64)> = Vec::new();
    for record in of(MesgNum::Record) {
        let (Some(bpm), Some(stamp)) = (
            integer(field(record, "heart_rate")),
            timestamp(field(record, "timestamp")),
        ) else {
            continue;
        };
        let second = (stamp - start).num_milliseconds() / 1000;
        if second >= 0 && heartrate.last().is_none_or(|(last, _)| second > *last) {
            heartrate.push((second, bpm));
        }
    }

    let set_timing = of(MesgNum::Set)
        .filter(|set| text(field(set, "set_type")) == Some("active"))
        .filter_map(|set| {
            Some((
                timestamp(field(set, "start_time"))?,
                number(field(set, "duration"))?,
            ))
        })
        .collect();

    let file_id = of(MesgNum::FileId).next();
    let device = file_id.and_then(|file_id| {
        if text(field(file_id, "manufacturer")) != Some("garmin") {
            return None;
        }
        text(field(file_id, "garmin_product"))
            .map(|product| format!("Garmin {}", product.replace('_', " ")))
    });

    Some(Recording {
        start,
        elapsed_s: py_round(elapsed),
        active_s: timer.filter(|t| *t > 0.0).map(py_round),
        calories: calories.map(py_round),
        utc_offset_s,
        heartrate,
        set_timing,
        device,
    })
}

fn from_workout(workout: &Value) -> Result<Recording, String> {
    let at = |key: &str| {
        workout
            .get(key)
            .and_then(Value::as_str)
            .and_then(crate::timefmt::parse_timestamp)
    };
    let (Some(start), Some(end)) = (at("start_time"), at("end_time")) else {
        return Err("workout has no usable time window".into());
    };
    if end <= start {
        return Err("workout has no usable time window".into());
    }
    Ok(Recording {
        start,
        elapsed_s: py_round((end - start).num_milliseconds() as f64 / 1000.0),
        active_s: None,
        calories: None,
        utc_offset_s: None,
        heartrate: Vec::new(),
        set_timing: Vec::new(),
        device: None,
    })
}

fn zone_offset(moment: DateTime<Utc>, tz_name: Option<&str>) -> i64 {
    match tz_name.filter(|tz| crate::timefmt::is_valid_timezone(tz)) {
        Some(tz) => i64::from(
            crate::timefmt::parse_tz(tz)
                .offset_from_utc_datetime(&moment.naive_utc())
                .fix()
                .local_minus_utc(),
        ),
        None => 0,
    }
}

fn iso(moment: DateTime<Utc>) -> String {
    moment.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}
