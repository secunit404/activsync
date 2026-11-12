use chrono::{DateTime, Datelike, Timelike, Utc};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::{config, hevy_seed, mock};
use crate::db::{self, DbResult, NewActivity};

/// Bumped whenever the generated rows change, so an older dev database is
/// rebuilt instead of mixing two generations of sample data.
pub const SEED_VERSION: i64 = 3;
const SEED_VERSION_KEY: &str = "rust_dev_seed_version";

struct Sample {
    activity_type: &'static str,
    title: &'static str,
    description: &'static str,
    duration: i64,
    distance: Option<i64>,
    status: &'static str,
}

const fn sample(
    activity_type: &'static str,
    title: &'static str,
    description: &'static str,
    duration: i64,
    distance: Option<i64>,
    status: &'static str,
) -> Sample {
    Sample { activity_type, title, description, duration, distance, status }
}

const SAMPLES: [Sample; 13] = [
    sample("running", "Morning run", "Easy loop before work.", 42 * 60, Some(7_850), "pending"),
    sample("cycling", "Saturday ride", "Longer ride with a few good climbs.", 2 * 3600 + 8 * 60, Some(52_400), "held"),
    sample("strength_training", "Strength workout", "Upper body and core.", 54 * 60, None, "published"),
    sample("walking", "Evening walk", "Recovery walk.", 38 * 60, Some(3_120), "excluded"),
    sample("trail_running", "Forest trail run", "Technical trails and soft ground.", 68 * 60, Some(10_200), "pending"),
    sample("hiking", "Mountain hike", "Weekend hike with a view.", 4 * 3600 + 12 * 60, Some(14_600), "held"),
    sample("swimming", "Pool intervals", "Technique and steady intervals.", 47 * 60, None, "published"),
    sample("indoor_cycling", "Indoor bike", "Tempo session indoors.", 51 * 60, Some(28_700), "pending"),
    sample("yoga", "Yoga recovery", "Mobility and breathing session.", 36 * 60, None, "excluded"),
    sample("rowing", "Rowing intervals", "Short, hard intervals.", 32 * 60, Some(6_400), "pending"),
    sample("elliptical", "Cross trainer", "Low-impact aerobic session.", 44 * 60, None, "held"),
    sample("cardio", "Cardio workout", "Mixed cardio session.", 39 * 60, None, "published"),
    sample("backcountry_skiing_snowboarding_ws", "Backcountry day", "Skinned up, rode down.", 5 * 3600 + 20 * 60, Some(9_400), "published"),
];

/// Fills a mock database with a year of sample activities. When `onboarded`,
/// it is also past the setup wizard (`initial_sync_done`, nothing else), even
/// on a database that already holds the current sample data.
///
/// Refuses to touch a database holding any activity it did not generate, so
/// pointing the mock server at a real database cannot alter it.
pub fn seed(conn: &mut Connection, now: DateTime<Utc>, onboarded: bool) -> DbResult<bool> {
    let existing = db::list_activities(conn, false)?;
    if existing.iter().any(|row| !row.content_hash.starts_with("dev-")) {
        return Ok(false);
    }
    if onboarded {
        db::set_config_value(conn, "initial_sync_done", &json!(true))?;
    }
    if db::get_config_value(conn, SEED_VERSION_KEY)? == Some(json!(SEED_VERSION)) {
        return Ok(true);
    }
    let now = now.with_nanosecond(0).unwrap_or(now);
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM activities", [])?;
    for table in [
        "hevy_workouts",
        "hevy_operations",
        "hevy_match_schedule",
        "exercise_templates",
        "exercise_mappings",
        "merge_backups",
        "hevy_events_seen",
        "publish_schedule",
    ] {
        tx.execute(&format!("DELETE FROM {table}"), [])?;
    }

    for offset in 0..12_i32 {
        let month_index = now.month() as i32 - offset;
        let year = now.year() + (month_index - 1).div_euclid(12);
        let month = ((month_index - 1).rem_euclid(12) + 1) as u32;
        for variant in 0..3_i32 {
            let number = (offset * 3 + variant) as i64;
            let sample = &SAMPLES[number as usize % SAMPLES.len()];
            let start = now
                .with_day(1)
                .and_then(|d| d.with_year(year))
                .and_then(|d| d.with_month(month))
                .and_then(|d| d.with_day((5 + variant * 8).min(28) as u32))
                .and_then(|d| d.with_hour((7 + variant * 3) as u32))
                .expect("sample date is valid");
            let id = 900001 + number;
            let title = format!("{} #{}", sample.title, variant + 1);
            let data = json!({
                "duration": sample.duration,
                "moving_duration": sample.duration - 90,
                "elapsed_duration": sample.duration + 180,
                "distance": sample.distance,
                "elevation_gain": 120 + number * 18,
                "elevation_loss": 110 + number * 14,
                "calories": 420 + number * 28,
                "avg_hr": 142 - number % 8,
                "max_hr": 168 - number % 6,
            });
            let hold_reason = match (sample.status, offset >= 6) {
                ("held", true) => Some("backlog"),
                ("held", false) => Some("category"),
                _ => None,
            };
            db::insert_activity(
                &tx,
                &NewActivity {
                    garmin_activity_id: id,
                    activity_type: sample.activity_type,
                    title: &title,
                    description: sample.description,
                    start_time: &start.format("%Y-%m-%d %H:%M:%S").to_string(),
                    content_hash: &format!("dev-{id}"),
                    publish_status: sample.status,
                    garmin_data: &db::json_dumps(&data),
                    hold_reason,
                },
                now,
            )?;
        }
    }
    hevy_seed::seed(&tx, now)?;

    let mut settings = config::defaults();
    settings.insert("hevy_match_mode".into(), json!("review"));
    let mut held: Vec<&str> = SAMPLES.iter().filter(|s| s.status == "held").map(|s| s.activity_type).collect();
    held.sort_unstable();
    held.dedup();
    settings.insert("held_activity_types".into(), json!(held));
    db::set_config_value(&tx, "settings", &Value::Object(settings))?;
    db::set_config_value(&tx, "garmin_activity_types", &Value::Array(mock::garmin_activity_types()))?;
    db::set_config_value(&tx, SEED_VERSION_KEY, &json!(SEED_VERSION))?;
    tx.commit()?;
    Ok(true)
}
