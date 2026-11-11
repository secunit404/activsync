use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use crate::auth_core::random_token;
use crate::db::{self, DbResult};

#[derive(Debug, Clone, PartialEq)]
pub struct Workout {
    pub hevy_id: String,
    pub title: String,
    pub start_time: String,
    pub end_time: String,
    pub source_updated_at: Option<String>,
    pub payload: String,
    pub status: String,
    pub applied_strategy: Option<String>,
    pub source_garmin_activity_id: Option<i64>,
    pub garmin_activity_id: Option<i64>,
    pub error: Option<String>,
    pub garmin_applied_updated_at: Option<String>,
    pub strava_applied_updated_at: Option<String>,
}

/// The statuses of a workout that has landed on Garmin.
pub const APPLIED_STATUSES: [&str; 4] = ["merged", "described", "replaced", "uploaded_passive"];

impl Workout {
    fn from_row(row: &Row<'_>) -> DbResult<Self> {
        Ok(Self {
            hevy_id: row.get("hevy_id")?,
            title: row.get("title")?,
            start_time: row.get("start_time")?,
            end_time: row.get("end_time")?,
            source_updated_at: row.get("source_updated_at")?,
            payload: row.get("payload")?,
            status: row.get("status")?,
            applied_strategy: row.get("applied_strategy")?,
            source_garmin_activity_id: row.get("source_garmin_activity_id")?,
            garmin_activity_id: row.get("garmin_activity_id")?,
            error: row.get("error")?,
            garmin_applied_updated_at: row.get("garmin_applied_updated_at")?,
            strava_applied_updated_at: row.get("strava_applied_updated_at")?,
        })
    }

    pub fn payload_json(&self) -> Value {
        serde_json::from_str(&self.payload).unwrap_or(Value::Null)
    }

    pub fn display_title(&self) -> &str {
        if self.title.is_empty() { &self.hevy_id } else { &self.title }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    pub id: i64,
    pub kind: String,
    pub phase: String,
    pub next_step: Option<String>,
    pub source_activity_id: Option<i64>,
    pub target_activity_id: Option<i64>,
    pub upload_id: Option<String>,
    pub pre_upload_ids: Vec<i64>,
    pub generated_calories: Option<f64>,
    pub generated_avg_hr: Option<f64>,
    pub attempt_count: i64,
    pub delete_attempt_count: i64,
    pub last_error: Option<String>,
}

impl Operation {
    fn from_row(row: &Row<'_>) -> DbResult<Self> {
        let pre_upload_ids: String = row.get("pre_upload_ids")?;
        Ok(Self {
            id: row.get("id")?,
            kind: row.get("kind")?,
            phase: row.get("phase")?,
            next_step: row.get("next_step")?,
            source_activity_id: row.get("source_activity_id")?,
            target_activity_id: row.get("target_activity_id")?,
            upload_id: row.get("upload_id")?,
            pre_upload_ids: parse_ids(&pre_upload_ids),
            generated_calories: row.get("generated_calories")?,
            generated_avg_hr: row.get("generated_avg_hr")?,
            attempt_count: row.get("attempt_count")?,
            delete_attempt_count: row.get("delete_attempt_count")?,
            last_error: row.get("last_error")?,
        })
    }
}

/// Python's `int()` over a stored id list: numbers, floats and numeric text
/// count, anything else is dropped.
fn parse_ids(raw: &str) -> Vec<i64> {
    let parsed: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    parsed.as_array().map(|ids| ids.iter().filter_map(id_of).collect()).unwrap_or_default()
}

pub fn id_of(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f.trunc() as i64)),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(i64::from(*b)),
        _ => None,
    }
}

/// `json.dumps` of an id list, so a row reads the same to either app.
fn ids_json(ids: &[i64]) -> String {
    format!("[{}]", ids.iter().map(i64::to_string).collect::<Vec<_>>().join(", "))
}

