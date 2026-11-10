//! The background poller, as the Python app's `poller.py`: Garmin fetch,
//! Strava publish and status check, and the Hevy leg each run on their own
//! interval, checked on a short shared tick.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use crate::config::{self, Config};
use crate::db::{self, Db};
use crate::events::EventBus;
use crate::garmin_client::{GarminClient, GarminError};
use crate::net::Http;
use crate::schedule::Clock;
use crate::strava_client::{StravaClient, StravaError};
use crate::sync::{self, GarminSyncStats, PublishStats, Services, StatusCheckStats, SyncError};

pub const DEFAULT_TICK_SECONDS: u64 = 60;
const SESSION_PRUNE_SECONDS: f64 = 3600.0;
const HEVY_UPLOAD_RECHECK_SECONDS: f64 = 60.0;
/// Right after a session the watch syncs and a Hevy workout often follows,
/// so Garmin is polled this often for a while instead of the configured
/// (slower) interval.
pub const ACTIVE_GARMIN_INTERVAL_SECONDS: f64 = 5.0 * 60.0;
const ACTIVE_WINDOW_HOURS: i64 = 2;
const MAX_SESSION_HOURS: i64 = 24;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Builds the service clients for one pass. Garmin is opened fresh each
/// time from the saved session, never by password.
pub trait Clients: Send + Sync {
    fn garmin(&self) -> BoxFuture<'_, Result<GarminClient, GarminError>>;
    fn strava(&self) -> StravaClient;
}

#[derive(Debug, Clone, PartialEq)]
pub enum HevyLegError {
    /// Hevy rejected the API key; the leg pauses until it is re-validated.
    Auth,
    Failed(String),
}

/// One pass of the Hevy integration. Returns whether anything changed on
/// Garmin.
pub trait HevyLeg: Send + Sync {
    fn run<'a>(
        &'a self,
        db: &'a Db,
        garmin: &'a GarminClient,
        strava: Option<&'a StravaClient>,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<bool, HevyLegError>>;
}

#[derive(Clone)]
pub struct Options {
    pub tick: StdDuration,
    pub garmin_interval_override: Option<f64>,
    pub strava_interval_override: Option<f64>,
    pub hevy_interval_override: Option<f64>,
    pub garmin_polling: bool,
    pub strava_polling: bool,
    pub hevy_polling: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            tick: StdDuration::from_secs(DEFAULT_TICK_SECONDS),
            garmin_interval_override: None,
            strava_interval_override: None,
            hevy_interval_override: None,
            garmin_polling: true,
            strava_polling: true,
            hevy_polling: true,
        }
    }
}

#[derive(Debug)]
pub enum PollError {
    Garmin(GarminError),
    Sync(SyncError),
    Db(rusqlite::Error),
}

impl std::fmt::Display for PollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Garmin(err) => write!(f, "{err}"),
            Self::Sync(err) => write!(f, "{err}"),
            Self::Db(err) => write!(f, "{err}"),
        }
    }
}

impl From<SyncError> for PollError {
    fn from(err: SyncError) -> Self {
        match err {
            SyncError::Garmin(err) => Self::Garmin(err),
            other => Self::Sync(other),
        }
    }
}

impl From<rusqlite::Error> for PollError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Db(err)
    }
}

pub struct Poller {
    db: Db,
    events: EventBus,
    clients: Arc<dyn Clients>,
    hevy: Option<Arc<dyn HevyLeg>>,
    options: Options,
    last_garmin_run: Option<DateTime<Utc>>,
    last_strava_run: Option<DateTime<Utc>>,
    last_hevy_run: Option<DateTime<Utc>>,
    strava_backoff_until: Option<DateTime<Utc>>,
    last_session_prune: Option<DateTime<Utc>>,
}

fn due(last_run: Option<DateTime<Utc>>, now: DateTime<Utc>, interval_seconds: f64) -> bool {
    last_run.is_none_or(|last| (now - last).num_milliseconds() as f64 / 1000.0 >= interval_seconds)
}

fn minutes_setting(cfg: &Config, key: &str) -> f64 {
    let value = cfg.get(key).and_then(Value::as_f64);
    value.or_else(|| config::defaults()[key].as_f64()).unwrap_or_default() * 60.0
}

fn has_workouts_with_status(conn: &rusqlite::Connection, status: &str) -> rusqlite::Result<bool> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM hevy_workouts WHERE status = ?1)", [status], |row| row.get(0))
}

