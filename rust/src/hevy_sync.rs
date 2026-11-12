//! The Hevy leg of the poller: read Hevy's events feed, park workouts with
//! unmapped exercises, match each workout to its watch activity, apply it
//! with the configured strategy, re-apply edits, and push edited titles and
//! descriptions to Strava copies.
//!
//! The events cursor advances only past events whose effect landed, and each
//! effect is applied before its dedupe key is recorded, so a crash anywhere
//! is absorbed on the next poll.

mod choice;
mod feed;
mod reapply;

use std::collections::HashMap;

use crate::poller::BoxFuture;
use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde_json::{Value, json};

pub use choice::{ChoiceError, RealMatchApplier, apply_match_choice};
pub use feed::CURSOR_KEY;

use crate::config::{self, Config};
use crate::db::{self, Db};
use crate::events::EventBus;
use crate::exercise_tables;
use crate::fit_builder::Profile;
use crate::garmin_client::{GarminClient, GarminError};
use crate::hevy_apply::{self, Apply, ApplyError};
use crate::hevy_client::{HevyClient, HevyError};
use crate::hevy_db::{self, APPLIED_STATUSES, Template, Workout};
use crate::hevy_match;
use crate::hevy_profile;
use crate::net::Http;
use crate::poller::{self, Clients, HevyLeg, HevyLegError};
use crate::strava_client::StravaClient;
use crate::sync;
use crate::timefmt::parse_timestamp;

const OVERLAP_MIN: f64 = 0.70;
const DRIFT_MAX_MINUTES: f64 = 20.0;
const GRACE_DEFAULT_MINUTES: i64 = 120;

/// `failed` and `needs_mapping` are left out on purpose: a definite rejection
/// must not re-upload every tick. They come back through the user's retry
/// or a saved mapping.
const ACTIONABLE: [&str; 3] = ["waiting_watch", "awaiting_match", "syncing"];

impl From<HevyError> for HevyLegError {
    fn from(err: HevyError) -> Self {
        match err {
            HevyError::Auth => Self::Auth,
            other => Self::Failed(other.to_string()),
        }
    }
}

impl From<rusqlite::Error> for HevyLegError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Failed(err.to_string())
    }
}

/// A failure while handling one workout. Only a Hevy auth failure escapes
/// the workout, so the poller can pause the leg.
#[derive(Debug)]
pub enum StepError {
    HevyAuth,
    Apply(ApplyError),
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HevyAuth => HevyError::Auth.fmt(f),
            Self::Apply(err) => err.fmt(f),
        }
    }
}

impl From<ApplyError> for StepError {
    fn from(err: ApplyError) -> Self {
        Self::Apply(err)
    }
}

impl From<rusqlite::Error> for StepError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Apply(ApplyError::Db(err))
    }
}

impl From<GarminError> for StepError {
    fn from(err: GarminError) -> Self {
        Self::Apply(ApplyError::Garmin(err))
    }
}

/// Hevy's exercise template in the table's shape. Hevy has named the
/// equipment field two ways; the fallbacks cover a template the API would
/// not return, known only by the id and title seen on the workout.
pub(crate) fn store_template(
    conn: &Connection,
    template: &Value,
    template_id: &str,
    title: &str,
) -> rusqlite::Result<()> {
    let field = |key: &str| template.get(key).and_then(Value::as_str);
    let secondary: Vec<&str> = template
        .get("secondary_muscle_groups")
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let equipment = field("equipment_category")
        .filter(|e| !e.is_empty())
        .or_else(|| field("equipment"));
    hevy_db::upsert_template(
        conn,
        &Template {
            id: field("id").unwrap_or(template_id),
            title: field("title").unwrap_or(title),
            primary_muscle_group: field("primary_muscle_group"),
            secondary_muscle_groups: &secondary,
            equipment_category: equipment,
            is_custom: db::truthy(template.get("is_custom")),
        },
    )
}

