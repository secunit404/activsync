//! The athlete profile for FIT calorie estimates: Garmin's values cached a
//! day, the owner's override winning field by field, defaults last.

use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value, json};

use crate::db::{self, Db};
use crate::fit_builder::Profile;
use crate::garmin_client::GarminClient;
use crate::garmin_errors::py_str;
use crate::hevy_settings::PROFILE_OVERRIDE_KEY;

pub const CACHE_KEY: &str = "garmin_user_profile";
const KEYS: [&str; 4] = ["weight_kg", "birth_year", "vo2max", "sex"];
pub(crate) const DEFAULT_WEIGHT_KG: f64 = 80.0;
pub(crate) const DEFAULT_BIRTH_YEAR: i64 = 1990;
pub(crate) const DEFAULT_VO2MAX: f64 = 45.0;
pub(crate) const DEFAULT_SEX: &str = "male";

pub(crate) fn object(value: Option<Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

/// Merges what Garmin returned into the cache rather than replacing it: the
/// profile and VO2max are read separately and half can fail. An all-empty
/// answer is a failure, since the client swallows its own errors.
async fn refresh_cache(
    db: &Db,
    garmin: &GarminClient,
    now: DateTime<Utc>,
) -> Result<Map<String, Value>, String> {
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
    let mut entry: Map<String, Value> = KEYS
        .iter()
        .filter_map(|k| {
            cached
                .get(*k)
                .filter(|v| !v.is_null())
                .map(|v| (k.to_string(), v.clone()))
        })
        .collect();
    entry.extend(fresh);
    entry.insert("fetched_at".into(), json!(db::iso(now)));
    // Its own row: a slow Garmin answer must never overwrite a settings save.
    db::set_config_value(&conn, CACHE_KEY, &Value::Object(entry.clone()))
        .map_err(|e| e.to_string())?;
    Ok(entry)
}

fn float(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn int(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|f| f.trunc() as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

/// Never fails: a failed refresh falls back to the cache, then defaults.
pub async fn get_profile(db: &Db, garmin: &GarminClient, now: DateTime<Utc>) -> Profile {
    let mut cache = object(db::get_config_value(&db.lock(), CACHE_KEY).ok().flatten());
    let fetched_at = cache
        .get("fetched_at")
        .and_then(Value::as_str)
        .and_then(crate::timefmt::parse_timestamp);
    if fetched_at.is_none_or(|at| now - at >= Duration::hours(24)) {
        match refresh_cache(db, garmin, now).await {
            Ok(entry) => cache = entry,
            Err(err) if cache.is_empty() => crate::log_warn!(
                "hevy_profile",
                "garmin profile fetch failed ({err}); using default profile"
            ),
            Err(err) => crate::log_warn!(
                "hevy_profile",
                "garmin profile fetch failed ({err}); using cached profile"
            ),
        }
    }
    // Read after the slow fetch, so a settings save made meanwhile applies.
    let settings = object(db::get_config_value(&db.lock(), "settings").ok().flatten());
    let overrides = settings
        .get(PROFILE_OVERRIDE_KEY)
        .and_then(Value::as_object);
    let pick = |key: &str| {
        [overrides, Some(&cache)]
            .into_iter()
            .flatten()
            .find_map(|source| source.get(key).filter(|v| !v.is_null()))
    };
    Profile {
        weight_kg: pick("weight_kg")
            .and_then(float)
            .unwrap_or(DEFAULT_WEIGHT_KG),
        birth_year: pick("birth_year")
            .and_then(int)
            .unwrap_or(DEFAULT_BIRTH_YEAR),
        vo2max: pick("vo2max").and_then(float).unwrap_or(DEFAULT_VO2MAX),
        sex: pick("sex").map_or_else(|| DEFAULT_SEX.into(), py_str),
    }
}
