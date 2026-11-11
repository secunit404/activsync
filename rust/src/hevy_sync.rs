//! The Hevy leg of the poller, as the Python app's `hevy_sync`: read Hevy's
//! events feed, park workouts with unmapped exercises, match each workout to
//! its watch activity, apply it with the configured strategy, re-apply
//! edits, and push edited titles and descriptions to Strava copies.
//!
//! The events cursor advances only past events whose effect landed, and each
//! effect is applied before its dedupe key is recorded, so a crash anywhere
//! is absorbed on the next poll.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use futures_util::future::BoxFuture;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::config::{self, Config};
use crate::db::{self, Db};
use crate::exercise_tables;
use crate::fit_builder::Profile;
use crate::garmin_client::{GarminClient, GarminError};
use crate::hevy_apply::{self, Apply, ApplyError, activity_metrics, build_exercise_sets_payload, resolve_exercises};
use crate::hevy_client::{HevyClient, HevyError};
use crate::hevy_db::{self, APPLIED_STATUSES, Template, Workout};
use crate::hevy_description::{generate_description, generate_title};
use crate::hevy_match;
use crate::hevy_profile;
use crate::net::Http;
use crate::poller::{HevyLeg, HevyLegError};
use crate::strava_client::StravaClient;
use crate::timefmt::parse_timestamp;

pub const CURSOR_KEY: &str = "hevy_events_cursor";
const CURSOR_LAG_MINUTES: i64 = 5;
const OVERLAP_MIN: f64 = 0.70;
const DRIFT_MAX_MINUTES: f64 = 20.0;
const GRACE_DEFAULT_MINUTES: i64 = 120;

/// A Hevy deletion for one of these is surfaced for review, never applied:
/// the workout has landed on Garmin or is on its way.
const SYNCED_STATUSES: [&str; 7] =
    ["merged", "described", "replaced", "uploaded_passive", "linked_existing", "syncing", "needs_review"];

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

fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// (type, workout id, timestamp): the dedupe key.
fn event_key(event: &Value) -> (&'static str, String, String) {
    if event["type"] == "deleted" {
        return ("deleted", text(event.get("id")), text(event.get("deleted_at")));
    }
    let workout = &event["workout"];
    ("updated", text(workout.get("id")), text(workout.get("updated_at")))
}

fn apply_updated(conn: &Connection, event: &Value) -> rusqlite::Result<()> {
    let workout = &event["workout"];
    let id = text(workout.get("id"));
    let updated_at = text(workout.get("updated_at"));
    let before = hevy_db::get_workout(conn, &id)?;
    let title = text(workout.get("title"));
    hevy_db::upsert_workout(
        conn,
        &id,
        &title,
        &text(workout.get("start_time")),
        &text(workout.get("end_time")),
        &updated_at,
        workout,
    )?;
    let after = hevy_db::get_workout(conn, &id)?;
    if before.is_none() && after.is_some() {
        eprintln!("hevy workout {id} ({title}) received, ended {}", text(workout.get("end_time")));
    }
    // A Hevy edit can remove the exercise that needed a mapping. The equality
    // wakes only the current revision and keeps a replay after a crash safe.
    if before.is_some_and(|b| b.status == "needs_mapping") && after.is_some_and(|a| a.source_updated_at.as_deref() == Some(&updated_at)) {
        hevy_db::wake_workout(conn, &id)?;
    }
    Ok(())
}

fn apply_deleted(conn: &Connection, event: &Value) -> rusqlite::Result<()> {
    let Some(row) = hevy_db::get_workout(conn, &text(event.get("id")))? else { return Ok(()) };
    if SYNCED_STATUSES.contains(&row.status.as_str()) {
        // Never delete on Garmin automatically; surface it instead.
        hevy_db::set_workout_status(conn, &row.hevy_id, "needs_review", Some("deleted in Hevy"))
    } else {
        hevy_db::delete_workout(conn, &row.hevy_id)
    }
}

