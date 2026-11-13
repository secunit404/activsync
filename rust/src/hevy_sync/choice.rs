use std::sync::Arc;

use crate::poller::BoxFuture;
use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::{StepError, apply_strategy, hevy_context, log_transition, record_failure};
use crate::db::{self, Db};
use crate::events::EventBus;
use crate::hevy_apply::Apply;
use crate::hevy_client::HevyClient;
use crate::hevy_db::{self, Workout};
use crate::hevy_match::{ApplyFailure, MatchApplier};
use crate::net::Http;
use crate::poller::{self, Clients};
use crate::sync;

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

fn validate_choice(
    conn: &Connection,
    hevy_id: &str,
    strategy: &str,
) -> Result<Workout, ChoiceError> {
    let row = hevy_db::get_workout(conn, hevy_id)?.ok_or(ChoiceError::Unknown)?;
    let Some(source) = row
        .source_garmin_activity_id
        .filter(|id| *id != 0 && row.status == "awaiting_match")
    else {
        return Err(ChoiceError::Invalid(
            "This workout is not waiting for a match decision.".into(),
        ));
    };
    if hevy_db::get_open_operation(conn, hevy_id)?.is_some() {
        return Err(ChoiceError::Invalid(
            "This workout already has an operation in progress.".into(),
        ));
    }
    if strategy == "replace" && db::get_activity(conn, source)?.is_some_and(|a| a.is_published()) {
        return Err(ChoiceError::Invalid(
            "Replace is unavailable because this activity is already on Strava. Choose Merge or Description only.".into(),
        ));
    }
    Ok(row)
}

/// What a failed choice leaves on the workout: still awaiting the choice if
/// nothing moved it, else what any failed workout is left in.
fn record_choice_failure(conn: &Connection, hevy_id: &str, error: &str) -> rusqlite::Result<()> {
    let awaiting =
        hevy_db::get_workout(conn, hevy_id)?.is_some_and(|row| row.status == "awaiting_match");
    if awaiting {
        hevy_db::set_workout_status(conn, hevy_id, "awaiting_match", Some(error))
    } else {
        record_failure(conn, hevy_id, error)
    }
}

/// A refused choice changes nothing: a scheduled choice can run after the
/// workout was applied, and recording the refusal would mark it failed.
pub async fn apply_match_choice(
    apply: &Apply<'_>,
    hevy: &HevyClient,
    hevy_id: &str,
    strategy: &str,
    now: DateTime<Utc>,
) -> Result<Option<Workout>, ChoiceError> {
    if !matches!(strategy, "merge" | "replace" | "describe") {
        return Err(ChoiceError::Invalid(format!(
            "unsupported Hevy match strategy: {strategy}"
        )));
    }
    let db = apply.db;
    let token = hevy_db::acquire_lease(&db.lock(), hevy_id, now)?.ok_or(ChoiceError::Busy)?;
    let checked = validate_choice(&db.lock(), hevy_id, strategy);
    let outcome = match checked {
        Ok(row) => apply_strategy(apply, hevy, &row, strategy)
            .await
            .map_err(ChoiceError::Failed),
        Err(err) => Err(err),
    };
    let conn = db.lock();
    if let Err(ChoiceError::Failed(err)) = &outcome
        && !matches!(err, StepError::HevyAuth)
    {
        crate::log_warn!(
            "hevy_sync",
            "applying Hevy match choice for {hevy_id} failed: {err}"
        );
        record_choice_failure(&conn, hevy_id, &err.to_string())?;
    }
    hevy_db::release_lease(&conn, hevy_id, &token)?;
    log_transition(&conn, hevy_id, "awaiting_match");
    outcome.and_then(|()| Ok(hevy_db::get_workout(&conn, hevy_id)?))
}

/// Reads Garmin back after applying so the dashboard shows the result.
pub struct RealMatchApplier {
    pub db: Db,
    pub events: EventBus,
    pub clients: Arc<dyn Clients>,
    pub hevy_http: Http,
    pub development: bool,
}

impl RealMatchApplier {
    async fn apply_now(&self, hevy_id: &str, strategy: &str) -> Result<String, ApplyFailure> {
        let now = Utc::now();
        let failed = |err: &dyn std::fmt::Display| ApplyFailure::Failed(err.to_string());
        let garmin = poller::open_garmin(&self.db, &self.events, self.clients.as_ref(), now)
            .await
            .map_err(|e| failed(&e))?;
        let (hevy, profile, cfg) =
            hevy_context(&self.db, &garmin, &self.hevy_http, self.development, now)
                .await
                .map_err(|e| failed(&e))?;
        let apply = Apply {
            db: &self.db,
            garmin: &garmin,
            cfg: &cfg,
            profile: &profile,
        };
        let row = match apply_match_choice(&apply, &hevy, hevy_id, strategy, now).await {
            Ok(row) => row.ok_or_else(|| ApplyFailure::Failed("the workout disappeared".into()))?,
            Err(ChoiceError::Failed(StepError::HevyAuth)) => return Err(ApplyFailure::HevyAuth),
            Err(err) => return Err(failed(&err)),
        };
        if matches!(row.status.as_str(), "merged" | "replaced" | "described") {
            sync::sync_garmin(&self.db, &garmin, &cfg, now, None, None)
                .await
                .map_err(|e| failed(&e))?;
        }
        self.events.publish("refresh");
        Ok(row.status)
    }
}

impl MatchApplier for RealMatchApplier {
    fn apply<'a>(
        &'a self,
        hevy_id: &'a str,
        strategy: &'a str,
    ) -> BoxFuture<'a, Result<String, ApplyFailure>> {
        Box::pin(self.apply_now(hevy_id, strategy))
    }
}
