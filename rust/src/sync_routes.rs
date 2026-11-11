//! The sync buttons: "Sync now" for Garmin or Strava, the first sync that
//! ends setup, and refreshing Garmin's activity types. Against real accounts
//! they call Garmin and Strava; against the mock accounts nothing new turns
//! up, so only the bookkeeping is recorded.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::garmin_client::GarminClient;
use crate::poller::Clients;
use crate::routes::{ApiError, development};
use crate::settings_routes::{ActionResult, action, conflict, invalid, require_garmin, require_not_onboarded};
use crate::strava_client::StravaError;
use crate::sync::{self, Services, SyncError};
use crate::{AppState, config, db, onboarding, view};

type ApiResult<T> = Result<Json<T>, ApiError>;

/// An unexpected failure, as the Python app's unhandled errors: logged, and
/// a bare 500 to the page.
fn server_error(context: &str, err: impl std::fmt::Display) -> ApiError {
    crate::log_error!("sync_routes", "{context}: {err}");
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
}

async fn open_garmin(clients: &dyn Clients) -> Result<GarminClient, String> {
    clients.garmin().await.map_err(|err| err.to_string())
}

pub(crate) async fn fetch_categories(state: &AppState, garmin: &GarminClient, hold_all: bool, now: DateTime<Utc>) -> Result<(), String> {
    let types = garmin.fetch_activity_types().await.map_err(|err| err.to_string())?;
    onboarding::save_garmin_categories(&state.db.lock(), development(state), types, hold_all, now).map_err(|err| err.to_string())
}

