use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use crate::db::{self, ActivityRow};
use crate::schedule;
use crate::sync::{self, Services, SyncResult};
use crate::update_check::REPO_URL;
use crate::view::{self, ActivityDetail, Connections};
use crate::{AppState, config, mock, onboarding, timefmt, version};

const PUBLISH_STATUSES: [&str; 5] = ["pending", "held", "published", "missing", "excluded"];
const REVIEW_STATUSES: [&str; 2] = ["held", "missing"];
const PAGE_SIZES: [i64; 4] = [10, 20, 50, 100];

pub struct ApiError {
    status: StatusCode,
    detail: String,
    header: Option<(HeaderName, String)>,
}

impl ApiError {
    pub fn new(status: StatusCode, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: detail.into(),
            header: None,
        }
    }

    pub fn with_header(self, name: HeaderName, value: String) -> Self {
        Self {
            header: Some((name, value)),
            ..self
        }
    }

    pub(crate) fn conflict(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, detail)
    }

    pub(crate) fn invalid(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, detail)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(json!({ "detail": self.detail }))).into_response();
        if let Some((name, value)) = self.header
            && let Ok(value) = HeaderValue::from_str(&value)
        {
            response.headers_mut().insert(name, value);
        }
        response
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(err: rusqlite::Error) -> Self {
        crate::log_error!("routes", "database error: {err}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        Self::invalid(rejection.body_text())
    }
}

