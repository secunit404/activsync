use serde_json::{Map, Value};

use std::sync::LazyLock;

use regex::Regex;

use crate::config::{DEFAULT_TEMPLATE, DEFAULT_TITLE_TEMPLATE};
use crate::hevy_settings::{validate_description_template, validate_title_template};
use crate::timefmt::parse_timestamp;

fn get<'a>(item: &'a Value, key: &str) -> Option<&'a Value> {
    item.get(key).filter(|v| !v.is_null())
}

fn truthy(value: Option<&Value>) -> bool {
    crate::db::truthy(value)
}

fn number(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0)
}

/// Python's `str()` of a JSON value.
fn py_str(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::String(s) => s.clone(),
        Value::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap_or_default();
            if f.fract() == 0.0 && f.abs() < 1e16 {
                format!("{f:.1}")
            } else {
                f.to_string()
            }
        }
        other => other.to_string(),
    }
}

/// Python `==`, where `8` and `8.0` are the same value.
fn py_eq(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) if !a.is_boolean() && !b.is_boolean() => x == y,
        _ => a == b,
    }
}

fn sets_label(n: usize) -> &'static str {
    if n == 1 { "set" } else { "sets" }
}

fn exercise_summary(workout: &Value) -> String {
    let empty = Vec::new();
    let exercises = get(workout, "exercises")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let mut lines = Vec::new();
    for exercise in exercises {
        let name = match exercise.get("title") {
            Some(title) if truthy(Some(title)) => py_str(title),
            _ => exercise
                .get("name")
                .map(py_str)
                .unwrap_or_else(|| "Unknown".into()),
        };
        let all_sets = get(exercise, "sets")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        let is_type =
            |item: &Value, kind: &str| item.get("type").and_then(Value::as_str) == Some(kind);
        let warmup: Vec<&Value> = all_sets.iter().filter(|s| is_type(s, "warmup")).collect();
        let working: Vec<&Value> = all_sets.iter().filter(|s| !is_type(s, "warmup")).collect();
        if working.is_empty() {
            if !warmup.is_empty() {
                lines.push(format!(
                    "• {name}: {} warmup {}",
                    warmup.len(),
                    sets_label(warmup.len())
                ));
            }
            continue;
        }
        let mut details = vec![format!("{} {}", working.len(), sets_label(working.len()))];
        let has_distance = working.iter().any(|s| truthy(s.get("distance_meters")));
        let has_duration = working.iter().any(|s| truthy(s.get("duration_seconds")));
        let has_weight = working
            .iter()
            .any(|s| truthy(s.get("weight_kg")) || truthy(s.get("weight")));
        if has_distance || (has_duration && !has_weight) {
            let distance: f64 = working
                .iter()
                .map(|s| number(s.get("distance_meters")))
                .sum();
            let duration: f64 = working
                .iter()
                .map(|s| number(s.get("duration_seconds")))
                .sum();
            if distance > 0.0 {
                details.push(format!("{:.1}km", distance / 1000.0));
            }
            if duration > 0.0 {
                details.push(format!("{}min", (duration / 60.0).floor() as i64));
            }
        } else {
            let weight = |s: &Value| {
                if truthy(s.get("weight_kg")) {
                    number(s.get("weight_kg"))
                } else {
                    number(s.get("weight"))
                }
            };
            let max_weight = working.iter().map(|s| weight(s)).fold(f64::MIN, f64::max);
            let zero = Value::from(0);
            let reps = working
                .iter()
                .map(|s| get(s, "reps").filter(|r| truthy(Some(r))).unwrap_or(&zero));
            let max_reps = reps.fold(&zero, |best, r| {
                if number(Some(r)) > number(Some(best)) {
                    r
                } else {
                    best
                }
            });
            details.push(format!("{max_weight:.1}kg × {}", py_str(max_reps)));
        }
        if !warmup.is_empty() {
            details.push(format!("{} warmup", warmup.len()));
        }
        for set_type in ["dropset", "failure"] {
            let count = working.iter().filter(|s| is_type(s, set_type)).count();
            if count > 0 {
                details.push(format!("{count} {set_type}"));
            }
        }
        for (key, label) in [("rpe", "RPE"), ("custom_metric", "metric")] {
            let mut values: Vec<&Value> = Vec::new();
            for value in all_sets.iter().filter_map(|s| get(s, key)) {
                if !values.iter().any(|seen| py_eq(seen, value)) {
                    values.push(value);
                }
            }
            if !values.is_empty() {
                let joined: Vec<String> = values.iter().map(|v| py_str(v)).collect();
                details.push(format!("{label} {}", joined.join(", ")));
            }
        }
        lines.push(format!("• {name}: {}", details.join(" - ")));
    }
    lines.join("\n")
}

fn duration_minutes(workout: &Value) -> i64 {
    let at = |key: &str| {
        workout
            .get(key)
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
    };
    match (at("start_time"), at("end_time")) {
        (Some(start), Some(end)) => ((end - start).num_seconds() / 60).max(0),
        _ => 0,
    }
}