/// Reads Hevy's events feed and applies the new ones; returns how many. A
/// missing cursor starts at `now`: history is never replayed here, backfill
/// is its own action.
pub async fn ingest_events(db: &Db, hevy: &HevyClient, now: DateTime<Utc>) -> Result<usize, HevyLegError> {
    let cursor = db::get_config_value(&db.lock(), CURSOR_KEY)?.filter(|c| db::truthy(Some(c)));
    let Some(cursor) = cursor else {
        db::set_config_value(&db.lock(), CURSOR_KEY, &json!(db::iso(now)))?;
        return Ok(0);
    };
    let cursor = text(Some(&cursor));
    let since = parse_timestamp(&cursor).map_or_else(|| cursor.clone(), |at| db::iso(at - Duration::minutes(CURSOR_LAG_MINUTES)));
    let mut events = hevy.iter_events_since(&since).await?;
    // Oldest first, a deletion last at equal timestamps: an old update must
    // never resurrect a newer deletion, whatever order the page came in.
    events.sort_by_cached_key(|event| {
        let (kind, _, ts) = event_key(event);
        (ts, kind == "deleted")
    });

    let conn = db.lock();
    let mut processed = cursor.clone();
    let mut fresh = 0;
    let mut outcome = Ok(());
    for event in &events {
        let (kind, workout_id, ts) = event_key(event);
        let seen = match hevy_db::event_seen(&conn, kind, &workout_id, &ts) {
            Ok(seen) => seen,
            Err(err) => {
                outcome = Err(err);
                break;
            }
        };
        if !seen {
            // The effect before the dedupe record: a crash between them
            // re-runs the idempotent effect rather than losing it.
            let applied = if kind == "updated" { apply_updated(&conn, event) } else { apply_deleted(&conn, event) }
                .and_then(|()| hevy_db::record_event_seen(&conn, kind, &workout_id, &ts));
            if let Err(err) = applied {
                outcome = Err(err);
                break;
            }
            fresh += 1;
        }
        // A seen event still moves the cursor, or it would lag until an
        // unrelated newer event arrived.
        if ts > processed {
            processed = ts;
        }
    }
    if processed != cursor {
        db::set_config_value(&conn, CURSOR_KEY, &json!(processed))?;
    }
    outcome?;
    Ok(fresh)
}

