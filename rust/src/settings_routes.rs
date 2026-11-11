use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Redirect;
use base64::Engine;
use chrono::Utc;
use rand::RngCore;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::db;
use crate::hevy_settings::{self, DeviceIdentity, HevySettingsState, ProfileOverride};
use crate::mock::{self, GarminLogin};
use crate::onboarding;
use crate::routes::{ApiError, UpdateState};
use crate::view::{self, Connections};
use crate::{AppState, GarminCredentials, config, timefmt, version};

type ApiResult<T> = Result<Json<T>, ApiError>;

const COMMON_TIMEZONES: &[&str] = &[
    "Pacific/Honolulu", "America/Anchorage", "America/Los_Angeles", "America/Denver",
    "America/Chicago", "America/New_York", "America/Halifax", "America/St_Johns",
    "America/Sao_Paulo", "Atlantic/Azores", "UTC", "Europe/London", "Europe/Paris",
    "Europe/Berlin", "Europe/Stockholm", "Europe/Helsinki", "Europe/Moscow", "Asia/Dubai",
    "Asia/Kolkata", "Asia/Bangkok", "Asia/Singapore", "Asia/Shanghai", "Asia/Tokyo",
    "Asia/Seoul", "Australia/Sydney", "Pacific/Auckland", "Pacific/Fiji",
];

const DEVICE_OPTIONS: &str = include_str!("../assets/device_options.json");

fn bad_request(detail: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, detail)
}

pub(crate) fn conflict(detail: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, detail)
}

pub(crate) fn invalid(detail: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, detail)
}

fn body<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    payload.map(|Json(value)| value).map_err(|rejection| invalid(rejection.body_text()))
}

fn onboarded(conn: &Connection) -> Result<bool, ApiError> {
    Ok(db::truthy(db::get_config_value(conn, "initial_sync_done")?.as_ref()))
}