fn has_unresolved_upload(conn: &rusqlite::Connection) -> rusqlite::Result<bool> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM hevy_operations WHERE phase = 'submission_unknown')", [], |row| row.get(0))
}

/// The duration in `garmin_data`, 0 when absent or unreadable: it only
/// widens a time window, so a bad blob reads as "unknown", never an error.
fn activity_duration_seconds(garmin_data: &str) -> f64 {
    let data: Value = serde_json::from_str(if garmin_data.is_empty() { "{}" } else { garmin_data }).unwrap_or(Value::Null);
    match data.get("duration") {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0.0),
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        _ => 0.0,
    }
}

/// Whether the user has just trained: an activity or Hevy workout ended in
/// the last two hours, or a workout is still waiting for its watch activity.
pub fn recently_active(conn: &rusqlite::Connection, now: DateTime<Utc>) -> rusqlite::Result<bool> {
    let window_start = now - Duration::hours(ACTIVE_WINDOW_HOURS);
    let scan_from = now - Duration::hours(MAX_SESSION_HOURS);

    let mut stmt = conn.prepare("SELECT start_time, garmin_data FROM activities WHERE start_time >= ?1")?;
    let activities = stmt.query_map([scan_from.format("%Y-%m-%d %H:%M:%S").to_string()], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for activity in activities {
        let (start, garmin_data) = activity?;
        let Some(start) = crate::timefmt::parse_timestamp(&start) else {
            continue;
        };
        let end = start + Duration::milliseconds((activity_duration_seconds(&garmin_data) * 1000.0) as i64);
        if end >= window_start {
            return Ok(true);
        }
    }

    let mut stmt = conn.prepare("SELECT end_time FROM hevy_workouts WHERE start_time >= ?1")?;
    let workouts = stmt.query_map([scan_from.format("%Y-%m-%d").to_string()], |row| row.get::<_, String>(0))?;
    for end in workouts {
        if crate::timefmt::parse_timestamp(&end?).is_some_and(|end| end >= window_start) {
            return Ok(true);
        }
    }
    Ok(has_workouts_with_status(conn, "waiting_watch")? || has_workouts_with_status(conn, "syncing")?)
}

impl Poller {
    pub fn new(db: Db, events: EventBus, clients: Arc<dyn Clients>, hevy: Option<Arc<dyn HevyLeg>>, options: Options) -> Self {
        Self {
            db,
            events,
            clients,
            hevy,
            options,
            last_garmin_run: None,
            last_strava_run: None,
            last_hevy_run: None,
            strava_backoff_until: None,
            last_session_prune: None,
        }
    }

    fn config(&self) -> Result<Config, PollError> {
        Ok(config::load_config(&self.db.lock(), false)?)
    }

    fn config_value(&self, key: &str) -> Option<Value> {
        db::get_config_value(&self.db.lock(), key).ok().flatten()
    }

    /// An expired session pauses Garmin durably: the Settings page shows it
    /// and the poller stops trying until the user reconnects. An outage is
    /// shown as a failed attempt and retried.
    async fn garmin(&self, now: DateTime<Utc>) -> Result<GarminClient, PollError> {
        match self.clients.garmin().await {
            Ok(garmin) => Ok(garmin),
            Err(err) => {
                if matches!(err, GarminError::SessionExpired | GarminError::Unavailable) {
                    sync::record_sync_outcome(&self.db, now, Some(&err))?;
                    self.events.publish("refresh");
                    eprintln!("{err}");
                }
                Err(PollError::Garmin(err))
            }
        }
    }

    pub async fn run_garmin_once(&self, now: DateTime<Utc>) -> Result<GarminSyncStats, PollError> {
        let cfg = self.config()?;
        let garmin = self.garmin(now).await?;
        Ok(sync::sync_garmin(&self.db, &garmin, &cfg, now, None, None).await?)
    }

    pub async fn run_strava_once(&self, now: DateTime<Utc>) -> Result<(PublishStats, StatusCheckStats), PollError> {
        let cfg = self.config()?;
        let garmin = self.garmin(now).await?;
        let strava = self.clients.strava();
        let services = Services { db: &self.db, garmin: &garmin, strava: &strava };
        let published = sync::publish_pending(&services, now, None).await?;
        let status = sync::check_strava_status(&self.db, &strava, &cfg, now, None).await?;
        Ok((published, status))
    }

    pub async fn run_hevy_once(&self, now: DateTime<Utc>) -> Result<bool, HevyLegError> {
        let Some(leg) = self.hevy.clone() else {
            return Ok(false);
        };
        let garmin = self.garmin(now).await.map_err(|e| HevyLegError::Failed(e.to_string()))?;
        let strava = self.strava_ready().then(|| self.clients.strava());
        let changed = leg.run(&self.db, &garmin, strava.as_ref(), now).await?;
        // A top-level key, not a setting: it drives the publish path's
        // settlement hold.
        let _ = db::set_config_value(&self.db.lock(), "hevy_last_success_at", &json!(db::iso(now)));
        Ok(changed)
    }

    fn garmin_interval_seconds(&self, now: DateTime<Utc>) -> Result<f64, PollError> {
        if let Some(seconds) = self.options.garmin_interval_override {
            return Ok(seconds);
        }
        let interval = minutes_setting(&self.config()?, "garmin_poll_interval_minutes");
        if recently_active(&self.db.lock(), now)? {
            return Ok(interval.min(ACTIVE_GARMIN_INTERVAL_SECONDS));
        }
        Ok(interval)
    }

    fn strava_interval_seconds(&self) -> Result<f64, PollError> {
        match self.options.strava_interval_override {
            Some(seconds) => Ok(seconds),
            None => Ok(minutes_setting(&self.config()?, "strava_poll_interval_minutes")),
        }
    }

    fn hevy_interval_seconds(&self) -> Result<f64, PollError> {
        let interval = match self.options.hevy_interval_override {
            Some(seconds) => seconds,
            None => minutes_setting(&self.config()?, "hevy_poll_interval_minutes"),
        };
        if has_unresolved_upload(&self.db.lock())? {
            return Ok(interval.min(HEVY_UPLOAD_RECHECK_SECONDS));
        }
        Ok(interval)
    }

    fn hevy_waiting_for_watch(&self) -> bool {
        has_workouts_with_status(&self.db.lock(), "waiting_watch").unwrap_or(false)
    }

    fn garmin_ready(&self) -> bool {
        db::truthy(self.config_value("garmin_credentials_verified").as_ref())
    }

    fn strava_ready(&self) -> bool {
        self.config_value("strava_tokens").is_some_and(|tokens| db::truthy(tokens.get("refresh_token")))
    }

    fn hevy_ready(&self) -> Result<bool, PollError> {
        if self.hevy.is_none() || !db::truthy(self.config()?.get("hevy_enabled")) {
            return Ok(false);
        }
        if !db::truthy(self.config_value("hevy_api_key").as_ref()) {
            return Ok(false);
        }
        Ok(self.config_value("hevy_auth_ok") != Some(json!(false)))
    }

    fn strava_rate_limited(&self, now: DateTime<Utc>) -> bool {
        self.strava_backoff_until.is_some_and(|until| now < until)
    }

    /// One Strava pass with failures absorbed; whether anything changed. A
    /// 429 means the quota is spent, so Strava is left alone until the time
    /// the client worked out when the 429 landed: retrying each tick is what
    /// keeps a quota exhausted.
    async fn run_strava_guarded(&mut self, now: DateTime<Utc>, failure: &str) -> bool {
        match self.run_strava_once(now).await {
            Ok((published, status)) => {
                self.strava_backoff_until = None;
                published.published > 0 || published.failed > 0 || status.flagged_missing > 0 || status.linked_existing > 0
            }
            Err(PollError::Sync(SyncError::Strava(StravaError::RateLimited(retry_at)))) => {
                self.strava_backoff_until = Some(retry_at);
                eprintln!("strava rate limit reached; pausing strava sync until {}", retry_at.format("%Y-%m-%d %H:%M:%S UTC"));
                false
            }
            Err(err) => {
                eprintln!("{failure}: {err}");
                false
            }
        }
    }

    /// Stays out of the way until first-run setup has finished: the wizard
    /// runs its own initial sync.
    fn ready_for_polling(&self) -> bool {
        db::truthy(self.config_value("initial_sync_done").as_ref())
    }

    fn prune_sessions(&mut self, now: DateTime<Utc>) {
        if due(self.last_session_prune, now, SESSION_PRUNE_SECONDS) {
            let now_epoch = now.timestamp_micros() as f64 / 1_000_000.0;
            if let Err(err) = crate::auth_core::prune_expired_sessions(&self.db.lock(), now_epoch) {
                eprintln!("session prune failed: {err}");
            }
            self.last_session_prune = Some(now);
        }
    }

    pub async fn loop_once(&mut self, now: DateTime<Utc>) -> Result<(), PollError> {
        self.prune_sessions(now);
        if !self.ready_for_polling() {
            return Ok(());
        }
        let mut changed = false;
        let mut strava_ran_this_tick = false;

        // Hevy runs first: when it changed Garmin, or a workout still waits
        // for its watch activity, the Garmin leg is made due this very tick
        // so the result reaches Strava now rather than an interval later.
        if self.options.hevy_polling && self.hevy_ready()? && due(self.last_hevy_run, now, self.hevy_interval_seconds()?) {
            match self.run_hevy_once(now).await {
                Ok(hevy_changed) => {
                    changed |= hevy_changed;
                    if self.options.garmin_polling && (hevy_changed || self.hevy_waiting_for_watch()) {
                        self.last_garmin_run = None;
                    }
                }
                Err(HevyLegError::Auth) => {
                    db::set_config_value(&self.db.lock(), "hevy_auth_ok", &json!(false))?;
                    eprintln!("hevy auth failed; pausing hevy sync until the key is updated");
                }
                Err(HevyLegError::Failed(err)) => eprintln!("hevy sync failed: {err}"),
            }
            self.last_hevy_run = Some(now);
        }

        // A side that is not ready keeps its stale last run, so the tick
        // after a reconnect is due at once instead of an interval later.
        if self.options.garmin_polling && self.garmin_ready() && due(self.last_garmin_run, now, self.garmin_interval_seconds(now)?) {
            match self.run_garmin_once(now).await {
                Ok(stats) => {
                    // Only new activities: "updated" can recur every poll and
                    // would ping-pong the two legs each tick.
                    if stats.new > 0 && self.hevy_waiting_for_watch() {
                        self.last_hevy_run = None;
                    }
                    if stats.new > 0 || stats.updated > 0 || stats.removed > 0 {
                        changed = true;
                        if self.options.strava_polling && self.strava_ready() && !self.strava_rate_limited(now) {
                            changed |= self.run_strava_guarded(now, "strava status check after garmin sync failed").await;
                            self.last_strava_run = Some(now);
                            strava_ran_this_tick = true;
                        }
                    }
                }
                Err(err) => eprintln!("garmin sync failed: {err}"),
            }
            self.last_garmin_run = Some(now);
        }

        if self.options.strava_polling
            && self.garmin_ready()
            && self.strava_ready()
            && !strava_ran_this_tick
            && !self.strava_rate_limited(now)
            && due(self.last_strava_run, now, self.strava_interval_seconds()?)
        {
            changed |= self.run_strava_guarded(now, "strava sync failed").await;
            self.last_strava_run = Some(now);
        }

        if changed {
            self.events.publish("refresh");
        }
        Ok(())
    }

    /// Runs until the task is aborted, one tick per `options.tick`.
    pub fn spawn(mut self, clock: Clock) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                let now = DateTime::from_timestamp_micros((clock() * 1_000_000.0) as i64).unwrap_or_default();
                if let Err(err) = self.loop_once(now).await {
                    eprintln!("poll tick failed: {err}");
                }
                tokio::time::sleep(self.options.tick).await;
            }
        })
    }
}

/// The real clients: Garmin from the token directory, Strava from the
/// stored tokens and the app's client credentials.
pub struct RealClients {
    pub db: Db,
    pub clock: Clock,
    pub garmin_http: Http,
    pub strava_http: Http,
    pub garmin_token_dir: String,
    pub strava_client_id: String,
    pub strava_client_secret: String,
}

impl Clients for RealClients {
    fn garmin(&self) -> BoxFuture<'_, Result<GarminClient, GarminError>> {
        Box::pin(GarminClient::open(self.garmin_http.clone(), self.clock.clone(), &self.garmin_token_dir))
    }

    fn strava(&self) -> StravaClient {
        StravaClient::new(
            self.strava_http.clone(),
            self.db.clone(),
            self.clock.clone(),
            &self.strava_client_id,
            &self.strava_client_secret,
        )
    }
}
