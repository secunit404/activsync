use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::AppState;
use crate::auth_core::{self, SESSION_ABSOLUTE_SECONDS};
use crate::auth_guard::{AuthState, RequestContext, cookie};
use crate::{db, oidc_config, oidc_logo};
use crate::routes::ApiError;

pub(crate) type ApiResult<T> = Result<Json<T>, ApiError>;

const MAX_FIELD: usize = 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    authenticated: bool,
    method: Option<String>,
    user: Option<String>,
    owner_claimed: bool,
    auth_disabled: bool,
    login: Value,
}

/// The login page offers single sign-on once an account is linked to it.
fn status_for(state: &AppState, auth: Option<&AuthState>) -> Result<AuthStatus, ApiError> {
    let conn = state.db.lock();
    let oidc = match oidc_config::load(&conn)?.filter(|c| c.linked.is_some()) {
        Some(config) => json!({
            "label": config.display_name,
            "autoRedirect": config.auto_redirect,
            "logoUrl": oidc_logo::current_url(&conn)?,
        }),
        None => Value::Null,
    };
    drop(conn);
    Ok(AuthStatus {
        authenticated: auth.is_some(),
        method: auth.map(|a| a.method.clone()),
        user: auth.and_then(|a| a.user.clone()),
        owner_claimed: auth_core::owner_claimed(&state.db.lock())?,
        auth_disabled: state.auth.disabled,
        login: json!({ "oidc": oidc }),
    })
}

pub async fn auth_status(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>) -> ApiResult<AuthStatus> {
    Ok(Json(status_for(&state, ctx.auth.as_ref())?))
}

pub(crate) fn body<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    payload
        .map(|Json(value)| value)
        .map_err(|rejection| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, rejection.body_text()))
}

pub(crate) fn too_long(fields: &[&str]) -> Result<(), ApiError> {
    if fields.iter().any(|f| f.chars().count() > MAX_FIELD) {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "That field is too long."));
    }
    Ok(())
}

fn retry_error(message: String, wait: i64) -> ApiError {
    ApiError::new(StatusCode::TOO_MANY_REQUESTS, message).with_header(header::RETRY_AFTER, wait.to_string())
}

fn limit_client(state: &AppState, ctx: &RequestContext) -> Result<(), ApiError> {
    let wait = state.limiter.retry_after(&ctx.client_ip, state.scheduler.now());
    if wait > 0 {
        return Err(retry_error(format!("Too many attempts. Try again in {wait} seconds."), wait));
    }
    Ok(())
}

pub(crate) fn trusted_device(state: &AppState, headers: &HeaderMap, ctx: &RequestContext) -> Result<Option<String>, ApiError> {
    let device = cookie(headers, ctx.device_cookie()).map(str::to_string);
    Ok(device.filter(|d| auth_core::is_trusted(&state.db.lock(), Some(d)).unwrap_or(false)))
}

pub(crate) async fn require_password(
    state: &AppState,
    password: &str,
    wrong_status: StatusCode,
    device: Option<&str>,
) -> Result<(), ApiError> {
    let reserved = auth_core::reserve_check(&state.db.lock(), state.scheduler.now(), device)?;
    let reserved = match reserved {
        Ok(reserved) => reserved,
        Err(wait) => return Err(retry_error(format!("Too many wrong passwords. Try again in {wait} seconds."), wait)),
    };
    let (stored, candidate, cost) = (reserved.stored.clone(), password.to_string(), state.auth.cost);
    let verdict = tokio::task::spawn_blocking(move || auth_core::verify_stored(stored.as_deref(), &candidate, cost))
        .await
        .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    if !auth_core::settle_check(&state.db.lock(), device, &reserved, &verdict)? {
        return Err(ApiError::new(wrong_status, "Wrong password."));
    }
    Ok(())
}

pub(crate) fn require_session(ctx: &RequestContext) -> Result<&AuthState, ApiError> {
    ctx.auth.as_ref().ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Not signed in"))
}

