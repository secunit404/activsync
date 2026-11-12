//! Garmin activities ↔ local status ↔ Strava: the Garmin fetch reconciles
//! the activity table, the publish pass uploads pending rows, and one Strava
//! window read links activities that reached Strava some other way and flags
//! ones deleted there.

mod publish;
mod python_json;

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use crate::config::Config;
use crate::db::{self, ActivityRow, Db, NewActivity};
use crate::garmin_client::{GarminClient, GarminError};
use crate::strava_client::{self, StravaClient, StravaError};

pub use crate::routes::content_hash;
pub use publish::{publish_now, publish_pending};
pub use python_json::garmin_data_json;

/// How far a reconnect catch-up may widen the fetch, so a long-dormant
/// instance does not hammer Garmin.
const CATCH_UP_MAX_DAYS: i64 = 90;

/// A category hold is a standing rule about a type, lifted when the type
/// becomes an autosync category; a backlog hold is about one activity and
/// only the user lifts it.
pub const HOLD_CATEGORY: &str = "category";
pub const HOLD_BACKLOG: &str = "backlog";

/// Widens the Strava window so an activity whose Strava start sits just
/// outside the cutoff still turns up instead of looking deleted.
const WINDOW_PADDING_MINUTES: i64 = 30;

const GARMIN_TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

#[derive(Debug)]
pub enum SyncError {
    Db(rusqlite::Error),
    Garmin(GarminError),
    Strava(StravaError),
    /// Publishing is unsafe until the associated Hevy work is resolved.
    Blocked(String),
    Invalid(String),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Db(err) => write!(f, "{err}"),
            Self::Garmin(err) => write!(f, "{err}"),
            Self::Strava(err) => write!(f, "{err}"),
            Self::Blocked(reason) | Self::Invalid(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<rusqlite::Error> for SyncError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Db(err)
    }
}

impl From<GarminError> for SyncError {
    fn from(err: GarminError) -> Self {
        Self::Garmin(err)
    }
}

impl From<StravaError> for SyncError {
    fn from(err: StravaError) -> Self {
        Self::Strava(err)
    }
}

