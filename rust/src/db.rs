use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS activities (
    garmin_activity_id INTEGER PRIMARY KEY,
    activity_type TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    start_time TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    publish_status TEXT NOT NULL,
    strava_activity_id INTEGER,
    first_seen_at TEXT NOT NULL,
    held_since TEXT,
    published_at TEXT,
    garmin_data TEXT NOT NULL DEFAULT '{}',
    hold_reason TEXT
);

CREATE TABLE IF NOT EXISTS app_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS auth_sessions (
    token_hash TEXT PRIMARY KEY,
    public_id TEXT NOT NULL UNIQUE,
    method TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    ip TEXT NOT NULL DEFAULT '',
    user_agent TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS publish_schedule (
    garmin_activity_id INTEGER PRIMARY KEY,
    batch_id TEXT NOT NULL,
    batch_kind TEXT NOT NULL,
    deadline REAL NOT NULL,
    state TEXT NOT NULL,
    prior_status TEXT NOT NULL,
    message TEXT,
    finished_at REAL
);

CREATE INDEX IF NOT EXISTS publish_schedule_batch ON publish_schedule (batch_id);

CREATE TABLE IF NOT EXISTS auth_oidc_logo (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    png BLOB NOT NULL,
    sha256 TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS hevy_workouts (
    hevy_id TEXT PRIMARY KEY,
    title TEXT NOT NULL DEFAULT '',
    start_time TEXT NOT NULL,
    end_time TEXT NOT NULL,
    source_updated_at TEXT,
    garmin_applied_updated_at TEXT,
    strava_applied_updated_at TEXT,
    payload TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'waiting_watch',
    applied_strategy TEXT,
    provenance TEXT NOT NULL DEFAULT 'activsync',
    source_garmin_activity_id INTEGER,
    garmin_activity_id INTEGER,
    locked_until TEXT,
    lease_owner TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_hw_source ON hevy_workouts(source_garmin_activity_id)
    WHERE source_garmin_activity_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_hw_target ON hevy_workouts(garmin_activity_id)
    WHERE garmin_activity_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS hevy_operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    hevy_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    phase TEXT NOT NULL,
    next_step TEXT,
    source_activity_id INTEGER,
    target_activity_id INTEGER,
    upload_id TEXT,
    pre_upload_ids TEXT NOT NULL DEFAULT '[]',
    generated_calories REAL,
    generated_avg_hr REAL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    delete_attempt_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    locked_until TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_op_open_hevy ON hevy_operations(hevy_id)
    WHERE phase NOT IN ('done','failed');
CREATE UNIQUE INDEX IF NOT EXISTS idx_op_open_source ON hevy_operations(source_activity_id)
    WHERE phase NOT IN ('done','failed') AND source_activity_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS exercise_templates (
    exercise_template_id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    primary_muscle_group TEXT,
    secondary_muscle_groups TEXT NOT NULL DEFAULT '[]',
    equipment_category TEXT,
    is_custom INTEGER NOT NULL DEFAULT 0,
    fetched_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS exercise_mappings (
    exercise_template_id TEXT PRIMARY KEY,
    category INTEGER NOT NULL,
    subcategory INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS strava_exercise_mappings (
    exercise_template_id TEXT PRIMARY KEY,
    exercise_type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS merge_backups (
    garmin_activity_id INTEGER PRIMARY KEY,
    hevy_id TEXT,
    replacement_activity_id INTEGER,
    original_sets TEXT,
    original_fit BLOB,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS hevy_events_seen (
    event_type TEXT NOT NULL,
    workout_id TEXT NOT NULL,
    event_timestamp TEXT NOT NULL,
    PRIMARY KEY (event_type, workout_id, event_timestamp)
);
"#;

pub type DbResult<T> = rusqlite::Result<T>;

/// One shared connection behind a mutex: every request and the publish
/// scheduler take it for a single short unit of work.
#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

impl Db {
    pub fn open(path: &Path) -> DbResult<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> DbResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> DbResult<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn columns(conn: &Connection, table: &str) -> DbResult<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |row| row.get::<_, String>("name"))?;
    names.collect()
}

fn migrate(conn: &Connection) -> DbResult<()> {
    let activity_cols = columns(conn, "activities")?;
    if !activity_cols.iter().any(|c| c == "garmin_data") {
        conn.execute(
            "ALTER TABLE activities ADD COLUMN garmin_data TEXT NOT NULL DEFAULT '{}'",
            [],
        )?;
    }
    if !activity_cols.iter().any(|c| c == "hold_reason") {
        conn.execute("ALTER TABLE activities ADD COLUMN hold_reason TEXT", [])?;
    }
    if !columns(conn, "hevy_workouts")?.iter().any(|c| c == "lease_owner") {
        conn.execute("ALTER TABLE hevy_workouts ADD COLUMN lease_owner TEXT", [])?;
    }
    let op_cols = columns(conn, "hevy_operations")?;
    if !op_cols.iter().any(|c| c == "generated_calories") {
        conn.execute("ALTER TABLE hevy_operations ADD COLUMN generated_calories REAL", [])?;
    }
    if !op_cols.iter().any(|c| c == "generated_avg_hr") {
        conn.execute("ALTER TABLE hevy_operations ADD COLUMN generated_avg_hr REAL", [])?;
    }
    conn.execute("DROP TABLE IF EXISTS sessions", [])?;
    conn.execute("DELETE FROM app_config WHERE key = 'auth'", [])?;
    conn.execute("DELETE FROM auth_sessions WHERE method = 'strava'", [])?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityRow {
    pub garmin_activity_id: i64,
    pub activity_type: String,
    pub title: String,
    pub description: String,
    pub start_time: String,
    pub content_hash: String,
    pub publish_status: String,
    pub strava_activity_id: Option<i64>,
    pub garmin_data: String,
    pub hold_reason: Option<String>,
}

impl ActivityRow {
    fn from_row(row: &Row<'_>) -> DbResult<Self> {
        Ok(Self {
            garmin_activity_id: row.get("garmin_activity_id")?,
            activity_type: row.get("activity_type")?,
            title: row.get("title")?,
            description: row.get("description")?,
            start_time: row.get("start_time")?,
            content_hash: row.get("content_hash")?,
            publish_status: row.get("publish_status")?,
            strava_activity_id: row.get("strava_activity_id")?,
            garmin_data: row.get("garmin_data")?,
            hold_reason: row.get("hold_reason")?,
        })
    }

    /// A row that actually reached Strava: the status alone can read
    /// "published" while the Strava id is still unset.
    pub fn is_published(&self) -> bool {
        self.publish_status == "published" && self.strava_activity_id.is_some()
    }
}

pub struct NewActivity<'a> {
    pub garmin_activity_id: i64,
    pub activity_type: &'a str,
    pub title: &'a str,
    pub description: &'a str,
    pub start_time: &'a str,
    pub content_hash: &'a str,
    pub publish_status: &'a str,
    pub garmin_data: &'a str,
    pub hold_reason: Option<&'a str>,
}

/// Python's `datetime.isoformat()` for an aware UTC value.
pub fn iso(now: DateTime<Utc>) -> String {
    let format = if now.timestamp_subsec_micros() == 0 {
        "%Y-%m-%dT%H:%M:%S+00:00"
    } else {
        "%Y-%m-%dT%H:%M:%S%.6f+00:00"
    };
    now.format(format).to_string()
}

pub fn insert_activity(conn: &Connection, new: &NewActivity<'_>, now: DateTime<Utc>) -> DbResult<()> {
    let held = new.publish_status == "held";
    let now_iso = iso(now);
    conn.execute(
        "INSERT INTO activities
           (garmin_activity_id, activity_type, title, description, start_time,
            content_hash, publish_status, first_seen_at, held_since, garmin_data,
            hold_reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            new.garmin_activity_id,
            new.activity_type,
            new.title,
            new.description,
            new.start_time,
            new.content_hash,
            new.publish_status,
            now_iso,
            held.then_some(&now_iso),
            new.garmin_data,
            if held { new.hold_reason } else { None },
        ],
    )?;
    Ok(())
}

pub fn get_activity(conn: &Connection, garmin_activity_id: i64) -> DbResult<Option<ActivityRow>> {
    conn.query_row(
        "SELECT * FROM activities WHERE garmin_activity_id = ?1",
        [garmin_activity_id],
        ActivityRow::from_row,
    )
    .optional()
}

pub fn list_activities(conn: &Connection, oldest_first: bool) -> DbResult<Vec<ActivityRow>> {
    let order = if oldest_first { "ASC" } else { "DESC" };
    let mut stmt = conn.prepare(&format!("SELECT * FROM activities ORDER BY start_time {order}"))?;
    let rows = stmt.query_map([], ActivityRow::from_row)?;
    rows.collect()
}

pub fn count_activities(conn: &Connection, statuses: &[&str]) -> DbResult<i64> {
    let placeholders = vec!["?"; statuses.len()].join(", ");
    conn.query_row(
        &format!("SELECT COUNT(*) FROM activities WHERE publish_status IN ({placeholders})"),
        rusqlite::params_from_iter(statuses),
        |row| row.get(0),
    )
}

pub fn update_activity_metadata(
    conn: &Connection,
    garmin_activity_id: i64,
    title: &str,
    description: &str,
    content_hash: &str,
) -> DbResult<()> {
    conn.execute(
        "UPDATE activities SET title = ?1, description = ?2, content_hash = ?3
         WHERE garmin_activity_id = ?4",
        params![title, description, content_hash, garmin_activity_id],
    )?;
    Ok(())
}

/// `hold_reason` only survives while the row is held.
pub fn set_publish_status(
    conn: &Connection,
    garmin_activity_id: i64,
    status: &str,
    hold_reason: Option<&str>,
) -> DbResult<()> {
    let reason = if status == "held" { hold_reason } else { None };
    conn.execute(
        "UPDATE activities SET publish_status = ?1, hold_reason = ?2 WHERE garmin_activity_id = ?3",
        params![status, reason, garmin_activity_id],
    )?;
    Ok(())
}

pub fn set_published(
    conn: &Connection,
    garmin_activity_id: i64,
    strava_activity_id: i64,
    now: DateTime<Utc>,
) -> DbResult<()> {
    conn.execute(
        "UPDATE activities
           SET publish_status = 'published', strava_activity_id = ?1, published_at = ?2,
               hold_reason = NULL
         WHERE garmin_activity_id = ?3",
        params![strava_activity_id, iso(now), garmin_activity_id],
    )?;
    Ok(())
}

/// Values are stored as JSON text, exactly as the Python app writes them.
pub fn get_config_value(conn: &Connection, key: &str) -> DbResult<Option<Value>> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM app_config WHERE key = ?1", [key], |row| row.get(0))
        .optional()?;
    Ok(raw.and_then(|text| serde_json::from_str(&text).ok()))
}

pub fn set_config_value(conn: &Connection, key: &str, value: &Value) -> DbResult<()> {
    conn.execute(
        "INSERT INTO app_config (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value.to_string()],
    )?;
    Ok(())
}

/// Python truthiness for a stored config value.
pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// garmin_activity_id → applied Hevy strategy, for both the linked activity
/// and the claimed watch source.
pub fn hevy_badges(conn: &Connection) -> DbResult<std::collections::HashMap<i64, String>> {
    let mut stmt = conn.prepare(
        "SELECT applied_strategy, garmin_activity_id, source_garmin_activity_id
         FROM hevy_workouts ORDER BY start_time DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<i64>>(1)?,
            row.get::<_, Option<i64>>(2)?,
        ))
    })?;
    let mut badges = std::collections::HashMap::new();
    for row in rows {
        let (strategy, target, source) = row?;
        let Some(strategy) = strategy.filter(|s| !s.is_empty()) else {
            continue;
        };
        for id in [target, source].into_iter().flatten() {
            badges.insert(id, strategy.clone());
        }
    }
    Ok(badges)
}

const BLOCKING_STATUSES: &str =
    "'needs_mapping', 'waiting_watch', 'awaiting_match', 'syncing', 'failed', 'needs_review'";

/// The Hevy workout that currently owns an activity, as (hevy_id, status).
pub fn blocking_workout_for_activity(
    conn: &Connection,
    garmin_activity_id: i64,
) -> DbResult<Option<(String, String)>> {
    conn.query_row(
        &format!(
            "SELECT workout.hevy_id, workout.status
             FROM hevy_workouts AS workout
             WHERE (
                 workout.status IN ({BLOCKING_STATUSES})
                 AND (workout.source_garmin_activity_id = ?1 OR workout.garmin_activity_id = ?1)
             ) OR EXISTS (
                 SELECT 1 FROM hevy_operations AS operation
                 WHERE operation.hevy_id = workout.hevy_id
                   AND operation.phase NOT IN ('done', 'failed')
                   AND (operation.source_activity_id = ?1 OR operation.target_activity_id = ?1)
             )
             ORDER BY CASE WHEN workout.status IN ({BLOCKING_STATUSES}) THEN 0 ELSE 1 END,
                      workout.created_at
             LIMIT 1"
        ),
        [garmin_activity_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}
