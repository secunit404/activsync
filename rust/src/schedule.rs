//! Server-side undo window for publishing, as in the Python app's
//! `publish_schedule.py`. Each state change is one guarded statement under the
//! connection lock, so the scheduler's claim and an undo can never both match.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, Utc};
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use tokio::sync::Notify;

use crate::db::{self, ActivityRow, Db, DbResult};
use crate::events::EventBus;
use crate::garmin_client::GarminClient;
use crate::poller::Clients;
use crate::strava_client::{StravaClient, StravaError};
use crate::sync::{self, Services, SyncError};

pub const UNDO_WINDOW_SECONDS: f64 = 5.0;
pub const PUBLISHABLE_STATUSES: [&str; 3] = ["held", "pending", "missing"];

pub const WAITING: &str = "waiting";
pub const SENDING: &str = "sending";
pub const PUBLISHED: &str = "published";
pub const FAILED: &str = "failed";
pub const BLOCKED: &str = "blocked";
pub const SKIPPED: &str = "skipped";

const RESULT_RETENTION_SECONDS: f64 = 3600.0;
const MAX_SLEEP_SECONDS: f64 = 30.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleRow {
    pub garmin_activity_id: i64,
    pub batch_id: String,
    pub batch_kind: String,
    pub deadline: f64,
    pub state: String,
    pub prior_status: String,
    pub message: Option<String>,
}

