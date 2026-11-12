use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

const SCHEMA: &str = r#"
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

CREATE TABLE IF NOT EXISTS hevy_match_schedule (
    hevy_id TEXT PRIMARY KEY,
    strategy TEXT NOT NULL,
    deadline REAL NOT NULL,
    state TEXT NOT NULL,
    message TEXT,
    finished_at REAL
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
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            let _ = std::fs::create_dir_all(parent);
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
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn columns(conn: &Connection, table: &str) -> DbResult<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |row| row.get::<_, String>("name"))?;
    names.collect()
}

fn migrate(conn: &Connection) -> DbResult<()> {
    for (table, column, definition) in [
        ("activities", "garmin_data", "TEXT NOT NULL DEFAULT '{}'"),
        ("activities", "hold_reason", "TEXT"),
        ("hevy_workouts", "lease_owner", "TEXT"),
        ("hevy_operations", "generated_calories", "REAL"),
        ("hevy_operations", "generated_avg_hr", "REAL"),
    ] {
        if !columns(conn, table)?.iter().any(|c| c == column) {
            conn.execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                [],
            )?;
        }
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

pub fn insert_activity(
    conn: &Connection,
    new: &NewActivity<'_>,
    now: DateTime<Utc>,
) -> DbResult<()> {
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
    let mut stmt = conn.prepare(&format!(
        "SELECT * FROM activities ORDER BY start_time {order}"
    ))?;
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

/// Where the database lives: `configured` (`ACTIVSYNC_DB_PATH`, else
/// `G2S_DB_PATH`), else the default for the mode. An install from before the
/// rename keeps its old database name until a new one exists.
pub fn resolve_path(
    configured: Option<String>,
    mock_mode: bool,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    if let Some(path) = configured.filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    let (preferred, legacy) = if mock_mode {
        (
            PathBuf::from("/config/activsync-dev.db"),
            PathBuf::from("/config/dev-mock.db"),
        )
    } else {
        (
            PathBuf::from("/config/activsync.db"),
            PathBuf::from("/config/garmin2strava.db"),
        )
    };
    if exists(&legacy) && !exists(&preferred) {
        legacy
    } else {
        preferred
    }
}

/// Values are stored as JSON text, exactly as the Python app writes them.
pub fn get_config_value(conn: &Connection, key: &str) -> DbResult<Option<Value>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM app_config WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(raw.and_then(|text| serde_json::from_str(&text).ok()))
}

pub fn set_config_value(conn: &Connection, key: &str, value: &Value) -> DbResult<()> {
    conn.execute(
        "INSERT INTO app_config (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, json_dumps(value)],
    )?;
    Ok(())
}

/// `json.dumps` with its defaults: `", "` and `": "` separators and every
/// non-ASCII character as a `\uXXXX` escape (surrogate pairs above U+FFFF),
/// so a row reads the same whichever server wrote it.
pub fn json_dumps(value: &Value) -> String {
    let mut out = String::new();
    dump_value(value, &mut out);
    out
}

fn dump_value(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) if n.is_f64() => {
            out.push_str(&python_float(n.as_f64().unwrap_or_default()))
        }
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => dump_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                dump_value(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (index, (key, item)) in map.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                dump_string(key, out);
                out.push_str(": ");
                dump_value(item, out);
            }
            out.push('}');
        }
    }
}