fn now_iso() -> String {
    db::iso(Utc::now())
}

/// Inserts a workout as waiting for its watch activity, or refreshes it from
/// a newer Hevy revision; status never changes here.
pub fn upsert_workout(
    conn: &Connection,
    hevy_id: &str,
    title: &str,
    start_time: &str,
    end_time: &str,
    source_updated_at: &str,
    payload: &Value,
) -> DbResult<()> {
    let now = now_iso();
    let existing: Option<Option<String>> = conn
        .query_row("SELECT source_updated_at FROM hevy_workouts WHERE hevy_id = ?1", [hevy_id], |r| r.get(0))
        .optional()?;
    match existing {
        None => {
            conn.execute(
                "INSERT INTO hevy_workouts
                   (hevy_id, title, start_time, end_time, source_updated_at, payload, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![hevy_id, title, start_time, end_time, source_updated_at, payload.to_string(), now],
            )?;
        }
        Some(current) => {
            if current.is_some_and(|c| source_updated_at <= c.as_str()) {
                return Ok(());
            }
            conn.execute(
                "UPDATE hevy_workouts SET title = ?1, start_time = ?2, end_time = ?3,
                   source_updated_at = ?4, payload = ?5, updated_at = ?6 WHERE hevy_id = ?7",
                params![title, start_time, end_time, source_updated_at, payload.to_string(), now, hevy_id],
            )?;
        }
    }
    Ok(())
}

pub fn get_workout(conn: &Connection, hevy_id: &str) -> DbResult<Option<Workout>> {
    conn.query_row("SELECT * FROM hevy_workouts WHERE hevy_id = ?1", [hevy_id], Workout::from_row).optional()
}

pub fn list_workouts(conn: &Connection, status: &str) -> DbResult<Vec<Workout>> {
    let mut stmt = conn.prepare("SELECT * FROM hevy_workouts WHERE status = ?1 ORDER BY start_time DESC")?;
    let rows = stmt.query_map([status], Workout::from_row)?;
    rows.collect()
}

pub fn set_workout_status(conn: &Connection, hevy_id: &str, status: &str, error: Option<&str>) -> DbResult<()> {
    conn.execute(
        "UPDATE hevy_workouts SET status = ?1, error = ?2, updated_at = ?3 WHERE hevy_id = ?4",
        params![status, error, now_iso(), hevy_id],
    )?;
    Ok(())
}

/// A claim is immutable once made; re-claiming the same source is a no-op.
pub fn claim_source(conn: &Connection, hevy_id: &str, source: i64) -> DbResult<bool> {
    let changed = conn.execute(
        "UPDATE hevy_workouts SET source_garmin_activity_id = ?1, updated_at = ?2
         WHERE hevy_id = ?3 AND (source_garmin_activity_id IS NULL OR source_garmin_activity_id = ?1)",
        params![source, now_iso(), hevy_id],
    );
    match changed {
        Ok(n) => Ok(n == 1),
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => Ok(false),
        Err(e) => Err(e),
    }
}

/// The applied strategy is set once and never changed afterwards.
pub fn link_target(conn: &Connection, hevy_id: &str, garmin_activity_id: i64, strategy: &str) -> DbResult<()> {
    conn.execute(
        "UPDATE hevy_workouts
         SET garmin_activity_id = ?1, applied_strategy = COALESCE(applied_strategy, ?2),
             provenance = 'activsync', updated_at = ?3
         WHERE hevy_id = ?4",
        params![garmin_activity_id, strategy, now_iso(), hevy_id],
    )?;
    Ok(())
}

/// Detaches a workout from its Garmin activities for a fresh sync. Callers
/// make sure no operation is open, or its journal would be orphaned.
pub fn reset_links(conn: &Connection, hevy_id: &str) -> DbResult<()> {
    conn.execute(
        "UPDATE hevy_workouts
         SET source_garmin_activity_id = NULL, garmin_activity_id = NULL, applied_strategy = NULL,
             garmin_applied_updated_at = NULL, strava_applied_updated_at = NULL, updated_at = ?1
         WHERE hevy_id = ?2",
        params![now_iso(), hevy_id],
    )?;
    Ok(())
}

pub fn delete_workout(conn: &Connection, hevy_id: &str) -> DbResult<()> {
    conn.execute("DELETE FROM hevy_workouts WHERE hevy_id = ?1", [hevy_id])?;
    Ok(())
}

pub fn set_applied_strava(conn: &Connection, hevy_id: &str, updated_at: &str) -> DbResult<()> {
    conn.execute(
        "UPDATE hevy_workouts SET strava_applied_updated_at = ?1, updated_at = ?2 WHERE hevy_id = ?3",
        params![updated_at, now_iso(), hevy_id],
    )?;
    Ok(())
}

pub fn event_seen(conn: &Connection, event_type: &str, workout_id: &str, timestamp: &str) -> DbResult<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM hevy_events_seen WHERE event_type = ?1 AND workout_id = ?2 AND event_timestamp = ?3",
            params![event_type, workout_id, timestamp],
            |r| r.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

/// The safety-lag overlap re-delivers events; the dedupe key absorbs them.
pub fn record_event_seen(conn: &Connection, event_type: &str, workout_id: &str, timestamp: &str) -> DbResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO hevy_events_seen (event_type, workout_id, event_timestamp) VALUES (?1, ?2, ?3)",
        params![event_type, workout_id, timestamp],
    )?;
    Ok(())
}

