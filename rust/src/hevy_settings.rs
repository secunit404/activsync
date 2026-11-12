use rusqlite::Connection;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::Config;
use crate::db::{self, DbResult};
use crate::garmin_errors::py_str;
use crate::hevy_profile::{self, object};
use crate::view;

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

fn identity_display(detected: &Map<String, Value>) -> String {
    if detected.is_empty() {
        return "not yet detected — set automatically from your watch".into();
    }
    let part = |key: &str| detected.get(key).map_or_else(|| "None".into(), py_str);
    let numbers = format!(
        "{}/{}/{}",
        part("manufacturer"),
        part("product"),
        part("serial")
    );
    if db::truthy(detected.get("product")) {
        format!("detected from your watch: {numbers}")
    } else {
        format!("generic Garmin fallback: {numbers}")
    }
}

/// A stored object setting, or the default when it is missing or unreadable.
fn setting<T: DeserializeOwned + Default>(settings: &Map<String, Value>, key: &str) -> T {
    settings
        .get(key)
        .and_then(|value| T::deserialize(value).ok())
        .unwrap_or_default()
}

/// The settings page's Hevy card. The detected device identity and the hand
/// override are kept apart, so a save with a blank override keeps detection.
pub fn hevy_settings_state(conn: &Connection, cfg: &Config) -> DbResult<HevySettingsState> {
    let settings = object(db::get_config_value(conn, "settings")?);
    let api_key_saved = db::truthy(db::get_config_value(conn, "hevy_api_key")?.as_ref());
    let (connected, status) = view::hevy_connection(conn)?;

    let cache = object(db::get_config_value(conn, hevy_profile::CACHE_KEY)?);
    let cached = |key: &str| cache.get(key).filter(|v| !v.is_null());
    let profile_from_garmin = ["weight_kg", "birth_year", "vo2max", "sex"]
        .iter()
        .any(|k| cached(k).is_some());
    let baseline = ProfileBaseline {
        weight_kg: cached("weight_kg")
            .and_then(Value::as_f64)
            .unwrap_or(hevy_profile::DEFAULT_WEIGHT_KG),
        birth_year: cached("birth_year")
            .and_then(Value::as_i64)
            .unwrap_or(hevy_profile::DEFAULT_BIRTH_YEAR),
        vo2max: cached("vo2max")
            .and_then(Value::as_f64)
            .unwrap_or(hevy_profile::DEFAULT_VO2MAX),
        sex: cached("sex")
            .and_then(Value::as_str)
            .unwrap_or(hevy_profile::DEFAULT_SEX)
            .to_string(),
    };
    let empty = Map::new();
    let detected = settings
        .get("hevy_device_identity")
        .and_then(Value::as_object)
        .unwrap_or(&empty);

    Ok(HevySettingsState {
        connected,
        status,
        api_key_saved,
        enabled: db::truthy(cfg.get("hevy_enabled")),
        watch_strategy: cfg.str("hevy_watch_strategy"),
        match_mode: cfg.str("hevy_match_mode"),
        title_template: cfg.str("hevy_title_template"),
        description_template: cfg.str("hevy_description_template"),
        summary_on_structured: cfg.summary_on_structured(),
        grace_minutes: cfg.int("hevy_grace_minutes"),
        poll_interval_minutes: cfg.int("hevy_poll_interval_minutes"),
        identity: setting(&settings, "hevy_device_identity_override"),
        identity_display: identity_display(detected),
        profile_override: setting(&settings, PROFILE_OVERRIDE_KEY),
        profile_baseline: baseline,
        profile_from_garmin,
    })
}

/// One `{...}` replacement field as `string.Formatter().parse` reports it.
struct Field {
    name: String,
    has_options: bool,
}

/// Walks a `str.format` template the way CPython's `MarkupIterator` does,
/// one piece at a time, so a parse error surfaces only after the fields
/// before it have been checked.
struct Markup {
    chars: Vec<char>,
    pos: usize,
}

impl Markup {
    fn next_field(&mut self) -> Option<Result<Option<Field>, String>> {
        if self.pos >= self.chars.len() {
            return None;
        }
        let mut last = '\0';
        let mut markup_follows = false;
        while self.pos < self.chars.len() {
            last = self.chars[self.pos];
            self.pos += 1;
            if last == '{' || last == '}' {
                markup_follows = true;
                break;
            }
        }
        let at_end = self.pos >= self.chars.len();
        if last == '}' && (at_end || self.chars[self.pos] != last) {
            return Some(Err("Single '}' encountered in format string".into()));
        }
        if at_end && last == '{' {
            return Some(Err("Single '{' encountered in format string".into()));
        }
        if !at_end && last == self.chars[self.pos] {
            self.pos += 1;
            markup_follows = false;
        }
        if !markup_follows {
            return Some(Ok(None));
        }
        Some(self.parse_field().map(Some))
    }

    fn parse_field(&mut self) -> Result<Field, String> {
        let start = self.pos;
        let mut last = '\0';
        while self.pos < self.chars.len() {
            last = self.chars[self.pos];
            self.pos += 1;
            match last {
                '{' => return Err("unexpected '{' in field name".into()),
                '[' => {
                    while self.pos < self.chars.len() && self.chars[self.pos] != ']' {
                        self.pos += 1;
                    }
                }
                '}' | ':' | '!' => break,
                _ => {}
            }
        }
        if !matches!(last, '}' | ':' | '!') {
            return Err("expected '}' before end of string".into());
        }
        let name: String = self.chars[start..self.pos - 1].iter().collect();
        if last == '}' {
            return Ok(Field {
                name,
                has_options: false,
            });
        }
        let mut has_conversion = false;
        if last == '!' {
            let Some(&conversion) = self.chars.get(self.pos) else {
                return Err("end of string while looking for conversion specifier".into());
            };
            self.pos += 1;
            has_conversion = conversion != '\0';
            if let Some(&next) = self.chars.get(self.pos) {
                self.pos += 1;
                if next == '}' {
                    return Ok(Field {
                        name,
                        has_options: has_conversion,
                    });
                }
                if next != ':' {
                    return Err("expected ':' after conversion specifier".into());
                }
            }
        }
        let spec_start = self.pos;
        let mut depth = 1;
        while let Some(&c) = self.chars.get(self.pos) {
            self.pos += 1;
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(Field {
                            name,
                            has_options: has_conversion || self.pos - 1 > spec_start,
                        });
                    }
                }
                _ => {}
            }
        }
        Err("unmatched '{' in format spec".into())
    }
}

fn is_blank(template: &str) -> bool {
    template
        .chars()
        .all(|c| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

fn validate(template: &str, allowed: &[&str], label: &str) -> Result<(), String> {
    if is_blank(template) {
        return Err(format!("{label} template cannot be empty."));
    }
    let lower = label.to_lowercase();
    let mut markup = Markup {
        chars: template.chars().collect(),
        pos: 0,
    };
    while let Some(piece) = markup.next_field() {
        let Some(field) = piece.map_err(|err| format!("Invalid {lower} template: {err}"))? else {
            continue;
        };
        if !allowed.contains(&field.name.as_str()) {
            return Err(format!("Unknown {lower} placeholder: {{{}}}.", field.name));
        }
        if field.has_options {
            return Err(format!(
                "{label} placeholders cannot use formatting options."
            ));
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
        assert_eq!(
            validate_title_template(" "),
            Err("Title template cannot be empty.".into())
        );
        assert_eq!(
            validate_title_template("{name}"),
            Err("Unknown title placeholder: {name}.".into())
        );
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