/// Whether the workout may go ahead under `strategy`. Strategies that push
/// structured sets need every exercise mapped; a miss parks the workout
/// listing every missing title and caches unknown templates for the
/// mappings page. Describe uses Hevy's own titles and always passes.
async fn apply_mapping_gate(
    db: &Db,
    row: &Workout,
    strategy: &str,
    hevy: &HevyClient,
) -> Result<bool, StepError> {
    if strategy == "describe" {
        return Ok(true);
    }
    let payload = row.payload_json();
    let mut misses: Vec<(String, Option<String>)> = Vec::new();
    {
        let conn = db.lock();
        for exercise in payload
            .get("exercises")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let title = exercise
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let template_id = exercise.get("exercise_template_id").and_then(Value::as_str);
            if exercise_tables::lookup_exercise(&conn, title, template_id)?.is_none() {
                misses.push((title.into(), template_id.map(Into::into)));
            }
        }
    }
    if misses.is_empty() {
        return Ok(true);
    }
    for (title, template_id) in &misses {
        let Some(template_id) = template_id.as_deref().filter(|id| !id.is_empty()) else {
            continue;
        };
        if hevy_db::template_exists(&db.lock(), template_id)? {
            continue;
        }
        let fetched = match hevy.get_exercise_template(template_id).await {
            Ok(template) => template,
            Err(HevyError::Auth) => return Err(StepError::HevyAuth),
            Err(err) => {
                crate::log_warn!(
                    "hevy_sync",
                    "template fetch failed for {template_id}: {err}"
                );
                None
            }
        };
        // A failed fetch must not leave a parked workout with no row to map;
        // a later catalog refresh fills in this placeholder.
        let template = fetched
            .filter(|t| db::truthy(Some(t)))
            .unwrap_or_else(|| json!({"id": template_id, "title": title}));
        store_template(&db.lock(), &template, template_id, title)?;
    }
    let titles: Vec<&str> = misses.iter().map(|(title, _)| title.as_str()).collect();
    let error = format!("unmapped exercises: {}", titles.join(", "));
    hevy_db::set_workout_status(&db.lock(), &row.hevy_id, "needs_mapping", Some(&error))?;
    Ok(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WatchMatch {
    One(i64),
    /// Several unclaimed candidates.
    Multiple,
    /// Candidates exist but other workouts own them all.
    Claimed,
    None,
}

/// The workout's watch activity among the local ones, with no Garmin call.
/// A candidate this workout already claimed counts as its own.
pub(crate) fn find_watch_match(conn: &Connection, row: &Workout) -> rusqlite::Result<WatchMatch> {
    let Some((hevy_start, hevy_end)) =
        hevy_apply::workout_window(row).filter(|(start, end)| end > start)
    else {
        return Ok(WatchMatch::None);
    };
    let hevy_duration = hevy_apply::total_seconds(hevy_end - hevy_start);

    let mut claims: HashMap<i64, String> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT hevy_id, source_garmin_activity_id, garmin_activity_id FROM hevy_workouts",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, Option<i64>>(2)?,
        ))
    })?;
    for claim in rows {
        let (hevy_id, source, target) = claim?;
        for id in [source, target].into_iter().flatten() {
            claims.insert(id, hevy_id.clone());
        }
    }

    let mut unclaimed = Vec::new();
    let mut claimed_by_others = 0;
    for activity in db::list_activities(conn, false)? {
        if !matches!(
            activity.activity_type.as_str(),
            "strength_training" | "other"
        ) {
            continue;
        }
        let Some(act_start) = parse_timestamp(&activity.start_time) else {
            continue;
        };
        let act_end =
            act_start + hevy_apply::seconds(hevy_apply::activity_duration_seconds(&activity));
        let overlap = hevy_apply::total_seconds(hevy_end.min(act_end) - hevy_start.max(act_start));
        if overlap <= 0.0 || overlap / hevy_duration < OVERLAP_MIN {
            continue;
        }
        if hevy_apply::total_seconds(act_start - hevy_start).abs() / 60.0 > DRIFT_MAX_MINUTES {
            continue;
        }
        match claims.get(&activity.garmin_activity_id) {
            Some(owner) if *owner != row.hevy_id => claimed_by_others += 1,
            _ => unclaimed.push(activity.garmin_activity_id),
        }
    }
    Ok(match unclaimed.as_slice() {
        [one] => WatchMatch::One(*one),
        [] if claimed_by_others > 0 => WatchMatch::Claimed,
        [] => WatchMatch::None,
        _ => WatchMatch::Multiple,
    })
}

