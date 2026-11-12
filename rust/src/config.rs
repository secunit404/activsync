use rusqlite::Connection;
use serde_json::{Map, Value, json};

use crate::db::{self, DbResult};

pub const DEFAULT_TITLE_TEMPLATE: &str = "{clean_title}";
pub const DEFAULT_TEMPLATE: &str = "{duration}\n{calories}\n{avg_hr}\n\n{exercises}\n\n{marker}";
const LEGACY_DEFAULT_TEMPLATE: &str = "🏋️ {title}\n{duration}\n{calories}\n{avg_hr}\n\n{exercises}\n\n{marker}";

pub fn defaults() -> Map<String, Value> {
    let Value::Object(map) = json!({
        "garmin_poll_interval_minutes": 20,
        "strava_poll_interval_minutes": 5,
        "lookback_days": 7,
        "undo_delay_seconds": 5,
        "display_timezone": "Europe/Stockholm",
        "hevy_enabled": false,
        "hevy_match_mode": "automatic",
        "hevy_title_template": DEFAULT_TITLE_TEMPLATE,
        "hevy_description_template": DEFAULT_TEMPLATE,
        "hevy_summary_on_structured": true,
        "hevy_watch_strategy": "replace",
        "hevy_poll_interval_minutes": 5,
        "hevy_grace_minutes": 120,
        "hevy_device_identity": null,
        "held_activity_types": [],
    }) else {
        unreachable!()
    };
    map
}

#[derive(Debug, Clone)]
pub struct Config(Map<String, Value>);

impl Config {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn set(&mut self, key: &str, value: Value) {
        self.0.insert(key.to_string(), value);
    }

    pub fn remove(&mut self, key: &str) {
        self.0.remove(key);
    }

    pub fn str(&self, key: &str) -> String {
        self.0.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
    }

    pub fn display_timezone(&self) -> String {
        self.0
            .get("display_timezone")
            .and_then(Value::as_str)
            .unwrap_or("Europe/Stockholm")
            .to_string()
    }

    pub fn int(&self, key: &str) -> i64 {
        self.0
            .get(key)
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.trunc() as i64)))
            .unwrap_or_default()
    }

    pub fn held_activity_types(&self) -> Vec<String> {
        self.0
            .get("held_activity_types")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }
}

/// Defaults overlaid with the stored `settings` blob. In development mode a
/// missing match mode reads as "review", as in the Python app.
pub fn load_config(conn: &Connection, development: bool) -> DbResult<Config> {
    let mut cfg = defaults();
    let stored = match db::get_config_value(conn, "settings")? {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    if development && !stored.contains_key("hevy_match_mode") {
        cfg.insert("hevy_match_mode".into(), json!("review"));
    }
    cfg.extend(stored);
    if cfg.get("hevy_description_template").and_then(Value::as_str) == Some(LEGACY_DEFAULT_TEMPLATE) {
        cfg.insert("hevy_description_template".into(), json!(DEFAULT_TEMPLATE));
    }
    Ok(Config(cfg))
}

/// Load, change and save the settings blob as one unit. The caller holds the
/// connection lock for the whole call, so no other writer can interleave.
pub fn edit(conn: &Connection, development: bool, change: impl FnOnce(&mut Config)) -> DbResult<()> {
    let mut cfg = load_config(conn, development)?;
    change(&mut cfg);
    db::set_config_value(conn, "settings", &Value::Object(cfg.0))
}
