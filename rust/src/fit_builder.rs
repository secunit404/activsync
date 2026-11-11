//! Strength-training FIT files from Hevy workouts, as the Python app's
//! `fit_builder.py` (from hevy2garmin, MIT): exercises arrive resolved, the
//! profile and device identity are injected, and calories use sex-correct
//! Keytel constants. The bytes match the Python build exactly.

use chrono::{DateTime, Datelike, Utc};
use serde_json::{Value, json};

use crate::fit_encode::{Encoder, FitValue, Mesg, mesg};

pub const WORKING_SET_S: f64 = 40.0;
pub const WARMUP_SET_S: f64 = 25.0;
pub const REST_BETWEEN_SETS_S: f64 = 75.0;
pub const REST_BETWEEN_EXERCISES_S: f64 = 120.0;
pub const MIN_SCALE: f64 = 0.3;
pub const MAX_SCALE: f64 = 2.0;
/// Calorie fallback when there is no heart rate at all.
const DEFAULT_HR_BPM: i64 = 90;
pub const UNKNOWN_CATEGORY: i64 = 65534;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub manufacturer: i64,
    pub product: i64,
    pub serial: i64,
}

/// A generic Garmin identity (manufacturer 1, product 0) is accepted and
/// still gets Training Effect and Load; installs persist their own serial
/// via [`new_fallback_identity`] rather than sharing this placeholder.
pub const GENERIC_GARMIN_IDENTITY: DeviceIdentity = DeviceIdentity { manufacturer: 1, product: 0, serial: 424242 };

/// Generated once per install and persisted (`hevy_device_identity`), so
/// installs do not share a serial.
pub fn new_fallback_identity() -> DeviceIdentity {
    DeviceIdentity { manufacturer: 1, product: 0, serial: rand::random_range(1..=i64::from(i32::MAX - 1)) }
}

/// The messages of a FIT file with enums as numbers and no CRC check.
/// Watch files come through Garmin's download endpoint, sometimes truncated,
/// so whatever was read before a fault is kept rather than nothing, as the
/// Python SDK's decoder does.
pub fn decode_fit(fit: &[u8]) -> Vec<fitparser::FitDataRecord> {
    use fitparser::de::{DecodeOption, FitObject, FitStreamProcessor};
    let mut processor = FitStreamProcessor::new();
    for option in [
        DecodeOption::ReturnNumericEnumValues,
        DecodeOption::SkipHeaderCrcValidation,
        DecodeOption::SkipDataCrcValidation,
        DecodeOption::UseGenericSubFieldName,
    ] {
        processor.add_option(option);
    }
    let mut records = Vec::new();
    let mut buffer = fit;
    while !buffer.is_empty() {
        let Ok((rest, object)) = processor.deserialize_next(buffer) else {
            break;
        };
        match object {
            FitObject::Crc(..) => processor.reset(),
            FitObject::DataMessage(message) => match processor.decode_message(message) {
                Ok(record) => records.push(record),
                Err(_) => break,
            },
            _ => {}
        }
        buffer = rest;
    }
    records
}

/// The identity in a watch FIT's file_id message, so each install uploads
/// as its owner's own watch; `None` when unreadable.
pub fn identity_from_fit(fit: &[u8]) -> Option<DeviceIdentity> {
    let records = decode_fit(fit);
    let file_id = records.iter().find(|r| r.kind() == fitparser::profile::MesgNum::FileId)?;
    let get = |name: &str| {
        use fitparser::Value as Fit;
        Some(match file_id.fields().iter().find(|f| f.name() == name)?.value() {
            Fit::Enum(v) | Fit::UInt8(v) | Fit::UInt8z(v) | Fit::Byte(v) => i64::from(*v),
            Fit::UInt16(v) | Fit::UInt16z(v) => i64::from(*v),
            Fit::UInt32(v) | Fit::UInt32z(v) => i64::from(*v),
            Fit::SInt8(v) => i64::from(*v),
            Fit::SInt16(v) => i64::from(*v),
            Fit::SInt32(v) => i64::from(*v),
            Fit::SInt64(v) => *v,
            Fit::UInt64(v) | Fit::UInt64z(v) => i64::try_from(*v).ok()?,
            _ => return None,
        })
    };
    Some(DeviceIdentity { manufacturer: get("manufacturer")?, product: get("product")?, serial: get("serial_number")? })
}

