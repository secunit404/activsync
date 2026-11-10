use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::config::Config;
use crate::db::{self, DbResult};
use crate::view;

const PROFILE_CACHE_KEY: &str = "garmin_user_profile";
pub const PROFILE_OVERRIDE_KEY: &str = "profile_override";
const TITLE_FIELDS: &[&str] = &["title", "clean_title"];
const DESCRIPTION_FIELDS: &[&str] = &["duration", "calories", "avg_hr", "exercises", "marker"];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceIdentity {
    pub manufacturer: Option<i64>,
    pub product: Option<i64>,
    pub serial: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfileOverride {
    #[serde(alias = "weight_kg")]
    pub weight_kg: Option<f64>,
    #[serde(alias = "birth_year")]
    pub birth_year: Option<i64>,
    pub vo2max: Option<f64>,
    pub sex: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileBaseline {
    weight_kg: f64,
    birth_year: i64,
    vo2max: f64,
    sex: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HevySettingsState {
    connected: bool,
    status: String,
    api_key_saved: bool,
    enabled: bool,
    watch_strategy: String,
    match_mode: String,
    title_template: String,
    description_template: String,
    summary_on_structured: bool,
    grace_minutes: i64,
    poll_interval_minutes: i64,
    identity: DeviceIdentity,
    identity_display: String,
    profile_override: ProfileOverride,
    profile_baseline: ProfileBaseline,
    profile_from_garmin: bool,
}

fn object(value: Option<Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

fn identity_display(detected: &Map<String, Value>) -> String {
    if detected.is_empty() {
        return "not yet detected — set automatically from your watch".into();
    }
    let part = |key: &str| match detected.get(key) {
        None | Some(Value::Null) => "None".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    };
    let numbers = format!("{}/{}/{}", part("manufacturer"), part("product"), part("serial"));
    if db::truthy(detected.get("product")) {
        format!("detected from your watch: {numbers}")
    } else {
        format!("generic Garmin fallback: {numbers}")
    }
}

/// The settings page's Hevy card. The detected device identity and the hand
/// override are kept apart, so a save with a blank override keeps detection.
pub fn hevy_settings_state(conn: &Connection, cfg: &Config) -> DbResult<HevySettingsState> {
    let settings = object(db::get_config_value(conn, "settings")?);
    let api_key_saved = db::truthy(db::get_config_value(conn, "hevy_api_key")?.as_ref());
    let (connected, status) = view::hevy_connection(conn)?;

    let cache = object(db::get_config_value(conn, PROFILE_CACHE_KEY)?);
    let cached = |key: &str| cache.get(key).filter(|v| !v.is_null());
    let profile_from_garmin = ["weight_kg", "birth_year", "vo2max", "sex"].iter().any(|k| cached(k).is_some());
    let baseline = ProfileBaseline {
        weight_kg: cached("weight_kg").and_then(Value::as_f64).unwrap_or(80.0),
        birth_year: cached("birth_year").and_then(Value::as_i64).unwrap_or(1990),
        vo2max: cached("vo2max").and_then(Value::as_f64).unwrap_or(45.0),
        sex: cached("sex").and_then(Value::as_str).unwrap_or("male").to_string(),
    };
    let parse = |key: &str| settings.get(key).cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));

    Ok(HevySettingsState {
        connected,
        status,
        api_key_saved,
        enabled: db::truthy(cfg.get("hevy_enabled")),
        watch_strategy: cfg.str("hevy_watch_strategy"),
        match_mode: cfg.str("hevy_match_mode"),
        title_template: cfg.str("hevy_title_template"),
        description_template: cfg.str("hevy_description_template"),
        summary_on_structured: db::truthy(cfg.get("hevy_summary_on_structured")),
        grace_minutes: cfg.int("hevy_grace_minutes"),
        poll_interval_minutes: cfg.int("hevy_poll_interval_minutes"),
        identity: serde_json::from_value(parse("hevy_device_identity_override")).unwrap_or_default(),
        identity_display: identity_display(&object(settings.get("hevy_device_identity").cloned())),
        profile_override: serde_json::from_value(parse(PROFILE_OVERRIDE_KEY)).unwrap_or_default(),
        profile_baseline: baseline,
        profile_from_garmin,
    })
}

struct Field {
    name: String,
    has_options: bool,
}

/// The placeholders in a `str.format` template, with Python's own error
/// messages for stray or unclosed braces.
fn parse_fields(template: &str) -> Result<Vec<Field>, String> {
    let chars: Vec<char> = template.chars().collect();
    let mut fields = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' if chars.get(i + 1) == Some(&'{') => i += 2,
            '}' if chars.get(i + 1) == Some(&'}') => i += 2,
            '}' => return Err("Single '}' encountered in format string".into()),
            '{' if i + 1 == chars.len() => return Err("Single '{' encountered in format string".into()),
            '{' => {
                let Some(close) = chars[i + 1..].iter().position(|c| *c == '}') else {
                    return Err("expected '}' before end of string".into());
                };
                let body: String = chars[i + 1..i + 1 + close].iter().collect();
                let name_end = body.find(['!', ':']).unwrap_or(body.len());
                fields.push(Field { name: body[..name_end].to_string(), has_options: name_end < body.len() });
                i += close + 2;
            }
            _ => i += 1,
        }
    }
    Ok(fields)
}

fn validate(template: &str, allowed: &[&str], label: &str) -> Result<(), String> {
    if template.trim().is_empty() {
        return Err(format!("{label} template cannot be empty."));
    }
    let lower = label.to_lowercase();
    let fields = parse_fields(template).map_err(|err| format!("Invalid {lower} template: {err}"))?;
    for field in fields {
        if !allowed.contains(&field.name.as_str()) {
            return Err(format!("Unknown {lower} placeholder: {{{}}}.", field.name));
        }
        if field.has_options {
            return Err(format!("{label} placeholders cannot use formatting options."));
        }
    }
    Ok(())
}

pub fn validate_title_template(template: &str) -> Result<(), String> {
    validate(template, TITLE_FIELDS, "Title")
}

pub fn validate_description_template(template: &str) -> Result<(), String> {
    validate(template, DESCRIPTION_FIELDS, "Description")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_follow_python_format_rules() {
        assert_eq!(validate_title_template("{clean_title} at the gym"), Ok(()));
        assert_eq!(validate_title_template("{{literal}} {title}"), Ok(()));
        assert_eq!(validate_title_template(" "), Err("Title template cannot be empty.".into()));
        assert_eq!(validate_title_template("{name}"), Err("Unknown title placeholder: {name}.".into()));
        assert_eq!(
            validate_description_template("{duration:>10}"),
            Err("Description placeholders cannot use formatting options.".into())
        );
        assert_eq!(
            validate_description_template("{duration"),
            Err("Invalid description template: expected '}' before end of string".into())
        );
        assert_eq!(
            validate_description_template("oops }"),
            Err("Invalid description template: Single '}' encountered in format string".into())
        );
    }
}