impl ScheduleRow {
    fn from_row(row: &Row<'_>) -> DbResult<Self> {
        Ok(Self {
            garmin_activity_id: row.get("garmin_activity_id")?,
            batch_id: row.get("batch_id")?,
            batch_kind: row.get("batch_kind")?,
            deadline: row.get("deadline")?,
            state: row.get("state")?,
            prior_status: row.get("prior_status")?,
            message: row.get("message")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Scheduled {
    pub batch_id: String,
    pub deadline: f64,
}

#[derive(Debug)]
pub enum ScheduleError {
    AlreadyScheduled,
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for ScheduleError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Db(err)
    }
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(", ")
}

fn new_batch_id() -> String {
    let mut bytes = [0u8; 9];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn schedule(
    conn: &mut Connection,
    activities: &[ActivityRow],
    now: f64,
    kind: &str,
    window_seconds: f64,
) -> Result<Scheduled, ScheduleError> {
    let ids: Vec<i64> = activities.iter().map(|a| a.garmin_activity_id).collect();
    let batch = Scheduled { batch_id: new_batch_id(), deadline: now + window_seconds };
    let tx = conn.transaction()?;
    let active: Option<i64> = tx
        .query_row(
            &format!(
                "SELECT 1 FROM publish_schedule WHERE garmin_activity_id IN ({}) \
                 AND state IN ('{WAITING}', '{SENDING}')",
                placeholders(ids.len())
            ),
            params_from_iter(&ids),
            |row| row.get(0),
        )
        .optional()?;
    if active.is_some() {
        return Err(ScheduleError::AlreadyScheduled);
    }
    for activity in activities {
        tx.execute(
            "INSERT INTO publish_schedule
               (garmin_activity_id, batch_id, batch_kind, deadline, state, prior_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(garmin_activity_id) DO UPDATE SET
                 batch_id = excluded.batch_id,
                 batch_kind = excluded.batch_kind,
                 deadline = excluded.deadline,
                 state = excluded.state,
                 prior_status = excluded.prior_status,
                 message = NULL,
                 finished_at = NULL",
            params![
                activity.garmin_activity_id,
                batch.batch_id,
                kind,
                batch.deadline,
                WAITING,
                activity.publish_status
            ],
        )?;
    }
    tx.commit()?;
    Ok(batch)
}

/// Drops every listed activity still waiting; one already being sent stays.
pub fn cancel(conn: &Connection, ids: &[i64]) -> DbResult<usize> {
    conn.execute(
        &format!(
            "DELETE FROM publish_schedule WHERE garmin_activity_id IN ({}) AND state = '{WAITING}'",
            placeholders(ids.len())
        ),
        params_from_iter(ids),
    )
}

pub fn claim_due(conn: &Connection, now: f64) -> DbResult<Vec<(i64, f64)>> {
    let mut stmt = conn.prepare(&format!(
        "UPDATE publish_schedule SET state = '{SENDING}' \
         WHERE state = '{WAITING}' AND deadline <= ?1 \
         RETURNING garmin_activity_id, deadline"
    ))?;
    let mut rows: Vec<(i64, f64)> = stmt
        .query_map([now], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<DbResult<_>>()?;
    rows.sort_by(|a, b| a.1.total_cmp(&b.1));
    Ok(rows)
}

pub fn finish(conn: &Connection, id: i64, state: &str, message: Option<&str>, now: f64) -> DbResult<()> {
    conn.execute(
        &format!(
            "UPDATE publish_schedule SET state = ?1, message = ?2, finished_at = ?3 \
             WHERE garmin_activity_id = ?4 AND state = '{SENDING}'"
        ),
        params![state, message, now, id],
    )?;
    Ok(())
}

/// Rows a previous process was sending go back to the queue; their deadline
/// has passed, so they go out on the next pass.
pub fn recover(conn: &Connection) -> DbResult<usize> {
    conn.execute(
        &format!("UPDATE publish_schedule SET state = '{WAITING}' WHERE state = '{SENDING}'"),
        [],
    )
}

pub fn prune(conn: &Connection, now: f64) -> DbResult<()> {
    conn.execute(
        "DELETE FROM publish_schedule WHERE finished_at IS NOT NULL AND finished_at < ?1",
        [now - RESULT_RETENTION_SECONDS],
    )?;
    Ok(())
}

pub fn next_deadline(conn: &Connection) -> DbResult<Option<f64>> {
    conn.query_row(
        &format!("SELECT MIN(deadline) FROM publish_schedule WHERE state = '{WAITING}'"),
        [],
        |row| row.get(0),
    )
}

pub fn active_by_activity(conn: &Connection) -> DbResult<std::collections::HashMap<i64, (f64, String)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT garmin_activity_id, deadline, state FROM publish_schedule \
         WHERE state IN ('{WAITING}', '{SENDING}')"
    ))?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))?;
    rows.collect()
}

pub fn get(conn: &Connection, id: i64) -> DbResult<Option<ScheduleRow>> {
    conn.query_row(
        "SELECT * FROM publish_schedule WHERE garmin_activity_id = ?1",
        [id],
        ScheduleRow::from_row,
    )
    .optional()
}

pub fn batch_rows(conn: &Connection, batch_id: &str) -> DbResult<Vec<ScheduleRow>> {
    let mut stmt = conn.prepare("SELECT * FROM publish_schedule WHERE batch_id = ?1")?;
    let rows = stmt.query_map([batch_id], ScheduleRow::from_row)?;
    rows.collect()
}

/// Python's `datetime.fromtimestamp(deadline, timezone.utc).isoformat()`.
pub fn deadline_iso(deadline: f64) -> String {
    let micros = (deadline * 1_000_000.0).round() as i64;
    DateTime::<Utc>::from_timestamp_micros(micros)
        .map(db::iso)
        .unwrap_or_default()
}

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(|| Utc::now().timestamp_micros() as f64 / 1_000_000.0)
}

/// Sends each scheduled publish once its deadline passes, whether or not the
/// page that asked for it is still open.
#[derive(Clone)]
pub struct PublishScheduler {
    db: Db,
    events: EventBus,
    clock: Clock,
    pub window_seconds: f64,
    wake: Arc<Notify>,
    /// The real Garmin and Strava; without them publishing is simulated.
    clients: Option<Arc<dyn Clients>>,
}

impl PublishScheduler {
    pub fn new(db: Db, events: EventBus, clock: Clock, window_seconds: f64) -> Self {
        Self { db, events, clock, window_seconds, wake: Arc::new(Notify::new()), clients: None }
    }

    pub fn with_clients(self, clients: Arc<dyn Clients>) -> Self {
        Self { clients: Some(clients), ..self }
    }

    pub fn now(&self) -> f64 {
        (self.clock)()
    }

    pub fn schedule(&self, activities: &[ActivityRow], kind: &str) -> Result<Scheduled, ScheduleError> {
        let batch = schedule(&mut self.db.lock(), activities, self.now(), kind, self.window_seconds)?;
        self.wake.notify_one();
        self.events.publish("refresh");
        Ok(batch)
    }

