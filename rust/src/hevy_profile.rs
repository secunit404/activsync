//! The athlete profile for FIT calorie estimates, as the Python app's
//! `hevy_profile.py`: Garmin's values cached a day in `garmin_user_profile`,
//! the owner's `profile_override` winning field by field, defaults last.

use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value, json};

use crate::db::{self, Db};
use crate::fit_builder::Profile;
use crate::garmin_client::GarminClient;

pub const CACHE_KEY: &str = "garmin_user_profile";
pub const OVERRIDE_KEY: &str = "profile_override";
const KEYS: [&str; 4] = ["weight_kg", "birth_year", "vo2max", "sex"];

pub fn defaults() -> Map<String, Value> {
    let Value::Object(map) = json!({"weight_kg": 80.0, "birth_year": 1990, "vo2max": 45.0, "sex": "male"}) else {
        unreachable!()
    };
    map
}

fn object(value: Option<Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

/// Merges what Garmin returned into the cache rather than replacing it: the
/// profile and VO2max are read separately and half can fail. An all-empty
/// answer is a failure, since the client swallows its own errors.
async fn refresh_cache(db: &Db, garmin: &GarminClient, now: DateTime<Utc>) -> Result<Map<String, Value>, String> {
    let fetched = garmin.fetch_user_profile().await;
    let mut fresh = Map::new();
    if let Some(weight) = fetched.weight_kg {
        fresh.insert("weight_kg".into(), json!(weight));
    }
    if let Some(year) = fetched.birth_year {
        fresh.insert("birth_year".into(), json!(year));
    }
    if let Some(vo2max) = fetched.vo2max {
        fresh.insert("vo2max".into(), json!(vo2max));
    }
    if let Some(sex) = fetched.sex {
        fresh.insert("sex".into(), json!(sex));
    }
    if fresh.is_empty() {
        return Err("garmin returned no usable profile fields".into());
    }
    let conn = db.lock();
    let cached = object(db::get_config_value(&conn, CACHE_KEY).map_err(|e| e.to_string())?);
    let mut entry: Map<String, Value> =
        KEYS.iter().filter_map(|k| cached.get(*k).filter(|v| !v.is_null()).map(|v| (k.to_string(), v.clone()))).collect();
    entry.extend(fresh);
    entry.insert("fetched_at".into(), json!(db::iso(now)));
    // Its own row: a slow Garmin answer must never overwrite a settings save.
    db::set_config_value(&conn, CACHE_KEY, &Value::Object(entry.clone())).map_err(|e| e.to_string())?;
    Ok(entry)
}

fn float(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.trim().parse().ok())
}

fn int(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_f64().map(|f| f.trunc() as i64)).or_else(|| value.as_str()?.trim().parse().ok())
}

/// Never fails: a failed refresh falls back to the cache, then defaults.
pub async fn get_profile(db: &Db, garmin: &GarminClient, now: DateTime<Utc>) -> Profile {
    let mut cache = object(db::get_config_value(&db.lock(), CACHE_KEY).ok().flatten());
    let fetched_at = cache.get("fetched_at").and_then(Value::as_str).and_then(crate::timefmt::parse_timestamp);
    if fetched_at.is_none_or(|at| now - at >= Duration::hours(24)) {
        match refresh_cache(db, garmin, now).await {
            Ok(entry) => cache = entry,
            Err(err) if cache.is_empty() => crate::log_warn!("hevy_profile", "garmin profile fetch failed ({err}); using default profile"),
            Err(err) => crate::log_warn!("hevy_profile", "garmin profile fetch failed ({err}); using cached profile"),
        }
    }
    // Read after the slow fetch, so a settings save made meanwhile applies.
    let settings = object(db::get_config_value(&db.lock(), "settings").ok().flatten());
    let overrides = settings.get(OVERRIDE_KEY).and_then(Value::as_object).cloned().unwrap_or_default();
    let mut merged = defaults();
    for source in [&cache, &overrides] {
        for key in KEYS {
            if let Some(value) = source.get(key).filter(|v| !v.is_null()) {
                merged.insert(key.into(), value.clone());
            }
        }
    }
    let fallback = defaults();
    Profile {
        weight_kg: float(&merged["weight_kg"]).unwrap_or_else(|| fallback["weight_kg"].as_f64().unwrap_or(80.0)),
        birth_year: int(&merged["birth_year"]).unwrap_or(1990),
        vo2max: float(&merged["vo2max"]).unwrap_or_else(|| fallback["vo2max"].as_f64().unwrap_or(45.0)),
        sex: merged["sex"].as_str().map_or_else(|| merged["sex"].to_string(), str::to_string),
    }
}
