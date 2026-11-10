//! The Hevy demo scenarios from the Python app's `dev_seed._seed_hevy`,
//! seeded in their end states because nothing polls Hevy in mock mode.

use chrono::{DateTime, Duration, Timelike, Utc};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::db::{self, DbResult, NewActivity};
use crate::hevy_db::{self, Template};

pub const MERGED_ID: &str = "hw-dev-merged";
pub const UNMAPPED_ID: &str = "hw-dev-unmapped";
pub const PASSIVE_ID: &str = "hw-dev-passive";
pub const MIDNIGHT_ID: &str = "hw-dev-midnight";
pub const SYNCING_ID: &str = "hw-dev-syncing";
pub const CUSTOM_TEMPLATE_ID: &str = "tpl-dev-custom";
pub const OVERRIDDEN_TEMPLATE_ID: &str = "tpl-dev-pushdown";

pub const MERGED_ACTIVITY_ID: i64 = 910001;
pub const SYNCING_ACTIVITY_ID: i64 = 910002;
pub const PASSIVE_ACTIVITY_ID: i64 = 910050;

const RESOLVING_TEMPLATES: [(&str, &str, &str, &str); 9] = [
    ("tpl-dev-bench", "Bench Press (Barbell)", "chest", "barbell"),
    ("tpl-dev-squat", "Squat (Barbell)", "quadriceps", "barbell"),
    ("tpl-dev-deadlift", "Deadlift (Barbell)", "hamstrings", "barbell"),
    ("tpl-dev-curl", "Bicep Curl (Dumbbell)", "biceps", "dumbbell"),
    ("tpl-dev-pulldown", "Lat Pulldown (Cable)", "lats", "cable"),
    ("tpl-dev-ohp", "Overhead Press (Barbell)", "shoulders", "barbell"),
    ("tpl-dev-plank", "Plank", "abdominals", "none"),
    ("tpl-dev-pullup", "Pull Up", "lats", "none"),
    ("tpl-dev-rdl", "Romanian Deadlift (Barbell)", "hamstrings", "barbell"),
];

fn hevy_iso(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
}

fn workout(id: &str, title: &str, start: DateTime<Utc>, end: DateTime<Utc>, exercise: &str, template: &str) -> Value {
    let sets: Vec<Value> = (0..3).map(|_| json!({"type": "normal", "weight_kg": 80.0, "reps": 5})).collect();
    json!({
        "id": id,
        "title": title,
        "start_time": hevy_iso(start),
        "end_time": hevy_iso(end),
        "updated_at": hevy_iso(end),
        "exercises": [{"title": exercise, "exercise_template_id": template, "sets": sets}],
    })
}

/// The workouts the queue shows, timestamped relative to `now`.
pub fn dev_workouts(now: DateTime<Utc>) -> Vec<Value> {
    let now = now.with_nanosecond(0).unwrap_or(now);
    let midnight = (now - Duration::days(1))
        .with_hour(23)
        .and_then(|d| d.with_minute(30))
        .and_then(|d| d.with_second(0))
        .expect("23:30 exists in UTC");
    let bench = ("Bench Press (Barbell)", "tpl-dev-bench");
    vec![
        workout(MERGED_ID, "Push day (Hevy)", now - Duration::hours(3), now - Duration::hours(2), bench.0, bench.1),
        workout(
            UNMAPPED_ID,
            "Ring circuit (Hevy)",
            now - Duration::hours(6),
            now - Duration::hours(5),
            "Bulgarian Ring Row",
            CUSTOM_TEMPLATE_ID,
        ),
        workout(PASSIVE_ID, "Forgot the watch (Hevy)", now - Duration::hours(27), now - Duration::hours(26), bench.0, bench.1),
        workout(MIDNIGHT_ID, "Midnight session (Hevy)", midnight, midnight + Duration::minutes(75), bench.0, bench.1),
        workout(SYNCING_ID, "Leg day (Hevy)", now - Duration::minutes(90), now - Duration::minutes(30), bench.0, bench.1),
    ]
}

/// Everything the mock Hevy account holds: the queue's workouts plus three
/// older ones nothing has imported yet, for the history import to find.
/// One of those has no template id and a title no table knows.
pub fn account_workouts(now: DateTime<Utc>) -> Vec<Value> {
    let now = now.with_nanosecond(0).unwrap_or(now);
    let old = |days: i64, minutes: i64| now - Duration::days(days) - Duration::minutes(minutes);
    let mut workouts = dev_workouts(now);
    for (id, title, start, exercise, template) in [
        ("hw-dev-backfill-importable", "Old push day (Hevy)", old(4, 41), "Bench Press (Barbell)", Some("tpl-dev-bench")),
        ("hw-dev-backfill-unmapped", "Old ring circuit (Hevy)", old(5, 17), "Bulgarian Ring Row", Some(CUSTOM_TEMPLATE_ID)),
        ("hw-dev-backfill-unmappable", "Old mystery session (Hevy)", old(6, 23), "Mystery Movement", None),
    ] {
        let mut entry = workout(id, title, start, start + Duration::hours(1), exercise, template.unwrap_or_default());
        if template.is_none() {
            entry["exercises"][0]["exercise_template_id"] = Value::Null;
        }
        workouts.push(entry);
    }
    workouts
}

