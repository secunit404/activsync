use chrono::{DateTime, Datelike, Timelike, Utc};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::{config, mock};
use crate::db::{self, DbResult, NewActivity};

/// Bumped whenever the generated rows change, so an older dev database is
/// rebuilt instead of mixing two generations of sample data.
pub const SEED_VERSION: i64 = 2;
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

const MERGED_HEVY_ACTIVITY_ID: i64 = 990001;

/// Fills a mock database with a year of sample activities. When `onboarded`,
/// it is also past the setup wizard with both services reading as connected;
/// otherwise the wizard starts at its first step.
///
/// Refuses to touch a database holding any activity it did not generate, so
/// pointing the mock server at a real database cannot alter it.
pub fn seed(conn: &mut Connection, now: DateTime<Utc>, onboarded: bool) -> DbResult<bool> {
    let existing = db::list_activities(conn, false)?;
    if existing.iter().any(|row| !row.content_hash.starts_with("dev-")) {
        return Ok(false);
    }
    if db::get_config_value(conn, SEED_VERSION_KEY)? == Some(json!(SEED_VERSION)) {
        return Ok(true);
    }
    let now = now.with_nanosecond(0).unwrap_or(now);
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM activities", [])?;
    tx.execute("DELETE FROM hevy_workouts", [])?;
    tx.execute("DELETE FROM publish_schedule", [])?;

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
                    garmin_data: &data.to_string(),
                    hold_reason,
                },
                now,
            )?;
        }
    }
    seed_hevy_badge(&tx, now)?;

    let mut settings = config::defaults();
    settings.insert("hevy_match_mode".into(), json!("review"));
    let mut held: Vec<&str> = SAMPLES.iter().filter(|s| s.status == "held").map(|s| s.activity_type).collect();
    held.sort_unstable();
    held.dedup();
    settings.insert("held_activity_types".into(), json!(held));
    db::set_config_value(&tx, "settings", &Value::Object(settings))?;
    db::set_config_value(&tx, "garmin_activity_types", &Value::Array(mock::garmin_activity_types()))?;
    if onboarded {
        db::set_config_value(&tx, "initial_sync_done", &json!(true))?;
        db::set_config_value(&tx, "garmin_credentials", &json!({"email": "dev@example.com", "password": "dev-password"}))?;
        db::set_config_value(&tx, "garmin_credentials_verified", &json!(true))?;
        db::set_config_value(&tx, "garmin_last_sync_at", &json!(db::iso(now)))?;
        db::set_config_value(&tx, "garmin_last_sync_ok", &json!(true))?;
        db::set_config_value(&tx, "strava_credentials", &json!({"client_id": "12345", "client_secret": "dev-secret"}))?;
        db::set_config_value(&tx, "strava_tokens", &json!({"refresh_token": "dev-refresh-token"}))?;
    }
    db::set_config_value(&tx, SEED_VERSION_KEY, &json!(SEED_VERSION))?;
    tx.commit()?;
    Ok(true)
}

/// One watch activity enriched by a merged Hevy workout, so the list shows
/// the Hevy badge.
fn seed_hevy_badge(conn: &Connection, now: DateTime<Utc>) -> DbResult<()> {
    let start = now - chrono::Duration::days(1);
    let start_text = start.format("%Y-%m-%d %H:%M:%S").to_string();
    db::insert_activity(
        conn,
        &NewActivity {
            garmin_activity_id: MERGED_HEVY_ACTIVITY_ID,
            activity_type: "strength_training",
            title: "Gym session (watch)",
            description: "",
            start_time: &start_text,
            content_hash: &format!("dev-{MERGED_HEVY_ACTIVITY_ID}"),
            publish_status: "pending",
            garmin_data: &json!({"duration": 3600, "avg_hr": 128, "calories": 380,
                                 "total_sets": 15, "total_reps": 75}).to_string(),
            hold_reason: None,
        },
        now,
    )?;
    let iso_now = db::iso(now);
    conn.execute(
        "INSERT INTO hevy_workouts
           (hevy_id, title, start_time, end_time, source_updated_at, status, applied_strategy,
            source_garmin_activity_id, garmin_activity_id, created_at, updated_at)
         VALUES ('dev-hevy-merged', 'Gym session', ?1, ?2, ?3, 'merged', 'merge', ?4, ?4, ?3, ?3)",
        params![
            db::iso(start),
            db::iso(start + chrono::Duration::hours(1)),
            iso_now,
            MERGED_HEVY_ACTIVITY_ID
        ],
    )?;
    Ok(())
}
