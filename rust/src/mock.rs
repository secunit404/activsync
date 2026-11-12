//! No-network stand-ins for Garmin, Strava and Hevy, matching what the Python
//! app's `dev_mock.py` answers so the dev UI walks the same paths.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value, json};

use crate::config::Config;
use crate::db::{self, Db, DbResult, NewActivity};
use crate::garmin_client::ActivityRecord;
use crate::strava_client::{self, StravaActivity};
use crate::sync::{self, CatchUpStats, GarminSyncStats, PublishStats, StatusCheckStats, SyncError, SyncResult};

/// A Garmin password of "mfa" asks for a verification code.
pub const MFA_TRIGGER_PASSWORD: &str = "mfa";
/// The one verification code the mock rejects.
pub const MFA_REJECT_CODE: &str = "000000";

const GARMIN_ACTIVITY_TYPE_KEYS: &[&str] = &[
    "american_football", "apnea_diving", "apnea_hunting", "archery", "assistance", "atv_v2",
    "auto_racing", "backcountry_skiing", "backcountry_skiing_snowboarding_ws",
    "backcountry_snowboarding", "badminton", "baseball", "basketball", "biketoruntransition_v2",
    "bmx", "boating_v2", "bouldering", "boxing", "breathwork", "casual_walking", "ccr_diving",
    "cricket", "cross_country_indoor_skiing", "cross_country_skiing_ws", "cycling", "cyclocross",
    "dance", "disc_golf", "diving", "downhill_biking", "driving_general", "e_bike_fitness",
    "e_bike_mountain", "e_enduro_mtb", "e_sport", "elliptical", "enduro_mtb", "field_hockey",
    "fishing_v2", "fitness_equipment", "floor_climbing", "flying", "gauge_diving", "golf",
    "gravel_cycling", "hand_cycling", "hang_gliding", "hiit", "hiking", "horseback_riding",
    "hunting", "hunting_fishing", "ice_hockey", "incident_detected", "indoor_cardio",
    "indoor_climbing", "indoor_cycling", "indoor_hand_cycling", "indoor_rowing", "indoor_running",
    "inline_skating", "jump_rope", "kayaking_v2", "kiteboarding_v2", "lacrosse", "lap_swimming",
    "meditation", "mixed_martial_arts", "mobility", "motocross_v2", "motorcycling_v2",
    "mountain_biking", "mountaineering", "multi_gas_diving", "multi_sport", "obstacle_run",
    "offshore_grinding_v2", "onshore_grinding_v2", "open_water_swimming", "other", "overland",
    "paddelball", "paddling_v2", "para_sports", "pickleball", "pilates", "platform_tennis",
    "pool_apnea", "racket_sports", "racquetball", "rc_drone", "recumbent_cycling", "resort_skiing",
    "resort_skiing_snowboarding_ws", "resort_snowboarding", "road_biking", "rock_climbing",
    "rowing_v2", "rucking", "rugby", "running", "runtobiketransition_v2", "safety", "sailing_v2",
    "single_gas_diving", "skate_skiing_ws", "skating_ws", "sky_diving", "snorkeling",
    "snow_shoe_ws", "snowmobiling_ws", "soccer", "softball", "speed_walking", "squash",
    "stair_climbing", "stand_up_paddleboarding_v2", "steps", "stop_watch", "street_running",
    "strength_training", "surfing_v2", "swimming", "swimtobiketransition_v2", "table_tennis",
    "team_sports", "tennis_v2", "track_cycling", "track_running", "trail_running", "transition_v2",
    "treadmill_running", "ultimate_disc", "ultra_run", "virtual_ride", "virtual_run", "volleyball",
    "wakeboarding_v2", "wakesurfing", "walking", "water_sports", "water_tubing", "waterskiing",
    "wheelchair_push_run", "wheelchair_push_walk", "whitewater_rafting_kayaking",
    "whitewater_rafting_v2", "wind_kite_surfing", "windsurfing_v2", "wingsuit_flying",
    "winter_sports", "yoga",
];

/// Python's `str.title()`: a letter is upper-cased when the character before
/// it is not a letter, so "atv v2" becomes "Atv V2".
pub fn python_title(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_is_letter = false;
    for ch in text.chars() {
        if ch.is_alphabetic() {
            if previous_is_letter {
                out.extend(ch.to_lowercase());
            } else {
                out.extend(ch.to_uppercase());
            }
            previous_is_letter = true;
        } else {
            out.push(ch);
            previous_is_letter = false;
        }
    }
    out
}