pub(crate) fn set_cookie(name: &str, value: &str, max_age: i64, https: bool) -> HeaderValue {
    let secure = if https { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("{name}={value}; HttpOnly; Max-Age={max_age}; Path=/; SameSite=lax{secure}"))
        .expect("cookie is ASCII")
}

pub(crate) fn clear_cookie(name: &str, https: bool) -> HeaderValue {
    let secure = if https { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{name}=\"\"; expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; Max-Age=0; Path=/; SameSite=lax{secure}"
    ))
    .expect("cookie is ASCII")
}

/// Rotates on every sign-in: the session the browser arrived with is dropped
/// and a fresh token issued. Every sign-in also trusts this browser.
pub(crate) fn start_session(state: &AppState, headers: &HeaderMap, ctx: &RequestContext, method: &str) -> Result<Vec<HeaderValue>, ApiError> {
    let conn = state.db.lock();
    if let Some(previous) = cookie(headers, ctx.session_cookie()).filter(|p| !p.is_empty()) {
        auth_core::revoke_session(&conn, &auth_core::hash_token(previous))?;
    }
    let user_agent = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or_default();
    let now = state.scheduler.now();
    let token = auth_core::create_session(&conn, method, &ctx.client_ip, user_agent, now)?;
    let device = auth_core::trust_device(&conn, now, cookie(headers, ctx.device_cookie()))?;
    Ok(vec![
        set_cookie(ctx.session_cookie(), &token, SESSION_ABSOLUTE_SECONDS, ctx.https),
        set_cookie(ctx.device_cookie(), &device, auth_core::DEVICE_COOKIE_MAX_AGE, ctx.https),
    ])
}

pub(crate) fn with_cookies(body: impl IntoResponse, cookies: Vec<HeaderValue>) -> Response {
    let mut response = body.into_response();
    for value in cookies {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimRequest {
    setup_code: String,
    password: String,
}

pub async fn claim(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
    payload: Result<Json<ClaimRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let payload = body(payload)?;
    if payload.setup_code.chars().count() > 64 {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "That field is too long."));
    }
    too_long(&[&payload.password])?;
    if auth_core::owner_claimed(&state.db.lock())? {
        return Err(ApiError::new(StatusCode::CONFLICT, "ActivSync is already claimed."));
    }
    limit_client(&state, &ctx)?;
    if let Some(problem) = auth_core::password_problem(&payload.password) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, problem));
    }
    if !state.setup_code.check(&payload.setup_code) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "That setup code is not right. Check the server log for the current one.",
        ));
    }
    let hash = auth_core::hash_password_blocking(payload.password.clone(), state.auth.cost)
        .await
        .map_err(|problem| ApiError::new(StatusCode::BAD_REQUEST, problem))?;
    if !auth_core::claim_owner(&state.db.lock(), &hash, state.scheduler.now())? {
        return Err(ApiError::new(StatusCode::CONFLICT, "ActivSync is already claimed."));
    }
    state.setup_code.discard();
    let cookies = start_session(&state, &headers, &ctx, "password")?;
    Ok(with_cookies(Json(status_for(&state, Some(&AuthState::of("password")))?), cookies))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    password: String,
}

pub async fn login(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
    payload: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let payload = body(payload)?;
    too_long(&[&payload.password])?;
    if !auth_core::owner_claimed(&state.db.lock())? {
        return Err(ApiError::new(StatusCode::CONFLICT, "ActivSync is not claimed yet."));
    }
    let device = trusted_device(&state, &headers, &ctx)?;
    if device.is_none() {
        limit_client(&state, &ctx)?;
    }
    require_password(&state, &payload.password, StatusCode::UNAUTHORIZED, device.as_deref()).await?;
    let cookies = start_session(&state, &headers, &ctx, "password")?;
    Ok(with_cookies(Json(status_for(&state, Some(&AuthState::of("password")))?), cookies))
}