pub fn set_applied_garmin(conn: &Connection, hevy_id: &str, updated_at: Option<&str>) -> DbResult<()> {
    conn.execute(
        "UPDATE hevy_workouts SET garmin_applied_updated_at = ?1, updated_at = ?2 WHERE hevy_id = ?3",
        params![updated_at, now_iso(), hevy_id],
    )?;
    Ok(())
}

/// The per-workout execution lease. The token must be shown to release it,
/// so a worker whose lease expired cannot clear its successor's.
pub fn acquire_lease(conn: &Connection, hevy_id: &str, now: DateTime<Utc>) -> DbResult<Option<String>> {
    let token = random_token(8);
    let until = db::iso(now + Duration::seconds(300));
    let taken = conn.execute(
        "UPDATE hevy_workouts SET locked_until = ?1, lease_owner = ?2
         WHERE hevy_id = ?3 AND (locked_until IS NULL OR locked_until <= ?4)",
        params![until, token, hevy_id, db::iso(now)],
    )?;
    Ok((taken == 1).then_some(token))
}

pub fn release_lease(conn: &Connection, hevy_id: &str, token: &str) -> DbResult<()> {
    conn.execute(
        "UPDATE hevy_workouts SET locked_until = NULL, lease_owner = NULL WHERE hevy_id = ?1 AND lease_owner = ?2",
        params![hevy_id, token],
    )?;
    Ok(())
}

pub fn get_open_operation(conn: &Connection, hevy_id: &str) -> DbResult<Option<Operation>> {
    conn.query_row(
        "SELECT * FROM hevy_operations WHERE hevy_id = ?1 AND phase NOT IN ('done','failed')",
        [hevy_id],
        Operation::from_row,
    )
    .optional()
}

/// Opens a journal row; `None` when an open operation already holds this
/// workout or this source activity (partial unique indexes).
pub fn open_operation(
    conn: &Connection,
    hevy_id: &str,
    kind: &str,
    source_activity_id: Option<i64>,
    pre_upload_ids: &[i64],
) -> DbResult<Option<i64>> {
    let now = now_iso();
    let inserted = conn.execute(
        "INSERT INTO hevy_operations
           (hevy_id, kind, phase, source_activity_id, pre_upload_ids, created_at, updated_at)
         VALUES (?1, ?2, 'preparing', ?3, ?4, ?5, ?5)",
        params![hevy_id, kind, source_activity_id, ids_json(pre_upload_ids), now],
    );
    match inserted {
        Ok(_) => Ok(Some(conn.last_insert_rowid())),
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => Ok(None),
        Err(e) => Err(e),
    }
}