/// Hevy's exercise template in the table's shape. Hevy has named the
/// equipment field two ways; the fallbacks cover a template the API would
/// not return, known only by the id and title seen on the workout.
fn store_template(conn: &Connection, template: &Value, template_id: &str, title: &str) -> rusqlite::Result<()> {
    let field = |key: &str| template.get(key).and_then(Value::as_str);
    let secondary: Vec<&str> =
        template.get("secondary_muscle_groups").and_then(Value::as_array).map(|v| v.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let equipment = field("equipment_category").filter(|e| !e.is_empty()).or_else(|| field("equipment"));
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
pub async fn apply_mapping_gate(db: &Db, row: &Workout, strategy: &str, hevy: &HevyClient) -> Result<bool, StepError> {
    if strategy == "describe" {
        return Ok(true);
    }
    let payload = row.payload_json();
    let mut misses: Vec<(String, Option<String>)> = Vec::new();
    {
        let conn = db.lock();
        for exercise in payload.get("exercises").and_then(Value::as_array).into_iter().flatten() {
            let title = exercise.get("title").and_then(Value::as_str).unwrap_or_default();
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
        let Some(template_id) = template_id.as_deref().filter(|id| !id.is_empty()) else { continue };
        if hevy_db::template_exists(&db.lock(), template_id)? {
            continue;
        }
        let fetched = match hevy.get_exercise_template(template_id).await {
            Ok(template) => template,
            Err(HevyError::Auth) => return Err(StepError::HevyAuth),
            Err(err) => {
                eprintln!("template fetch failed for {template_id}: {err}");
                None
            }
        };
        // A failed fetch must not leave a parked workout with no row to map;
        // a later catalog refresh fills in this placeholder.
        let template = fetched.filter(|t| db::truthy(Some(t))).unwrap_or_else(|| json!({"id": template_id, "title": title}));
        store_template(&db.lock(), &template, template_id, title)?;
    }
    let titles: Vec<&str> = misses.iter().map(|(title, _)| title.as_str()).collect();
    let error = format!("unmapped exercises: {}", titles.join(", "));
    hevy_db::set_workout_status(&db.lock(), &row.hevy_id, "needs_mapping", Some(&error))?;
    Ok(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchMatch {
    One(i64),
    /// Several unclaimed candidates.
    Multiple,
    /// Candidates exist but other workouts own them all.
    Claimed,
    None,
}

/// The workout's watch activity among the local ones, with no Garmin call.
/// A candidate this workout already claimed counts as its own.
pub fn find_watch_match(conn: &Connection, row: &Workout) -> rusqlite::Result<WatchMatch> {
    let Some((hevy_start, hevy_end)) = hevy_apply::workout_window(row).filter(|(start, end)| end > start) else {
        return Ok(WatchMatch::None);
    };
    let hevy_duration = hevy_apply::total_seconds(hevy_end - hevy_start);

    let mut claims: HashMap<i64, String> = HashMap::new();
    let mut stmt = conn.prepare("SELECT hevy_id, source_garmin_activity_id, garmin_activity_id FROM hevy_workouts")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<i64>>(2)?)))?;
    for claim in rows {
        let (hevy_id, source, target) = claim?;
        for id in [source, target].into_iter().flatten() {
            claims.insert(id, hevy_id.clone());
        }
    }

    let mut unclaimed = Vec::new();
    let mut claimed_by_others = 0;
    for activity in db::list_activities(conn, false)? {
        if !matches!(activity.activity_type.as_str(), "strength_training" | "other") {
            continue;
        }
        let Some(act_start) = parse_timestamp(&activity.start_time) else { continue };
        let act_end = act_start + hevy_apply::seconds(hevy_apply::activity_duration_seconds(&activity));
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
    let Ok(Some(after)) = hevy_db::get_workout(conn, hevy_id) else { return };
    if after.status != before {
        let reason = after.error.as_deref().map(|e| format!(" ({e})")).unwrap_or_default();
        eprintln!("hevy workout {hevy_id} ({}): {before} -> {}{reason}", after.title, after.status);
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
    let status = if hevy_db::get_open_operation(conn, hevy_id)?.is_some() { "syncing" } else { "failed" };
    hevy_db::set_workout_status(conn, hevy_id, status, Some(error))
}

async fn execute(apply: &Apply<'_>, row: &Workout, strategy: &str) -> Result<(), ApplyError> {
    match strategy {
        "describe" => apply.execute_describe(row).await,
        "replace" => apply.execute_replace(row).await,
        _ => apply.execute_merge(row).await,
    }
}

async fn decide(apply: &Apply<'_>, hevy: &HevyClient, row: &Workout, now: DateTime<Utc>) -> Result<(), StepError> {
    let (db, cfg) = (apply.db, apply.cfg);
    let hevy_id = row.hevy_id.as_str();
    if row.status == "awaiting_match" && (review_mode(cfg) || source_published(&db.lock(), row)?) {
        return Ok(());
    }
    if hevy_db::get_open_operation(&db.lock(), hevy_id)?.is_some() {
        return Ok(apply.advance_operation(row).await?);
    }
    let strategy = cfg.get("hevy_watch_strategy").and_then(Value::as_str).unwrap_or("merge");
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
                let row = hevy_db::get_workout(&conn, hevy_id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
                if db::get_activity(&conn, activity_id)?.is_some_and(|a| a.is_published()) {
                    let error = "Already published to Strava. Choose Merge or Description only.";
                    hevy_db::set_workout_status(&conn, hevy_id, "awaiting_match", Some(error))?;
                    return Ok(());
                }
                if review_mode(cfg) {
                    hevy_db::set_workout_status(&conn, hevy_id, "awaiting_match", Some("Choose how to apply this Hevy workout."))?;
                    return Ok(());
                }
                row
            };
            if apply_mapping_gate(db, &row, strategy, hevy).await? {
                execute(apply, &row, strategy).await?;
            }
        }
        WatchMatch::Multiple => {
            hevy_db::set_workout_status(&db.lock(), hevy_id, "needs_review", Some("multiple matching watch activities"))?;
        }
        WatchMatch::Claimed => {
            let error = "matching watch activity already claimed by another workout";
            hevy_db::set_workout_status(&db.lock(), hevy_id, "needs_review", Some(error))?;
        }
        WatchMatch::None => {
            let grace = cfg.get("hevy_grace_minutes").map_or(GRACE_DEFAULT_MINUTES, |_| cfg.int("hevy_grace_minutes"));
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
pub async fn process_workout(apply: &Apply<'_>, hevy: &HevyClient, row: &Workout, now: DateTime<Utc>) -> Result<(), HevyLegError> {
    let db = apply.db;
    if row.status == "awaiting_match" && hevy_match::is_active(&db.lock(), &row.hevy_id)? {
        return Ok(());
    }
    let Some(token) = hevy_db::acquire_lease(&db.lock(), &row.hevy_id, now)? else { return Ok(()) };
    let outcome = decide(apply, hevy, row, now).await;
    let conn = db.lock();
    let result = match outcome {
        Ok(()) => Ok(()),
        Err(StepError::HevyAuth) => Err(HevyLegError::Auth),
        Err(StepError::Apply(err)) => {
            eprintln!("processing hevy workout {} failed: {err}", row.hevy_id);
            record_failure(&conn, &row.hevy_id, &err.to_string()).map_err(HevyLegError::from)
        }
    };
    hevy_db::release_lease(&conn, &row.hevy_id, &token)?;
    log_transition(&conn, &row.hevy_id, &row.status);
    result
}

#[derive(Debug)]
pub enum ChoiceError {
    Busy,
    Unknown,
    Invalid(String),
    Failed(StepError),
}

impl std::fmt::Display for ChoiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => f.write_str("Workout is being processed right now."),
            Self::Unknown => f.write_str("Unknown Hevy workout."),
            Self::Invalid(msg) => f.write_str(msg),
            Self::Failed(err) => err.fmt(f),
        }
    }
}

impl From<StepError> for ChoiceError {
    fn from(err: StepError) -> Self {
        Self::Failed(err)
    }
}

impl From<rusqlite::Error> for ChoiceError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Failed(err.into())
    }
}