pub async fn logout(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>) -> Result<Response, ApiError> {
    let auth = require_session(&ctx)?;
    let Some(token_hash) = &auth.token_hash else {
        return Err(ApiError::new(StatusCode::CONFLICT, "You are signed in through your proxy; sign out there."));
    };
    auth_core::revoke_session(&state.db.lock(), token_hash)?;
    Ok(with_cookies(StatusCode::NO_CONTENT, vec![clear_cookie(ctx.session_cookie(), ctx.https)]))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    public_id: String,
    method: String,
    created_at: String,
    last_seen_at: String,
    ip: String,
    user_agent: String,
    current: bool,
}

fn iso(epoch: i64) -> String {
    DateTime::from_timestamp(epoch, 0).map(db::iso).unwrap_or_default()
}

pub async fn list_sessions(State(state): State<AppState>, Extension(ctx): Extension<RequestContext>) -> ApiResult<Vec<SessionInfo>> {
    let auth = require_session(&ctx)?;
    let sessions = auth_core::list_sessions(&state.db.lock())?;
    Ok(Json(
        sessions
            .into_iter()
            .map(|s| SessionInfo {
                current: auth.token_hash.as_deref() == Some(s.token_hash.as_str()),
                public_id: s.public_id,
                method: s.method,
                created_at: iso(s.created_at),
                last_seen_at: iso(s.last_seen_at),
                ip: s.ip,
                user_agent: s.user_agent,
            })
            .collect(),
    ))
}

pub async fn revoke_one(State(state): State<AppState>, Path(public_id): Path<String>) -> Result<StatusCode, ApiError> {
    if !auth_core::revoke_public_id(&state.db.lock(), &public_id)? {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "That session has already ended."));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn revoke_others(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
) -> ApiResult<Value> {
    let auth = require_session(&ctx)?;
    let device = trusted_device(&state, &headers, &ctx)?;
    let conn = state.db.lock();
    let revoked = auth_core::revoke_all(&conn, auth.token_hash.as_deref())?;
    auth_core::clear_trusted_devices(&conn, device.as_deref())?;
    Ok(Json(json!({ "message": format!("Signed out {revoked} other session(s).") })))
}

#[derive(Deserialize)]
pub struct PasswordChangeRequest {
    current: String,
    new: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    headers: HeaderMap,
    payload: Result<Json<PasswordChangeRequest>, JsonRejection>,
) -> ApiResult<Value> {
    let payload = body(payload)?;
    too_long(&[&payload.current, &payload.new])?;
    let auth = require_session(&ctx)?;
    let device = trusted_device(&state, &headers, &ctx)?;
    require_password(&state, &payload.current, StatusCode::BAD_REQUEST, device.as_deref()).await?;
    let hash = auth_core::hash_password_blocking(payload.new.clone(), state.auth.cost)
        .await
        .map_err(|problem| ApiError::new(StatusCode::BAD_REQUEST, problem))?;
    let conn = state.db.lock();
    auth_core::set_password_hash(&conn, &hash, state.scheduler.now())?;
    auth_core::revoke_all(&conn, auth.token_hash.as_deref())?;
    auth_core::clear_trusted_devices(&conn, device.as_deref())?;
    Ok(Json(json!({ "message": "Password changed. Other sessions were signed out." })))
}

pub async fn auth_settings(State(state): State<AppState>) -> Json<Value> {
    let proxy = state.auth.proxy.as_ref().map(|p| {
        json!({
            "header": p.header,
            "trusted": p.trusted.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "user": p.user,
        })
    });
    Json(json!({ "proxy": proxy }))
}

/// Mock only: drops the owner so an end-to-end test can walk the claim step.
pub async fn dev_unclaim(State(state): State<AppState>) -> ApiResult<Value> {
    if !state.mock_mode {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "Not found"));
    }
    db::set_config_value(&state.db.lock(), auth_core::OWNER_KEY, &Value::Null)?;
    Ok(Json(json!({ "setupCode": state.setup_code.reissue() })))
}