pub fn identity_from_config(stored: Option<&Value>) -> DeviceIdentity {
    let int = |v: &Value, key: &str| {
        let value = v.get(key)?;
        value.as_i64().or_else(|| value.as_f64().map(|f| f as i64)).or_else(|| value.as_str()?.trim().parse().ok())
    };
    stored
        .filter(|v| v.is_object())
        .and_then(|v| Some(DeviceIdentity { manufacturer: int(v, "manufacturer")?, product: int(v, "product")?, serial: int(v, "serial")? }))
        .unwrap_or(GENERIC_GARMIN_IDENTITY)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub weight_kg: f64,
    pub birth_year: i64,
    pub vo2max: f64,
    pub sex: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedExercise {
    pub title: String,
    pub category: i64,
    pub subcategory: i64,
    pub sets: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildSummary {
    pub exercises: usize,
    pub total_sets: usize,
    pub hr_samples: usize,
    pub calories: i64,
    pub avg_hr: Option<i64>,
    pub duration_s: f64,
}

/// Keytel et al. 2005 energy expenditure.
pub fn keytel_kcal_per_min(hr: i64, profile: &Profile, age: i64) -> f64 {
    let (hr, age) = (hr as f64, age as f64);
    if profile.sex == "female" {
        (-59.3954 + 0.45 * hr + 0.380 * profile.vo2max + 0.103 * profile.weight_kg + 0.274 * age) / 4.184
    } else {
        (-95.7735 + 0.634 * hr + 0.404 * profile.vo2max + 0.394 * profile.weight_kg + 0.271 * age) / 4.184
    }
}

fn calc_calories(hr_bpm: &[i64], duration_s: f64, workout_year: i64, profile: &Profile) -> i64 {
    let age = workout_year - profile.birth_year;
    let default = [DEFAULT_HR_BPM];
    let samples = if hr_bpm.is_empty() { &default[..] } else { hr_bpm };
    let interval_min = (duration_s / samples.len() as f64) / 60.0;
    let mut total = 0.0;
    for hr in samples {
        total += keytel_kcal_per_min(*hr, profile, age).max(0.0) * interval_min;
    }
    total.round_ties_even() as i64
}

fn parse_time(raw: Option<&Value>) -> Option<DateTime<Utc>> {
    let text = raw?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    if text.contains('T') { crate::timefmt::parse_timestamp(text) } else { crate::timefmt::parse_garmin(text) }
}

fn py_repr(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "None".into(),
        Some(Value::String(s)) => format!("'{s}'"),
        Some(other) => other.to_string(),
    }
}

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64)
}

/// `datetime.fromtimestamp(round(ms / 1000))`: FIT time is whole seconds,
/// rounded so sub-second positions never walk backwards.
fn at(ms: i64) -> FitValue {
    FitValue::Time((ms as f64 / 1000.0).round_ties_even() as i64)
}

struct SetInfo<'a> {
    exercise: usize,
    set: &'a Value,
    set_dur: f64,
    rest_dur: f64,
    start_offset: f64,
    end_offset: f64,
}

enum Kind {
    Record,
    Set,
}