async fn first_sync(state: &AppState, clients: &dyn Clients, now: DateTime<Utc>) -> Result<(), String> {
    let garmin = open_garmin(clients).await?;
    let has_types = matches!(db::get_config_value(&state.db.lock(), "garmin_activity_types").map_err(|e| e.to_string())?,
        Some(Value::Array(types)) if !types.is_empty());
    if !has_types {
        fetch_categories(state, &garmin, true, now).await?;
    }
    let cfg = config::load_config(&state.db.lock(), development(state)).map_err(|e| e.to_string())?;
    sync::sync_garmin(&state.db, &garmin, &cfg, now, None, None).await.map_err(|e| e.to_string())?;
    sync::check_strava_status(&state.db, &clients.strava(), &cfg, now, None).await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn initial_sync(State(state): State<AppState>) -> ApiResult<ActionResult> {
    let now = Utc::now();
    {
        let conn = state.db.lock();
        require_not_onboarded(&conn)?;
        let connections = view::connection_status(&conn, now)?;
        if !connections.garmin.connected || !connections.strava.connected {
            return Err(conflict("Connect Garmin and Strava before syncing."));
        }
    }
    match state.clients.clone() {
        Some(clients) => first_sync(&state, clients.as_ref(), now)
            .await
            .map_err(|err| ApiError::new(StatusCode::BAD_GATEWAY, format!("Initial sync failed: {err}")))?,
        None => {
            let conn = state.db.lock();
            let has_types = matches!(db::get_config_value(&conn, "garmin_activity_types")?, Some(Value::Array(a)) if !a.is_empty());
            if !has_types {
                onboarding::store_garmin_categories(&conn, development(&state), true, now)?;
            }
            onboarding::record_garmin_sync(&conn, now)?;
        }
    }
    let conn = state.db.lock();
    db::set_config_value(&conn, "initial_sync_done", &json!(true))?;
    state.events.publish("refresh");
    action(&state, &conn, "Setup complete. Your activities are ready.", false)
}

pub async fn refresh_activity_types(State(state): State<AppState>) -> ApiResult<ActionResult> {
    let now = Utc::now();
    require_garmin(&state.db.lock(), "Connect to Garmin before refreshing activity categories.")?;
    match state.clients.clone() {
        Some(clients) => {
            let garmin = open_garmin(clients.as_ref()).await.map_err(|err| server_error("Garmin activity types", err))?;
            fetch_categories(&state, &garmin, false, now).await.map_err(|err| server_error("Garmin activity types", err))?;
        }
        None => onboarding::store_garmin_categories(&state.db.lock(), development(&state), false, now)?,
    }
    state.events.publish("refresh");
    action(&state, &state.db.lock(), "Garmin activity categories refreshed.", false)
}

async fn sync_garmin_now(state: &AppState, clients: &dyn Clients, strava_connected: bool, now: DateTime<Utc>) -> Result<(), ApiError> {
    let cfg = config::load_config(&state.db.lock(), development(state))?;
    let garmin = open_garmin(clients).await.map_err(|err| server_error("Garmin sync", err))?;
    sync::sync_garmin(&state.db, &garmin, &cfg, now, None, None).await.map_err(|err| server_error("Garmin sync", err))?;
    if strava_connected {
        sync::check_strava_status(&state.db, &clients.strava(), &cfg, now, None)
            .await
            .map_err(|err| server_error("Strava status check", err))?;
    }
    Ok(())
}

async fn sync_strava_now(state: &AppState, clients: &dyn Clients, now: DateTime<Utc>) -> Result<(), ApiError> {
    let cfg = config::load_config(&state.db.lock(), development(state))?;
    match sync::check_strava_status(&state.db, &clients.strava(), &cfg, now, None).await {
        Ok(_) => Ok(()),
        Err(SyncError::Strava(StravaError::Auth(message))) => Err(conflict(message)),
        Err(err) => Err(server_error("Strava status check", err)),
    }
}

pub async fn manual_sync(State(state): State<AppState>, Path(service): Path<String>) -> ApiResult<ActionResult> {
    let now = Utc::now();
    let connections = view::connection_status(&state.db.lock(), now)?;
    let message = match service.as_str() {
        "garmin" => {
            if !connections.garmin.connected {
                return Err(conflict("Garmin is disconnected — reconnect it to sync."));
            }
            match state.clients.clone() {
                Some(clients) => sync_garmin_now(&state, clients.as_ref(), connections.strava.connected, now).await?,
                None => onboarding::record_garmin_sync(&state.db.lock(), now)?,
            }
            "Garmin synced."
        }
        "strava" => {
            if !connections.strava.connected {
                return Err(conflict("Strava is disconnected — reconnect it to sync."));
            }
            if let Some(clients) = state.clients.clone() {
                sync_strava_now(&state, clients.as_ref(), now).await?;
            }
            "Strava synced."
        }
        _ => return Err(invalid("service must be garmin or strava")),
    };
    state.events.publish("refresh");
    action(&state, &state.db.lock(), message, false)
}

/// Reconnect catch-up, best effort: the outage window is synced and the
/// dashboard summarises what turned up, or the summary is cleared when
/// nothing did. `last_sync_ok_at` is `None` to read the stored value.
pub(crate) async fn run_catch_up(state: &AppState, last_sync_ok_at: Option<Option<String>>) -> Result<(), ApiError> {
    let now = Utc::now();
    let Some(clients) = state.clients.clone() else {
        onboarding::run_catch_up(&state.db.lock(), now)?;
        state.events.publish("refresh");
        return Ok(());
    };
    let caught_up = async {
        let garmin = open_garmin(clients.as_ref()).await?;
        let strava = clients.strava();
        let cfg = config::load_config(&state.db.lock(), development(state)).map_err(|e| e.to_string())?;
        let services = Services { db: &state.db, garmin: &garmin, strava: &strava };
        sync::catch_up_sync(&services, &cfg, now, last_sync_ok_at).await.map_err(|e| e.to_string())
    };
    match caught_up.await {
        Ok(stats) => {
            let found = stats.garmin.new > 0 || stats.status.linked_existing > 0;
            let report = found.then(|| {
                json!({"new": stats.garmin.new, "held": stats.garmin.held_backlog,
                       "linked": stats.status.linked_existing, "days": stats.lookback_days})
            });
            db::set_config_value(&state.db.lock(), "catch_up_report", &report.unwrap_or(Value::Null))?;
        }
        Err(err) => crate::log_warn!("sync_routes", "catch-up sync after reconnect failed: {err}"),
    }
    state.events.publish("refresh");
    Ok(())
}