pub(crate) fn require_not_onboarded(conn: &Connection) -> Result<(), ApiError> {
    if onboarded(conn)? {
        return Err(conflict("Setup is already complete."));
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    message: String,
    setup_step: Option<&'static str>,
    mfa_required: bool,
}

pub(crate) fn action(state: &AppState, conn: &Connection, message: &str, mfa_required: bool) -> ApiResult<ActionResult> {
    let connections = view::connection_status(conn, Utc::now())?;
    Ok(Json(ActionResult {
        message: message.into(),
        setup_step: onboarding::setup_step(conn, &connections, state.auth.disabled)?,
        mfa_required,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupProgress {
    complete: bool,
    step: Option<&'static str>,
    mfa_required: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialState {
    garmin_email: String,
    garmin_password_saved: bool,
    strava_client_id: String,
    strava_client_secret_saved: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferenceState {
    display_timezone: String,
    garmin_poll_interval_minutes: i64,
    strava_poll_interval_minutes: i64,
    lookback_days: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityTypeState {
    type_key: String,
    label: String,
    autosync: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsState {
    version: String,
    release: bool,
    update: UpdateState,
    development: bool,
    setup: SetupProgress,
    connections: Connections,
    credentials: CredentialState,
    preferences: PreferenceState,
    timezones: Vec<&'static str>,
    activity_types: Vec<ActivityTypeState>,
    hevy: HevySettingsState,
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub async fn settings_state(State(state): State<AppState>) -> ApiResult<SettingsState> {
    let mfa_required = state.pending_mfa().is_some();
    let conn = state.db.lock();
    let connections = view::connection_status(&conn, Utc::now())?;
    let step = onboarding::setup_step(&conn, &connections, state.auth.disabled)?;
    let cfg = config::load_config(&conn, state.mock_mode)?;
    let garmin = db::get_config_value(&conn, "garmin_credentials")?.unwrap_or(Value::Null);
    let strava = db::get_config_value(&conn, "strava_credentials")?.unwrap_or(Value::Null);
    let held = cfg.held_activity_types();
    let types = match db::get_config_value(&conn, "garmin_activity_types")? {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    };
    let display = version::display_version();
    Ok(Json(SettingsState {
        release: display == *version::VERSION,
        version: display,
        update: UpdateState::none(),
        development: state.mock_mode,
        setup: SetupProgress { complete: step.is_none(), step, mfa_required },
        credentials: CredentialState {
            garmin_email: text(&garmin, "email"),
            garmin_password_saved: db::truthy(garmin.get("password")),
            strava_client_id: text(&strava, "client_id"),
            strava_client_secret_saved: db::truthy(strava.get("client_secret")),
        },
        preferences: PreferenceState {
            display_timezone: cfg.display_timezone(),
            garmin_poll_interval_minutes: cfg.int("garmin_poll_interval_minutes"),
            strava_poll_interval_minutes: cfg.int("strava_poll_interval_minutes"),
            lookback_days: cfg.int("lookback_days"),
        },
        timezones: COMMON_TIMEZONES.to_vec(),
        activity_types: types
            .iter()
            .map(|item| {
                let type_key = text(item, "type_key");
                ActivityTypeState { autosync: !held.contains(&type_key), label: text(item, "label"), type_key }
            })
            .collect(),
        hevy: hevy_settings::hevy_settings_state(&conn, &cfg)?,
        connections,
    }))
}

enum GarminOutcome {
    Connected,
    Mfa,
}

/// Starts a Garmin sign-in. A pending verification from an earlier attempt is
/// dropped first, so a code can only ever finish the latest attempt.
fn save_and_verify_garmin(state: &AppState, conn: &Connection, email: &str, password: &str) -> Result<GarminOutcome, ApiError> {
    state.set_pending_mfa(None);
    match mock::begin_garmin_login(email, password) {
        GarminLogin::MfaRequired => {
            state.set_pending_mfa(Some(GarminCredentials { email: email.into(), password: password.into() }));
            Ok(GarminOutcome::Mfa)
        }
        GarminLogin::Connected => {
            db::set_config_value(conn, "garmin_credentials", &json!({"email": email, "password": password}))?;
            onboarding::finalize_garmin_connect(conn, state.mock_mode, Utc::now())?;
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
    let conn = state.db.lock();
    require_not_onboarded(&conn)?;
    let (email, password) = garmin_login_fields(&conn, &payload.email, &payload.password)?;
    config::edit(&conn, state.mock_mode, |cfg| {
        cfg.set("lookback_days", json!(payload.lookback_days));
        if timefmt::is_valid_timezone(&payload.detected_timezone) {
            cfg.set("display_timezone", json!(payload.detected_timezone));
        }
    })?;
    if let GarminOutcome::Mfa = save_and_verify_garmin(&state, &conn, &email, &password)? {
        return action(&state, &conn, "Garmin sent a verification code.", true);
    }
    state.events.publish("refresh");
    action(&state, &conn, "Garmin connected.", false)
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
    let conn = state.db.lock();
    let (email, password) = garmin_login_fields(&conn, &payload.email, &payload.password)?;
    if let GarminOutcome::Mfa = save_and_verify_garmin(&state, &conn, &email, &password)? {
        return action(&state, &conn, "Garmin sent a verification code.", true);
    }
    onboarding::run_catch_up(&conn, Utc::now())?;
    state.events.publish("refresh");
    action(&state, &conn, "Garmin reconnected and catch-up sync started.", false)
}

#[derive(Deserialize)]
pub struct MfaRequest {
    code: String,
}

pub async fn complete_mfa(
    State(state): State<AppState>,
    payload: Result<Json<MfaRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let payload = body(payload)?;
    let Some(credentials) = state.pending_mfa() else {
        return Err(conflict("The Garmin verification session expired."));
    };
    mock::complete_garmin_login(&payload.code).map_err(|err| ApiError::new(StatusCode::UNAUTHORIZED, err))?;
    state.set_pending_mfa(None);
    let conn = state.db.lock();
    db::set_config_value(
        &conn,
        "garmin_credentials",
        &json!({"email": credentials.email, "password": credentials.password}),
    )?;
    onboarding::finalize_garmin_connect(&conn, state.mock_mode, Utc::now())?;
    if onboarded(&conn)? {
        onboarding::run_catch_up(&conn, Utc::now())?;
    }
    state.events.publish("refresh");
    action(&state, &conn, "Garmin verified and connected.", false)
}

pub async fn cancel_mfa(State(state): State<AppState>) -> ApiResult<ActionResult> {
    state.set_pending_mfa(None);
    action(&state, &state.db.lock(), "Garmin verification cancelled.", false)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StravaCredentialsRequest {
    client_id: String,
    #[serde(default)]
    client_secret: String,
}

pub async fn save_strava_credentials(
    State(state): State<AppState>,
    payload: Result<Json<StravaCredentialsRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let payload = body(payload)?;
    let conn = state.db.lock();
    let existing = db::get_config_value(&conn, "strava_credentials")?.unwrap_or(Value::Null);
    let client_id = payload.client_id.trim();
    let client_secret = if payload.client_secret.is_empty() {
        text(&existing, "client_secret")
    } else {
        payload.client_secret
    };
    if client_id.is_empty() || client_secret.is_empty() {
        return Err(bad_request("Enter both Strava client credentials before saving."));
    }
    onboarding::persist_strava_credentials(&conn, client_id, &client_secret)?;
    state.events.publish("refresh");
    action(&state, &conn, "Strava credentials saved. Continue to authorization.", false)
}

fn strava_callback_url(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("127.0.0.1:8383");
    format!("http://{host}/strava/callback")
}

fn new_state_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub async fn strava_authorize(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Value> {
    let conn = state.db.lock();
    let credentials = db::get_config_value(&conn, "strava_credentials")?.unwrap_or(Value::Null);
    if !db::truthy(credentials.get("client_id")) || !db::truthy(credentials.get("client_secret")) {
        return Err(conflict("Save your Strava client ID and client secret before connecting."));
    }
    let token = new_state_token();
    db::set_config_value(&conn, "strava_oauth_state", &json!(token))?;
    Ok(Json(json!({ "url": mock::strava_authorize_url(&strava_callback_url(&headers), &token) })))
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

pub async fn strava_callback(State(state): State<AppState>, Query(query): Query<StravaCallback>) -> Result<Redirect, ApiError> {
    let conn = state.db.lock();
    let expected = db::get_config_value(&conn, "strava_oauth_state")?.and_then(|v| v.as_str().map(str::to_string));
    db::set_config_value(&conn, "strava_oauth_state", &Value::Null)?;
    let landing = if onboarded(&conn)? { "/settings" } else { "/setup" };
    let fail = |message: &str| {
        let query = format!("stravaError={}", urlencode(message));
        Ok(Redirect::to(&format!("{landing}?{query}")))
    };
    if query.error.as_deref() == Some("access_denied") {
        return fail(
            "The Strava authorization was declined, so nothing was connected. \
             Press Connect and choose Authorize on Strava's page to continue.",
        );
    }
    if query.error.is_some() || query.code.as_deref().unwrap_or_default().is_empty() {
        return fail(
            "Strava did not send an authorization back. Try again and check \
             the Authorization Callback Domain if the problem continues.",
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
    db::set_config_value(&conn, "strava_tokens", &mock::strava_tokens(query.scope.as_deref(), Utc::now().timestamp()))?;
    if !view::scope_is_sufficient(query.scope.as_deref()) {
        return fail(
            "Strava didn't grant upload access. Press Connect with Strava \
             again and leave both permissions ticked.",
        );
    }
    let connections = view::connection_status(&conn, Utc::now())?;
    if onboarding::setup_step(&conn, &connections, state.auth.disabled)?.is_some() {
        state.events.publish("refresh");
        return Ok(Redirect::to("/setup"));
    }
    onboarding::run_catch_up(&conn, Utc::now())?;
    state.events.publish("refresh");
    Ok(Redirect::to("/settings"))
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

pub async fn disconnect_strava(State(state): State<AppState>) -> ApiResult<ActionResult> {
    let conn = state.db.lock();
    db::set_config_value(&conn, "strava_tokens", &Value::Null)?;
    state.events.publish("refresh");
    action(&state, &conn, "Strava disconnected. Publishing is paused.", false)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HevyCredentialsRequest {
    api_key: String,
}

/// Any key validates against the mock Hevy account. The exercise-template
/// prefetch waits for the Hevy port.
fn connect_hevy(state: &AppState, conn: &Connection, api_key: &str, in_setup: bool) -> Result<(), ApiError> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(bad_request("Enter your Hevy API key."));
    }
    db::set_config_value(conn, "hevy_api_key", &json!(api_key))?;
    db::set_config_value(conn, "hevy_auth_ok", &json!(true))?;
    if !db::truthy(db::get_config_value(conn, "hevy_events_cursor")?.as_ref()) {
        db::set_config_value(conn, "hevy_events_cursor", &json!(db::iso(Utc::now())))?;
    }
    if in_setup {
        db::set_config_value(conn, "setup_hevy_done", &json!(true))?;
        config::edit(conn, state.mock_mode, |cfg| cfg.set("hevy_enabled", json!(true)))?;
    }
    Ok(())
}

pub async fn setup_hevy(
    State(state): State<AppState>,
    payload: Result<Json<HevyCredentialsRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let payload = body(payload)?;
    let conn = state.db.lock();
    require_not_onboarded(&conn)?;
    connect_hevy(&state, &conn, &payload.api_key, true)?;
    state.events.publish("refresh");
    action(&state, &conn, "Hevy connected.", false)
}

pub async fn skip_hevy(State(state): State<AppState>) -> ApiResult<ActionResult> {
    let conn = state.db.lock();
    require_not_onboarded(&conn)?;
    db::set_config_value(&conn, "setup_hevy_done", &json!(true))?;
    state.events.publish("refresh");
    action(&state, &conn, "Hevy skipped. You can connect it later in Settings.", false)
}

pub async fn save_hevy_credentials(
    State(state): State<AppState>,
    payload: Result<Json<HevyCredentialsRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let payload = body(payload)?;
    let conn = state.db.lock();
    connect_hevy(&state, &conn, &payload.api_key, false)?;
    state.events.publish("refresh");
    action(&state, &conn, "Hevy connected.", false)
}

pub async fn disconnect_hevy(State(state): State<AppState>) -> ApiResult<ActionResult> {
    let conn = state.db.lock();
    db::set_config_value(&conn, "hevy_api_key", &Value::Null)?;
    db::set_config_value(&conn, "hevy_auth_ok", &Value::Null)?;
    config::edit(&conn, state.mock_mode, |cfg| cfg.set("hevy_enabled", json!(false)))?;
    state.events.publish("refresh");
    action(&state, &conn, "Hevy disconnected.", false)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesRequest {
    display_timezone: String,
    garmin_poll_interval_minutes: i64,
    strava_poll_interval_minutes: i64,
    lookback_days: i64,
}

pub async fn save_preferences(
    State(state): State<AppState>,
    payload: Result<Json<PreferencesRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let p = body(payload)?;
    if p.garmin_poll_interval_minutes < 1 || p.strava_poll_interval_minutes < 1 || p.lookback_days < 1 {
        return Err(invalid("Poll intervals and lookback days must be at least 1"));
    }
    if !timefmt::is_valid_timezone(&p.display_timezone) {
        return Err(bad_request(format!("Unknown timezone: {}", p.display_timezone)));
    }
    let conn = state.db.lock();
    config::edit(&conn, state.mock_mode, |cfg| {
        cfg.set("display_timezone", json!(p.display_timezone));
        cfg.set("garmin_poll_interval_minutes", json!(p.garmin_poll_interval_minutes));
        cfg.set("strava_poll_interval_minutes", json!(p.strava_poll_interval_minutes));
        cfg.set("lookback_days", json!(p.lookback_days));
    })?;
    state.events.publish("refresh");
    action(&state, &conn, "Preferences saved.", false)
}

pub(crate) fn require_garmin(conn: &Connection, detail: &str) -> Result<(), ApiError> {
    if view::connection_status(conn, Utc::now())?.garmin.connected {
        Ok(())
    } else {
        Err(conflict(detail))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityTypesRequest {
    autosync_types: Vec<String>,
}

pub async fn save_activity_types(
    State(state): State<AppState>,
    payload: Result<Json<ActivityTypesRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let payload = body(payload)?;
    let conn = state.db.lock();
    require_garmin(&conn, "Connect to Garmin before changing activity categories.")?;
    let known: std::collections::BTreeSet<String> = match db::get_config_value(&conn, "garmin_activity_types")? {
        Some(Value::Array(items)) => items.iter().map(|i| text(i, "type_key")).collect(),
        _ => Default::default(),
    };
    if payload.autosync_types.iter().any(|t| !known.contains(t)) {
        return Err(bad_request("Unknown activity category."));
    }
    let held: Vec<String> = known.into_iter().filter(|k| !payload.autosync_types.contains(k)).collect();
    config::edit(&conn, state.mock_mode, |cfg| cfg.set("held_activity_types", json!(held)))?;
    onboarding::reconcile_held_activities(&conn, &held)?;
    state.events.publish("refresh");
    action(&state, &conn, "Autosync categories saved.", false)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HevySettingsRequest {
    enabled: bool,
    watch_strategy: String,
    match_mode: String,
    #[serde(default = "default_title_template")]
    title_template: String,
    #[serde(default = "default_description_template")]
    description_template: String,
    #[serde(default = "default_true")]
    summary_on_structured: bool,
    grace_minutes: i64,
    poll_interval_minutes: i64,
    #[serde(default)]
    identity: DeviceIdentity,
    #[serde(default)]
    profile_override: ProfileOverride,
}

fn default_title_template() -> String {
    config::DEFAULT_TITLE_TEMPLATE.into()
}

fn default_description_template() -> String {
    config::DEFAULT_TEMPLATE.into()
}

fn default_true() -> bool {
    true
}

fn check_hevy_request(p: &HevySettingsRequest) -> Result<(), ApiError> {
    let chars = |s: &str| s.chars().count();
    let problems = [
        (!["replace", "merge", "describe"].contains(&p.watch_strategy.as_str()), "watchStrategy must be replace, merge, or describe"),
        (!["review", "automatic"].contains(&p.match_mode.as_str()), "matchMode must be review or automatic"),
        (!(1..=200).contains(&chars(&p.title_template)), "titleTemplate must be 1 to 200 characters"),
        (!(1..=4000).contains(&chars(&p.description_template)), "descriptionTemplate must be 1 to 4000 characters"),
        (!(0..=1440).contains(&p.grace_minutes), "graceMinutes must be between 0 and 1440"),
        (!(1..=120).contains(&p.poll_interval_minutes), "pollIntervalMinutes must be between 1 and 120"),
        (p.profile_override.sex.as_deref().is_some_and(|s| s != "male" && s != "female"), "sex must be male or female"),
    ];
    match problems.iter().find(|(bad, _)| *bad) {
        Some((_, detail)) => Err(invalid(*detail)),
        None => Ok(()),
    }
}

pub async fn save_hevy_settings(
    State(state): State<AppState>,
    payload: Result<Json<HevySettingsRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let p = body(payload)?;
    check_hevy_request(&p)?;
    hevy_settings::validate_title_template(&p.title_template).map_err(bad_request)?;
    hevy_settings::validate_description_template(&p.description_template).map_err(bad_request)?;
    let identity = &p.identity;
    let provided = [identity.manufacturer, identity.product, identity.serial].iter().filter(|v| v.is_some()).count();
    if provided != 0 && provided != 3 {
        return Err(bad_request("Device identity override needs all three numbers — or none."));
    }
    let profile = &p.profile_override;
    let mut stored_profile = serde_json::Map::new();
    if let Some(v) = profile.weight_kg {
        stored_profile.insert("weight_kg".into(), json!(v));
    }
    if let Some(v) = profile.birth_year {
        stored_profile.insert("birth_year".into(), json!(v));
    }
    if let Some(v) = profile.vo2max {
        stored_profile.insert("vo2max".into(), json!(v));
    }
    if let Some(v) = &profile.sex {
        stored_profile.insert("sex".into(), json!(v));
    }
    let conn = state.db.lock();
    config::edit(&conn, state.mock_mode, |cfg| {
        cfg.set("hevy_enabled", json!(p.enabled));
        cfg.set("hevy_watch_strategy", json!(p.watch_strategy));
        cfg.set("hevy_match_mode", json!(p.match_mode));
        cfg.set("hevy_title_template", json!(p.title_template));
        cfg.set("hevy_description_template", json!(p.description_template));
        cfg.set("hevy_summary_on_structured", json!(p.summary_on_structured));
        cfg.set("hevy_grace_minutes", json!(p.grace_minutes));
        cfg.set("hevy_poll_interval_minutes", json!(p.poll_interval_minutes));
        cfg.set(
            "hevy_device_identity_override",
            if provided == 3 {
                json!({"manufacturer": identity.manufacturer, "product": identity.product, "serial": identity.serial})
            } else {
                Value::Null
            },
        );
        if stored_profile.is_empty() {
            cfg.remove(hevy_settings::PROFILE_OVERRIDE_KEY);
        } else {
            cfg.set(hevy_settings::PROFILE_OVERRIDE_KEY, Value::Object(stored_profile));
        }
    })?;
    state.events.publish("refresh");
    action(&state, &conn, "Hevy settings saved.", false)
}

pub async fn hevy_device_options() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    ([(header::CONTENT_TYPE, "application/json")], DEVICE_OPTIONS)
}

#[derive(Deserialize)]
pub struct DevOnboardingStateRequest {
    complete: bool,
}

/// Mock only: flips onboarding so an end-to-end test can reach the wizard.
pub async fn set_dev_onboarding_state(
    State(state): State<AppState>,
    payload: Result<Json<DevOnboardingStateRequest>, JsonRejection>,
) -> ApiResult<Value> {
    if !state.mock_mode {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "Not found"));
    }
    let payload = body(payload)?;
    state.set_pending_mfa(None);
    db::set_config_value(&state.db.lock(), "initial_sync_done", &json!(payload.complete))?;
    Ok(Json(json!({"ok": true})))
}
