//! Connecting Strava: the authorization link and Strava's callback. Against
//! real accounts the code is exchanged with Strava; the mock accounts hand
//! out stand-in tokens.

use axum::extract::{Extension, Query, State};
use axum::http::HeaderMap;
use axum::response::Redirect;
use axum::Json;
use base64::Engine;
use chrono::Utc;
use rand::RngCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth_guard::RequestContext;
use crate::routes::ApiError;
use crate::settings_routes::{conflict, onboarded};
use crate::sync_routes::run_catch_up;
use crate::{AppState, db, mock, onboarding, oidc_routes, view};

fn new_state_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub async fn strava_authorize(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Value>, ApiError> {
    let conn = state.db.lock();
    let credentials = db::get_config_value(&conn, "strava_credentials")?.unwrap_or(Value::Null);
    if !db::truthy(credentials.get("client_id")) || !db::truthy(credentials.get("client_secret")) {
        return Err(conflict("Save your Strava client ID and client secret before connecting."));
    }
    let callback = format!("{}/strava/callback", oidc_routes::origin(&state, &headers, &ctx));
    let token = new_state_token();
    db::set_config_value(&conn, "strava_oauth_state", &json!(token))?;
    drop(conn);
    let url = match &state.clients {
        Some(clients) => clients.strava().authorize_url(&callback, &token),
        None => mock::strava_authorize_url(&callback, &token),
    };
    Ok(Json(json!({ "url": url })))
}

#[derive(Deserialize)]
pub struct StravaCallback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    scope: Option<String>,
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `urllib.parse.urlencode`'s quoting: spaces become `+`.
fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub async fn strava_callback(State(state): State<AppState>, Query(query): Query<StravaCallback>) -> Result<Redirect, ApiError> {
    let (expected, landing) = {
        let conn = state.db.lock();
        let expected = db::get_config_value(&conn, "strava_oauth_state")?.and_then(|v| v.as_str().map(str::to_string));
        db::set_config_value(&conn, "strava_oauth_state", &Value::Null)?;
        (expected, if onboarded(&conn)? { "/settings" } else { "/setup" })
    };
    let fail = |message: &str| Ok(Redirect::to(&format!("{landing}?stravaError={}", urlencode(message))));
    if query.error.as_deref() == Some("access_denied") {
        return fail(
            "Strava authorization was declined, so nothing was connected. \
             Press Connect with Strava and choose Authorize on Strava's page.",
        );
    }
    let code = query.code.as_deref().unwrap_or_default();
    if query.error.as_deref().is_some_and(|error| !error.is_empty()) || code.is_empty() {
        crate::log_warn!("strava_routes", "strava authorization did not complete: error={:?}", query.error);
        return fail(
            "Strava did not return an authorization. Try again. If the problem \
             continues, check the Authorization Callback Domain of your Strava \
             API application.",
        );
    }
    let state_matches = match (&expected, &query.state) {
        (Some(expected), Some(got)) if !expected.is_empty() && !got.is_empty() => constant_time_eq(expected, got),
        _ => false,
    };
    if !state_matches {
        return fail(
            "That Strava response did not match the connection request, so it \
             was ignored. Start the connection again.",
        );
    }
    match &state.clients {
        Some(clients) => {
            if let Err(err) = clients.strava().exchange_code(code, query.scope.as_deref()).await {
                crate::log_warn!("strava_routes", "strava code exchange failed: {err}");
                return fail(
                    "Could not complete the Strava connection. Check your client ID \
                     and client secret, then try again.",
                );
            }
        }
        None => {
            let tokens = mock::strava_tokens(query.scope.as_deref(), Utc::now().timestamp());
            db::set_config_value(&state.db.lock(), "strava_tokens", &tokens)?;
        }
    }
    if !view::scope_is_sufficient(query.scope.as_deref()) {
        return fail(
            "Strava didn't grant upload access. Press Connect with Strava \
             again and leave both permissions ticked.",
        );
    }
    let in_setup = {
        let conn = state.db.lock();
        let connections = view::connection_status(&conn, Utc::now())?;
        onboarding::setup_step(&conn, &connections, state.auth.disabled)?.is_some()
    };
    if in_setup {
        return Ok(Redirect::to("/setup"));
    }
    run_catch_up(&state, None).await?;
    Ok(Redirect::to("/settings"))
}