struct Entry {
    ts: i64,
    kind: Kind,
    timestamp: i64,
    start_time: Option<i64>,
    duration: Option<f64>,
    fields: Vec<(&'static str, Option<FitValue>)>,
}

impl Entry {
    fn mesg(&self) -> Mesg {
        let mut fields = vec![("timestamp", Some(at(self.timestamp)))];
        if let Some(start) = self.start_time {
            fields.push(("start_time", Some(at(start))));
        }
        if let Some(duration) = self.duration {
            fields.push(("duration", Some(FitValue::Float(duration))));
        }
        fields.extend(self.fields.iter().cloned());
        mesg(if matches!(self.kind, Kind::Record) { "record" } else { "set" }, fields)
    }
}

/// HR samples are timestamped objects (`{"time": seconds_from_start, "hr":
/// bpm}`) placed at their real offsets, or plain bpm numbers spread evenly.
pub fn build_fit(
    workout: &Value,
    resolved: &[ResolvedExercise],
    hr_samples: &[Value],
    profile: &Profile,
    identity: DeviceIdentity,
) -> Result<(Vec<u8>, BuildSummary), String> {
    let mut hr_timed: Option<Vec<(f64, i64)>> = None;
    let mut hr_bpm: Vec<i64> = Vec::new();
    if let Some(first) = hr_samples.first() {
        if first.is_object() {
            let timed: Vec<(f64, i64)> = hr_samples
                .iter()
                .filter(|s| s.get("hr").is_some_and(|hr| !hr.is_null()))
                .map(|s| (number(s.get("time")).unwrap_or(0.0).max(0.0), number(s.get("hr")).unwrap_or(0.0) as i64))
                .collect();
            hr_bpm = timed.iter().map(|(_, bpm)| *bpm).collect();
            hr_timed = Some(timed);
        } else {
            hr_bpm = hr_samples.iter().map(|x| number(Some(x)).unwrap_or(0.0) as i64).collect();
        }
    }

    let title = workout.get("title").and_then(Value::as_str).unwrap_or("?");
    let (start, end) = match (parse_time(workout.get("start_time")), parse_time(workout.get("end_time"))) {
        (Some(start), Some(end)) => (start, end),
        _ => {
            return Err(format!(
                "Workout '{title}' missing valid start/end time (start={}, end={})",
                py_repr(workout.get("start_time")),
                py_repr(workout.get("end_time"))
            ));
        }
    };
    let duration_s = (end - start).num_microseconds().unwrap_or(0) as f64 / 1_000_000.0;
    if duration_s <= 0.0 {
        return Err(format!(
            "Workout '{title}' has non-positive duration ({} → {})",
            py_repr(workout.get("start_time")),
            py_repr(workout.get("end_time"))
        ));
    }
    if let Some(timed) = hr_timed.as_mut() {
        // Buffered samples that cannot land in the activity must not move
        // calories or the averages either.
        timed.retain(|(offset, _)| *offset <= duration_s);
        hr_bpm = timed.iter().map(|(_, bpm)| *bpm).collect();
    }
    if let Some(unknown) = resolved.iter().find(|e| e.category == UNKNOWN_CATEGORY) {
        return Err(format!("exercise '{}' resolved to UNKNOWN", unknown.title));
    }
    let start_ms = (start.timestamp_micros() as f64 / 1_000_000.0 * 1000.0).round_ties_even() as i64;
    let end_ms = start_ms + (duration_s * 1000.0).round_ties_even() as i64;
    let calories = calc_calories(&hr_bpm, duration_s, i64::from(start.year()), profile);

    let mut sets: Vec<SetInfo> = Vec::new();
    for (ex_idx, exercise) in resolved.iter().enumerate() {
        for (s_idx, set) in exercise.sets.iter().enumerate() {
            let warmup = set.get("type").and_then(Value::as_str).unwrap_or("normal") == "warmup";
            let explicit = number(set.get("duration_seconds")).filter(|d| *d > 0.0);
            let set_dur = explicit.unwrap_or(if warmup { WARMUP_SET_S } else { WORKING_SET_S });
            let last_set = s_idx == exercise.sets.len() - 1;
            let rest_dur = match (last_set, ex_idx == resolved.len() - 1) {
                (true, true) => 0.0,
                (true, false) => REST_BETWEEN_EXERCISES_S,
                _ => REST_BETWEEN_SETS_S,
            };
            sets.push(SetInfo { exercise: ex_idx, set, set_dur, rest_dur, start_offset: 0.0, end_offset: 0.0 });
        }
    }
    let ideal: f64 = sets.iter().map(|s| s.set_dur + s.rest_dur).sum();
    let scale = if ideal > 0.0 { MIN_SCALE.max(MAX_SCALE.min(duration_s / ideal)) } else { 1.0 };
    let mut cursor = 0.0;
    for info in &mut sets {
        info.start_offset = cursor;
        let scaled = info.set_dur * scale;
        info.end_offset = cursor + scaled;
        cursor += scaled + info.rest_dur * scale;
    }

    let mut encoder = Encoder::new();
    let write = |encoder: &mut Encoder, message: Mesg| encoder.write(&message).map_err(|e| format!("Could not write Message: {e}"));
    write(&mut encoder, mesg("file_id", vec![
        ("type", Some(FitValue::Name("activity"))),
        ("manufacturer", Some(FitValue::Int(identity.manufacturer))),
        ("product", Some(FitValue::Int(identity.product))),
        ("serial_number", Some(FitValue::Int(identity.serial))),
        ("time_created", Some(at(start_ms))),
    ]))?;
    write(&mut encoder, mesg("sport", vec![
        ("sport", Some(FitValue::Name("training"))),
        ("sub_sport", Some(FitValue::Name("strength_training"))),
    ]))?;
    for (index, exercise) in resolved.iter().enumerate() {
        write(&mut encoder, mesg("exercise_title", vec![
            ("message_index", Some(FitValue::Int(index as i64))),
            ("exercise_category", Some(FitValue::Int(exercise.category))),
            ("exercise_name", Some(FitValue::Int(exercise.subcategory))),
            ("wkt_step_name", Some(FitValue::Text(exercise.title.clone()))),
        ]))?;
    }
    write(&mut encoder, mesg("event", vec![
        ("timestamp", Some(at(start_ms))),
        ("event", Some(FitValue::Name("timer"))),
        ("event_type", Some(FitValue::Name("start"))),
    ]))?;

    let record = |ms: i64, field: &'static str, value: FitValue| Entry {
        ts: ms,
        kind: Kind::Record,
        timestamp: ms,
        start_time: None,
        duration: None,
        fields: vec![(field, Some(value))],
    };
    let mut timeline: Vec<Entry> = Vec::new();
    if let Some(timed) = hr_timed.as_ref().filter(|t| !t.is_empty()) {
        for (offset, bpm) in timed {
            timeline.push(record(start_ms + (offset * 1000.0).round_ties_even() as i64, "heart_rate", FitValue::Int(*bpm)));
        }
    } else if !hr_bpm.is_empty() {
        let interval = if hr_bpm.len() == 1 { 0 } else { (duration_s * 1000.0 / (hr_bpm.len() - 1) as f64).round_ties_even() as i64 };
        for (i, bpm) in hr_bpm.iter().enumerate() {
            timeline.push(record(start_ms + i as i64 * interval, "heart_rate", FitValue::Int(*bpm)));
        }
    }