/// The taxonomy the way the real Garmin client reports it: labels derived
/// from the key, sorted by label.
pub fn garmin_activity_types() -> Vec<Value> {
    let mut types: Vec<(String, &str)> = GARMIN_ACTIVITY_TYPE_KEYS
        .iter()
        .map(|key| (python_title(&key.replace('_', " ")), *key))
        .collect();
    types.sort();
    types.into_iter().map(|(label, key)| json!({"type_key": key, "label": label})).collect()
}

pub enum GarminLogin {
    Connected,
    MfaRequired,
}

pub fn begin_garmin_login(_email: &str, password: &str) -> GarminLogin {
    if password == MFA_TRIGGER_PASSWORD {
        GarminLogin::MfaRequired
    } else {
        GarminLogin::Connected
    }
}

pub fn complete_garmin_login(code: &str) -> Result<(), String> {
    if code == MFA_REJECT_CODE {
        Err("Invalid MFA code (dev mock rejects 000000)".into())
    } else {
        Ok(())
    }
}

/// Skips Strava entirely: bounces straight back to our own callback with a
/// placeholder code, echoing the state as the real Strava would.
pub fn strava_authorize_url(redirect_uri: &str, state: &str) -> String {
    let separator = if redirect_uri.contains('?') { '&' } else { '?' };
    format!("{redirect_uri}{separator}code=dev-mock-code&state={state}")
}

pub fn strava_tokens(granted_scope: Option<&str>, now_epoch: i64) -> Value {
    let mut tokens = json!({
        "access_token": "dev-mock-access-token",
        "refresh_token": "dev-mock-refresh-token",
        "expires_at": now_epoch + 6 * 3600,
    });
    if let Some(scope) = granted_scope {
        tokens["scope"] = json!(scope);
    }
    tokens
}

const REMOTE_ACTIVITIES_KEY: &str = "dev_garmin_remote_activities";
const DELETED_ACTIVITIES_KEY: &str = "dev_garmin_deleted_activity_ids";
pub const STRAVA_ID_BASE: i64 = 9_000_000;
const GARMIN_TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

const METRIC_KEYS: [&str; 20] = [
    "distance", "duration", "moving_duration", "elapsed_duration", "elevation_gain", "elevation_loss",
    "calories", "avg_hr", "max_hr", "avg_power", "max_power", "norm_power", "aerobic_te", "anaerobic_te",
    "training_load", "avg_cadence", "max_cadence", "total_sets", "total_reps", "total_volume",
];

fn remote_activities(conn: &rusqlite::Connection) -> DbResult<Map<String, Value>> {
    Ok(match db::get_config_value(conn, REMOTE_ACTIVITIES_KEY)? {
        Some(Value::Object(activities)) => activities,
        _ => Map::new(),
    })
}

fn deleted_ids(conn: &rusqlite::Connection) -> DbResult<HashSet<i64>> {
    Ok(match db::get_config_value(conn, DELETED_ACTIVITIES_KEY)? {
        Some(Value::Array(ids)) => ids.iter().filter_map(|id| id.as_i64().or_else(|| id.as_str()?.parse().ok())).collect(),
        _ => HashSet::new(),
    })
}

fn garmin_data(raw: &str) -> Map<String, Value> {
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(data)) => data,
        _ => Map::new(),
    }
}

