//! Server-side undo window for Hevy match decisions, as in the Python app's
//! `hevy_match_schedule.py`. The scheduler claims with `WHERE state =
//! 'waiting'` and undo deletes with the same guard, so only one of them can
//! match a row.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::Notify;

use crate::db::{Db, DbResult};
use crate::events::EventBus;
use crate::hevy_db;
use crate::schedule::Clock;

pub const WAITING: &str = "waiting";
pub const APPLYING: &str = "applying";
pub const DONE: &str = "done";
pub const FAILED: &str = "failed";
pub const SKIPPED: &str = "skipped";

pub const FAILURE_MESSAGE: &str = "Could not apply that Hevy match. Check the server log for details.";

const RESULT_RETENTION_SECONDS: f64 = 3600.0;
const MAX_SLEEP_SECONDS: f64 = 30.0;

#[derive(Debug, Clone)]
pub struct Scheduled {
    pub strategy: String,
    pub deadline: f64,
}

#[derive(Debug)]
pub enum MatchError {
    AlreadyScheduled,
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for MatchError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Db(err)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub strategy: String,
    pub deadline: f64,
    pub state: String,
    pub message: Option<String>,
}

pub fn schedule(
    conn: &mut Connection,
    hevy_id: &str,
    strategy: &str,
    now: f64,
    window_seconds: f64,
) -> Result<Scheduled, MatchError> {
    let scheduled = Scheduled { strategy: strategy.into(), deadline: now + window_seconds };
    let tx = conn.transaction()?;
    if is_active(&tx, hevy_id)? {
        return Err(MatchError::AlreadyScheduled);
    }
    tx.execute(
        "INSERT INTO hevy_match_schedule (hevy_id, strategy, deadline, state)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(hevy_id) DO UPDATE SET
             strategy = excluded.strategy,
             deadline = excluded.deadline,
             state = excluded.state,
             message = NULL,
             finished_at = NULL",
        params![hevy_id, strategy, scheduled.deadline, WAITING],
    )?;
    tx.commit()?;
    Ok(scheduled)
}

pub fn cancel(conn: &Connection, hevy_id: &str) -> DbResult<usize> {
    conn.execute(
        &format!("DELETE FROM hevy_match_schedule WHERE hevy_id = ?1 AND state = '{WAITING}'"),
        [hevy_id],
    )
}

pub fn claim_due(conn: &Connection, now: f64) -> DbResult<Vec<(String, String, f64)>> {
    let mut stmt = conn.prepare(&format!(
        "UPDATE hevy_match_schedule SET state = '{APPLYING}' \
         WHERE state = '{WAITING}' AND deadline <= ?1 \
         RETURNING hevy_id, strategy, deadline"
    ))?;
    let mut rows: Vec<(String, String, f64)> = stmt
        .query_map([now], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<DbResult<_>>()?;
    rows.sort_by(|a, b| a.2.total_cmp(&b.2));
    Ok(rows)
}

pub fn finish(conn: &Connection, hevy_id: &str, state: &str, message: Option<&str>, now: f64) -> DbResult<()> {
    conn.execute(
        &format!(
            "UPDATE hevy_match_schedule SET state = ?1, message = ?2, finished_at = ?3 \
             WHERE hevy_id = ?4 AND state = '{APPLYING}'"
        ),
        params![state, message, now, hevy_id],
    )?;
    Ok(())
}

/// Re-applying an interrupted decision is safe: the apply step re-reads the
/// workout and finishes as skipped unless it is still awaiting a match.
pub fn recover(conn: &Connection) -> DbResult<usize> {
    conn.execute(
        &format!("UPDATE hevy_match_schedule SET state = '{WAITING}' WHERE state = '{APPLYING}'"),
        [],
    )
}

pub fn prune(conn: &Connection, now: f64) -> DbResult<()> {
    conn.execute(
        "DELETE FROM hevy_match_schedule WHERE finished_at IS NOT NULL AND finished_at < ?1",
        [now - RESULT_RETENTION_SECONDS],
    )?;
    Ok(())
}

pub fn next_deadline(conn: &Connection) -> DbResult<Option<f64>> {
    conn.query_row(
        &format!("SELECT MIN(deadline) FROM hevy_match_schedule WHERE state = '{WAITING}'"),
        [],
        |row| row.get(0),
    )
}

pub fn active_by_workout(conn: &Connection) -> DbResult<HashMap<String, (f64, String)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT hevy_id, deadline, state FROM hevy_match_schedule WHERE state IN ('{WAITING}', '{APPLYING}')"
    ))?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))?;
    rows.collect()
}