fn validate_choice(conn: &Connection, hevy_id: &str, strategy: &str) -> Result<Workout, ChoiceError> {
    let row = hevy_db::get_workout(conn, hevy_id)?.ok_or(ChoiceError::Unknown)?;
    let Some(source) = row.source_garmin_activity_id.filter(|id| *id != 0 && row.status == "awaiting_match") else {
        return Err(ChoiceError::Invalid("This workout is not waiting for a match decision.".into()));
    };
    if hevy_db::get_open_operation(conn, hevy_id)?.is_some() {
        return Err(ChoiceError::Invalid("This workout already has an operation in progress.".into()));
    }
    if strategy == "replace" && db::get_activity(conn, source)?.is_some_and(|a| a.is_published()) {
        return Err(ChoiceError::Invalid(
            "Replace is unavailable because this activity is already on Strava. Choose Merge or Description only.".into(),
        ));
    }
    Ok(row)
}

async fn choose(apply: &Apply<'_>, hevy: &HevyClient, row: &Workout, strategy: &str) -> Result<(), StepError> {
    if apply_mapping_gate(apply.db, row, strategy, hevy).await? {
        execute(apply, row, strategy).await?;
    }
    Ok(())
}

/// What a failed choice leaves on the workout: still awaiting the choice if
/// nothing moved it, `syncing` with an open operation, else `failed`.
fn record_choice_failure(conn: &Connection, hevy_id: &str, error: &str) -> rusqlite::Result<()> {
    let awaiting = hevy_db::get_workout(conn, hevy_id)?.is_some_and(|row| row.status == "awaiting_match");
    let status = match hevy_db::get_open_operation(conn, hevy_id)? {
        _ if awaiting => "awaiting_match",
        Some(_) => "syncing",
        None => "failed",
    };
    hevy_db::set_workout_status(conn, hevy_id, status, Some(error))
}

