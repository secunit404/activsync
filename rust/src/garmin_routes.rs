//! Connecting Garmin: the setup step, reconnecting, and Garmin's one-time
//! code. Against real accounts the email and password go to Garmin's
//! sign-in; the mock account takes any password but the MFA trigger.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use chrono::Utc;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::garmin_login::Login;
use crate::mock::{self, GarminLogin};
use crate::routes::{ApiError, development};
use crate::settings_routes::{ActionResult, action, bad_request, body, conflict, invalid, onboarded, require_not_onboarded, text};
use crate::sync_routes::{fetch_categories, run_catch_up};
use crate::{AppState, GarminCredentials, PendingMfa, config, db, onboarding, timefmt};

type ApiResult<T> = Result<Json<T>, ApiError>;

enum GarminOutcome {
    Connected,
    Mfa,
}

/// Marks Garmin connected, then fetches its activity categories, best
/// effort; on the first run every category starts held.
async fn finalize_garmin_connect(state: &AppState) -> Result<(), ApiError> {
    let now = Utc::now();
    let Some(clients) = state.clients.clone() else {
        onboarding::finalize_garmin_connect(&state.db.lock(), development(state), now)?;
        return Ok(());
    };
    let first_run = {
        let conn = state.db.lock();
        db::set_config_value(&conn, "garmin_credentials_verified", &json!(true))?;
        db::set_config_value(&conn, "garmin_last_sync_error", &Value::Null)?;
        !onboarded(&conn)?
    };
    let fetched = match clients.garmin().await {
        Ok(garmin) => fetch_categories(state, &garmin, first_run, now).await,
        Err(err) => Err(err.to_string()),
    };
    if let Err(err) = fetched {
        crate::log_warn!("garmin_routes", "Garmin category prefetch after connect failed: {err}");
    }
    Ok(())
}

fn save_credentials(state: &AppState, credentials: &GarminCredentials) -> Result<(), ApiError> {
    let value = json!({"email": credentials.email, "password": credentials.password});
    db::set_config_value(&state.db.lock(), "garmin_credentials", &value)?;
    Ok(())
}

/// Starts a Garmin sign-in. A pending verification from an earlier attempt is
/// dropped first, so a code can only ever finish the latest attempt.
async fn save_and_verify_garmin(state: &AppState, email: &str, password: &str) -> Result<GarminOutcome, ApiError> {
    state.set_pending_mfa(None);
    let credentials = GarminCredentials { email: email.into(), password: password.into() };
    let login = match &state.garmin_sign_in {
        Some(sign_in) => sign_in
            .begin(email, password)
            .await
            .map_err(|err| ApiError::new(StatusCode::BAD_GATEWAY, format!("Could not connect to Garmin: {err}")))?,
        None => match mock::begin_garmin_login(email, password) {
            GarminLogin::MfaRequired => Login::MfaRequired(Default::default()),
            GarminLogin::Connected => Login::Connected,
        },
    };
    match login {
        Login::MfaRequired(pending) => {
            state.set_pending_mfa(Some(PendingMfa { credentials, login: pending }));
            Ok(GarminOutcome::Mfa)
        }
        Login::Connected => {
            save_credentials(state, &credentials)?;
            finalize_garmin_connect(state).await?;
            Ok(GarminOutcome::Connected)
        }
    }
}

fn garmin_login_fields(conn: &Connection, email: &str, password: &str) -> Result<(String, String), ApiError> {
    let existing = db::get_config_value(conn, "garmin_credentials")?.unwrap_or(Value::Null);
    let email = if email.is_empty() { text(&existing, "email") } else { email.to_string() };
    let email = email.trim().to_string();
    let password = if password.is_empty() { text(&existing, "password") } else { password.to_string() };
    if email.is_empty() || password.is_empty() {
        return Err(bad_request("Enter your Garmin email and password."));
    }
    Ok((email, password))
}

fn last_sync_ok_at(state: &AppState) -> Result<Option<String>, ApiError> {
    let value = db::get_config_value(&state.db.lock(), "garmin_last_sync_ok_at")?;
    Ok(value.and_then(|v| v.as_str().map(str::to_string)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GarminSetupRequest {
    email: String,
    #[serde(default)]
    password: String,
    #[serde(default = "default_lookback")]
    lookback_days: i64,
    #[serde(default)]
    detected_timezone: String,
}

fn default_lookback() -> i64 {
    7
}

pub async fn setup_garmin(
    State(state): State<AppState>,
    payload: Result<Json<GarminSetupRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let payload = body(payload)?;
    if payload.lookback_days < 1 {
        return Err(invalid("lookbackDays must be at least 1"));
    }
    let (email, password) = {
        let conn = state.db.lock();
        require_not_onboarded(&conn)?;
        let fields = garmin_login_fields(&conn, &payload.email, &payload.password)?;
        config::edit(&conn, development(&state), |cfg| {
            cfg.set("lookback_days", json!(payload.lookback_days));
            if timefmt::is_valid_timezone(&payload.detected_timezone) {
                cfg.set("display_timezone", json!(payload.detected_timezone));
            }
        })?;
        fields
    };
    if let GarminOutcome::Mfa = save_and_verify_garmin(&state, &email, &password).await? {
        return action(&state, &state.db.lock(), "Garmin sent a verification code.", true);
    }
    state.events.publish("refresh");
    action(&state, &state.db.lock(), "Garmin connected.", false)
}

#[derive(Deserialize)]
pub struct GarminReconnectRequest {
    email: String,
    #[serde(default)]
    password: String,
}

pub async fn reconnect_garmin(
    State(state): State<AppState>,
    payload: Result<Json<GarminReconnectRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let payload = body(payload)?;
    let (email, password) = garmin_login_fields(&state.db.lock(), &payload.email, &payload.password)?;
    let last_ok = last_sync_ok_at(&state)?;
    if let GarminOutcome::Mfa = save_and_verify_garmin(&state, &email, &password).await? {
        return action(&state, &state.db.lock(), "Garmin sent a verification code.", true);
    }
    run_catch_up(&state, Some(last_ok)).await?;
    action(&state, &state.db.lock(), "Garmin reconnected and catch-up sync started.", false)
}

#[derive(Deserialize)]
pub struct MfaRequest {
    code: String,
}

/// A wrong code keeps the challenge open for another try.
pub async fn complete_mfa(
    State(state): State<AppState>,
    payload: Result<Json<MfaRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let payload = body(payload)?;
    let Some(pending) = state.pending_mfa() else {
        return Err(conflict("The Garmin verification session expired. Sign in to Garmin again to get a new code."));
    };
    let last_ok = last_sync_ok_at(&state)?;
    let completed = match &state.garmin_sign_in {
        Some(sign_in) => sign_in.complete(&pending.login, &payload.code).await.map_err(|err| err.to_string()),
        None => mock::complete_garmin_login(&payload.code),
    };
    completed.map_err(|err| ApiError::new(StatusCode::UNAUTHORIZED, err))?;
    state.set_pending_mfa(None);
    save_credentials(&state, &pending.credentials)?;
    finalize_garmin_connect(&state).await?;
    if onboarded(&state.db.lock())? {
        run_catch_up(&state, Some(last_ok)).await?;
    } else {
        state.events.publish("refresh");
    }
    action(&state, &state.db.lock(), "Garmin verified and connected.", false)
}

pub async fn cancel_mfa(State(state): State<AppState>) -> ApiResult<ActionResult> {
    state.set_pending_mfa(None);
    action(&state, &state.db.lock(), "Garmin verification cancelled.", false)
}