/// One journal column to change.
#[derive(Debug, Clone, PartialEq)]
pub enum OpField {
    Phase(&'static str),
    NextStep(&'static str),
    TargetActivityId(i64),
    UploadId(Option<String>),
    PreUploadIds(Vec<i64>),
    GeneratedCalories(Option<f64>),
    GeneratedAvgHr(Option<f64>),
    AttemptCount(i64),
    DeleteAttemptCount(i64),
    LastError(String),
}

impl OpField {
    fn column(&self) -> (&'static str, rusqlite::types::Value) {
        use rusqlite::types::Value as Sql;
        let text = |s: Option<String>| s.map_or(Sql::Null, Sql::Text);
        let real = |f: Option<f64>| f.map_or(Sql::Null, Sql::Real);
        match self {
            Self::Phase(v) => ("phase", Sql::Text((*v).into())),
            Self::NextStep(v) => ("next_step", Sql::Text((*v).into())),
            Self::TargetActivityId(v) => ("target_activity_id", Sql::Integer(*v)),
            Self::UploadId(v) => ("upload_id", text(v.clone())),
            Self::PreUploadIds(v) => ("pre_upload_ids", Sql::Text(ids_json(v))),
            Self::GeneratedCalories(v) => ("generated_calories", real(*v)),
            Self::GeneratedAvgHr(v) => ("generated_avg_hr", real(*v)),
            Self::AttemptCount(v) => ("attempt_count", Sql::Integer(*v)),
            Self::DeleteAttemptCount(v) => ("delete_attempt_count", Sql::Integer(*v)),
            Self::LastError(v) => ("last_error", Sql::Text(v.clone())),
        }
    }
}

pub fn update_operation(conn: &Connection, op_id: i64, fields: &[OpField]) -> DbResult<()> {
    if fields.is_empty() {
        return Ok(());
    }
    let (names, mut values): (Vec<_>, Vec<_>) = fields.iter().map(OpField::column).unzip();
    let assignments: Vec<String> = names.iter().map(|name| format!("{name} = ?")).collect();
    values.push(rusqlite::types::Value::Text(now_iso()));
    values.push(rusqlite::types::Value::Integer(op_id));
    conn.execute(
        &format!("UPDATE hevy_operations SET {}, updated_at = ? WHERE id = ?", assignments.join(", ")),
        rusqlite::params_from_iter(values),
    )?;
    Ok(())
}

/// Parks or fails an operation and its workout together: a closed journal
/// behind a `syncing` row could open and submit a second upload.
pub fn set_operation_outcome(
    conn: &Connection,
    op_id: i64,
    hevy_id: &str,
    phase: &str,
    status: &str,
    error: Option<&str>,
) -> DbResult<()> {
    debug_assert!(matches!(phase, "failed" | "needs_review"), "invalid operation outcome phase: {phase:?}");
    let now = now_iso();
    let tx = conn.unchecked_transaction()?;
    let op = tx.execute(
        "UPDATE hevy_operations SET phase = ?1, last_error = ?2, updated_at = ?3 WHERE id = ?4 AND hevy_id = ?5",
        params![phase, error, now, op_id, hevy_id],
    )?;
    let workout = tx.execute(
        "UPDATE hevy_workouts SET status = ?1, error = ?2, updated_at = ?3 WHERE hevy_id = ?4",
        params![status, error, now, hevy_id],
    )?;
    if op != 1 || workout != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    tx.commit()
}

/// The terminal transition in one transaction: close the operation, link
/// the target, set the status and record the applied revision. Split, a
/// closed journal could strand an unlinked replacement and a later tick
/// would upload again.
pub fn complete_operation(
    conn: &Connection,
    op_id: i64,
    hevy_id: &str,
    target_activity_id: i64,
    strategy: &str,
    status: &str,
    applied_updated_at: Option<&str>,
) -> DbResult<()> {
    let now = now_iso();
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE hevy_operations SET phase = 'done', updated_at = ?1 WHERE id = ?2", params![now, op_id])?;
    tx.execute(
        "UPDATE hevy_workouts
         SET garmin_activity_id = ?1, applied_strategy = COALESCE(applied_strategy, ?2),
             status = ?3, error = NULL, garmin_applied_updated_at = ?4, updated_at = ?5
         WHERE hevy_id = ?6",
        params![target_activity_id, strategy, status, applied_updated_at, now, hevy_id],
    )?;
    tx.commit()
}

/// The first backup is the truth and is never overwritten, except that a
/// missing FIT (a merge backs up none) is filled in by a later replace, or
/// the rebuilt activity would lose its heart rate.
pub fn save_backup(
    conn: &Connection,
    garmin_activity_id: i64,
    hevy_id: &str,
    original_sets: &Value,
    original_fit: Option<&[u8]>,
) -> DbResult<()> {
    conn.execute(
        "INSERT INTO merge_backups (garmin_activity_id, hevy_id, original_sets, original_fit, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(garmin_activity_id) DO UPDATE SET original_fit = excluded.original_fit
         WHERE merge_backups.original_fit IS NULL AND excluded.original_fit IS NOT NULL",
        params![garmin_activity_id, hevy_id, original_sets.to_string(), original_fit, now_iso()],
    )?;
    Ok(())
}

pub fn backup_fit(conn: &Connection, garmin_activity_id: i64) -> DbResult<Option<Vec<u8>>> {
    let fit: Option<Option<Vec<u8>>> = conn
        .query_row("SELECT original_fit FROM merge_backups WHERE garmin_activity_id = ?1", [garmin_activity_id], |r| r.get(0))
        .optional()?;
    Ok(fit.flatten())
}

/// Hands a journalled operation back to its resume point with fresh counters.
pub fn resume_operation(conn: &Connection, id: i64, phase: &str) -> DbResult<()> {
    conn.execute(
        "UPDATE hevy_operations SET phase = ?1, attempt_count = 0, delete_attempt_count = 0,
           last_error = NULL, updated_at = ?2 WHERE id = ?3",
        params![phase, now_iso(), id],
    )?;
    Ok(())
}

pub struct Template<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub primary_muscle_group: Option<&'a str>,
    pub secondary_muscle_groups: &'a [&'a str],
    pub equipment_category: Option<&'a str>,
    pub is_custom: bool,
}

