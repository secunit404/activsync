use axum::Json;
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::auth_guard::{RequestContext, cookie, quote};
use crate::auth_routes::{
    ApiResult, body, require_password, require_session, set_cookie, start_session, trusted_device, with_cookies,
};
use crate::oidc_config::{self, DEFAULT_SCOPES, Linked, NewConfig, OIDC_METHOD};
use crate::oidc_flow::{self, Callback, OidcError};
use crate::oidc_logo;
use crate::routes::ApiError;

const OIDC_COOKIE: &str = "activsync_oidc";
const SECURE_OIDC_COOKIE: &str = "__Host-activsync_oidc";
const OIDC_COOKIE_MAX_AGE: i64 = 600;

fn oidc_cookie(ctx: &RequestContext) -> &'static str {
    if ctx.https { SECURE_OIDC_COOKIE } else { OIDC_COOKIE }
}

/// Where the provider sends the browser back: the public URL when one is
/// set, else the origin this request arrived on.
pub(crate) fn origin(state: &AppState, headers: &HeaderMap, ctx: &RequestContext) -> String {
    if let Some(public) = &state.auth.public_url {
        return public.clone();
    }
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default();
    format!("{}://{host}", if ctx.https { "https" } else { "http" })
}

fn redirect_uri(state: &AppState, headers: &HeaderMap, ctx: &RequestContext) -> String {
    format!("{}/auth/oidc/callback", origin(state, headers, ctx))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcConfigState {
    configured: bool,
    display_name: String,
    issuer: String,
    client_id: String,
    has_secret: bool,
    scopes: String,
    auto_redirect: bool,
    redirect_uri: String,
    linked_label: Option<String>,
    logo_url: Option<String>,
}

pub async fn oidc_state(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(ctx): Extension<RequestContext>,
) -> ApiResult<OidcConfigState> {
    let conn = state.db.lock();
    let config = oidc_config::load(&conn)?;
    Ok(Json(OidcConfigState {
        configured: config.is_some(),
        display_name: config.as_ref().map(|c| c.display_name.clone()).unwrap_or_default(),
        issuer: config.as_ref().map(|c| c.issuer.clone()).unwrap_or_default(),
        client_id: config.as_ref().map(|c| c.client_id.clone()).unwrap_or_default(),
        has_secret: config.as_ref().is_some_and(|c| !c.client_secret.is_empty()),
        scopes: config.as_ref().map_or(DEFAULT_SCOPES.into(), |c| c.scopes.clone()),
        auto_redirect: config.as_ref().is_some_and(|c| c.auto_redirect),
        redirect_uri: redirect_uri(&state, &headers, &ctx),
        linked_label: config.as_ref().and_then(|c| c.linked.as_ref()).map(|l| l.label.clone()),
        logo_url: oidc_logo::current_url(&conn)?,
    }))
}

#[derive(Serialize)]
pub struct AuthActionResult {
    message: String,
}

fn message(text: &str) -> Json<AuthActionResult> {
    Json(AuthActionResult { message: text.into() })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcConfigRequest {
    display_name: String,
    issuer: String,
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
    #[serde(default = "default_scopes")]
    scopes: String,
    #[serde(default)]
    auto_redirect: bool,
    current_password: String,
}

fn default_scopes() -> String {
    DEFAULT_SCOPES.into()
}

fn length(value: &str, min: usize, max: usize, field: &str) -> Result<(), ApiError> {
    let n = value.chars().count();
    if n < min || n > max {
        return Err(ApiError::invalid(format!("{field} must be {min} to {max} characters.")));
    }
    Ok(())
}

pub async fn save_oidc(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(ctx): Extension<RequestContext>,
    payload: Result<Json<OidcConfigRequest>, JsonRejection>,
) -> ApiResult<AuthActionResult> {
    let request = body(payload)?;
    length(&request.display_name, 1, 64, "displayName")?;
    length(&request.issuer, 1, 512, "issuer")?;
    length(&request.client_id, 1, 256, "clientId")?;
    length(request.client_secret.as_deref().unwrap_or_default(), 0, 1024, "clientSecret")?;
    length(&request.scopes, 0, 512, "scopes")?;
    length(&request.current_password, 0, 1024, "currentPassword")?;
    let device = trusted_device(&state, &headers, &ctx)?;
    require_password(&state, &request.current_password, StatusCode::BAD_REQUEST, device.as_deref())?;
    let config = oidc_config::save(
        &state.db.lock(),
        &NewConfig {
            display_name: request.display_name.trim(),
            issuer: &request.issuer,
            client_id: request.client_id.trim(),
            client_secret: request.client_secret.as_deref(),
            scopes: &request.scopes,
            auto_redirect: request.auto_redirect,
        },
    )
    .map_err(|err| ApiError::new(StatusCode::BAD_REQUEST, err))?;
    Ok(message(if config.linked.is_some() {
        "Single sign-on settings saved."
    } else {
        "Single sign-on settings saved. Link your account to finish."
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentPasswordRequest {
    current_password: String,
}

fn confirm_password(
    state: &AppState,
    headers: &HeaderMap,
    ctx: &RequestContext,
    payload: Result<Json<CurrentPasswordRequest>, JsonRejection>,
) -> Result<(), ApiError> {
    let request = body(payload)?;
    length(&request.current_password, 0, 1024, "currentPassword")?;
    let device = trusted_device(state, headers, ctx)?;
    require_password(state, &request.current_password, StatusCode::BAD_REQUEST, device.as_deref())
}

pub async fn delete_oidc(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(ctx): Extension<RequestContext>,
    payload: Result<Json<CurrentPasswordRequest>, JsonRejection>,
) -> ApiResult<AuthActionResult> {
    confirm_password(&state, &headers, &ctx, payload)?;
    oidc_config::delete(&state.db.lock())?;
    Ok(message("Single sign-on removed."))
}

#[derive(Serialize)]
pub struct AuthorizeUrl {
    url: String,
}

/// Linking needs the password just now, so a borrowed session alone cannot
/// swap the owner's identity provider account.
pub async fn start_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(ctx): Extension<RequestContext>,
    payload: Result<Json<CurrentPasswordRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_session(&ctx)?;
    if oidc_config::load(&state.db.lock())?.is_none() {
        return Err(ApiError::conflict("Set up single sign-on first."));
    }
    confirm_password(&state, &headers, &ctx, payload)?;
    let key = state.oidc_sessions.grant_link(cookie(&headers, oidc_cookie(&ctx)), state.scheduler.now());
    let url = format!("/auth/oidc/login?intent=link&next={}", quote("/settings"));
    Ok(with_cookies(
        Json(AuthorizeUrl { url }),
        vec![set_cookie(oidc_cookie(&ctx), &key, OIDC_COOKIE_MAX_AGE, ctx.https)],
    ))
}

pub async fn logo(State(state): State<AppState>) -> Result<Response, ApiError> {
    let Some(png) = oidc_logo::load(&state.db.lock())? else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "No logo uploaded"));
    };
    let mut response = png.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; sandbox"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(response)
}

pub async fn upload_logo(State(state): State<AppState>, headers: HeaderMap, upload: Body) -> ApiResult<AuthActionResult> {
    if oidc_config::load(&state.db.lock())?.is_none() {
        return Err(ApiError::conflict("Set up single sign-on first."));
    }
    let too_large = || ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, oidc_logo::TOO_LARGE);
    let declared = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<usize>().ok());
    if declared.is_some_and(|n| n > oidc_logo::MAX_UPLOAD_BYTES) {
        return Err(too_large());
    }
    let data = axum::body::to_bytes(upload, oidc_logo::MAX_UPLOAD_BYTES).await.map_err(|_| too_large())?;
    let png = tokio::task::spawn_blocking(move || oidc_logo::normalize(&data))
        .await
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error"))?
        .map_err(|err| ApiError::new(StatusCode::BAD_REQUEST, err))?;
    oidc_logo::save(&state.db.lock(), &png, state.scheduler.now())?;
    Ok(message("Logo saved."))
}