fn log_transition(conn: &Connection, hevy_id: &str, before: &str) {
    let Ok(Some(after)) = hevy_db::get_workout(conn, hevy_id) else {
        return;
    };
    if after.status != before {
        let reason = after
            .error
            .as_deref()
            .map(|e| format!(" ({e})"))
            .unwrap_or_default();
        crate::log_info!(
            "hevy_sync",
            "hevy workout {hevy_id} ({}): {before} -> {}{reason}",
            after.title,
            after.status
        );
    }
}

fn review_mode(cfg: &Config) -> bool {
    cfg.get("hevy_match_mode").and_then(Value::as_str) == Some("review")
}

fn source_published(conn: &Connection, row: &Workout) -> rusqlite::Result<bool> {
    Ok(match row.source_garmin_activity_id.filter(|id| *id != 0) {
        Some(id) => db::get_activity(conn, id)?.is_some_and(|a| a.is_published()),
        None => false,
    })
}

/// What a workout's failure leaves behind: still `syncing` while an
/// operation is open, so the journal resumes it, else `failed`.
fn record_failure(conn: &Connection, hevy_id: &str, error: &str) -> rusqlite::Result<()> {
    let status = if hevy_db::get_open_operation(conn, hevy_id)?.is_some() {
        "syncing"
    } else {
        "failed"
    };
    hevy_db::set_workout_status(conn, hevy_id, status, Some(error))
}

async fn apply_strategy(
    apply: &Apply<'_>,
    hevy: &HevyClient,
    row: &Workout,
    strategy: &str,
) -> Result<(), StepError> {
    if !apply_mapping_gate(apply.db, row, strategy, hevy).await? {
        return Ok(());
    }
    match strategy {
        "describe" => apply.execute_describe(row).await?,
        "replace" => apply.execute_replace(row).await?,
        _ => apply.execute_merge(row).await?,
    }
    Ok(())
}