/// Applies the owner's explicit choice to a claimed watch match. A refused
/// choice changes nothing: a scheduled choice can run after the workout was
/// applied, and recording the refusal would mark it failed.
pub async fn apply_match_choice(
    apply: &Apply<'_>,
    hevy: &HevyClient,
    hevy_id: &str,
    strategy: &str,
    now: DateTime<Utc>,
) -> Result<Option<Workout>, ChoiceError> {
    if !matches!(strategy, "merge" | "replace" | "describe") {
        return Err(ChoiceError::Invalid(format!("unsupported Hevy match strategy: {strategy}")));
    }
    let db = apply.db;
    let token = hevy_db::acquire_lease(&db.lock(), hevy_id, now)?.ok_or(ChoiceError::Busy)?;
    let checked = validate_choice(&db.lock(), hevy_id, strategy);
    let outcome = match checked {
        Ok(row) => choose(apply, hevy, &row, strategy).await.map_err(ChoiceError::Failed),
        Err(err) => Err(err),
    };
    let conn = db.lock();
    if let Err(ChoiceError::Failed(err)) = &outcome {
        if !matches!(err, StepError::HevyAuth) {
            eprintln!("applying Hevy match choice for {hevy_id} failed: {err}");
            record_choice_failure(&conn, hevy_id, &err.to_string())?;
        }
    }
    hevy_db::release_lease(&conn, hevy_id, &token)?;
    log_transition(&conn, hevy_id, "awaiting_match");
    outcome.and_then(|()| Ok(hevy_db::get_workout(&conn, hevy_id)?))
}

async fn redo(apply: &Apply<'_>, hevy: &HevyClient, row: &Workout) -> Result<(), StepError> {
    let strategy = row.applied_strategy.as_deref().unwrap_or("merge");
    if !apply_mapping_gate(apply.db, row, strategy, hevy).await? {
        return Ok(());
    }
    let target = row.garmin_activity_id.ok_or_else(|| ApplyError::Invalid(format!("workout {} has no linked activity", row.hevy_id)))?;
    if row.applied_strategy.as_deref() != Some("describe") {
        let (resolved, window) = {
            let conn = apply.db.lock();
            let resolved = match resolve_exercises(&conn, &row.payload_json()) {
                Ok(resolved) => resolved,
                Err(ApplyError::MappingMiss { title, .. }) => {
                    let error = format!("unmapped exercises: {title}");
                    hevy_db::set_workout_status(&conn, &row.hevy_id, "needs_mapping", Some(&error))?;
                    return Ok(());
                }
                Err(err) => return Err(err.into()),
            };
            (resolved, hevy_apply::activity_window(&conn, target, row)?)
        };
        let mut payload = build_exercise_sets_payload(&resolved, &window.0, window.1).map_err(ApplyError::Invalid)?;
        payload["activityId"] = json!(target);
        apply.garmin.put_exercise_sets(target, &payload).await?;
    }
    let write_summary = matches!(strategy, "describe" | "passive") || apply.summary_on_structured();
    apply.apply_metadata(target, row, write_summary, (None, None)).await?;
    hevy_db::set_applied_garmin(&apply.db.lock(), &row.hevy_id, row.source_updated_at.as_deref())?;
    Ok(())
}

/// A newer Hevy revision of an applied workout, re-applied with the
/// strategy it was first applied with, never the current setting.
pub async fn reapply(apply: &Apply<'_>, hevy: &HevyClient, row: &Workout, now: DateTime<Utc>) -> rusqlite::Result<()> {
    let db = apply.db;
    let Some(token) = hevy_db::acquire_lease(&db.lock(), &row.hevy_id, now)? else { return Ok(()) };
    let outcome = redo(apply, hevy, row).await;
    let conn = db.lock();
    match outcome {
        Ok(()) => {}
        Err(StepError::Apply(ApplyError::Garmin(GarminError::ActivityGone(_)))) => {
            let error = "linked activity deleted on Garmin — re-sync as fresh upload";
            hevy_db::set_workout_status(&conn, &row.hevy_id, "needs_review", Some(error))?;
        }
        Err(err) => {
            eprintln!("re-applying hevy workout {} failed: {err}", row.hevy_id);
            hevy_db::set_workout_status(&conn, &row.hevy_id, &row.status, Some(&err.to_string()))?;
        }
    }
    hevy_db::release_lease(&conn, &row.hevy_id, &token)?;
    log_transition(&conn, &row.hevy_id, &row.status);
    Ok(())
}

fn fingerprint(row: Option<&Workout>) -> Option<(String, Option<i64>, Option<String>, Option<String>)> {
    row.map(|r| (r.status.clone(), r.garmin_activity_id, r.garmin_applied_updated_at.clone(), r.error.clone()))
}