fn text<'a>(workout: &'a Value, key: &str) -> &'a str {
    workout[key].as_str().unwrap_or_default()
}

fn store(conn: &Connection, workout: &Value) -> DbResult<()> {
    hevy_db::upsert_workout(
        conn,
        text(workout, "id"),
        text(workout, "title"),
        text(workout, "start_time"),
        text(workout, "end_time"),
        text(workout, "updated_at"),
        workout,
    )
}

fn watch_activity(conn: &Connection, id: i64, title: &str, workout: &Value, now: DateTime<Utc>) -> DbResult<()> {
    let start = text(workout, "start_time").replace('T', " ");
    let start = start.split('+').next().unwrap_or_default();
    let data = json!({"duration": 3600, "avg_hr": 128, "calories": 380, "total_sets": 15, "total_reps": 75});
    db::insert_activity(
        conn,
        &NewActivity {
            garmin_activity_id: id,
            activity_type: "strength_training",
            title,
            description: "",
            start_time: start,
            content_hash: &format!("dev-{id}"),
            publish_status: "pending",
            garmin_data: &data.to_string(),
            hold_reason: None,
        },
        now,
    )
}

pub fn seed(conn: &Connection, now: DateTime<Utc>) -> DbResult<()> {
    for (id, title, muscle, equipment) in RESOLVING_TEMPLATES {
        hevy_db::upsert_template(
            conn,
            &Template {
                id,
                title,
                primary_muscle_group: muscle,
                secondary_muscle_groups: &[],
                equipment_category: equipment,
                is_custom: false,
            },
        )?;
    }
    hevy_db::upsert_template(
        conn,
        &Template {
            id: OVERRIDDEN_TEMPLATE_ID,
            title: "Triceps Pushdown (Cable)",
            primary_muscle_group: "triceps",
            secondary_muscle_groups: &[],
            equipment_category: "cable",
            is_custom: false,
        },
    )?;
    hevy_db::upsert_template(
        conn,
        &Template {
            id: CUSTOM_TEMPLATE_ID,
            title: "Bulgarian Ring Row",
            primary_muscle_group: "upper_back",
            secondary_muscle_groups: &["biceps"],
            equipment_category: "other",
            is_custom: true,
        },
    )?;
    hevy_db::save_mapping(conn, OVERRIDDEN_TEMPLATE_ID, 30, 3)?;

    let workouts = dev_workouts(now);
    let [merged, unmapped, passive, midnight, syncing] = &workouts[..] else {
        unreachable!("dev_workouts returns five workouts");
    };

    watch_activity(conn, MERGED_ACTIVITY_ID, "Gym session (watch)", merged, now)?;
    store(conn, merged)?;
    hevy_db::claim_source(conn, MERGED_ID, MERGED_ACTIVITY_ID)?;
    hevy_db::link_target(conn, MERGED_ID, MERGED_ACTIVITY_ID, "merge")?;
    hevy_db::set_workout_status(conn, MERGED_ID, "merged", None)?;
    hevy_db::set_applied_garmin(conn, MERGED_ID, Some(text(merged, "updated_at")))?;

    store(conn, unmapped)?;
    hevy_db::set_workout_status(conn, UNMAPPED_ID, "needs_mapping", Some("unmapped exercises: Bulgarian Ring Row"))?;

    watch_activity(conn, PASSIVE_ACTIVITY_ID, "Forgot the watch (Hevy)", passive, now)?;
    store(conn, passive)?;
    hevy_db::link_target(conn, PASSIVE_ID, PASSIVE_ACTIVITY_ID, "passive")?;
    hevy_db::set_workout_status(conn, PASSIVE_ID, "uploaded_passive", None)?;
    hevy_db::set_applied_garmin(conn, PASSIVE_ID, Some(text(passive, "updated_at")))?;

    store(conn, midnight)?;

    watch_activity(conn, SYNCING_ACTIVITY_ID, "Strength (watch)", syncing, now)?;
    store(conn, syncing)?;
    hevy_db::claim_source(conn, SYNCING_ID, SYNCING_ACTIVITY_ID)?;
    hevy_db::set_workout_status(conn, SYNCING_ID, "awaiting_match", Some("Choose how to apply this Hevy workout."))?;
    Ok(())
}