    pub fn cancel(&self, ids: &[i64]) -> DbResult<usize> {
        let cancelled = cancel(&self.db.lock(), ids)?;
        if cancelled > 0 {
            self.events.publish("refresh");
        }
        Ok(cancelled)
    }

    /// Sends every publish whose deadline has passed. Returns how many.
    pub async fn run_due(&self) -> DbResult<usize> {
        let claimed = claim_due(&self.db.lock(), self.now())?;
        if claimed.is_empty() {
            return Ok(0);
        }
        self.events.publish("refresh");
        let clients = match &self.clients {
            None => None,
            Some(clients) => match clients.garmin().await {
                Ok(garmin) => Some((garmin, clients.strava())),
                Err(err) => {
                    crate::log_warn!("schedule", "scheduled publish could not reach Garmin or Strava: {err}");
                    let message = format!("Could not publish activity: {err}");
                    for (id, _) in &claimed {
                        finish(&self.db.lock(), *id, FAILED, Some(&message), self.now())?;
                    }
                    self.events.publish("refresh");
                    return Ok(claimed.len());
                }
            },
        };
        for (id, _) in &claimed {
            let (state, message) = match &clients {
                Some((garmin, strava)) => self.send(*id, garmin, strava).await?,
                None => mock_send(&self.db.lock(), *id)?,
            };
            finish(&self.db.lock(), *id, state, message.as_deref(), self.now())?;
            self.events.publish("refresh");
        }
        prune(&self.db.lock(), self.now())?;
        Ok(claimed.len())
    }

    /// The owner's later decision wins: an activity excluded from another
    /// device, or removed on Garmin, during the window is skipped.
    async fn send(&self, id: i64, garmin: &GarminClient, strava: &StravaClient) -> DbResult<(&'static str, Option<String>)> {
        let activity = db::get_activity(&self.db.lock(), id)?;
        match activity {
            Some(activity) if activity.is_published() => return Ok((PUBLISHED, None)),
            Some(activity) if PUBLISHABLE_STATUSES.contains(&activity.publish_status.as_str()) => {}
            _ => return Ok((SKIPPED, None)),
        }
        let services = Services { db: &self.db, garmin, strava };
        Ok(match sync::publish_now(&services, id, Utc::now()).await {
            Ok(()) => {
                crate::log_info!("schedule", "published activity {id} to Strava");
                (PUBLISHED, None)
            }
            Err(SyncError::Blocked(reason)) => (BLOCKED, Some(format!("Activity was blocked by Hevy sync: {reason}."))),
            Err(SyncError::Strava(StravaError::Auth(message))) => (FAILED, Some(message)),
            Err(err) => {
                crate::log_warn!("schedule", "scheduled publish failed for activity {id}: {err}");
                (FAILED, Some(format!("Could not publish activity: {err}")))
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
                    crate::log_info!("schedule", "resuming {recovered} publish(es) interrupted by a restart");
                }
            }
            loop {
                let timeout = match self.run_due().await.and_then(|_| self.sleep_seconds()) {
                    Ok(seconds) => seconds,
                    Err(err) => {
                        crate::log_warn!("schedule", "publish scheduler pass failed: {err}");
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

/// Mock-mode stand-in for the Garmin download and Strava upload: the owner's
/// later decision wins, Hevy work still blocks, and anything else publishes.
fn mock_send(conn: &Connection, id: i64) -> DbResult<(&'static str, Option<String>)> {
    let Some(activity) = db::get_activity(conn, id)? else {
        return Ok((SKIPPED, None));
    };
    if activity.is_published() {
        return Ok((PUBLISHED, None));
    }
    if !PUBLISHABLE_STATUSES.contains(&activity.publish_status.as_str()) {
        return Ok((SKIPPED, None));
    }
    if let Some((hevy_id, status)) = db::blocking_workout_for_activity(conn, id)? {
        return Ok((
            BLOCKED,
            Some(format!("Activity was blocked by Hevy sync: hevy workout {hevy_id} is {status}.")),
        ));
    }
    db::set_published(conn, id, crate::mock::STRAVA_ID_BASE + id, Utc::now())?;
    Ok((PUBLISHED, None))
}
