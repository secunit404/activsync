use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::{StatusCode, header};
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::db;
use crate::hevy_client::{HevyClient, HevyError};
use crate::hevy_settings::{self, DeviceIdentity, HevySettingsState, ProfileOverride};
use crate::{hevy_sync, onboarding};
use crate::routes::{ApiError, UpdateState, development};
use crate::view::{self, Connections};
use crate::{AppState, config, mock, timefmt, version};

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

pub(crate) fn bad_request(detail: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, detail)
}

pub(crate) fn conflict(detail: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, detail)
}

pub(crate) fn invalid(detail: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, detail)
}

pub(crate) fn body<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    payload.map(|Json(value)| value).map_err(|rejection| invalid(rejection.body_text()))
}

pub(crate) fn onboarded(conn: &Connection) -> Result<bool, ApiError> {
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
    undo_delay_seconds: i64,
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

pub(crate) fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub async fn settings_state(State(state): State<AppState>) -> ApiResult<SettingsState> {
    let mfa_required = state.pending_mfa().is_some();
    let conn = state.db.lock();
    let connections = view::connection_status(&conn, Utc::now())?;
    let step = onboarding::setup_step(&conn, &connections, state.auth.disabled)?;
    let cfg = config::load_config(&conn, development(&state))?;
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
        update: UpdateState::of(&state),
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
            undo_delay_seconds: cfg.int("undo_delay_seconds"),
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

pub async fn disconnect_strava(State(state): State<AppState>) -> ApiResult<ActionResult> {
    match &state.clients {
        Some(clients) => clients.strava().disconnect().await,
        None => db::set_config_value(&state.db.lock(), "strava_tokens", &Value::Null)?,
    }
    state.events.publish("refresh");
    action(&state, &state.db.lock(), "Strava disconnected. Publishing is paused.", false)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HevyCredentialsRequest {
    api_key: String,
}

/// Asks Hevy whether the key works before anything is saved. The mock
/// account takes any key.
async fn validate_hevy_key(state: &AppState, api_key: &str) -> Result<Option<HevyClient>, ApiError> {
    let Some(client) = state.real_hevy(api_key) else {
        return Ok(None);
    };
    let unreachable = |err: &dyn std::fmt::Display| {
        ApiError::new(StatusCode::BAD_GATEWAY, format!("Could not reach Hevy to validate the key: {err}"))
    };
    let client = client.map_err(|err| unreachable(&err))?;
    match client.get_user_info().await {
        Ok(_) => Ok(Some(client)),
        Err(HevyError::Auth) => {
            Err(ApiError::new(StatusCode::UNAUTHORIZED, "Hevy rejected that API key — check it and try again."))
        }
        Err(err) => Err(unreachable(&err)),
    }
}

/// Best effort: caching every exercise template now fills the mappings page
/// before the first workout arrives. Nothing is written unless every page
/// was fetched.
async fn prefetch_templates(state: &AppState, hevy: Option<&HevyClient>) {
    let templates = match hevy {
        Some(hevy) => match hevy.iter_all_exercise_templates().await {
            Ok(templates) => templates,
            Err(err) => return crate::log_warn!("settings_routes", "Hevy template prefetch after connect failed: {err}"),
        },
        None => mock::hevy_templates(),
    };
    let conn = state.db.lock();
    for template in &templates {
        let Some(id) = template.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()) else {
            continue;
        };
        if let Err(err) = hevy_sync::store_template(&conn, template, id, "") {
            return crate::log_warn!("settings_routes", "Hevy template prefetch after connect failed: {err}");
        }
    }
}

async fn connect_hevy(state: &AppState, api_key: &str, in_setup: bool) -> Result<(), ApiError> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(bad_request("Enter your Hevy API key."));
    }
    let hevy = validate_hevy_key(state, api_key).await?;
    {
        let conn = state.db.lock();
        db::set_config_value(&conn, "hevy_api_key", &json!(api_key))?;
        db::set_config_value(&conn, "hevy_auth_ok", &json!(true))?;
        if !db::truthy(db::get_config_value(&conn, "hevy_events_cursor")?.as_ref()) {
            db::set_config_value(&conn, "hevy_events_cursor", &json!(db::iso(Utc::now())))?;
        }
    }
    prefetch_templates(state, hevy.as_ref()).await;
    if in_setup {
        let conn = state.db.lock();
        db::set_config_value(&conn, "setup_hevy_done", &json!(true))?;
        config::edit(&conn, development(&state), |cfg| cfg.set("hevy_enabled", json!(true)))?;
    }
    Ok(())
}

pub async fn setup_hevy(
    State(state): State<AppState>,
    payload: Result<Json<HevyCredentialsRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let payload = body(payload)?;
    require_not_onboarded(&state.db.lock())?;
    connect_hevy(&state, &payload.api_key, true).await?;
    state.events.publish("refresh");
    action(&state, &state.db.lock(), "Hevy connected.", false)
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
    connect_hevy(&state, &payload.api_key, false).await?;
    state.events.publish("refresh");
    action(&state, &state.db.lock(), "Hevy connected.", false)
}

pub async fn disconnect_hevy(State(state): State<AppState>) -> ApiResult<ActionResult> {
    let conn = state.db.lock();
    db::set_config_value(&conn, "hevy_api_key", &Value::Null)?;
    db::set_config_value(&conn, "hevy_auth_ok", &Value::Null)?;
    config::edit(&conn, development(&state), |cfg| cfg.set("hevy_enabled", json!(false)))?;
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
    undo_delay_seconds: i64,
}

const UNDO_DELAYS: [i64; 4] = [0, 3, 5, 10];

pub async fn save_preferences(
    State(state): State<AppState>,
    payload: Result<Json<PreferencesRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let p = body(payload)?;
    if p.garmin_poll_interval_minutes < 1 || p.strava_poll_interval_minutes < 1 || p.lookback_days < 1 {
        return Err(invalid("Poll intervals and lookback days must be at least 1"));
    }
    if !UNDO_DELAYS.contains(&p.undo_delay_seconds) {
        return Err(invalid("undoDelaySeconds must be one of 0, 3, 5 or 10"));
    }
    if !timefmt::is_valid_timezone(&p.display_timezone) {
        return Err(bad_request(format!("Unknown timezone: {}", p.display_timezone)));
    }
    let conn = state.db.lock();
    config::edit(&conn, development(&state), |cfg| {
        cfg.set("display_timezone", json!(p.display_timezone));
        cfg.set("garmin_poll_interval_minutes", json!(p.garmin_poll_interval_minutes));
        cfg.set("strava_poll_interval_minutes", json!(p.strava_poll_interval_minutes));
        cfg.set("lookback_days", json!(p.lookback_days));
        cfg.set("undo_delay_seconds", json!(p.undo_delay_seconds));
    })?;
    crate::logging::set_timezone(&p.display_timezone);
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
    config::edit(&conn, development(&state), |cfg| cfg.set("held_activity_types", json!(held)))?;
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
    config::edit(&conn, development(&state), |cfg| {
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
