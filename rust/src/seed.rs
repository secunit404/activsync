use chrono::{DateTime, Datelike, Timelike, Utc};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::db::{self, DbResult, NewActivity};
use crate::timefmt::GARMIN_TIME_FORMAT;
use crate::{config, hevy_seed, mock};

/// Bumped whenever the generated rows change, so an older dev database is
/// rebuilt instead of mixing two generations of sample data.
const SEED_VERSION: i64 = 3;
const SEED_VERSION_KEY: &str = "rust_dev_seed_version";

struct Sample {
    activity_type: &'static str,
    title: &'static str,
    description: &'static str,
    duration: i64,
    distance: Option<i64>,
    status: &'static str,
}

const SAMPLES: [Sample; 13] = [
    Sample {
        activity_type: "running",
        title: "Morning run",
        description: "Easy loop before work.",
        duration: 42 * 60,
        distance: Some(7_850),
        status: "pending",
    },
    Sample {
        activity_type: "cycling",
        title: "Saturday ride",
        description: "Longer ride with a few good climbs.",
        duration: 2 * 3600 + 8 * 60,
        distance: Some(52_400),
        status: "held",
    },
    Sample {
        activity_type: "strength_training",
        title: "Strength workout",
        description: "Upper body and core.",
        duration: 54 * 60,
        distance: None,
        status: "published",
    },
    Sample {
        activity_type: "walking",
        title: "Evening walk",
        description: "Recovery walk.",
        duration: 38 * 60,
        distance: Some(3_120),
        status: "excluded",
    },
    Sample {
        activity_type: "trail_running",
        title: "Forest trail run",
        description: "Technical trails and soft ground.",
        duration: 68 * 60,
        distance: Some(10_200),
        status: "pending",
    },
    Sample {
        activity_type: "hiking",
        title: "Mountain hike",
        description: "Weekend hike with a view.",
        duration: 4 * 3600 + 12 * 60,
        distance: Some(14_600),
        status: "held",
    },
    Sample {
        activity_type: "swimming",
        title: "Pool intervals",
        description: "Technique and steady intervals.",
        duration: 47 * 60,
        distance: None,
        status: "published",
    },
    Sample {
        activity_type: "indoor_cycling",
        title: "Indoor bike",
        description: "Tempo session indoors.",
        duration: 51 * 60,
        distance: Some(28_700),
        status: "pending",
    },
    Sample {
        activity_type: "yoga",
        title: "Yoga recovery",
        description: "Mobility and breathing session.",
        duration: 36 * 60,
        distance: None,
        status: "excluded",
    },
    Sample {
        activity_type: "rowing",
        title: "Rowing intervals",
        description: "Short, hard intervals.",
        duration: 32 * 60,
        distance: Some(6_400),
        status: "pending",
    },
    Sample {
        activity_type: "elliptical",
        title: "Cross trainer",
        description: "Low-impact aerobic session.",
        duration: 44 * 60,
        distance: None,
        status: "held",
    },
    Sample {
        activity_type: "cardio",
        title: "Cardio workout",
        description: "Mixed cardio session.",
        duration: 39 * 60,
        distance: None,
        status: "published",
    },
    Sample {
        activity_type: "backcountry_skiing_snowboarding_ws",
        title: "Backcountry day",
        description: "Skinned up, rode down.",
        duration: 5 * 3600 + 20 * 60,
        distance: Some(9_400),
        status: "published",
    },
];

/// Fills a mock database with a year of sample activities. When `onboarded`,
/// it is also past the setup wizard (`initial_sync_done`, nothing else), even
/// on a database that already holds the current sample data.
///
/// Refuses to touch a database holding any activity it did not generate, so
/// pointing the mock server at a real database cannot alter it.
pub fn seed(conn: &mut Connection, now: DateTime<Utc>, onboarded: bool) -> DbResult<bool> {
    let existing = db::list_activities(conn, false)?;
    if existing
        .iter()
        .any(|row| !row.content_hash.starts_with("dev-"))
    {
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
    for table in [
        "activities",
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
                    start_time: &start.format(GARMIN_TIME_FORMAT).to_string(),
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
    let mut held: Vec<&str> = SAMPLES
        .iter()
        .filter(|s| s.status == "held")
        .map(|s| s.activity_type)
        .collect();
    held.sort_unstable();
    held.dedup();
    settings.insert("held_activity_types".into(), json!(held));
    db::set_config_value(&tx, "settings", &Value::Object(settings))?;
    db::set_config_value(
        &tx,
        "garmin_activity_types",
        &Value::Array(mock::garmin_activity_types()),
    )?;
    db::set_config_value(&tx, SEED_VERSION_KEY, &json!(SEED_VERSION))?;
    tx.commit()?;
    Ok(true)
}