    let mut total_distance = 0.0;
    let mut message_index = 0;
    for info in &sets {
        let exercise = &resolved[info.exercise];
        let set_start = start_ms + (info.start_offset * 1000.0).round_ties_even() as i64;
        let set_end = start_ms + (info.end_offset * 1000.0).round_ties_even() as i64;
        let reps = info.set.get("reps").filter(|r| !r.is_null()).map(|r| FitValue::Int(number(Some(r)).unwrap_or(0.0) as i64));
        let weight = info.set.get("weight_kg").filter(|w| !w.is_null()).map(|w| FitValue::Float(number(Some(w)).unwrap_or(0.0).max(0.0)));
        let active = Entry {
            ts: set_end,
            kind: Kind::Set,
            timestamp: set_end,
            start_time: Some(set_start),
            duration: Some(info.end_offset - info.start_offset),
            fields: vec![
                ("set_type", Some(FitValue::Name("active"))),
                ("category", Some(FitValue::List(vec![FitValue::Int(exercise.category)]))),
                ("category_subtype", Some(FitValue::List(vec![FitValue::Int(exercise.subcategory)]))),
                ("message_index", Some(FitValue::Int(message_index))),
                ("wkt_step_index", Some(FitValue::Int(info.exercise as i64))),
                ("repetitions", reps),
                ("weight", weight),
            ],
        };
        if let Some(distance) = number(info.set.get("distance_meters")).filter(|d| *d > 0.0) {
            total_distance += distance;
            timeline.push(record(set_end, "distance", FitValue::Float(distance)));
        }
        timeline.push(active);
        message_index += 1;

        if info.rest_dur > 0.0 {
            let rest_scaled = info.rest_dur * scale;
            let rest_end = set_end + (rest_scaled * 1000.0).round_ties_even() as i64;
            timeline.push(Entry {
                ts: rest_end,
                kind: Kind::Set,
                timestamp: rest_end,
                start_time: Some(set_end),
                duration: Some(rest_scaled),
                fields: vec![
                    ("set_type", Some(FitValue::Name("rest"))),
                    ("message_index", Some(FitValue::Int(message_index))),
                    ("wkt_step_index", Some(FitValue::Int(info.exercise as i64))),
                ],
            });
            message_index += 1;
        }
    }

