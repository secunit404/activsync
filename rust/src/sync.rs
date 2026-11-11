//! Garmin activities ↔ local status ↔ Strava, as the Python app's `sync.py`:
//! the Garmin fetch reconciles the activity table, the publish pass uploads
//! pending rows, and one Strava window read links activities that reached
//! Strava some other way and flags ones deleted there.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use crate::config::{self, Config};
use crate::db::{self, ActivityRow, Db, NewActivity};
use crate::garmin_client::{ActivityRecord, GarminClient, GarminError};
use crate::strava_client::{self, StravaActivity, StravaClient, StravaError};

/// How far a reconnect catch-up may widen the fetch, so a long-dormant
/// instance does not hammer Garmin.
pub const CATCH_UP_MAX_DAYS: i64 = 90;

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

pub fn content_hash(title: &str, description: &str, activity_type: &str, garmin_data: &str) -> String {
    crate::routes::content_hash(title, description, activity_type, garmin_data)
}

/// The record's metrics as `json.dumps` writes them: only those present, in
/// this order, with Python's separators.
pub fn garmin_data_json(act: &ActivityRecord) -> String {
    let fields = [
        ("distance", &act.distance),
        ("duration", &act.duration),
        ("moving_duration", &act.moving_duration),
        ("elapsed_duration", &act.elapsed_duration),
        ("elevation_gain", &act.elevation_gain),
        ("elevation_loss", &act.elevation_loss),
        ("calories", &act.calories),
        ("avg_hr", &act.avg_hr),
        ("max_hr", &act.max_hr),
        ("avg_power", &act.avg_power),
        ("max_power", &act.max_power),
        ("norm_power", &act.norm_power),
        ("aerobic_te", &act.aerobic_te),
        ("anaerobic_te", &act.anaerobic_te),
        ("training_load", &act.training_load),
        ("avg_cadence", &act.avg_cadence),
        ("max_cadence", &act.max_cadence),
        ("total_sets", &act.total_sets),
        ("total_reps", &act.total_reps),
        ("total_volume", &act.total_volume),
    ];
    let present: Vec<String> =
        fields.iter().filter_map(|(key, value)| value.as_ref().map(|v| format!("\"{key}\": {}", py_number(v)))).collect();
    if present.is_empty() { "{}".into() } else { format!("{{{}}}", present.join(", ")) }
}

/// Python's `repr()` of a JSON number: integers as is, floats with the
/// shortest round-trip digits, positional for exponents -4..16 (always with a
/// fractional part), otherwise `1e-05` / `1e+16` style.
fn py_number(n: &serde_json::Number) -> String {
    let Some(value) = n.as_f64().filter(|_| n.is_f64()) else {
        return n.to_string();
    };
    if !value.is_finite() {
        return if value.is_nan() { "NaN".into() } else if value > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let (sign, mantissa) = mantissa.strip_prefix('-').map_or(("", mantissa), |m| ("-", m));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    if (-4..16).contains(&exponent) {
        let point = exponent + 1;
        let body = if point <= 0 {
            format!("0.{}{digits}", "0".repeat((-point) as usize))
        } else if point as usize >= digits.len() {
            format!("{digits}{}.0", "0".repeat(point as usize - digits.len()))
        } else {
            format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
        };
        return format!("{sign}{body}");
    }
    let mantissa = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits };
    let exp_sign = if exponent < 0 { "-" } else { "+" };
    format!("{sign}{mantissa}e{exp_sign}{:02}", exponent.abs())
}

fn parse_start(row: &ActivityRow) -> SyncResult<DateTime<Utc>> {
    crate::timefmt::parse_garmin(&row.start_time)
        .ok_or_else(|| SyncError::Invalid(format!("time data {:?} does not match format '{GARMIN_TIME_FORMAT}'", row.start_time)))
}

