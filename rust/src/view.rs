use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::db::{self, ActivityRow, DbResult};
use crate::timefmt;

pub const GARMIN_ACTIVITY_URL: &str = "https://connect.garmin.com/modern/activity/";
pub const STRAVA_ACTIVITY_URL: &str = "https://www.strava.com/activities/";
const REQUIRED_STRAVA_SCOPES: [&str; 2] = ["activity:read_all", "activity:write"];

pub fn strava_url_for(row: &ActivityRow) -> Option<String> {
    row.is_published()
        .then(|| format!("{STRAVA_ACTIVITY_URL}{}", row.strava_activity_id.unwrap_or_default()))
}

/// `None` means the grant was never reported (tokens stored before scopes
/// were recorded); only a reported, narrower grant is a problem.
pub fn scope_is_sufficient(granted: Option<&str>) -> bool {
    match granted {
        None => true,
        Some(scope) => {
            let granted: Vec<&str> = scope.split(',').collect();
            REQUIRED_STRAVA_SCOPES.iter().all(|s| granted.contains(s))
        }
    }
}

fn number(data: &Map<String, Value>, key: &str) -> Option<f64> {
    data.get(key).and_then(Value::as_f64)
}

fn present(data: &Map<String, Value>, key: &str) -> Option<f64> {
    number(data, key).filter(|n| *n != 0.0)
}

fn fmt_duration(seconds: Option<f64>) -> String {
    let Some(seconds) = seconds else {
        return String::new();
    };
    let total = seconds.trunc() as i64;
    let (h, rest) = (total.div_euclid(3600), total.rem_euclid(3600));
    let (m, s) = (rest / 60, rest % 60);
    if h != 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else {
        format!("{m}m {s:02}s")
    }
}

/// Whole-minute duration for summary tiles: "7h 42m", "42m", "0m".
pub fn fmt_duration_coarse(seconds: f64) -> String {
    let minutes = (seconds.trunc() as i64).div_euclid(60);
    let (h, m) = (minutes.div_euclid(60), minutes.rem_euclid(60));
    if h != 0 { format!("{h}h {m:02}m") } else { format!("{m}m") }
}

fn fmt_distance(metres: Option<f64>) -> String {
    metres.map(|m| format!("{:.2} km", m / 1000.0)).unwrap_or_default()
}

fn fmt_pace(distance: Option<f64>, duration: Option<f64>) -> String {
    let (Some(distance), Some(duration)) = (distance.filter(|d| *d != 0.0), duration.filter(|d| *d != 0.0)) else {
        return String::new();
    };
    let km = distance / 1000.0;
    if km < 0.01 {
        return String::new();
    }
    let per_km = (duration / km).trunc() as i64;
    format!("{}:{:02} /km", per_km.div_euclid(60), per_km.rem_euclid(60))
}

fn fmt_speed(distance: Option<f64>, duration: Option<f64>) -> String {
    let (Some(distance), Some(duration)) = (distance.filter(|d| *d != 0.0), duration.filter(|d| *d != 0.0)) else {
        return String::new();
    };
    let hours = duration / 3600.0;
    if hours < 0.001 {
        return String::new();
    }
    format!("{:.1} km/h", distance / 1000.0 / hours)
}

fn with_unit(value: Option<f64>, unit: &str) -> String {
    value.map(|v| format!("{}{unit}", v.trunc() as i64)).unwrap_or_default()
}