pub async fn delete_logo(State(state): State<AppState>) -> ApiResult<AuthActionResult> {
    oidc_logo::delete(&state.db.lock())?;
    Ok(message("Logo removed. The sign-in button shows a key again."))
}

const SINGLE_DOTS: [&str; 2] = [".", "%2e"];
const DOUBLE_DOTS: [&str; 4] = ["..", ".%2e", "%2e.", "%2e%2e"];

/// Dot segments removed the way browsers do, including percent-encoded
/// dots and a `..` at the root.
fn resolved_path(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').skip(1).collect();
    let mut resolved: Vec<&str> = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        let last = index + 1 == segments.len();
        let lowered = segment.to_lowercase();
        if DOUBLE_DOTS.contains(&lowered.as_str()) {
            resolved.pop();
            if last {
                resolved.push("");
            }
        } else if SINGLE_DOTS.contains(&lowered.as_str()) {
            if last {
                resolved.push("");
            }
        } else {
            resolved.push(segment);
        }
    }
    format!("/{}", resolved.join("/"))
}

/// A same-site path to land on after signing in; anything that could leave
/// the site becomes `/`.
pub fn safe_next(value: Option<&str>) -> String {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return "/".into();
    };
    let path = value.split(['?', '#']).next().unwrap_or_default();
    let unsafe_ = !value.starts_with('/')
        || value.starts_with("//")
        || value.contains('\\')
        || value.chars().any(|c| (c as u32) < 0x20)
        || resolved_path(path).starts_with("//");
    if unsafe_ { "/".into() } else { value.into() }
}