fn text(data: &Map<String, Value>, key: &str) -> String {
    data.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn record(identity: (i64, String, String, String, String), data: &Map<String, Value>) -> ActivityRecord {
    let (garmin_activity_id, activity_type, title, description, start_time) = identity;
    let n = |key: &str| match data.get(key) {
        Some(Value::Number(n)) => Some(n.clone()),
        _ => None,
    };
    ActivityRecord {
        garmin_activity_id,
        activity_type,
        title,
        description,
        start_time,
        distance: n("distance"),
        duration: n("duration"),
        moving_duration: n("moving_duration"),
        elapsed_duration: n("elapsed_duration"),
        elevation_gain: n("elevation_gain"),
        elevation_loss: n("elevation_loss"),
        calories: n("calories"),
        avg_hr: n("avg_hr"),
        max_hr: n("max_hr"),
        avg_power: n("avg_power"),
        max_power: n("max_power"),
        norm_power: n("norm_power"),
        aerobic_te: n("aerobic_te"),
        anaerobic_te: n("anaerobic_te"),
        training_load: n("training_load"),
        avg_cadence: n("avg_cadence"),
        max_cadence: n("max_cadence"),
        total_sets: n("total_sets"),
        total_reps: n("total_reps"),
        total_volume: n("total_volume"),
    }
}

/// What the mock Garmin holds for an edit: the local row's fields, or a
/// stand-in strength workout for an id nothing knows.
fn remote_activity(conn: &rusqlite::Connection, activities: &Map<String, Value>, id: i64) -> DbResult<Map<String, Value>> {
    if let Some(Value::Object(existing)) = activities.get(&id.to_string()) {
        return Ok(existing.clone());
    }
    let mut activity = Map::new();
    activity.insert("garmin_activity_id".into(), json!(id));
    let Some(row) = db::get_activity(conn, id)? else {
        for (key, value) in [
            ("activity_type", json!("strength_training")),
            ("title", json!("Strength Training")),
            ("description", json!("")),
            ("start_time", json!(Utc::now().format(GARMIN_TIME_FORMAT).to_string())),
            ("duration", json!(3600.0)),
        ] {
            activity.insert(key.into(), value);
        }
        return Ok(activity);
    };
    activity.insert("activity_type".into(), json!(row.activity_type));
    activity.insert("title".into(), json!(row.title));
    activity.insert("description".into(), json!(row.description));
    activity.insert("start_time".into(), json!(row.start_time));
    let data = garmin_data(&row.garmin_data);
    for key in METRIC_KEYS {
        activity.insert(key.into(), data.get(key).cloned().unwrap_or(Value::Null));
    }
    Ok(activity)
}

/// The mock Garmin's metadata edit, kept as an overlay that later fetches
/// report, as the Python app's fake does.
pub fn update_garmin_metadata(conn: &rusqlite::Connection, id: i64, title: &str, description: &str) -> DbResult<()> {
    let mut activities = remote_activities(conn)?;
    let mut activity = remote_activity(conn, &activities, id)?;
    activity.insert("title".into(), json!(title));
    activity.insert("description".into(), json!(description));
    activities.insert(id.to_string(), Value::Object(activity));
    db::set_config_value(conn, REMOTE_ACTIVITIES_KEY, &Value::Object(activities))
}

/// `sync::edit_activity_metadata` against the mock accounts.
pub fn edit_activity_metadata(db: &Db, id: i64, title: &str, description: &str) -> SyncResult<()> {
    let (title, description) = (title.trim(), description.trim());
    if title.is_empty() {
        return Err(SyncError::Invalid("Activity title cannot be blank".into()));
    }
    let conn = db.lock();
    let row = db::get_activity(&conn, id)?.ok_or_else(|| SyncError::Invalid(format!("Activity {id} was not found")))?;
    update_garmin_metadata(&conn, id, title, description)?;
    let data = if row.garmin_data.is_empty() { "{}" } else { &row.garmin_data };
    let hash = sync::content_hash(title, description, &row.activity_type, data);
    db::update_activity_metadata(&conn, id, title, description, &hash)?;
    Ok(())
}

/// The local table echoed back as what the mock Garmin holds, the overlay of
/// edits winning, minus anything deleted.
fn fetch_recent_activities(conn: &rusqlite::Connection, lookback_days: i64, now: DateTime<Utc>) -> DbResult<Vec<ActivityRecord>> {
    let cutoff = (now - Duration::days(lookback_days)).format(GARMIN_TIME_FORMAT).to_string();
    let deleted = deleted_ids(conn)?;
    let mut records: Vec<ActivityRecord> = Vec::new();
    for row in db::list_activities(conn, false)? {
        if deleted.contains(&row.garmin_activity_id) || row.start_time < cutoff {
            continue;
        }
        let identity = (row.garmin_activity_id, row.activity_type, row.title, row.description, row.start_time);
        records.push(record(identity, &garmin_data(&row.garmin_data)));
    }
    for raw in remote_activities(conn)?.values() {
        let Some(raw) = raw.as_object() else { continue };
        let Some(id) = raw.get("garmin_activity_id").and_then(Value::as_i64) else { continue };
        let start_time = text(raw, "start_time");
        if deleted.contains(&id) || start_time < cutoff {
            continue;
        }
        let overlay = record((id, text(raw, "activity_type"), text(raw, "title"), text(raw, "description"), start_time), raw);
        match records.iter_mut().find(|r| r.garmin_activity_id == id) {
            Some(existing) => *existing = overlay,
            None => records.push(overlay),
        }
    }
    Ok(records)
}

/// `sync::sync_garmin` against the mock Garmin.
pub fn sync_garmin(
    db: &Db,
    cfg: &Config,
    now: DateTime<Utc>,
    lookback_days: Option<i64>,
    hold_before: Option<DateTime<Utc>>,
) -> SyncResult<GarminSyncStats> {
    let held_types: HashSet<String> = cfg.held_activity_types().into_iter().collect();
    let lookback = lookback_days.unwrap_or_else(|| cfg.int("lookback_days"));
    let hold_cutoff = hold_before.map(|at| at.format(GARMIN_TIME_FORMAT).to_string());
    let fetched = fetch_recent_activities(&db.lock(), lookback, now)?;

    let mut stats = GarminSyncStats::default();
    let mut fetched_ids = HashSet::new();
    let conn = db.lock();
    for act in &fetched {
        fetched_ids.insert(act.garmin_activity_id);
        let data = sync::garmin_data_json(act);
        let new_hash = sync::content_hash(&act.title, &act.description, &act.activity_type, &data);
        let Some(existing) = db::get_activity(&conn, act.garmin_activity_id)? else {
            let (status, hold_reason) = if hold_cutoff.as_deref().is_some_and(|cutoff| act.start_time.as_str() < cutoff) {
                stats.held_backlog += 1;
                ("held", Some(sync::HOLD_BACKLOG))
            } else if held_types.contains(&act.activity_type) {
                ("held", Some(sync::HOLD_CATEGORY))
            } else {
                ("pending", None)
            };
            let new = NewActivity {
                garmin_activity_id: act.garmin_activity_id,
                activity_type: &act.activity_type,
                title: &act.title,
                description: &act.description,
                start_time: &act.start_time,
                content_hash: &new_hash,
                publish_status: status,
                garmin_data: &data,
                hold_reason,
            };
            db::insert_activity(&conn, &new, now)?;
            stats.new += 1;
            continue;
        };
        if existing.content_hash == new_hash {
            continue;
        }
        db::update_activity_content(
            &conn,
            act.garmin_activity_id,
            &act.title,
            &act.description,
            &act.activity_type,
            &new_hash,
            &existing.publish_status,
            &data,
        )?;
        stats.updated += 1;
    }
    let window_start = (now - Duration::days(lookback) + Duration::minutes(5)).format(GARMIN_TIME_FORMAT).to_string();
    let mut gone: Vec<i64> = db::list_active_ids_since(&conn, &window_start)?.difference(&fetched_ids).copied().collect();
    gone.sort_unstable();
    for id in gone {
        db::mark_removed(&conn, id)?;
        stats.removed += 1;
    }
    drop(conn);
    sync::record_sync_outcome(db, now, None)?;
    Ok(stats)
}

/// `sync::check_strava_status` against the mock Strava, which reports back
/// exactly the activities this app published.
pub fn check_strava_status(db: &Db, cfg: &Config, now: DateTime<Utc>, lookback_days: Option<i64>) -> SyncResult<StatusCheckStats> {
    let mut stats = StatusCheckStats::default();
    let lookback = lookback_days.unwrap_or_else(|| cfg.int("lookback_days"));
    let cutoff = now - Duration::days(lookback);
    let rows_within = |statuses: &[&str]| -> SyncResult<Vec<(db::ActivityRow, DateTime<Utc>)>> {
        let conn = db.lock();
        let mut rows = Vec::new();
        for status in statuses {
            for row in db::list_activities_with_status(&conn, status)? {
                let start = crate::timefmt::parse_garmin(&row.start_time).ok_or_else(|| {
                    SyncError::Invalid(format!("time data {:?} does not match format '{GARMIN_TIME_FORMAT}'", row.start_time))
                })?;
                if start >= cutoff {
                    rows.push((row, start));
                }
            }
        }
        Ok(rows)
    };
    let to_link = rows_within(&["pending", "held"])?;
    let to_verify = rows_within(&["published"])?;
    if to_link.is_empty() && to_verify.is_empty() {
        return Ok(stats);
    }
    let padding = Duration::minutes(30);
    let (after, before) = (cutoff - padding, now + padding);
    let conn = db.lock();
    let mut on_strava = Vec::new();
    for row in db::list_activities_with_status(&conn, "published")? {
        let Some(id) = row.strava_activity_id else { continue };
        let Some(start) = crate::timefmt::parse_garmin(&row.start_time) else { continue };
        if after <= start && start <= before {
            on_strava.push(StravaActivity { id, start_date: start });
        }
    }
    let mut unclaimed = on_strava.clone();
    for (row, start) in &to_link {
        let Some(existing) = strava_client::match_closest_activity(&unclaimed, *start, strava_client::DEFAULT_MATCH_TOLERANCE_MINUTES)
        else {
            continue;
        };
        unclaimed.retain(|a| a.id != existing);
        db::set_published(&conn, row.garmin_activity_id, existing, now)?;
        stats.linked_existing += 1;
    }
    let live: HashSet<i64> = on_strava.iter().map(|a| a.id).collect();
    for (row, _) in &to_verify {
        if !row.strava_activity_id.is_some_and(|id| live.contains(&id)) {
            db::set_publish_status(&conn, row.garmin_activity_id, "missing", None)?;
            stats.flagged_missing += 1;
        }
    }
    Ok(stats)
}

/// `sync::catch_up_sync` against the mock accounts.
pub fn catch_up_sync(db: &Db, cfg: &Config, now: DateTime<Utc>, last_sync_ok_at: Option<Option<String>>) -> SyncResult<CatchUpStats> {
    let (lookback, garmin_connected) = {
        let conn = db.lock();
        let lookback = sync::catch_up_lookback_days(&conn, cfg, now, last_sync_ok_at)?;
        (lookback, db::truthy(db::get_config_value(&conn, "garmin_credentials_verified")?.as_ref()))
    };
    let hold_before = now - Duration::days(cfg.int("lookback_days"));
    let garmin = if garmin_connected {
        sync_garmin(db, cfg, now, Some(lookback), Some(hold_before))?
    } else {
        GarminSyncStats::default()
    };
    let status_lookback = sync::strava_status_lookback_days(&db.lock(), cfg, now)?;
    let status = check_strava_status(db, cfg, now, Some(status_lookback))?;
    Ok(CatchUpStats { garmin, status, lookback_days: lookback })
}

/// `sync::publish_now` against the mock accounts.
pub fn publish_now(db: &Db, id: i64, now: DateTime<Utc>) -> SyncResult<()> {
    let conn = db.lock();
    if let Some((hevy_id, status)) = db::blocking_workout_for_activity(&conn, id)? {
        return Err(SyncError::Blocked(format!("hevy workout {hevy_id} is {status}")));
    }
    let row = db::get_activity(&conn, id)?.ok_or_else(|| SyncError::Invalid(format!("Activity {id} was not found")))?;
    if !row.is_published() {
        db::set_published(&conn, id, STRAVA_ID_BASE + id, now)?;
    }
    Ok(())
}

/// A manual `sync::publish_pending` against the mock accounts.
pub fn publish_selected(db: &Db, selected: &HashSet<i64>, now: DateTime<Utc>) -> SyncResult<PublishStats> {
    let mut stats = PublishStats::default();
    for status in ["pending", "missing", "held"] {
        let rows = db::list_activities_with_status(&db.lock(), status)?;
        for row in rows.iter().filter(|r| selected.contains(&r.garmin_activity_id)) {
            match publish_now(db, row.garmin_activity_id, now) {
                Ok(()) => stats.published += 1,
                Err(SyncError::Blocked(reason)) => {
                    stats.blocked += 1;
                    stats.blocked_reasons.push(reason);
                }
                Err(_) => stats.failed += 1,
            }
        }
    }
    Ok(stats)
}

/// The exercise templates the mock Hevy account holds.
pub fn hevy_templates() -> Vec<Value> {
    let mut templates: Vec<Value> = [
        ("tpl-dev-bench", "Bench Press (Barbell)", "chest", "barbell"),
        ("tpl-dev-squat", "Squat (Barbell)", "quadriceps", "barbell"),
        ("tpl-dev-deadlift", "Deadlift (Barbell)", "hamstrings", "barbell"),
        ("tpl-dev-curl", "Bicep Curl (Dumbbell)", "biceps", "dumbbell"),
        ("tpl-dev-pulldown", "Lat Pulldown (Cable)", "lats", "cable"),
        ("tpl-dev-ohp", "Overhead Press (Barbell)", "shoulders", "barbell"),
        ("tpl-dev-plank", "Plank", "abdominals", "none"),
        ("tpl-dev-pullup", "Pull Up", "lats", "none"),
        ("tpl-dev-rdl", "Romanian Deadlift (Barbell)", "hamstrings", "barbell"),
        (crate::hevy_seed::OVERRIDDEN_TEMPLATE_ID, "Triceps Pushdown (Cable)", "triceps", "cable"),
    ]
    .into_iter()
    .map(|(id, title, muscle, equipment)| {
        json!({"id": id, "title": title, "primary_muscle_group": muscle, "secondary_muscle_groups": [],
               "equipment_category": equipment, "is_custom": false})
    })
    .collect();
    templates.push(json!({"id": crate::hevy_seed::CUSTOM_TEMPLATE_ID, "title": "Bulgarian Ring Row",
        "primary_muscle_group": "upper_back", "secondary_muscle_groups": ["biceps"],
        "equipment_category": "other", "is_custom": true}));
    templates
}