/// Python's `str()` of a JSON number: ints stay ints, floats keep ".0".
fn python_str(data: &Map<String, Value>, key: &str) -> String {
    match data.get(key) {
        Some(Value::Number(n)) if n.as_f64().is_some_and(|f| f != 0.0) => match n.as_i64() {
            Some(i) => i.to_string(),
            None => {
                let f = n.as_f64().unwrap_or_default();
                if f.fract() == 0.0 { format!("{f:.1}") } else { f.to_string() }
            }
        },
        _ => String::new(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDetail {
    pub description: Option<String>,
    pub distance: String,
    pub duration: String,
    pub moving_time: String,
    pub elapsed_time: String,
    pub pace: String,
    pub speed: String,
    pub elev_gain: String,
    pub elev_loss: String,
    pub calories: String,
    pub avg_hr: String,
    pub max_hr: String,
    pub avg_power: String,
    pub max_power: String,
    pub norm_power: String,
    pub aerobic_te: String,
    pub anaerobic_te: String,
    pub training_load: String,
    pub avg_cadence: String,
    pub max_cadence: String,
    pub total_sets: String,
    pub total_reps: String,
    pub total_volume: String,
}

#[derive(Debug, Clone)]
pub struct ActivityView {
    pub row: ActivityRow,
    pub duration_seconds: Option<f64>,
    pub start_date_display: String,
    pub start_month_year_display: String,
    pub start_clock_display: String,
    pub garmin_url: String,
    pub hevy_badge: Option<String>,
    pub detail: ActivityDetail,
}

fn parse_garmin_data(raw: &str) -> Map<String, Value> {
    match serde_json::from_str(raw) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

pub fn detail_for(row: &ActivityRow) -> (Option<f64>, ActivityDetail) {
    let gd = parse_garmin_data(&row.garmin_data);
    let duration = number(&gd, "duration");
    let distance = number(&gd, "distance");
    let detail = ActivityDetail {
        description: Some(row.description.clone()).filter(|d| !d.is_empty()),
        distance: fmt_distance(distance),
        duration: fmt_duration(duration),
        moving_time: fmt_duration(number(&gd, "moving_duration")),
        elapsed_time: fmt_duration(number(&gd, "elapsed_duration")),
        pace: fmt_pace(distance, duration),
        speed: fmt_speed(distance, duration),
        elev_gain: with_unit(number(&gd, "elevation_gain"), " m"),
        elev_loss: with_unit(number(&gd, "elevation_loss"), " m"),
        calories: with_unit(present(&gd, "calories"), ""),
        avg_hr: with_unit(number(&gd, "avg_hr"), " bpm"),
        max_hr: with_unit(number(&gd, "max_hr"), " bpm"),
        avg_power: with_unit(present(&gd, "avg_power"), " W"),
        max_power: with_unit(present(&gd, "max_power"), " W"),
        norm_power: with_unit(present(&gd, "norm_power"), " W"),
        aerobic_te: present(&gd, "aerobic_te").map(|v| format!("{v:.1}")).unwrap_or_default(),
        anaerobic_te: present(&gd, "anaerobic_te").map(|v| format!("{v:.1}")).unwrap_or_default(),
        training_load: present(&gd, "training_load").map(|v| format!("{v:.0}")).unwrap_or_default(),
        avg_cadence: with_unit(present(&gd, "avg_cadence"), " spm"),
        max_cadence: with_unit(present(&gd, "max_cadence"), " spm"),
        total_sets: python_str(&gd, "total_sets"),
        total_reps: python_str(&gd, "total_reps"),
        total_volume: present(&gd, "total_volume").map(|v| format!("{v:.0} kg")).unwrap_or_default(),
    };
    (duration, detail)
}

pub fn activities_view(conn: &Connection, oldest_first: bool, tz_name: &str) -> DbResult<Vec<ActivityView>> {
    let tz = timefmt::parse_tz(tz_name);
    let badges = db::hevy_badges(conn)?;
    let rows = db::list_activities(conn, oldest_first)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let (duration_seconds, detail) = detail_for(&row);
            ActivityView {
                duration_seconds,
                start_date_display: timefmt::format_local_date(&row.start_time, tz),
                start_month_year_display: timefmt::format_local_month_year(&row.start_time, tz),
                start_clock_display: timefmt::format_local_clock(&row.start_time, tz),
                garmin_url: format!("{GARMIN_ACTIVITY_URL}{}", row.garmin_activity_id),
                hevy_badge: badges.get(&row.garmin_activity_id).cloned(),
                detail,
                row,
            }
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GarminConnection {
    pub connected: bool,
    pub status: String,
    pub meta: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceConnection {
    pub connected: bool,
    pub status: String,
    pub meta: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connections {
    pub garmin: GarminConnection,
    pub strava: ServiceConnection,
    pub broken: Vec<&'static str>,
}

fn garmin_status(conn: &Connection, now: DateTime<Utc>) -> DbResult<(String, String)> {
    let Some(last) = db::get_config_value(conn, "garmin_last_sync_at")? else {
        return Ok(("Not yet synced".into(), String::new()));
    };
    let synced_at = last
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    let Some(synced_at) = synced_at else {
        return Ok(("Not yet synced".into(), String::new()));
    };
    if db::truthy(db::get_config_value(conn, "garmin_last_sync_ok")?.as_ref()) {
        let minutes = ((now - synced_at).num_seconds().div_euclid(60)).max(0);
        return Ok(("Connected".into(), format!("last synced {minutes} min ago")));
    }
    let error = db::get_config_value(conn, "garmin_last_sync_error")?
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown error".into());
    Ok((
        "Needs attention".into(),
        format!("last attempt at {} failed: {error}", synced_at.format("%H:%M")),
    ))
}

/// Whether each service is usable. `garmin_credentials_verified` is the
/// verdict; the sync status only adds detail.
pub fn connection_status(conn: &Connection, now: DateTime<Utc>) -> DbResult<Connections> {
    let creds = db::get_config_value(conn, "garmin_credentials")?.unwrap_or(Value::Null);
    let garmin_connected = db::truthy(db::get_config_value(conn, "garmin_credentials_verified")?.as_ref());
    let tokens = db::get_config_value(conn, "strava_tokens")?.unwrap_or(Value::Null);
    let strava_connected = db::truthy(tokens.get("refresh_token"));
    let strava_scope_ok = scope_is_sufficient(tokens.get("scope").and_then(Value::as_str));

    let (garmin_line, garmin_meta) = if garmin_connected {
        garmin_status(conn, now)?
    } else {
        ("Disconnected — sync paused".into(), String::new())
    };
    let mut broken = Vec::new();
    if !garmin_connected {
        broken.push("garmin");
    }
    if !(strava_connected && strava_scope_ok) {
        broken.push("strava");
    }
    let strava_status = if !strava_connected {
        "Disconnected — publishing paused"
    } else if !strava_scope_ok {
        "Reconnect needed — Strava did not grant upload access"
    } else {
        "Connected"
    };
    Ok(Connections {
        garmin: GarminConnection {
            connected: garmin_connected,
            status: garmin_line,
            meta: garmin_meta,
            email: creds.get("email").and_then(Value::as_str).unwrap_or_default().to_string(),
        },
        strava: ServiceConnection {
            connected: strava_connected,
            status: strava_status.into(),
            meta: String::new(),
        },
        broken,
    })
}

/// The Hevy card's connection line.
pub fn hevy_connection(conn: &Connection) -> DbResult<(bool, String)> {
    let api_key = db::truthy(db::get_config_value(conn, "hevy_api_key")?.as_ref());
    let auth_ok = db::get_config_value(conn, "hevy_auth_ok")?;
    let rejected = matches!(auth_ok, Some(Value::Bool(false)));
    let status = if !api_key {
        "Not connected"
    } else if rejected {
        "Needs attention — Hevy rejected the API key"
    } else {
        "Connected"
    };
    Ok((api_key && !rejected, status.into()))
}