fn rows_within(db: &Db, statuses: &[&str], cutoff: DateTime<Utc>) -> SyncResult<Vec<(ActivityRow, DateTime<Utc>)>> {
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

/// A Hevy-eligible activity waits until a Hevy poll has run late enough
/// after it ended to have seen the workout, so it is not published bare and
/// then replaced.
fn under_hevy_settlement_hold(conn: &rusqlite::Connection, row: &ActivityRow) -> SyncResult<bool> {
    let cfg = config::load_config(conn, false)?;
    if !db::truthy(cfg.get("hevy_enabled")) {
        return Ok(false);
    }
    if row.activity_type != "strength_training" && row.activity_type != "other" {
        return Ok(false);
    }
    if db::has_linked_workout(conn, row.garmin_activity_id)? {
        return Ok(false);
    }
    let garmin_data = if row.garmin_data.is_empty() { "{}" } else { &row.garmin_data };
    let Ok(garmin_data) = serde_json::from_str::<Value>(garmin_data) else {
        return Ok(true);
    };
    let duration = garmin_data.get("duration").filter(|d| d.is_number()).and_then(Value::as_f64);
    let Some(duration) = duration.filter(|d| *d >= 0.0) else {
        return Ok(true);
    };
    let default_interval = config::defaults()["hevy_poll_interval_minutes"].as_f64().unwrap_or(5.0);
    let interval = match cfg.get("hevy_poll_interval_minutes") {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(default_interval),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(default_interval),
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        _ => default_interval,
    };
    let end = parse_start(row)? + Duration::milliseconds((duration * 1000.0) as i64);
    let release_after = end + Duration::milliseconds(((interval.max(0.0) + 5.0) * 60_000.0) as i64);
    let last_success = db::get_config_value(conn, "hevy_last_success_at")?
        .and_then(|v| v.as_str().and_then(crate::timefmt::parse_timestamp));
    Ok(last_success.is_none_or(|at| at < release_after))
}

/// Two callers uploading the same activity at once would both miss the
/// other in Strava's search (the first is still processing) and post a
/// duplicate, so one activity publishes at a time.
static PUBLISH_LOCKS: LazyLock<Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Mutex::default);

fn publish_lock(garmin_activity_id: i64) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = PUBLISH_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(garmin_activity_id).or_default().clone()
}

pub struct Services<'a> {
    pub db: &'a Db,
    pub garmin: &'a GarminClient,
    pub strava: &'a StravaClient,
}

async fn publish_row(services: &Services<'_>, garmin_activity_id: i64, now: DateTime<Utc>, manual: bool) -> SyncResult<()> {
    let lock = publish_lock(garmin_activity_id);
    let _guard = lock.lock().await;
    let row = {
        let conn = services.db.lock();
        if let Some((hevy_id, status)) = db::blocking_workout_for_activity(&conn, garmin_activity_id)? {
            return Err(SyncError::Blocked(format!("hevy workout {hevy_id} is {status}")));
        }
        let row = db::get_activity(&conn, garmin_activity_id)?
            .ok_or_else(|| SyncError::Invalid(format!("Activity {garmin_activity_id} was not found")))?;
        if row.is_published() {
            return Ok(());
        }
        if !manual && under_hevy_settlement_hold(&conn, &row)? {
            return Err(SyncError::Blocked("settlement hold".into()));
        }
        row
    };
    let start = parse_start(&row)?;
    let existing = services.strava.find_existing_activity(start, strava_client::DEFAULT_MATCH_TOLERANCE_MINUTES).await?;
    if let Some(strava_id) = existing {
        db::set_published(&services.db.lock(), garmin_activity_id, strava_id, now)?;
        return Ok(());
    }

    let fit = services.garmin.download_fit(garmin_activity_id).await?;
    let description = Some(row.description.as_str()).filter(|d| !d.is_empty());
    let document = strength_document(services.db, garmin_activity_id, &fit)?;
    let mut strava_id = None;
    if let Some(document) = document {
        match services.strava.publish_strength(garmin_activity_id, &document, Some(&row.title), description).await {
            Ok(id) => strava_id = Some(id),
            Err(StravaError::Rejected(reason)) => {
                crate::log_warn!("sync", "strava upload {garmin_activity_id}: Strava rejected Hevy's sets, uploading the FIT instead: {reason}");
            }
            Err(err) => return Err(err.into()),
        }
    }
    let strava_id = match strava_id {
        Some(id) => id,
        None => services.strava.publish(garmin_activity_id, &fit, Some(&row.title), description).await?,
    };
    db::set_published(&services.db.lock(), garmin_activity_id, strava_id, now)?;
    Ok(())
}