pub fn upsert_template(conn: &Connection, template: &Template<'_>) -> DbResult<()> {
    conn.execute(
        "INSERT INTO exercise_templates
           (exercise_template_id, title, primary_muscle_group, secondary_muscle_groups,
            equipment_category, is_custom, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(exercise_template_id) DO UPDATE SET
           title = excluded.title, primary_muscle_group = excluded.primary_muscle_group,
           secondary_muscle_groups = excluded.secondary_muscle_groups,
           equipment_category = excluded.equipment_category, is_custom = excluded.is_custom,
           fetched_at = excluded.fetched_at",
        params![
            template.id,
            template.title,
            template.primary_muscle_group,
            serde_json::to_string(template.secondary_muscle_groups).unwrap_or_else(|_| "[]".into()),
            template.equipment_category,
            i64::from(template.is_custom),
            now_iso()
        ],
    )?;
    Ok(())
}

pub fn save_mapping(conn: &Connection, template_id: &str, category: i64, subcategory: i64) -> DbResult<()> {
    let now = now_iso();
    conn.execute(
        "INSERT INTO exercise_mappings (exercise_template_id, category, subcategory, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(exercise_template_id) DO UPDATE SET
           category = excluded.category, subcategory = excluded.subcategory, updated_at = excluded.updated_at",
        params![template_id, category, subcategory, now],
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateRow {
    pub id: String,
    pub title: String,
    pub primary_muscle_group: Option<String>,
    pub is_custom: bool,
}

pub fn list_templates(conn: &Connection) -> DbResult<Vec<TemplateRow>> {
    let mut stmt = conn.prepare(
        "SELECT exercise_template_id, title, primary_muscle_group, is_custom FROM exercise_templates ORDER BY title",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(TemplateRow {
            id: row.get(0)?,
            title: row.get(1)?,
            primary_muscle_group: row.get(2)?,
            is_custom: row.get::<_, i64>(3)? != 0,
        })
    })?;
    rows.collect()
}

pub fn template_exists(conn: &Connection, template_id: &str) -> DbResult<bool> {
    let found: Option<i64> = conn
        .query_row("SELECT 1 FROM exercise_templates WHERE exercise_template_id = ?1", [template_id], |r| r.get(0))
        .optional()?;
    Ok(found.is_some())
}

pub fn delete_mapping(conn: &Connection, template_id: &str) -> DbResult<()> {
    conn.execute("DELETE FROM exercise_mappings WHERE exercise_template_id = ?1", [template_id])?;
    Ok(())
}

pub fn save_strava_mapping(conn: &Connection, template_id: &str, exercise_type: &str) -> DbResult<()> {
    let now = now_iso();
    conn.execute(
        "INSERT INTO strava_exercise_mappings (exercise_template_id, exercise_type, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3)
         ON CONFLICT(exercise_template_id) DO UPDATE SET
           exercise_type = excluded.exercise_type, updated_at = excluded.updated_at",
        params![template_id, exercise_type, now],
    )?;
    Ok(())
}

pub fn delete_strava_mapping(conn: &Connection, template_id: &str) -> DbResult<()> {
    conn.execute("DELETE FROM strava_exercise_mappings WHERE exercise_template_id = ?1", [template_id])?;
    Ok(())
}

/// Wakes the parked workouts that use one template, back to the state their
/// applied strategy implies; returns their ids. Unrelated parked workouts are
/// left alone, since their mappings have not changed.
pub fn wake_needs_mapping(conn: &Connection, template_id: &str) -> DbResult<Vec<String>> {
    let affected: Vec<String> = list_workouts(conn, "needs_mapping")?
        .into_iter()
        .filter(|row| {
            row.payload_json()["exercises"].as_array().is_some_and(|exercises| {
                exercises.iter().any(|e| e.get("exercise_template_id").and_then(Value::as_str) == Some(template_id))
            })
        })
        .map(|row| row.hevy_id)
        .collect();
    wake(conn, affected)
}

/// Wakes one parked workout, for a newer Hevy revision of it.
pub fn wake_workout(conn: &Connection, hevy_id: &str) -> DbResult<Vec<String>> {
    let parked = get_workout(conn, hevy_id)?.is_some_and(|row| row.status == "needs_mapping");
    wake(conn, if parked { vec![hevy_id.to_string()] } else { Vec::new() })
}

fn wake(conn: &Connection, affected: Vec<String>) -> DbResult<Vec<String>> {
    let now = now_iso();
    for hevy_id in &affected {
        conn.execute(
            "UPDATE hevy_workouts
             SET status = CASE applied_strategy
                     WHEN 'merge' THEN 'merged'
                     WHEN 'describe' THEN 'described'
                     WHEN 'replace' THEN 'replaced'
                     WHEN 'passive' THEN 'uploaded_passive'
                     ELSE 'waiting_watch'
                 END,
                 error = NULL, updated_at = ?1
             WHERE hevy_id = ?2 AND status = 'needs_mapping'",
            params![now, hevy_id],
        )?;
    }
    Ok(affected)
}