async fn decide(
    apply: &Apply<'_>,
    hevy: &HevyClient,
    row: &Workout,
    now: DateTime<Utc>,
) -> Result<(), StepError> {
    let (db, cfg) = (apply.db, apply.cfg);
    let hevy_id = row.hevy_id.as_str();
    if row.status == "awaiting_match" && (review_mode(cfg) || source_published(&db.lock(), row)?) {
        return Ok(());
    }
    if hevy_db::get_open_operation(&db.lock(), hevy_id)?.is_some() {
        return Ok(apply.advance_operation(row).await?);
    }
    let strategy = cfg
        .get("hevy_watch_strategy")
        .and_then(Value::as_str)
        .unwrap_or("merge");
    // Backfill already made and claimed this match; matching again by
    // overlap can disagree with its exact-start match and strand it.
    let found = match row.source_garmin_activity_id {
        Some(source) => WatchMatch::One(source),
        None => find_watch_match(&db.lock(), row)?,
    };
    match found {
        WatchMatch::One(activity_id) => {
            let row = {
                let conn = db.lock();
                if !hevy_db::claim_source(&conn, hevy_id, activity_id)? {
                    let error = format!("watch activity {activity_id} claim conflict");
                    hevy_db::set_workout_status(&conn, hevy_id, "needs_review", Some(&error))?;
                    return Ok(());
                }
                let row = hevy_db::get_workout(&conn, hevy_id)?
                    .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
                if db::get_activity(&conn, activity_id)?.is_some_and(|a| a.is_published()) {
                    let error = "Already published to Strava. Choose Merge or Description only.";
                    hevy_db::set_workout_status(&conn, hevy_id, "awaiting_match", Some(error))?;
                    return Ok(());
                }
                if review_mode(cfg) {
                    hevy_db::set_workout_status(
                        &conn,
                        hevy_id,
                        "awaiting_match",
                        Some("Choose how to apply this Hevy workout."),
                    )?;
                    return Ok(());
                }
                row
            };
            apply_strategy(apply, hevy, &row, strategy).await?;
        }
        WatchMatch::Multiple => {
            hevy_db::set_workout_status(
                &db.lock(),
                hevy_id,
                "needs_review",
                Some("multiple matching watch activities"),
            )?;
        }
        WatchMatch::Claimed => {
            let error = "matching watch activity already claimed by another workout";
            hevy_db::set_workout_status(&db.lock(), hevy_id, "needs_review", Some(error))?;
        }
        WatchMatch::None => {
            let grace = cfg
                .get("hevy_grace_minutes")
                .map_or(GRACE_DEFAULT_MINUTES, |_| cfg.int("hevy_grace_minutes"));
            let ended = parse_timestamp(&row.end_time);
            if ended.is_some_and(|end| now - end < Duration::minutes(grace)) {
                hevy_db::set_workout_status(&db.lock(), hevy_id, "waiting_watch", None)?;
            } else if apply_mapping_gate(db, row, "passive", hevy).await? {
                // The passive upload pushes structured sets, so it is gated
                // even when the configured strategy is not.
                apply.execute_passive(row).await?;
            }
        }
    }
    Ok(())
}

/// One workout's turn this tick, under its lease.
async fn process_workout(
    apply: &Apply<'_>,
    hevy: &HevyClient,
    row: &Workout,
    now: DateTime<Utc>,
) -> Result<(), HevyLegError> {
    let db = apply.db;
    if row.status == "awaiting_match" && hevy_match::is_active(&db.lock(), &row.hevy_id)? {
        return Ok(());
    }
    let Some(token) = hevy_db::acquire_lease(&db.lock(), &row.hevy_id, now)? else {
        return Ok(());
    };
    let outcome = decide(apply, hevy, row, now).await;
    let conn = db.lock();
    let result = match outcome {
        Ok(()) => Ok(()),
        Err(StepError::HevyAuth) => Err(HevyLegError::Auth),
        Err(StepError::Apply(err)) => {
            crate::log_warn!(
                "hevy_sync",
                "processing hevy workout {} failed: {err}",
                row.hevy_id
            );
            record_failure(&conn, &row.hevy_id, &err.to_string()).map_err(HevyLegError::from)
        }
    };
    hevy_db::release_lease(&conn, &row.hevy_id, &token)?;
    log_transition(&conn, &row.hevy_id, &row.status);
    result
}

fn fingerprint(row: &Workout) -> (&str, Option<i64>, Option<&str>, Option<&str>) {
    (
        &row.status,
        row.garmin_activity_id,
        row.garmin_applied_updated_at.as_deref(),
        row.error.as_deref(),
    )
}

/// One tick of the Hevy leg; whether anything changed, which hands off to
/// the Garmin leg and refreshes open pages.
pub async fn run_hevy_leg(
    apply: &Apply<'_>,
    hevy: &HevyClient,
    strava: Option<&StravaClient>,
    now: DateTime<Utc>,
) -> Result<bool, HevyLegError> {
    let db = apply.db;
    let mut changed = feed::ingest_events(db, hevy, now).await? > 0;
    for status in ACTIONABLE {
        let rows = hevy_db::list_workouts(&db.lock(), status)?;
        for row in rows {
            process_workout(apply, hevy, &row, now).await?;
            let after = hevy_db::get_workout(&db.lock(), &row.hevy_id)?;
            if after.as_ref().map(fingerprint) != Some(fingerprint(&row)) {
                changed = true;
            }
        }
    }
    for status in APPLIED_STATUSES {
        let rows = hevy_db::list_workouts(&db.lock(), status)?;
        for row in rows {
            let applied = row.garmin_applied_updated_at.as_deref().unwrap_or_default();
            if row
                .source_updated_at
                .as_deref()
                .is_some_and(|source| !source.is_empty() && source > applied)
            {
                reapply::reapply(apply, hevy, &row, now).await?;
                changed = true;
            }
        }
    }
    if let Some(strava) = strava {
        changed = reapply::strava_catchup(apply, strava).await? || changed;
    }
    Ok(changed)
}

