use std::path::{Path, PathBuf};

use activsync::db::{self, Db};
use activsync::seed;
use chrono::Utc;
use serde_json::{Value, json};

fn stored(db: &Db, key: &str) -> String {
    db.lock()
        .query_row(
            "SELECT value FROM app_config WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn config_values_are_stored_as_python_json_dumps_writes_them() {
    let db = Db::open_in_memory().unwrap();
    let value = json!({
        "a": "é😀\n\"\\\u{7f}\u{1}",
        "b": [1, 2.5, 1e-5, 1e16, 100.0, 0.0001, null, true, -3],
        "c": {},
        "d": []
    });
    db::set_config_value(&db.lock(), "settings", &value).unwrap();
    assert_eq!(
        stored(&db, "settings"),
        r#"{"a": "\u00e9\ud83d\ude00\n\"\\\u007f\u0001", "b": [1, 2.5, 1e-05, 1e+16, 100.0, 0.0001, null, true, -3], "c": {}, "d": []}"#
    );
    assert_eq!(
        db::get_config_value(&db.lock(), "settings").unwrap(),
        Some(value)
    );
    assert_eq!(
        db::json_dumps(&json!(["Näck", "x"])),
        r#"["N\u00e4ck", "x"]"#
    );
}

#[test]
fn hevy_json_columns_use_python_separators() {
    let db = Db::open_in_memory().unwrap();
    seed::seed(&mut db.lock(), Utc::now(), false).unwrap();
    let conn = db.lock();
    let payload: String = conn
        .query_row("SELECT payload FROM hevy_workouts LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(payload.contains("\": "), "{payload}");
    assert!(payload.is_ascii());
    let data: String = conn
        .query_row(
            "SELECT garmin_data FROM activities WHERE garmin_activity_id = 900001",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        data.starts_with("{\"") && data.contains("\": ") && data.contains(", \""),
        "{data}"
    );
}

#[test]
fn database_path_follows_python_resolution() {
    let nothing = |_: &Path| false;
    let only = |name: &'static str| move |path: &Path| path == Path::new(name);
    let both = |path: &Path| {
        path == Path::new("/config/activsync.db") || path == Path::new("/config/garmin2strava.db")
    };

    assert_eq!(
        db::resolve_path(Some("x.db".into()), true, nothing),
        PathBuf::from("x.db")
    );
    assert_eq!(
        db::resolve_path(None, false, nothing),
        PathBuf::from("/config/activsync.db")
    );
    assert_eq!(
        db::resolve_path(None, false, only("/config/garmin2strava.db")),
        PathBuf::from("/config/garmin2strava.db")
    );
    assert_eq!(
        db::resolve_path(None, false, both),
        PathBuf::from("/config/activsync.db")
    );
    assert_eq!(
        db::resolve_path(None, true, nothing),
        PathBuf::from("/config/activsync-dev.db")
    );
    assert_eq!(
        db::resolve_path(None, true, only("/config/dev-mock.db")),
        PathBuf::from("/config/dev-mock.db")
    );
    assert_eq!(
        db::resolve_path(None, true, only("/config/garmin2strava.db")),
        PathBuf::from("/config/activsync-dev.db")
    );
}

#[test]
fn onboarding_only_sets_initial_sync_done() {
    let db = Db::open_in_memory().unwrap();
    seed::seed(&mut db.lock(), Utc::now(), false).unwrap();
    assert_eq!(
        db::get_config_value(&db.lock(), "initial_sync_done").unwrap(),
        None
    );

    seed::seed(&mut db.lock(), Utc::now(), true).unwrap();
    let conn = db.lock();
    assert_eq!(
        db::get_config_value(&conn, "initial_sync_done").unwrap(),
        Some(json!(true))
    );
    for key in [
        "garmin_credentials",
        "garmin_credentials_verified",
        "strava_credentials",
        "strava_tokens",
    ] {
        assert_eq!(db::get_config_value(&conn, key).unwrap(), None, "{key}");
    }
}

#[test]
fn reseeding_wipes_the_merge_backups_and_seen_events() {
    let db = Db::open_in_memory().unwrap();
    seed::seed(&mut db.lock(), Utc::now(), false).unwrap();
    {
        let conn = db.lock();
        conn.execute(
            "INSERT INTO merge_backups (garmin_activity_id, hevy_id, original_sets, created_at) VALUES (1, 'h', '[]', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hevy_events_seen (event_type, workout_id, event_timestamp) VALUES ('updated', 'h', 't')",
            [],
        )
        .unwrap();
        db::set_config_value(&conn, "rust_dev_seed_version", &Value::from(0)).unwrap();
    }
    seed::seed(&mut db.lock(), Utc::now(), false).unwrap();
    let conn = db.lock();
    for table in ["merge_backups", "hevy_events_seen"] {
        let rows: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0, "{table}");
    }
}