fn see_other(location: &str) -> Response {
    Redirect::to(location).into_response()
}

fn login_error(code: &str) -> Response {
    see_other(&format!("/login?error={}", quote(code)))
}

fn link_error(code: &str) -> Response {
    see_other(&format!("/settings?oidcError={}", quote(code)))
}

fn error_response(err: &OidcError) -> Response {
    if err.intent == "link" { link_error(err.code) } else { login_error(err.code) }
}

#[derive(Deserialize)]
pub struct LoginQuery {
    intent: Option<String>,
    next: Option<String>,
}

pub async fn oidc_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(ctx): Extension<RequestContext>,
    Query(query): Query<LoginQuery>,
) -> Result<Response, ApiError> {
    let intent = query.intent.unwrap_or_else(|| "login".into());
    if intent != "login" && intent != "link" {
        return Err(ApiError::invalid("intent must be login or link."));
    }
    let now = state.scheduler.now();
    let cookie_value = cookie(&headers, oidc_cookie(&ctx));
    if intent == "link" {
        if ctx.auth.is_none() {
            return Ok(see_other("/login?next=%2Fsettings"));
        }
        if !state.oidc_sessions.consume_link_grant(cookie_value, now) {
            return Ok(link_error("link_password"));
        }
    }
    let Some(config) = oidc_config::load(&state.db.lock())? else {
        return Ok(error_response(&OidcError { code: "oidc_unavailable", intent }));
    };
    let next = safe_next(query.next.as_deref());
    let uri = redirect_uri(&state, &headers, &ctx);
    match oidc_flow::begin(&state.oidc_http, &state.oidc_sessions, &config, cookie_value, now, &uri, &intent, &next).await {
        Ok((url, key)) => {
            let mut response = (StatusCode::FOUND, [(header::LOCATION, url)]).into_response();
            response.headers_mut().append(header::SET_COOKIE, set_cookie(oidc_cookie(&ctx), &key, OIDC_COOKIE_MAX_AGE, ctx.https));
            Ok(response)
        }
        Err(err) => Ok(error_response(&err)),
    }
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

pub async fn oidc_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(ctx): Extension<RequestContext>,
    Query(query): Query<CallbackQuery>,
) -> Result<Response, ApiError> {
    let flow = state.oidc_sessions.take_flow(cookie(&headers, oidc_cookie(&ctx)));
    let config = oidc_config::load(&state.db.lock())?;
    let callback = Callback { state: query.state.as_deref(), code: query.code.as_deref(), error: query.error.as_deref() };
    let result = match oidc_flow::finish(&state.oidc_http, config.as_ref(), flow, callback).await {
        Ok(result) => result,
        Err(err) => return Ok(error_response(&err)),
    };
    if result.intent == "link" {
        let Some(auth) = &ctx.auth else {
            return Ok(login_error("oidc_state"));
        };
        let linked = Linked { iss: result.iss, sub: result.sub, label: result.label };
        oidc_config::set_linked(&state.db.lock(), linked, auth.token_hash.as_deref())?;
        return Ok(see_other("/settings?oidc=linked"));
    }
    let linked = config.and_then(|c| c.linked);
    if !linked.is_some_and(|l| l.iss == result.iss && l.sub == result.sub) {
        crate::log_warn!("oidc_routes", "OIDC sign-in by {} at {} refused; it is not the linked account", result.sub, result.iss);
        return Ok(login_error("oidc_mismatch"));
    }
    let cookies = start_session(&state, &headers, &ctx, OIDC_METHOD)?;
    Ok(with_cookies(see_other(&result.next), cookies))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_paths_follow_python_rules() {
        for (value, expected) in [
            (None, "/"),
            (Some("/settings?tab=access"), "/settings?tab=access"),
            (Some("//evil.example"), "/"),
            (Some("https://evil.example"), "/"),
            (Some("/\\evil"), "/"),
            (Some("/a/../..//evil"), "/"),
            (Some("/%2e%2e//evil.example"), "/"),
            (Some("/ok/./path"), "/ok/./path"),
        ] {
            assert_eq!(safe_next(value), expected, "{value:?}");
        }
    }
}
