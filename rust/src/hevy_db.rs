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
}

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
    pub phase: String,
    pub next_step: Option<String>,
    pub target_activity_id: Option<i64>,
    pub upload_id: Option<String>,
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
        "SELECT id, phase, next_step, target_activity_id, upload_id FROM hevy_operations
         WHERE hevy_id = ?1 AND phase NOT IN ('done','failed')",
        [hevy_id],
        |row| {
            Ok(Operation {
                id: row.get(0)?,
                phase: row.get(1)?,
                next_step: row.get(2)?,
                target_activity_id: row.get(3)?,
                upload_id: row.get(4)?,
            })
        },
    )
    .optional()
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
    pub primary_muscle_group: &'a str,
    pub secondary_muscle_groups: &'a [&'a str],
    pub equipment_category: &'a str,
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