pub fn is_active(conn: &Connection, hevy_id: &str) -> DbResult<bool> {
    let found: Option<i64> = conn
        .query_row(
            &format!(
                "SELECT 1 FROM hevy_match_schedule WHERE hevy_id = ?1 AND state IN ('{WAITING}', '{APPLYING}')"
            ),
            [hevy_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

pub fn get(conn: &Connection, hevy_id: &str) -> DbResult<Option<Decision>> {
    conn.query_row(
        "SELECT strategy, deadline, state, message FROM hevy_match_schedule WHERE hevy_id = ?1",
        [hevy_id],
        |row| Ok(Decision { strategy: row.get(0)?, deadline: row.get(1)?, state: row.get(2)?, message: row.get(3)? }),
    )
    .optional()
}

/// Mock-mode stand-in for applying the choice to Garmin: records it on the
/// workout and its claimed watch activity without any network call.
pub fn mock_apply(conn: &Connection, hevy_id: &str, strategy: &str) -> DbResult<Option<&'static str>> {
    let Some(workout) = hevy_db::get_workout(conn, hevy_id)? else {
        return Ok(None);
    };
    let Some(source) = workout.source_garmin_activity_id.filter(|id| *id != 0) else {
        return Ok(None);
    };
    let status = match strategy {
        "merge" => "merged",
        "replace" => "replaced",
        "describe" => "described",
        _ => return Ok(None),
    };
    hevy_db::link_target(conn, hevy_id, source, strategy)?;
    hevy_db::set_workout_status(conn, hevy_id, status, None)?;
    hevy_db::set_applied_garmin(conn, hevy_id, workout.source_updated_at.as_deref())?;
    Ok(Some(status))
}

fn label(status: &str) -> &'static str {
    match status {
        "merged" => "Merged",
        "replaced" => "Replaced",
        "described" => "Described",
        _ => "Updated",
    }
}

/// Applies each scheduled match once its deadline passes, whether or not the
/// page that chose it is still open.
#[derive(Clone)]
pub struct HevyMatchScheduler {
    db: Db,
    events: EventBus,
    clock: Clock,
    pub window_seconds: f64,
    wake: Arc<Notify>,
}

impl HevyMatchScheduler {
    pub fn new(db: Db, events: EventBus, clock: Clock, window_seconds: f64) -> Self {
        Self { db, events, clock, window_seconds, wake: Arc::new(Notify::new()) }
    }

    pub fn now(&self) -> f64 {
        (self.clock)()
    }

    pub fn schedule(&self, hevy_id: &str, strategy: &str) -> Result<Scheduled, MatchError> {
        let scheduled = schedule(&mut self.db.lock(), hevy_id, strategy, self.now(), self.window_seconds)?;
        self.wake.notify_one();
        self.events.publish("refresh");
        Ok(scheduled)
    }

    pub fn cancel(&self, conn: &Connection, hevy_id: &str) -> DbResult<usize> {
        let cancelled = cancel(conn, hevy_id)?;
        if cancelled > 0 {
            self.events.publish("refresh");
        }
        Ok(cancelled)
    }

    pub fn run_due(&self) -> DbResult<usize> {
        let claimed = claim_due(&self.db.lock(), self.now())?;
        if claimed.is_empty() {
            return Ok(0);
        }
        self.events.publish("refresh");
        for (hevy_id, strategy, _) in &claimed {
            let conn = self.db.lock();
            let (state, message) = run(&conn, hevy_id, strategy)?;
            finish(&conn, hevy_id, state, message.as_deref(), self.now())?;
            drop(conn);
            self.events.publish("refresh");
        }
        prune(&self.db.lock(), self.now())?;
        Ok(claimed.len())
    }

    fn sleep_seconds(&self) -> DbResult<f64> {
        Ok(match next_deadline(&self.db.lock())? {
            None => MAX_SLEEP_SECONDS,
            Some(deadline) => (deadline - self.now()).clamp(0.0, MAX_SLEEP_SECONDS),
        })
    }

    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if let Ok(recovered) = recover(&self.db.lock()) {
                if recovered > 0 {
                    eprintln!("resuming {recovered} Hevy match decision(s) interrupted by a restart");
                }
            }
            loop {
                let timeout = match self.run_due().and_then(|_| self.sleep_seconds()) {
                    Ok(seconds) => seconds,
                    Err(err) => {
                        eprintln!("Hevy match scheduler pass failed: {err}");
                        1.0
                    }
                };
                tokio::select! {
                    _ = self.wake.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs_f64(timeout)) => {}
                }
            }
        })
    }
}

/// A workout skipped or handled elsewhere during the window finishes as
/// skipped: the owner's later decision wins.
fn run(conn: &Connection, hevy_id: &str, strategy: &str) -> DbResult<(&'static str, Option<String>)> {
    let Some(workout) = hevy_db::get_workout(conn, hevy_id)? else {
        return Ok((SKIPPED, None));
    };
    if workout.status != "awaiting_match" {
        return Ok((SKIPPED, None));
    }
    match mock_apply(conn, hevy_id, strategy) {
        Ok(Some(status)) => Ok((DONE, Some(format!("{} {}.", label(status), workout.display_title())))),
        Ok(None) => {
            eprintln!("Hevy match decision failed for {hevy_id}: no claimed watch activity");
            Ok((FAILED, Some(FAILURE_MESSAGE.into())))
        }
        Err(err) => {
            eprintln!("Hevy match decision failed for {hevy_id}: {err}");
            Ok((FAILED, Some(FAILURE_MESSAGE.into())))
        }
    }
}