/// Pushes edited titles and descriptions to published Strava copies. The
/// Garmin side can succeed while Strava is down, so this retries each tick
/// until Strava has the applied Garmin revision.
pub async fn strava_catchup(db: &Db, strava: &StravaClient, cfg: &Config) -> Result<bool, HevyLegError> {
    let mut changed = false;
    for status in APPLIED_STATUSES {
        let rows = hevy_db::list_workouts(&db.lock(), status)?;
        for row in rows {
            let garmin_applied = row.garmin_applied_updated_at.clone().unwrap_or_default();
            if garmin_applied.is_empty() || garmin_applied <= row.strava_applied_updated_at.clone().unwrap_or_default() {
                continue;
            }
            let (activity, metrics) = {
                let conn = db.lock();
                let activity = match row.garmin_activity_id.filter(|id| *id != 0) {
                    Some(id) => db::get_activity(&conn, id)?,
                    None => None,
                };
                // Not published yet: the publish carries the current text.
                let Some(activity) = activity.filter(|a| a.strava_activity_id.is_some_and(|id| id != 0)) else { continue };
                let metrics = activity_metrics(&conn, &row, activity.garmin_activity_id, (None, None))
                    .map_err(|err| HevyLegError::Failed(err.to_string()))?;
                (activity, metrics)
            };
            let payload = row.payload_json();
            let strategy = row.applied_strategy.as_deref().unwrap_or("merge");
            let write_summary =
                matches!(strategy, "describe" | "passive") || cfg.get("hevy_summary_on_structured").is_none_or(|v| db::truthy(Some(v)));
            let template = |key: &str| cfg.get(key).and_then(Value::as_str);
            let texts = generate_title(&payload, template("hevy_title_template")).and_then(|title| {
                let description = if write_summary {
                    generate_description(&payload, metrics.0, metrics.1, template("hevy_description_template"))?
                } else {
                    activity.description.clone()
                };
                Ok((title, description))
            });
            let strava_id = activity.strava_activity_id.unwrap_or_default();
            let pushed = match texts {
                Ok((title, description)) => strava.update_activity_metadata(strava_id, &title, &description).await.map_err(|e| e.to_string()),
                Err(err) => Err(err),
            };
            if let Err(err) = pushed {
                eprintln!("strava catch-up failed for {}: {err}", row.hevy_id);
                continue;
            }
            hevy_db::set_applied_strava(&db.lock(), &row.hevy_id, &garmin_applied)?;
            changed = true;
        }
    }
    Ok(changed)
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
    let mut changed = ingest_events(db, hevy, now).await? > 0;
    for status in ACTIONABLE {
        let rows = hevy_db::list_workouts(&db.lock(), status)?;
        for row in rows {
            process_workout(apply, hevy, &row, now).await?;
            let after = hevy_db::get_workout(&db.lock(), &row.hevy_id)?;
            if fingerprint(after.as_ref()) != fingerprint(Some(&row)) {
                changed = true;
            }
        }
    }
    for status in APPLIED_STATUSES {
        let rows = hevy_db::list_workouts(&db.lock(), status)?;
        for row in rows {
            let applied = row.garmin_applied_updated_at.clone().unwrap_or_default();
            if row.source_updated_at.as_ref().is_some_and(|source| !source.is_empty() && *source > applied) {
                reapply(apply, hevy, &row, now).await?;
                changed = true;
            }
        }
    }
    if let Some(strava) = strava {
        changed = strava_catchup(db, strava, apply.cfg).await? || changed;
    }
    Ok(changed)
}

/// The poller's Hevy leg against the real Hevy API. The profile and config
/// are read fresh each tick, so settings changes apply on the next poll.
pub struct RealHevyLeg {
    pub http: Http,
    pub development: bool,
}

impl RealHevyLeg {
    async fn tick(&self, db: &Db, garmin: &GarminClient, strava: Option<&StravaClient>, now: DateTime<Utc>) -> Result<bool, HevyLegError> {
        let key = db::get_config_value(&db.lock(), "hevy_api_key")?;
        let key = key.as_ref().and_then(Value::as_str).unwrap_or_default();
        let hevy = HevyClient::new(self.http.clone(), key).map_err(|e| HevyLegError::Failed(e.into()))?;
        let profile: Profile = hevy_profile::get_profile(db, garmin, now).await;
        let cfg = config::load_config(&db.lock(), self.development)?;
        let apply = Apply { db, garmin, cfg: &cfg, profile: &profile };
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