/// Hevy and the calorie profile for a tick, with the config snapshot.
async fn hevy_context(
    db: &Db,
    garmin: &GarminClient,
    http: &Http,
    development: bool,
    now: DateTime<Utc>,
) -> Result<(HevyClient, Profile, Config), String> {
    let key = db::get_config_value(&db.lock(), "hevy_api_key").map_err(|e| e.to_string())?;
    let key = key.as_ref().and_then(Value::as_str).unwrap_or_default();
    let hevy = HevyClient::new(http.clone(), key).map_err(String::from)?;
    let profile = hevy_profile::get_profile(db, garmin, now).await;
    let cfg = config::load_config(&db.lock(), development).map_err(|e| e.to_string())?;
    Ok((hevy, profile, cfg))
}

/// Starts one workout now rather than at the next Hevy poll, as backfill and
/// a saved mapping hand off; every safety gate of the poller still applies.
/// Returns the workout's new status.
pub async fn process_now(
    db: &Db,
    events: &EventBus,
    clients: &dyn Clients,
    hevy_http: &Http,
    development: bool,
    hevy_id: &str,
) -> Result<String, String> {
    let row = hevy_db::get_workout(&db.lock(), hevy_id)
        .map_err(|e| e.to_string())?
        .ok_or(format!("unknown Hevy workout: {hevy_id}"))?;
    let now = Utc::now();
    let garmin = poller::open_garmin(db, events, clients, now)
        .await
        .map_err(|e| e.to_string())?;
    let (hevy, profile, cfg) = hevy_context(db, &garmin, hevy_http, development, now).await?;
    let apply = Apply {
        db,
        garmin: &garmin,
        cfg: &cfg,
        profile: &profile,
    };
    process_workout(&apply, &hevy, &row, now)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let status = hevy_db::get_workout(&db.lock(), hevy_id)
        .map_err(|e| e.to_string())?
        .map(|w| w.status)
        .unwrap_or_default();
    if matches!(
        status.as_str(),
        "merged" | "replaced" | "described" | "uploaded_passive"
    ) {
        sync::sync_garmin(db, &garmin, &cfg, now, None, None)
            .await
            .map_err(|e| e.to_string())?;
    }
    events.publish("refresh");
    Ok(status)
}

/// The poller's Hevy leg against the real Hevy API. The profile and config
/// are read fresh each tick, so settings changes apply on the next poll.
pub struct RealHevyLeg {
    pub http: Http,
    pub development: bool,
}

impl RealHevyLeg {
    async fn tick(
        &self,
        db: &Db,
        garmin: &GarminClient,
        strava: Option<&StravaClient>,
        now: DateTime<Utc>,
    ) -> Result<bool, HevyLegError> {
        let (hevy, profile, cfg) = hevy_context(db, garmin, &self.http, self.development, now)
            .await
            .map_err(HevyLegError::Failed)?;
        let apply = Apply {
            db,
            garmin,
            cfg: &cfg,
            profile: &profile,
        };
        run_hevy_leg(&apply, &hevy, strava, now).await
    }
}

impl HevyLeg for RealHevyLeg {
    fn run<'a>(
        &'a self,
        db: &'a Db,
        garmin: &'a GarminClient,
        strava: Option<&'a StravaClient>,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<bool, HevyLegError>> {
        Box::pin(self.tick(db, garmin, strava, now))
    }
}