pub(crate) type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupState {
    complete: bool,
    step: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HevyState {
    enabled: bool,
    connected: bool,
    status: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    latest: Option<String>,
    available: bool,
    repo_url: String,
    release_url: String,
}

impl UpdateState {
    /// What GitHub last said; nothing until a check has answered, and never
    /// with mock data, which does not check.
    pub fn of(state: &AppState) -> Self {
        let status = state.update.get();
        Self {
            available: status.as_ref().is_some_and(|s| s.available),
            latest: status.map(|s| s.latest),
            repo_url: REPO_URL.into(),
            release_url: format!("{REPO_URL}/releases/latest"),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PollIntervals {
    garmin_minutes: i64,
    strava_minutes: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateResponse {
    name: &'static str,
    version: String,
    release: bool,
    development: bool,
    setup: SetupState,
    connections: Connections,
    hevy: HevyState,
    catch_up_report: Option<Value>,
    update: UpdateState,
    review_count: i64,
    poll_intervals: PollIntervals,
}

pub async fn app_state(State(state): State<AppState>) -> ApiResult<AppStateResponse> {
    let conn = state.db.lock();
    let connections = view::connection_status(&conn, Utc::now())?;
    let step = onboarding::setup_step(&conn, &connections, state.auth.disabled)?;
    let (hevy_connected, hevy_status) = view::hevy_connection(&conn)?;
    let cfg = config::load_config(&conn, state.development)?;
    let catch_up_report = db::get_config_value(&conn, "catch_up_report")?.filter(|v| !v.is_null());
    let display = version::display_version();
    Ok(Json(AppStateResponse {
        name: "ActivSync",
        release: display == *version::VERSION,
        version: display,
        development: state.mock_mode,
        setup: SetupState {
            complete: step.is_none(),
            step,
        },
        connections,
        hevy: HevyState {
            enabled: db::truthy(cfg.get("hevy_enabled")),
            connected: hevy_connected,
            status: hevy_status,
        },
        catch_up_report,
        update: UpdateState::of(&state),
        review_count: db::count_activities(&conn, &REVIEW_STATUSES)?,
        poll_intervals: PollIntervals {
            garmin_minutes: cfg.int("garmin_poll_interval_minutes"),
            strava_minutes: cfg.int("strava_poll_interval_minutes"),
        },
    }))
}

pub async fn dismiss_catch_up_report(
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    db::set_config_value(&state.db.lock(), "catch_up_report", &Value::Null)?;
    state.events.publish("refresh");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ActivitiesQuery {
    sort: Option<String>,
    status: Option<String>,
    page: Option<String>,
    #[serde(rename = "pageSize")]
    page_size: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    garmin_activity_id: i64,
    activity_type: String,
    title: String,
    description: String,
    start_time: String,
    publish_status: String,
    strava_activity_id: Option<i64>,
    hold_reason: Option<String>,
    start_date_display: String,
    start_month_year_display: String,
    start_clock_display: String,
    garmin_url: String,
    strava_url: Option<String>,
    hevy_badge: Option<String>,
    detail: ActivityDetail,
    publish_scheduled_at: Option<String>,
    publish_phase: Option<String>,
}

#[derive(Serialize, Default)]
pub struct StatusCounts {
    pending: i64,
    held: i64,
    published: i64,
    missing: i64,
    excluded: i64,
}

impl StatusCounts {
    fn add(&mut self, status: &str) {
        match status {
            "pending" => self.pending += 1,
            "held" => self.held += 1,
            "published" => self.published += 1,
            "missing" => self.missing += 1,
            "excluded" => self.excluded += 1,
            _ => {}
        }
    }
}

#[derive(Serialize)]
pub struct WeekTotal {
    seconds: i64,
    display: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationState {
    page: i64,
    page_size: i64,
    page_count: i64,
    total_count: i64,
    first_item: i64,
    last_item: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitiesPage {
    items: Vec<Activity>,
    sort: String,
    status: Option<String>,
    counts: StatusCounts,
    week_total: WeekTotal,
    pagination: PaginationState,
}

/// An integer as Python reads it: one beyond 64 bits keeps its sign and
/// saturates, so a huge `page` clamps to the last page instead of failing.
fn saturating_int(text: &str) -> Option<i64> {
    let text = text.trim();
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(text.parse().unwrap_or(if text.starts_with('-') {
        i64::MIN
    } else {
        i64::MAX
    }))
}

fn parse_int(raw: Option<&str>, default: i64, name: &str) -> Result<i64, ApiError> {
    match raw {
        None => Ok(default),
        Some(text) => saturating_int(text)
            .ok_or_else(|| ApiError::invalid(format!("{name} must be an integer"))),
    }
}

/// The activity id in the path; anything but an integer is a JSON 422 like
/// the rest of the API's errors, and one past 64 bits names no activity.
pub struct ActivityId(pub i64);

impl<S: Send + Sync> FromRequestParts<S> for ActivityId {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let Path(raw) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::new(rejection.status(), rejection.body_text()))?;
        saturating_int(&raw).map(Self).ok_or_else(|| {
            ApiError::invalid(
                "Input should be a valid integer, unable to parse string as an integer",
            )
        })
    }
}

pub async fn method_not_allowed() -> ApiError {
    ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed")
}

pub(crate) fn internal(context: &str, err: impl std::fmt::Display) -> ApiError {
    crate::log_error!("routes", "{context}: {err}");
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
}

pub async fn activities_page(
    State(state): State<AppState>,
    Query(query): Query<ActivitiesQuery>,
) -> ApiResult<ActivitiesPage> {
    let sort = query.sort.unwrap_or_else(|| "newest".into());
    if sort != "newest" && sort != "oldest" {
        return Err(ApiError::invalid("sort must be newest or oldest"));
    }
    let status = query.status;
    if let Some(status) = &status
        && status != "review"
        && !PUBLISH_STATUSES.contains(&status.as_str())
    {
        return Err(ApiError::invalid(
            "status must be one of pending, held, published, missing, excluded, or review",
        ));
    }
    let page = parse_int(query.page.as_deref(), 1, "page")?;
    if page < 1 {
        return Err(ApiError::invalid("page must be at least 1"));
    }
    let page_size = parse_int(query.page_size.as_deref(), 20, "pageSize")?;
    if !PAGE_SIZES.contains(&page_size) {
        return Err(ApiError::invalid(
            "pageSize must be one of 10, 20, 50, or 100",
        ));
    }

    let conn = state.db.lock();
    let tz_name = config::load_config(&conn, state.development)?.display_timezone();
    let tz = timefmt::parse_tz(&tz_name);
    let all = view::activities_view(&conn, sort == "oldest", &tz_name)?;
    let (week_start, week_end) = timefmt::current_week(Utc::now(), tz);

    let mut counts = StatusCounts::default();
    let mut week_seconds = 0.0;
    for activity in &all {
        counts.add(&activity.row.publish_status);
        if activity.row.publish_status == "excluded" {
            continue;
        }
        if let Some(local) = timefmt::to_local(&activity.row.start_time, tz)
            && week_start <= local
            && local < week_end
        {
            week_seconds += activity.duration_seconds.unwrap_or_default();
        }
    }

    let filtered: Vec<_> = all
        .into_iter()
        .filter(|a| match status.as_deref() {
            None => true,
            Some("review") => REVIEW_STATUSES.contains(&a.row.publish_status.as_str()),
            Some(wanted) => a.row.publish_status == wanted,
        })
        .collect();
    let total_count = filtered.len() as i64;
    let page_count = ((total_count + page_size - 1) / page_size).max(1);
    let current_page = page.min(page_count);
    let first_index = (current_page - 1) * page_size;
    let scheduled = schedule::active_by_activity(&conn)?;

    let items: Vec<Activity> = filtered
        .into_iter()
        .skip(first_index as usize)
        .take(page_size as usize)
        .map(|a| {
            let entry = scheduled.get(&a.row.garmin_activity_id);
            Activity {
                strava_url: view::strava_url_for(&a.row),
                publish_scheduled_at: entry.map(|(deadline, _)| schedule::deadline_iso(*deadline)),
                publish_phase: entry.map(|(_, phase)| phase.clone()),
                garmin_activity_id: a.row.garmin_activity_id,
                activity_type: a.row.activity_type,
                title: a.row.title,
                description: a.row.description,
                start_time: a.row.start_time,
                publish_status: a.row.publish_status,
                strava_activity_id: a.row.strava_activity_id,
                hold_reason: a.row.hold_reason,
                start_date_display: a.start_date_display,
                start_month_year_display: a.start_month_year_display,
                start_clock_display: a.start_clock_display,
                garmin_url: a.garmin_url,
                hevy_badge: a.hevy_badge,
                detail: a.detail,
            }
        })
        .collect();
    let item_count = items.len() as i64;
    Ok(Json(ActivitiesPage {
        items,
        sort,
        status,
        counts,
        week_total: WeekTotal {
            seconds: week_seconds.round_ties_even() as i64,
            display: view::fmt_duration_coarse(week_seconds),
        },
        pagination: PaginationState {
            page: current_page,
            page_size,
            page_count,
            total_count,
            first_item: if total_count > 0 { first_index + 1 } else { 0 },
            last_item: first_index + item_count,
        },
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    pub(crate) message: String,
    pub(crate) severity: &'static str,
    pub(crate) published_count: i64,
    pub(crate) failed_count: i64,
    pub(crate) blocked_count: i64,
}

impl ActionResult {
    pub(crate) fn success(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            severity: "success",
            published_count: 0,
            failed_count: 0,
            blocked_count: 0,
        }
    }
}

pub(crate) fn require_activity(
    conn: &rusqlite::Connection,
    id: i64,
) -> Result<ActivityRow, ApiError> {
    db::get_activity(conn, id)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Activity was not found"))
}

pub(crate) fn is_publishable(row: &ActivityRow) -> bool {
    schedule::PUBLISHABLE_STATUSES.contains(&row.publish_status.as_str())
}

pub(crate) fn require_publish_connections(conn: &rusqlite::Connection) -> Result<(), ApiError> {
    let connections = view::connection_status(conn, Utc::now())?;
    if connections.garmin.connected && connections.strava.connected {
        return Ok(());
    }
    Err(ApiError::conflict(
        "Both Garmin and Strava must be connected to publish. Reconnect to continue.",
    ))
}

#[derive(Deserialize)]
pub struct EditActivityRequest {
    title: String,
    #[serde(default)]
    description: String,
}

/// SHA-256 over the same NUL-joined fields as the Python app, so a row edited
/// by either server hashes the same.
pub fn content_hash(
    title: &str,
    description: &str,
    activity_type: &str,
    garmin_data: &str,
) -> String {
    crate::auth_core::sha256_hex(
        format!("{title}\0{description}\0{activity_type}\0{garmin_data}").as_bytes(),
    )
}

async fn save_metadata(
    state: &AppState,
    id: i64,
    title: &str,
    description: &str,
) -> SyncResult<()> {
    let Some(clients) = state.clients.clone() else {
        return mock::edit_activity_metadata(&state.db, id, title, description);
    };
    let garmin = clients.garmin().await?;
    let strava = clients.strava();
    sync::edit_activity_metadata(
        &Services {
            db: &state.db,
            garmin: &garmin,
            strava: &strava,
        },
        id,
        title,
        description,
    )
    .await
}

pub async fn edit_activity(
    State(state): State<AppState>,
    ActivityId(id): ActivityId,
    payload: Result<Json<EditActivityRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let Json(payload) = payload?;
    let activity = require_activity(&state.db.lock(), id)?;
    if payload.title.trim().is_empty() {
        return Err(ApiError::invalid("Activity title cannot be blank"));
    }
    save_metadata(&state, id, &payload.title, &payload.description)
        .await
        .map_err(|err| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                format!("Could not save activity: {err}"),
            )
        })?;
    state.events.publish("refresh");
    Ok(Json(ActionResult::success(format!(
        "Saved changes to {}",
        activity.title
    ))))
}

pub async fn exclude_activity(
    State(state): State<AppState>,
    ActivityId(id): ActivityId,
) -> ApiResult<ActionResult> {
    let conn = state.db.lock();
    let activity = require_activity(&conn, id)?;
    if !is_publishable(&activity) {
        return Err(ApiError::conflict(
            "Only pending, held, or missing activities can be excluded",
        ));
    }
    db::set_publish_status(&conn, id, "excluded", None)?;
    drop(conn);
    state.events.publish("refresh");
    Ok(Json(ActionResult::success(format!(
        "Excluded {}",
        activity.title
    ))))
}

fn present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreActivityRequest {
    status: String,
    #[serde(default, deserialize_with = "present")]
    hold_reason: Option<Option<String>>,
    #[serde(default, rename = "hold_reason", deserialize_with = "present")]
    hold_reason_snake: Option<Option<String>>,
}

impl RestoreActivityRequest {
    /// The camelCase key wins when both are sent, as pydantic reads an alias.
    fn hold_reason(&self) -> Option<&str> {
        self.hold_reason
            .as_ref()
            .or(self.hold_reason_snake.as_ref())
            .and_then(|r| r.as_deref())
    }
}

pub async fn restore_activity(
    State(state): State<AppState>,
    ActivityId(id): ActivityId,
    body: axum::body::Bytes,
) -> ApiResult<ActionResult> {
    let payload: Option<RestoreActivityRequest> = if body.iter().all(u8::is_ascii_whitespace) {
        None
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(Value::Null) => None,
            Ok(value) => {
                Some(serde_json::from_value(value).map_err(|e| ApiError::invalid(e.to_string()))?)
            }
            Err(err) => return Err(ApiError::invalid(err.to_string())),
        }
    };
    if let Some(p) = &payload {
        if !["pending", "held", "missing"].contains(&p.status.as_str()) {
            return Err(ApiError::invalid(
                "status must be pending, held, or missing",
            ));
        }
        if p.hold_reason()
            .is_some_and(|r| r != "category" && r != "backlog")
        {
            return Err(ApiError::invalid("holdReason must be category or backlog"));
        }
    }
    let conn = state.db.lock();
    let activity = require_activity(&conn, id)?;
    if activity.publish_status != "excluded" {
        return Err(ApiError::conflict("Activity is not excluded"));
    }
    match payload {
        Some(p) => db::set_publish_status(&conn, id, &p.status, p.hold_reason())?,
        None => {
            let held = config::load_config(&conn, state.development)?.held_activity_types();
            if held.contains(&activity.activity_type) {
                db::set_publish_status(&conn, id, "held", Some("category"))?;
            } else {
                db::set_publish_status(&conn, id, "pending", None)?;
            }
        }
    }
    drop(conn);
    state.events.publish("refresh");
    Ok(Json(ActionResult::success(format!(
        "Restored {}",
        activity.title
    ))))
}

pub fn icon(png: &'static [u8]) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/png")], png)
}

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

pub async fn sse_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(state.events.subscribe())
        .filter_map(|event| event.ok())
        // Not `data("")`: axum then omits the data line, and browsers drop
        // an event that has none, so `refresh` would never fire.
        .map(|name| Ok(Event::default().event(name).data(" ")));
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

pub async fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "Not Found")
}
