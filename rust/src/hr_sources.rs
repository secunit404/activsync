//! Heart rate for generated FITs (after hevy2garmin, MIT). Best-effort: HR must
//! never break a sync, so a failure gives no samples.
//! Samples are `{"time": seconds_from_start, "hr": bpm}`, sorted by time.

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use crate::garmin_client::GarminClient;

const WINDOW_BUFFER_MS: f64 = 60_000.0;

fn sample(time: f64, hr: i64) -> Value {
    json!({"time": time, "hr": hr})
}

fn time_of(sample: &Value) -> f64 {
    sample["time"].as_f64().unwrap_or_default()
}

fn sorted(mut samples: Vec<Value>) -> Vec<Value> {
    samples.sort_by(|a, b| time_of(a).total_cmp(&time_of(b)));
    samples
}

pub fn extract_fit_hr(fit: &[u8], workout_start_ms: i64) -> Vec<Value> {
    use fitparser::Value as Fit;
    let mut samples = Vec::new();
    for record in crate::fit_builder::decode_fit(fit) {
        if record.kind() != fitparser::profile::MesgNum::Record {
            continue;
        }
        let field = |name: &str| {
            record
                .fields()
                .iter()
                .find(|f| f.name() == name)
                .map(|f| f.value().clone())
        };
        let (Some(Fit::Timestamp(at)), Some(bpm)) = (field("timestamp"), field("heart_rate"))
        else {
            continue;
        };
        let bpm = match bpm {
            Fit::UInt8(v) | Fit::UInt8z(v) | Fit::Byte(v) | Fit::Enum(v) => i64::from(v),
            Fit::UInt16(v) => i64::from(v),
            _ => continue,
        };
        let at_ms = at.timestamp_micros() as f64 / 1_000_000.0 * 1000.0;
        if at_ms >= workout_start_ms as f64 && 0 < bpm && bpm < 256 {
            samples.push(sample((at_ms - workout_start_ms as f64) / 1000.0, bpm));
        }
    }
    sorted(samples)
}

fn dates_touched(start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<String> {
    let mut dates = Vec::new();
    let mut day = start.date_naive();
    while day <= end.date_naive() {
        dates.push(day.format("%Y-%m-%d").to_string());
        day += Duration::days(1);
    }
    dates
}

/// Fetches every date the window touches, so a session across midnight keeps
/// both halves.
pub async fn passive_hr_for_window(
    garmin: &GarminClient,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<Value> {
    let ms = |at: DateTime<Utc>| (at.timestamp_micros() as f64 / 1_000_000.0 * 1000.0).trunc();
    let (start_ms, end_ms) = (ms(start), ms(end));
    let mut samples = Vec::new();
    for date in dates_touched(start, end) {
        let daily = match garmin.get_daily_heart_rates(&date).await {
            Ok(daily) => daily,
            Err(err) => {
                crate::log_warn!("hr_sources", "daily HR fetch failed for {date}: {err}");
                return Vec::new();
            }
        };
        let Some(values) = daily.get("heartRateValues").and_then(Value::as_array) else {
            continue;
        };
        for entry in values {
            let Some(pair) = entry.as_array().filter(|p| p.len() >= 2 && !p[1].is_null()) else {
                continue;
            };
            let at = pair[0]
                .as_f64()
                .or_else(|| pair[0].as_str()?.trim().parse().ok());
            let bpm = pair[1]
                .as_f64()
                .map(|b| b.trunc() as i64)
                .or_else(|| pair[1].as_str()?.trim().parse().ok());
            let (Some(at), Some(bpm)) = (at, bpm) else {
                continue;
            };
            if start_ms - WINDOW_BUFFER_MS <= at && at <= end_ms + WINDOW_BUFFER_MS {
                samples.push(sample(((at - start_ms) / 1000.0).max(0.0), bpm));
            }
        }
    }
    sorted(samples)
}

pub fn merge_hr_sources(primary: &[Value], secondary: &[Value]) -> Vec<Value> {
    if primary.is_empty() {
        return sorted(secondary.to_vec());
    }
    if secondary.is_empty() {
        return sorted(primary.to_vec());
    }
    let mut chosen = std::collections::BTreeMap::new();
    for sample in secondary.iter().chain(primary) {
        chosen.insert((time_of(sample) / 10.0).floor() as i64, sample.clone());
    }
    chosen.into_values().collect()
}