    // Nothing may be stamped after TIMER STOP_ALL: HR carries a buffer and
    // the scale floor can push synthetic sets past the end, so set bounds
    // are clamped to the end and late records dropped.
    let mut clipped = Vec::new();
    for mut entry in timeline {
        if matches!(entry.kind, Kind::Record) {
            if entry.ts <= end_ms {
                clipped.push(entry);
            }
            continue;
        }
        let set_start = entry.start_time.unwrap_or_default();
        if set_start >= end_ms {
            continue;
        }
        if entry.ts > end_ms {
            entry.timestamp = end_ms;
            entry.duration = Some((end_ms - set_start) as f64 / 1000.0);
            entry.ts = end_ms;
        }
        clipped.push(entry);
    }
    clipped.sort_by_key(|e| (e.ts, if matches!(e.kind, Kind::Record) { 0 } else { 1 }));
    for entry in &clipped {
        write(&mut encoder, entry.mesg())?;
    }

    write(&mut encoder, mesg("event", vec![
        ("timestamp", Some(at(end_ms))),
        ("event", Some(FitValue::Name("timer"))),
        ("event_type", Some(FitValue::Name("stop_all"))),
    ]))?;

    let avg_hr = (!hr_bpm.is_empty()).then(|| (hr_bpm.iter().sum::<i64>() as f64 / hr_bpm.len() as f64).round_ties_even() as i64);
    let max_hr = hr_bpm.iter().max().copied();
    let summary = |extra: Vec<(&'static str, Option<FitValue>)>| {
        let mut fields = vec![
            ("timestamp", Some(at(end_ms))),
            ("start_time", Some(at(start_ms))),
            ("total_elapsed_time", Some(FitValue::Float(duration_s))),
            ("total_timer_time", Some(FitValue::Float(duration_s))),
            ("sport", Some(FitValue::Name("training"))),
            ("sub_sport", Some(FitValue::Name("strength_training"))),
            ("message_index", Some(FitValue::Int(0))),
            ("event", Some(FitValue::Name("lap"))),
            ("event_type", Some(FitValue::Name("stop"))),
            ("avg_heart_rate", avg_hr.map(FitValue::Int)),
            ("max_heart_rate", max_hr.map(FitValue::Int)),
            ("total_distance", (total_distance > 0.0).then_some(FitValue::Float(total_distance))),
            ("total_calories", Some(FitValue::Int(calories))),
        ];
        fields.extend(extra);
        fields
    };
    write(&mut encoder, mesg("lap", summary(vec![])))?;
    write(&mut encoder, mesg("session", summary(vec![
        ("first_lap_index", Some(FitValue::Int(0))),
        ("num_laps", Some(FitValue::Int(1))),
    ])))?;
    write(&mut encoder, mesg("activity", vec![
        ("timestamp", Some(at(end_ms))),
        ("total_timer_time", Some(FitValue::Float(duration_s))),
        ("num_sessions", Some(FitValue::Int(1))),
        ("type", Some(FitValue::Name("manual"))),
        ("event", Some(FitValue::Name("activity"))),
        ("event_type", Some(FitValue::Name("stop"))),
    ]))?;

    let summary = BuildSummary {
        exercises: resolved.len(),
        total_sets: sets.len(),
        hr_samples: hr_bpm.len(),
        calories,
        avg_hr,
        duration_s,
    };
    Ok((encoder.close(), summary))
}

impl BuildSummary {
    pub fn to_json(&self) -> Value {
        json!({
            "exercises": self.exercises, "total_sets": self.total_sets, "hr_samples": self.hr_samples,
            "calories": self.calories, "avg_hr": self.avg_hr, "duration_s": self.duration_s,
        })
    }
}