/// Strava's JSON strength upload for a Hevy-linked activity, or `None` to
/// upload the FIT. A "described" workout kept the watch's sets on Garmin, so
/// Strava should match those.
fn strength_document(db: &Db, garmin_activity_id: i64, fit: &[u8]) -> SyncResult<Option<Value>> {
    let conn = db.lock();
    let Some((status, payload)) = db::applied_workout_for_activity(&conn, garmin_activity_id)? else {
        return Ok(None);
    };
    if status == "described" {
        return Ok(None);
    }
    let payload = if payload.is_empty() { "{}" } else { &payload };
    let tz = config::load_config(&conn, false)?.get("display_timezone").and_then(Value::as_str).map(str::to_string);
    let built = serde_json::from_str::<Value>(payload)
        .map_err(|e| e.to_string())
        .and_then(|workout| crate::strava_strength::build_strength_upload(&conn, &workout, fit, tz.as_deref()));
    match built {
        Ok(document) => Ok(Some(document)),
        Err(reason) => {
            crate::log_warn!("sync", "strava upload {garmin_activity_id}: uploading the FIT instead of Hevy's sets: {reason}");
            Ok(None)
        }
    }
}

/// Only a rejection clears `garmin_credentials_verified`, which pauses the
/// Garmin leg until a reconnect; an outage is retried on the next poll.
pub fn record_sync_outcome(db: &Db, now: DateTime<Utc>, error: Option<&GarminError>) -> SyncResult<()> {
    let conn = db.lock();
    let at = json!(db::iso(now));
    db::set_config_value(&conn, "garmin_last_sync_at", &at)?;
    if error.is_none() {
        db::set_config_value(&conn, "garmin_last_sync_ok_at", &at)?;
    }
    db::set_config_value(&conn, "garmin_last_sync_ok", &json!(error.is_none()))?;
    db::set_config_value(&conn, "garmin_last_sync_error", &json!(error.map(ToString::to_string)))?;
    if !error.is_some_and(GarminError::is_transient) {
        db::set_config_value(&conn, "garmin_credentials_verified", &json!(error.is_none()))?;
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
        let new_hash = content_hash(&act.title, &act.description, &act.activity_type, &garmin_data);
        let Some(existing) = db::get_activity(&conn, act.garmin_activity_id)? else {
            let (status, hold_reason) = if hold_cutoff.as_deref().is_some_and(|cutoff| act.start_time.as_str() < cutoff) {
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
    let window_start = (now - Duration::days(lookback) + Duration::minutes(5)).format(GARMIN_TIME_FORMAT).to_string();
    let mut gone: Vec<i64> = db::list_active_ids_since(&conn, &window_start)?.difference(&fetched_ids).copied().collect();
    gone.sort_unstable();
    for garmin_activity_id in gone {
        db::mark_removed(&conn, garmin_activity_id)?;
        stats.removed += 1;
    }
    drop(conn);
    record_sync_outcome(db, now, None)?;
    crate::log_info!("sync", "garmin sync: {} new, {} updated, {} removed", stats.new, stats.updated, stats.removed);
    Ok(stats)
}

/// The poller (`selected` is `None`) publishes only pending rows: missing
/// and held rows are the user's call. A manual bulk publish names its rows,
/// and every status the dashboard lets the user check is publishable.
pub async fn publish_pending(services: &Services<'_>, now: DateTime<Utc>, selected: Option<&HashSet<i64>>) -> SyncResult<PublishStats> {
    let mut stats = PublishStats::default();
    let manual = selected.is_some();
    let statuses: &[&str] = if manual { &["pending", "missing", "held"] } else { &["pending"] };
    for status in statuses {
        let rows = db::list_activities_with_status(&services.db.lock(), status)?;
        for row in rows {
            let id = row.garmin_activity_id;
            if selected.is_some_and(|ids| !ids.contains(&id)) {
                continue;
            }
            // Read per row: the owner can press Publish while this pass
            // uploads earlier rows, and a row in its undo window stays unsent.
            if !manual && db::scheduled_publish_ids(&services.db.lock())?.contains(&id) {
                continue;
            }
            match publish_row(services, id, now, manual).await {
                Ok(()) => stats.published += 1,
                Err(SyncError::Blocked(reason)) => {
                    if manual {
                        crate::log_warn!("sync", "publish blocked for activity {id}: {reason}");
                    }
                    stats.blocked += 1;
                    stats.blocked_reasons.push(reason);
                }
                Err(err) => {
                    crate::log_warn!("sync", "publish failed for activity {id}: {err}");
                    stats.failed += 1;
                }
            }
        }
    }
    if stats.published > 0 || stats.failed > 0 || (manual && stats.blocked > 0) {
        crate::log_info!("sync", "strava publish: {} published, {} failed, {} blocked", stats.published, stats.failed, stats.blocked);
    }
    Ok(stats)
}

pub async fn publish_now(services: &Services<'_>, garmin_activity_id: i64, now: DateTime<Utc>) -> SyncResult<()> {
    publish_row(services, garmin_activity_id, now, true).await
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
    let on_strava = strava.list_activities_between(cutoff - padding, now + padding).await?;

    let conn = db.lock();
    let mut unclaimed: Vec<StravaActivity> = on_strava.clone();
    for (row, start) in &to_link {
        let Some(existing) = strava_client::match_closest_activity(&unclaimed, *start, strava_client::DEFAULT_MATCH_TOLERANCE_MINUTES)
        else {
            continue;
        };
        // Each Strava entry is the twin of one Garmin activity only, or two
        // activities minutes apart would both claim it.
        unclaimed.retain(|a| a.id != existing);
        db::set_published(&conn, row.garmin_activity_id, existing, now)?;
        stats.linked_existing += 1;
    }
    let live: HashSet<i64> = on_strava.iter().map(|a| a.id).collect();
    for (row, _) in &to_verify {
        if !row.strava_activity_id.is_some_and(|id| live.contains(&id)) {
            db::set_publish_status(&conn, row.garmin_activity_id, "missing", None)?;
            stats.flagged_missing += 1;
        }
    }
    if stats.flagged_missing > 0 {
        crate::log_info!("sync", "strava status check: {} activities flagged missing", stats.flagged_missing);
    }
    if stats.linked_existing > 0 {
        crate::log_info!("sync", "strava status check: {} activities linked to existing Strava entries", stats.linked_existing);
    }
    Ok(stats)
}

/// Un-holds category holds whose type became an autosync category. Backlog
/// holds stay: an autosync rule says nothing about a three-week-old run.
pub fn reconcile_held_activities(conn: &rusqlite::Connection, held_activity_types: &[String]) -> SyncResult<i64> {
    let mut flipped = 0;
    for row in db::list_activities_with_status(conn, "held")? {
        if row.hold_reason.as_deref() == Some(HOLD_BACKLOG) {
            continue;
        }
        if !held_activity_types.contains(&row.activity_type) {
            db::set_publish_status(conn, row.garmin_activity_id, "pending", None)?;
            flipped += 1;
        }
    }
    Ok(flipped)
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
    let row = db::get_activity(&services.db.lock(), garmin_activity_id)?
        .ok_or_else(|| SyncError::Invalid(format!("Activity {garmin_activity_id} was not found")))?;
    services.garmin.update_activity_metadata(garmin_activity_id, title, description).await?;
    if row.publish_status == "published" {
        if let Some(strava_id) = row.strava_activity_id.filter(|id| *id != 0) {
            services.strava.update_activity_metadata(strava_id, title, description).await?;
        }
    }
    let garmin_data = if row.garmin_data.is_empty() { "{}" } else { &row.garmin_data };
    let hash = content_hash(title, description, &row.activity_type, garmin_data);
    db::update_activity_metadata(&services.db.lock(), garmin_activity_id, title, description, &hash)?;
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
        None => db::get_config_value(conn, "garmin_last_sync_ok_at")?.and_then(|v| v.as_str().map(str::to_string)),
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
pub fn strava_status_lookback_days(conn: &rusqlite::Connection, cfg: &Config, now: DateTime<Utc>) -> SyncResult<i64> {
    let lookback = cfg.int("lookback_days");
    let Some(oldest) = db::oldest_unpublished_start_time(conn)? else {
        return Ok(lookback);
    };
    let oldest = crate::timefmt::parse_garmin(&oldest)
        .ok_or_else(|| SyncError::Invalid(format!("time data {oldest:?} does not match format '{GARMIN_TIME_FORMAT}'")))?;
    let backlog_days = days_between(now, oldest) + 1;
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
        (lookback, db::truthy(db::get_config_value(&conn, "garmin_credentials_verified")?.as_ref()))
    };
    let hold_before = now - Duration::days(cfg.int("lookback_days"));
    let garmin = if garmin_connected {
        sync_garmin(services.db, services.garmin, cfg, now, Some(lookback), Some(hold_before)).await?
    } else {
        crate::log_info!("sync", "catch-up: garmin is not connected, skipping the garmin fetch");
        GarminSyncStats::default()
    };
    let status_lookback = strava_status_lookback_days(&services.db.lock(), cfg, now)?;
    let status = check_strava_status(services.db, services.strava, cfg, now, Some(status_lookback)).await?;
    crate::log_info!(
        "sync",
        "catch-up sync over {lookback} days (status {status_lookback} days): {} new ({} held as backlog), {} linked",
        garmin.new, garmin.held_backlog, status.linked_existing
    );
    Ok(CatchUpStats { garmin, status, lookback_days: lookback })
}

pub fn exclude(conn: &rusqlite::Connection, garmin_activity_id: i64) -> SyncResult<()> {
    Ok(db::set_publish_status(conn, garmin_activity_id, "excluded", None)?)
}

/// Puts an excluded row back exactly where it was, for an undone exclude.
pub fn restore_status(conn: &rusqlite::Connection, garmin_activity_id: i64, status: &str, hold_reason: Option<&str>) -> SyncResult<()> {
    Ok(db::set_publish_status(conn, garmin_activity_id, status, hold_reason)?)
}

pub fn unexclude(conn: &rusqlite::Connection, garmin_activity_id: i64, cfg: &Config) -> SyncResult<()> {
    let row = db::get_activity(conn, garmin_activity_id)?
        .ok_or_else(|| SyncError::Invalid(format!("Activity {garmin_activity_id} was not found")))?;
    if cfg.held_activity_types().contains(&row.activity_type) {
        db::set_publish_status(conn, garmin_activity_id, "held", Some(HOLD_CATEGORY))?;
    } else {
        db::set_publish_status(conn, garmin_activity_id, "pending", None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::py_number;

    #[test]
    fn numbers_read_as_python_writes_them() {
        let repr = |text: &str| py_number(&serde_json::from_str(text).unwrap());
        for (input, python) in [
            ("1e-05", "1e-05"), ("1e16", "1e+16"), ("0.0001", "0.0001"), ("123456789012345.6", "123456789012345.6"),
            ("2990", "2990"), ("151.0", "151.0"), ("3.4", "3.4"), ("-0.5", "-0.5"), ("1.5e300", "1.5e+300"),
            ("0.30000000000000004", "0.30000000000000004"), ("1e15", "1000000000000000.0"), ("-2.5e-07", "-2.5e-07"),
        ] {
            assert_eq!(repr(input), python, "{input}");
        }
    }
}