pub type SyncResult<T> = Result<T, SyncError>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GarminSyncStats {
    pub new: i64,
    pub updated: i64,
    pub removed: i64,
    pub held_backlog: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PublishStats {
    pub published: i64,
    pub failed: i64,
    pub blocked: i64,
    pub blocked_reasons: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatusCheckStats {
    pub flagged_missing: i64,
    pub linked_existing: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatchUpStats {
    pub garmin: GarminSyncStats,
    pub status: StatusCheckStats,
    pub lookback_days: i64,
}

fn parse_garmin_time(text: &str) -> SyncResult<DateTime<Utc>> {
    crate::timefmt::parse_garmin(text).ok_or_else(|| {
        SyncError::Invalid(format!(
            "time data {text:?} does not match format '{GARMIN_TIME_FORMAT}'"
        ))
    })
}

fn parse_start(row: &ActivityRow) -> SyncResult<DateTime<Utc>> {
    parse_garmin_time(&row.start_time)
}

/// Python's `float()` of a stored setting: numbers, numeric strings and
/// booleans; `None` for anything else.
pub(crate) fn as_python_float(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

fn rows_within(
    db: &Db,
    statuses: &[&str],
    cutoff: DateTime<Utc>,
) -> SyncResult<Vec<(ActivityRow, DateTime<Utc>)>> {
    let conn = db.lock();
    let mut rows = Vec::new();
    for status in statuses {
        for row in db::list_activities_with_status(&conn, status)? {
            let start = parse_start(&row)?;
            if start >= cutoff {
                rows.push((row, start));
            }
        }
    }
    Ok(rows)
}

pub struct Services<'a> {
    pub db: &'a Db,
    pub garmin: &'a GarminClient,
    pub strava: &'a StravaClient,
}

/// Only a rejection clears `garmin_credentials_verified`, which pauses the
/// Garmin leg until a reconnect; an outage is retried on the next poll.
pub fn record_sync_outcome(
    db: &Db,
    now: DateTime<Utc>,
    error: Option<&GarminError>,
) -> SyncResult<()> {
    let conn = db.lock();
    let at = json!(db::iso(now));
    db::set_config_value(&conn, "garmin_last_sync_at", &at)?;
    if error.is_none() {
        db::set_config_value(&conn, "garmin_last_sync_ok_at", &at)?;
    }
    db::set_config_value(&conn, "garmin_last_sync_ok", &json!(error.is_none()))?;
    db::set_config_value(
        &conn,
        "garmin_last_sync_error",
        &json!(error.map(ToString::to_string)),
    )?;
    if !error.is_some_and(GarminError::is_transient) {
        db::set_config_value(
            &conn,
            "garmin_credentials_verified",
            &json!(error.is_none()),
        )?;
    }
    Ok(())
}

/// Fetches recent Garmin activities into the table and records the outcome
/// for the Settings page. `hold_before` holds newly seen activities that
/// started before it, so an outage backlog is reviewed rather than
/// blast-published; rows already known keep their status.
pub async fn sync_garmin(
    db: &Db,
    garmin: &GarminClient,
    cfg: &Config,
    now: DateTime<Utc>,
    lookback_days: Option<i64>,
    hold_before: Option<DateTime<Utc>>,
) -> SyncResult<GarminSyncStats> {
    let held_types: HashSet<String> = cfg.held_activity_types().into_iter().collect();
    let lookback = lookback_days.unwrap_or_else(|| cfg.int("lookback_days"));
    let hold_cutoff = hold_before.map(|at| at.format(GARMIN_TIME_FORMAT).to_string());

    let fetched = match garmin.fetch_recent_activities(lookback).await {
        Ok(fetched) => fetched,
        Err(err) => {
            record_sync_outcome(db, now, Some(&err))?;
            crate::log_warn!("sync", "garmin fetch failed: {err}");
            return Err(err.into());
        }
    };

    let mut stats = GarminSyncStats::default();
    let mut fetched_ids = HashSet::new();
    let conn = db.lock();
    for act in &fetched {
        fetched_ids.insert(act.garmin_activity_id);
        let garmin_data = garmin_data_json(act);
        let new_hash = content_hash(
            &act.title,
            &act.description,
            &act.activity_type,
            &garmin_data,
        );
        let Some(existing) = db::get_activity(&conn, act.garmin_activity_id)? else {
            let (status, hold_reason) = if hold_cutoff
                .as_deref()
                .is_some_and(|cutoff| act.start_time.as_str() < cutoff)
            {
                stats.held_backlog += 1;
                ("held", Some(HOLD_BACKLOG))
            } else if held_types.contains(&act.activity_type) {
                ("held", Some(HOLD_CATEGORY))
            } else {
                ("pending", None)
            };
            let new = NewActivity {
                garmin_activity_id: act.garmin_activity_id,
                activity_type: &act.activity_type,
                title: &act.title,
                description: &act.description,
                start_time: &act.start_time,
                content_hash: &new_hash,
                publish_status: status,
                garmin_data: &garmin_data,
                hold_reason,
            };
            db::insert_activity(&conn, &new, now)?;
            stats.new += 1;
            continue;
        };
        if existing.content_hash == new_hash {
            continue;
        }
        db::update_activity_content(
            &conn,
            act.garmin_activity_id,
            &act.title,
            &act.description,
            &act.activity_type,
            &new_hash,
            &existing.publish_status,
            &garmin_data,
        )?;
        stats.updated += 1;
    }

    // The fetch measured its cutoff a moment after `now`, so its span can be
    // slightly narrower; shrink the removal window so an activity in that
    // sliver is not dropped merely for missing this fetch.
    let window_start = (now - Duration::days(lookback) + Duration::minutes(5))
        .format(GARMIN_TIME_FORMAT)
        .to_string();
    let mut gone: Vec<i64> = db::list_active_ids_since(&conn, &window_start)?
        .difference(&fetched_ids)
        .copied()
        .collect();
    gone.sort_unstable();
    for garmin_activity_id in gone {
        db::mark_removed(&conn, garmin_activity_id)?;
        stats.removed += 1;
    }
    drop(conn);
    record_sync_outcome(db, now, None)?;
    crate::log_info!(
        "sync",
        "garmin sync: {} new, {} updated, {} removed",
        stats.new,
        stats.updated,
        stats.removed
    );
    Ok(stats)
}

/// Links pending and held activities that reached Strava another way (held
/// only means "not automatically"), and flags published ones deleted on
/// Strava as missing. One window read answers both: a request per activity
/// would exhaust Strava's short-term quota on any busy week.
pub async fn check_strava_status(
    db: &Db,
    strava: &StravaClient,
    cfg: &Config,
    now: DateTime<Utc>,
    lookback_days: Option<i64>,
) -> SyncResult<StatusCheckStats> {
    let mut stats = StatusCheckStats::default();
    let lookback = lookback_days.unwrap_or_else(|| cfg.int("lookback_days"));
    let cutoff = now - Duration::days(lookback);
    let to_link = rows_within(db, &["pending", "held"], cutoff)?;
    let to_verify = rows_within(db, &["published"], cutoff)?;
    if to_link.is_empty() && to_verify.is_empty() {
        return Ok(stats);
    }
    let padding = Duration::minutes(WINDOW_PADDING_MINUTES);
    let on_strava = strava
        .list_activities_between(cutoff - padding, now + padding)
        .await?;

    let live: HashSet<i64> = on_strava.iter().map(|a| a.id).collect();
    let mut unclaimed = on_strava;
    let conn = db.lock();
    for (row, start) in &to_link {
        let Some(existing) = strava_client::match_closest_activity(
            &unclaimed,
            *start,
            strava_client::DEFAULT_MATCH_TOLERANCE_MINUTES,
        ) else {
            continue;
        };
        // Each Strava entry is the twin of one Garmin activity only, or two
        // activities minutes apart would both claim it.
        unclaimed.retain(|a| a.id != existing);
        db::set_published(&conn, row.garmin_activity_id, existing, now)?;
        stats.linked_existing += 1;
    }
    for (row, _) in &to_verify {
        if !row.strava_activity_id.is_some_and(|id| live.contains(&id)) {
            db::set_publish_status(&conn, row.garmin_activity_id, "missing", None)?;
            stats.flagged_missing += 1;
        }
    }
    if stats.flagged_missing > 0 {
        crate::log_info!(
            "sync",
            "strava status check: {} activities flagged missing",
            stats.flagged_missing
        );
    }
    if stats.linked_existing > 0 {
        crate::log_info!(
            "sync",
            "strava status check: {} activities linked to existing Strava entries",
            stats.linked_existing
        );
    }
    Ok(stats)
}

pub async fn edit_activity_metadata(
    services: &Services<'_>,
    garmin_activity_id: i64,
    title: &str,
    description: &str,
) -> SyncResult<()> {
    let (title, description) = (title.trim(), description.trim());
    if title.is_empty() {
        return Err(SyncError::Invalid("Activity title cannot be blank".into()));
    }
    let row = db::get_activity(&services.db.lock(), garmin_activity_id)?.ok_or_else(|| {
        SyncError::Invalid(format!("Activity {garmin_activity_id} was not found"))
    })?;
    services
        .garmin
        .update_activity_metadata(garmin_activity_id, title, description)
        .await?;
    if row.publish_status == "published"
        && let Some(strava_id) = row.strava_activity_id.filter(|id| *id != 0)
    {
        services
            .strava
            .update_activity_metadata(strava_id, title, description)
            .await?;
    }
    let garmin_data = if row.garmin_data.is_empty() {
        "{}"
    } else {
        &row.garmin_data
    };
    let hash = content_hash(title, description, &row.activity_type, garmin_data);
    db::update_activity_metadata(
        &services.db.lock(),
        garmin_activity_id,
        title,
        description,
        &hash,
    )?;
    Ok(())
}

/// Whole days between two instants, floored as Python's `timedelta.days`.
fn days_between(later: DateTime<Utc>, earlier: DateTime<Utc>) -> i64 {
    (later - earlier).num_seconds().div_euclid(86_400)
}

/// How far back a reconnect must look to cover the outage: never narrower
/// than the configured lookback, never wider than the cap. `last_sync_ok_at`
/// overrides the stored value; `Some(None)` means "never synced".
pub fn catch_up_lookback_days(
    conn: &rusqlite::Connection,
    cfg: &Config,
    now: DateTime<Utc>,
    last_sync_ok_at: Option<Option<String>>,
) -> SyncResult<i64> {
    let last_ok = match last_sync_ok_at {
        Some(value) => value,
        None => db::get_config_value(conn, "garmin_last_sync_ok_at")?
            .and_then(|v| v.as_str().map(str::to_string)),
    };
    let lookback = cfg.int("lookback_days");
    let Some(last_ok) = last_ok else {
        return Ok(lookback);
    };
    let last_ok = DateTime::parse_from_rfc3339(&last_ok)
        .map_err(|_| SyncError::Invalid(format!("Invalid isoformat string: {last_ok:?}")))?
        .with_timezone(&Utc);
    let outage_days = days_between(now, last_ok) + 1;
    Ok(lookback.max(outage_days.min(CATCH_UP_MAX_DAYS)))
}

/// Measured from the oldest activity still awaiting a decision, not from
/// the Garmin outage: during a Strava-only outage pending rows pile up while
/// Garmin syncs fine, and publishing them unchecked would duplicate whatever
/// Garmin's own Strava sync already pushed.
pub fn strava_status_lookback_days(
    conn: &rusqlite::Connection,
    cfg: &Config,
    now: DateTime<Utc>,
) -> SyncResult<i64> {
    let lookback = cfg.int("lookback_days");
    let Some(oldest) = db::oldest_unpublished_start_time(conn)? else {
        return Ok(lookback);
    };
    let backlog_days = days_between(now, parse_garmin_time(&oldest)?) + 1;
    Ok(lookback.max(backlog_days.min(CATCH_UP_MAX_DAYS)))
}

/// Brings the activity list up to date after a reconnect: the window spans
/// the outage, activities visible only because of that are held, and Strava
/// status is reconciled across the backlog. The Garmin half is skipped while
/// Garmin is disconnected, so reconnecting Strava first still reconciles.
pub async fn catch_up_sync(
    services: &Services<'_>,
    cfg: &Config,
    now: DateTime<Utc>,
    last_sync_ok_at: Option<Option<String>>,
) -> SyncResult<CatchUpStats> {
    let (lookback, garmin_connected) = {
        let conn = services.db.lock();
        let lookback = catch_up_lookback_days(&conn, cfg, now, last_sync_ok_at)?;
        (
            lookback,
            db::truthy(db::get_config_value(&conn, "garmin_credentials_verified")?.as_ref()),
        )
    };
    let hold_before = now - Duration::days(cfg.int("lookback_days"));
    let garmin = if garmin_connected {
        sync_garmin(
            services.db,
            services.garmin,
            cfg,
            now,
            Some(lookback),
            Some(hold_before),
        )
        .await?
    } else {
        crate::log_info!(
            "sync",
            "catch-up: garmin is not connected, skipping the garmin fetch"
        );
        GarminSyncStats::default()
    };
    let status_lookback = strava_status_lookback_days(&services.db.lock(), cfg, now)?;
    let status = check_strava_status(
        services.db,
        services.strava,
        cfg,
        now,
        Some(status_lookback),
    )
    .await?;
    crate::log_info!(
        "sync",
        "catch-up sync over {lookback} days (status {status_lookback} days): {} new ({} held as backlog), {} linked",
        garmin.new,
        garmin.held_backlog,
        status.linked_existing
    );
    Ok(CatchUpStats {
        garmin,
        status,
        lookback_days: lookback,
    })
}