fn dump_string(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(ch),
            _ => {
                let mut units = [0u16; 2];
                for unit in ch.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

/// Python's `repr()` of a float: shortest round-trip digits, `1e-05` and
/// `1e+16` style exponents outside 1e-4..1e16.
fn python_float(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    let text = format!("{value:?}");
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return text;
    };
    let (sign, digits) = exponent
        .strip_prefix('-')
        .map_or(("+", exponent), |digits| ("-", digits));
    format!("{mantissa}e{sign}{digits:0>2}")
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
pub fn hevy_badges(conn: &Connection) -> DbResult<HashMap<i64, String>> {
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
    let mut badges = HashMap::new();
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

pub fn list_activities_with_status(conn: &Connection, status: &str) -> DbResult<Vec<ActivityRow>> {
    let mut stmt = conn
        .prepare("SELECT * FROM activities WHERE publish_status = ?1 ORDER BY start_time DESC")?;
    let rows = stmt.query_map([status], ActivityRow::from_row)?;
    rows.collect()
}

/// `hold_reason` is kept only while the row stays held.
#[allow(clippy::too_many_arguments)]
pub fn update_activity_content(
    conn: &Connection,
    garmin_activity_id: i64,
    title: &str,
    description: &str,
    activity_type: &str,
    content_hash: &str,
    publish_status: &str,
    garmin_data: &str,
) -> DbResult<()> {
    conn.execute(
        "UPDATE activities
           SET title = ?1, description = ?2, activity_type = ?3, content_hash = ?4,
               publish_status = ?5, garmin_data = ?6,
               hold_reason = CASE WHEN ?5 = 'held' THEN hold_reason ELSE NULL END
         WHERE garmin_activity_id = ?7",
        params![
            title,
            description,
            activity_type,
            content_hash,
            publish_status,
            garmin_data,
            garmin_activity_id
        ],
    )?;
    Ok(())
}

/// Garmin is the source of truth: a row it no longer returns is dropped,
/// while any Strava copy is left alone. Should the activity reappear, the
/// publish path re-links it by start time instead of uploading again.
pub fn mark_removed(conn: &Connection, garmin_activity_id: i64) -> DbResult<()> {
    conn.execute(
        "DELETE FROM activities WHERE garmin_activity_id = ?1",
        [garmin_activity_id],
    )?;
    Ok(())
}

/// Scoped by window: a sync speaks only for the span it fetched.
pub fn list_active_ids_since(conn: &Connection, window_start: &str) -> DbResult<HashSet<i64>> {
    let mut stmt =
        conn.prepare("SELECT garmin_activity_id FROM activities WHERE start_time >= ?1")?;
    let ids = stmt.query_map([window_start], |row| row.get(0))?;
    ids.collect()
}

/// The oldest activity still awaiting a decision: the real extent of the
/// backlog, visible even when only Strava was down.
pub fn oldest_unpublished_start_time(conn: &Connection) -> DbResult<Option<String>> {
    let oldest: Option<String> = conn.query_row(
        "SELECT MIN(start_time) FROM activities WHERE publish_status IN ('pending', 'held')",
        [],
        |row| row.get(0),
    )?;
    Ok(oldest.filter(|s| !s.is_empty()))
}

/// Activities waiting out their undo window or being sent by the publish
/// scheduler; nothing else may publish them meanwhile.
pub fn scheduled_publish_ids(conn: &Connection) -> DbResult<HashSet<i64>> {
    let mut stmt = conn.prepare(
        "SELECT garmin_activity_id FROM publish_schedule WHERE state IN ('waiting', 'sending')",
    )?;
    let ids = stmt.query_map([], |row| row.get(0))?;
    ids.collect()
}

/// Whether any Hevy workout is durably linked to the activity, whatever its
/// status.
pub fn has_linked_workout(conn: &Connection, garmin_activity_id: i64) -> DbResult<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM hevy_workouts WHERE source_garmin_activity_id = ?1 OR garmin_activity_id = ?1)",
        [garmin_activity_id],
        |row| row.get(0),
    )
}

/// The Hevy workout whose content now lives on the activity, as
/// (status, payload).
pub fn applied_workout_for_activity(
    conn: &Connection,
    garmin_activity_id: i64,
) -> DbResult<Option<(String, String)>> {
    conn.query_row(
        "SELECT status, payload FROM hevy_workouts
         WHERE garmin_activity_id = ?1 AND status IN ('merged', 'described', 'replaced', 'uploaded_passive')",
        [garmin_activity_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}
