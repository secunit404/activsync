//! Server-side undo window for Hevy match decisions, as in the Python app's
//! `hevy_match_schedule.py`. The scheduler claims with `WHERE state =
//! 'waiting'` and undo deletes with the same guard, so only one of them can
//! match a row.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
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
pub const AUTH_MESSAGE: &str = "Hevy rejected the stored API key — reconnect Hevy.";
pub const MAPPING_MESSAGE: &str = "Map the missing exercises before choosing Merge or Replace.";

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

#[derive(Debug, Clone, PartialEq)]
pub enum ApplyFailure {
    /// Hevy rejected the stored API key.
    HevyAuth,
    Failed(String),
}

/// Applies one owner's choice once its undo window has passed; returns the
/// workout's new status.
pub trait MatchApplier: Send + Sync {
    fn apply<'a>(&'a self, hevy_id: &'a str, strategy: &'a str) -> BoxFuture<'a, Result<String, ApplyFailure>>;
}

pub struct MockApplier {
    pub db: Db,
}

impl MatchApplier for MockApplier {
    fn apply<'a>(&'a self, hevy_id: &'a str, strategy: &'a str) -> BoxFuture<'a, Result<String, ApplyFailure>> {
        let outcome = match mock_apply(&self.db.lock(), hevy_id, strategy) {
            Ok(Some(status)) => Ok(status.to_string()),
            Ok(None) => Err(ApplyFailure::Failed("no claimed watch activity".into())),
            Err(err) => Err(ApplyFailure::Failed(err.to_string())),
        };
        Box::pin(async move { outcome })
    }
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
    applier: Arc<dyn MatchApplier>,
}

impl HevyMatchScheduler {
    pub fn new(db: Db, events: EventBus, clock: Clock, window_seconds: f64) -> Self {
        let applier = Arc::new(MockApplier { db: db.clone() });
        Self { db, events, clock, window_seconds, wake: Arc::new(Notify::new()), applier }
    }

    pub fn with_applier(self, applier: Arc<dyn MatchApplier>) -> Self {
        Self { applier, ..self }
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

    pub async fn run_due(&self) -> DbResult<usize> {
        let claimed = claim_due(&self.db.lock(), self.now())?;
        if claimed.is_empty() {
            return Ok(0);
        }
        self.events.publish("refresh");
        for (hevy_id, strategy, _) in &claimed {
            let (state, message) = self.run(hevy_id, strategy).await?;
            finish(&self.db.lock(), hevy_id, state, message.as_deref(), self.now())?;
            self.events.publish("refresh");
        }
        prune(&self.db.lock(), self.now())?;
        Ok(claimed.len())
    }

    /// A workout skipped or handled elsewhere during the window finishes as
    /// skipped: the owner's later decision wins.
    async fn run(&self, hevy_id: &str, strategy: &str) -> DbResult<(&'static str, Option<String>)> {
        let Some(workout) = hevy_db::get_workout(&self.db.lock(), hevy_id)? else {
            return Ok((SKIPPED, None));
        };
        if workout.status != "awaiting_match" {
            return Ok((SKIPPED, None));
        }
        Ok(match self.applier.apply(hevy_id, strategy).await {
            Ok(status) if status == "needs_mapping" => (FAILED, Some(MAPPING_MESSAGE.into())),
            Ok(status) => (DONE, Some(format!("{} {}.", label(&status), workout.display_title()))),
            Err(ApplyFailure::HevyAuth) => {
                crate::db::set_config_value(&self.db.lock(), "hevy_auth_ok", &serde_json::json!(false))?;
                (FAILED, Some(AUTH_MESSAGE.into()))
            }
            Err(ApplyFailure::Failed(err)) => {
                crate::log_warn!("hevy_match", "Hevy match decision failed for {hevy_id}: {err}");
                (FAILED, Some(FAILURE_MESSAGE.into()))
            }
        })
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
                    crate::log_info!("hevy_match", "resuming {recovered} Hevy match decision(s) interrupted by a restart");
                }
            }
            loop {
                let timeout = match self.run_due().await.and_then(|_| self.sleep_seconds()) {
                    Ok(seconds) => seconds,
                    Err(err) => {
                        crate::log_warn!("hevy_match", "Hevy match scheduler pass failed: {err}");
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

