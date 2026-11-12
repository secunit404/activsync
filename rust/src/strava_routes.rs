//! Connecting Strava: the authorization link and Strava's callback. Against
//! real accounts the code is exchanged with Strava; the mock accounts hand
//! out stand-in tokens.

use axum::Json;
use axum::extract::{Extension, Query, State};
use axum::http::HeaderMap;
use axum::response::Redirect;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth_core::constant_time_eq;
use crate::auth_guard::RequestContext;
use crate::routes::ApiError;
use crate::settings_routes::onboarded;
use crate::sync_routes::run_catch_up;
use crate::{AppState, db, mock, oidc_routes, onboarding, view};

pub async fn strava_authorize(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(ctx): Extension<RequestContext>,
) -> Result<Json<Value>, ApiError> {
    let conn = state.db.lock();
    let credentials = db::get_config_value(&conn, "strava_credentials")?.unwrap_or(Value::Null);
    if !db::truthy(credentials.get("client_id")) || !db::truthy(credentials.get("client_secret")) {
        return Err(ApiError::conflict(
            "Save your Strava client ID and client secret before connecting.",
        ));
    }
    let callback = format!(
        "{}/strava/callback",
        oidc_routes::origin(&state, &headers, &ctx)
    );
    let token = crate::auth_core::random_token(32);
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

/// `urllib.parse.urlencode`'s quoting: spaces become `+`.
fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub async fn strava_callback(
    State(state): State<AppState>,
    Query(query): Query<StravaCallback>,
) -> Result<Redirect, ApiError> {
    let (expected, landing) = {
        let conn = state.db.lock();
        let expected = db::get_config_value(&conn, "strava_oauth_state")?
            .and_then(|v| v.as_str().map(str::to_string));
        db::set_config_value(&conn, "strava_oauth_state", &Value::Null)?;
        (
            expected,
            if onboarded(&conn)? {
                "/settings"
            } else {
                "/setup"
            },
        )
    };
    let fail = |message: &str| {
        Ok(Redirect::to(&format!(
            "{landing}?stravaError={}",
            urlencode(message)
        )))
    };
    if query.error.as_deref() == Some("access_denied") {
        return fail(
            "Strava authorization was declined, so nothing was connected. \
             Press Connect with Strava and choose Authorize on Strava's page.",
        );
    }
    let code = query.code.as_deref().unwrap_or_default();
    if query
        .error
        .as_deref()
        .is_some_and(|error| !error.is_empty())
        || code.is_empty()
    {
        crate::log_warn!(
            "strava_routes",
            "strava authorization did not complete: error={:?}",
            query.error
        );
        return fail(
            "Strava did not return an authorization. Try again. If the problem \
             continues, check the Authorization Callback Domain of your Strava \
             API application.",
        );
    }
    let state_matches =
        expected
            .as_deref()
            .zip(query.state.as_deref())
            .is_some_and(|(expected, got)| {
                !expected.is_empty() && constant_time_eq(expected.as_bytes(), got.as_bytes())
            });
    if !state_matches {
        return fail(
            "That Strava response did not match the connection request, so it \
             was ignored. Start the connection again.",
        );
    }
    match &state.clients {
        Some(clients) => {
            if let Err(err) = clients
                .strava()
                .exchange_code(code, query.scope.as_deref())
                .await
            {
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