/// `str.format_map` for a template that already passed validation.
fn render(template: &str, values: &Map<String, Value>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(pos) = rest.find(['{', '}']) {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
        } else if let Some(close) = tail.find('}').filter(|_| tail.starts_with('{')) {
            if let Some(value) = values.get(&tail[1..close]).and_then(Value::as_str) {
                out.push_str(value);
            }
            rest = &tail[close + 1..];
        } else {
            out.push_str(&tail[..1]);
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Python's `str.splitlines()` boundaries.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let boundary = matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if boundary {
            lines.push(&text[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' && chars.peek().map(|(_, n)| *n) == Some('\n') {
                chars.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

static EMOJI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("[\u{1F1E6}-\u{1FAFF}\u{2300}-\u{23FF}\u{2600}-\u{27BF}]").expect("emoji pattern")
});

/// Python's `str.isspace()`, which also counts the ASCII separators.
fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// The title without emoji presentation characters, ordinary text kept.
pub fn clean_title(value: &Value) -> String {
    let title = if truthy(Some(value)) {
        py_str(value)
    } else {
        String::new()
    };
    let title = EMOJI
        .replace_all(&title, "")
        .replace(['\u{fe0e}', '\u{fe0f}', '\u{200d}', '\u{20e3}'], "");
    let words: Vec<&str> = title.split(is_py_space).filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        "Workout".into()
    } else {
        words.join(" ")
    }
}

pub fn generate_title(workout: &Value, template: Option<&str>) -> Result<String, String> {
    let chosen = template
        .filter(|t| !t.is_empty())
        .unwrap_or(DEFAULT_TITLE_TEMPLATE);
    validate_title_template(chosen)?;
    let title = workout
        .get("title")
        .cloned()
        .unwrap_or_else(|| Value::from("Workout"));
    let mut values = Map::new();
    values.insert("title".into(), Value::String(py_str(&title)));
    values.insert("clean_title".into(), Value::String(clean_title(&title)));
    let rendered = render(chosen, &values);
    let trimmed = rendered.trim_matches(is_py_space);
    Ok(if trimmed.is_empty() {
        "Workout".into()
    } else {
        trimmed.into()
    })
}

/// Calories and heart rate come from the Garmin activity the workout is
/// applied to; the preview passes neither.
pub fn generate_description(
    workout: &Value,
    calories: Option<i64>,
    avg_hr: Option<i64>,
    template: Option<&str>,
) -> Result<String, String> {
    let chosen = template
        .filter(|t| !t.is_empty())
        .unwrap_or(DEFAULT_TEMPLATE);
    validate_description_template(chosen)?;
    let minutes = duration_minutes(workout);
    let mut values = Map::new();
    let duration = if minutes > 0 {
        format!("⏱️ {minutes} min")
    } else {
        String::new()
    };
    let calories = calories
        .filter(|c| *c != 0)
        .map(|c| format!("🔥 {c} kcal"))
        .unwrap_or_default();
    let avg_hr = avg_hr
        .filter(|hr| *hr != 0)
        .map(|hr| format!("❤️ avg {hr} bpm"))
        .unwrap_or_default();
    values.insert("duration".into(), Value::String(duration));
    values.insert("calories".into(), Value::String(calories));
    values.insert("avg_hr".into(), Value::String(avg_hr));
    values.insert("exercises".into(), Value::String(exercise_summary(workout)));
    values.insert(
        "marker".into(),
        Value::String("— synced by activsync".into()),
    );
    let rendered = render(chosen, &values);
    let mut cleaned: Vec<&str> = Vec::new();
    for raw in split_lines(&rendered) {
        let line = raw.trim_end();
        if !line.is_empty() || cleaned.last().is_some_and(|last| !last.is_empty()) {
            cleaned.push(line);
        }
    }
    while cleaned.last() == Some(&"") {
        cleaned.pop();
    }
    Ok(cleaned.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_python_for_mixed_sets() {
        let workout = json!({
            "start_time": "2026-11-10T10:00:00+00:00",
            "end_time": "2026-11-10T11:05:30Z",
            "exercises": [
                {"title": "Bench Press (Barbell)", "sets": [
                    {"type": "warmup", "weight_kg": 40, "reps": 10},
                    {"type": "normal", "weight_kg": 80.0, "reps": 5, "rpe": 8},
                    {"type": "failure", "weight_kg": 82.5, "reps": 3, "rpe": 9.5},
                    {"type": "dropset", "weight_kg": 60, "reps": 8, "rpe": 8.0}
                ]},
                {"title": "Row", "sets": [{"type": "normal", "distance_meters": 2000, "duration_seconds": 470}]},
                {"name": "Plank", "sets": [{"type": "normal", "duration_seconds": 90, "custom_metric": 2}]},
                {"title": "", "sets": [{"type": "warmup"}]}
            ]
        });
        assert_eq!(
            generate_description(&workout, None, None, None).unwrap(),
            "⏱️ 65 min\n\n• Bench Press (Barbell): 3 sets - 82.5kg × 8 - 1 warmup - 1 dropset - 1 failure - RPE 8, 9.5\n\
             • Row: 1 set - 2.0km - 7min\n• Plank: 1 set - 1min - metric 2\n• Unknown: 1 warmup set\n\n— synced by activsync"
        );
    }

    #[test]
    fn custom_template_collapses_blank_lines() {
        let workout = json!({"start_time": "bad", "exercises": []});
        assert_eq!(
            generate_description(
                &workout,
                None,
                None,
                Some("\n\n{{Gym}} {duration}\n{calories}\n\n\n{marker}  \n\n")
            )
            .unwrap(),
            "{Gym}\n\n— synced by activsync"
        );
    }
}
