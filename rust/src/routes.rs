use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::db::{self, ActivityRow};
use crate::schedule::{self, ScheduleError};
use crate::view::{self, ActivityDetail, Connections};
use crate::{AppState, config, onboarding, timefmt, version};

const PUBLISH_STATUSES: [&str; 5] = ["pending", "held", "published", "missing", "excluded"];
const REVIEW_STATUSES: [&str; 2] = ["held", "missing"];
const PAGE_SIZES: [i64; 4] = [10, 20, 50, 100];
const REPO_URL: &str = "https://github.com/secunit404/activsync";

pub struct ApiError {
    status: StatusCode,
    detail: String,
    header: Option<(axum::http::HeaderName, String)>,
}

impl ApiError {
    pub fn new(status: StatusCode, detail: impl Into<String>) -> Self {
        Self { status, detail: detail.into(), header: None }
    }

    pub fn with_header(self, name: axum::http::HeaderName, value: String) -> Self {
        Self { header: Some((name, value)), ..self }
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
        if let Some((name, value)) = self.header {
            if let Ok(value) = axum::http::HeaderValue::from_str(&value) {
                response.headers_mut().insert(name, value);
            }
        }
        response
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(err: rusqlite::Error) -> Self {
        eprintln!("database error: {err}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        Self::invalid(rejection.body_text())
    }
}

pub(crate) type ApiResult<T> = Result<Json<T>, ApiError>;

pub(crate) fn development(state: &AppState) -> bool {
    state.mock_mode
}

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
    /// Mock mode never asks GitHub for releases.
    pub fn none() -> Self {
        Self {
            latest: None,
            available: false,
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
    let cfg = config::load_config(&conn, development(&state))?;
    let catch_up_report = db::get_config_value(&conn, "catch_up_report")?.filter(|v| !v.is_null());
    let display = version::display_version();
    Ok(Json(AppStateResponse {
        name: "ActivSync",
        release: display == *version::VERSION,
        version: display,
        development: development(&state),
        setup: SetupState { complete: step.is_none(), step },
        connections,
        hevy: HevyState {
            enabled: db::truthy(cfg.get("hevy_enabled")),
            connected: hevy_connected,
            status: hevy_status,
        },
        catch_up_report,
        update: UpdateState::none(),
        review_count: db::count_activities(&conn, &REVIEW_STATUSES)?,
        poll_intervals: PollIntervals {
            garmin_minutes: cfg.int("garmin_poll_interval_minutes"),
            strava_minutes: cfg.int("strava_poll_interval_minutes"),
        },
    }))
}

/// Single sign-on is not ported yet; the Access section shows it unconfigured.
pub async fn oidc_config(headers: axum::http::HeaderMap) -> Json<Value> {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("127.0.0.1:8383");
    Json(json!({
        "configured": false,
        "displayName": "",
        "issuer": "",
        "clientId": "",
        "hasSecret": false,
        "scopes": "openid profile email",
        "autoRedirect": false,
        "redirectUri": format!("http://{host}/auth/oidc/callback"),
        "linkedLabel": null,
        "logoUrl": null,
    }))
}

pub async fn dismiss_catch_up_report(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
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

fn parse_int(raw: Option<&str>, default: i64, name: &str) -> Result<i64, ApiError> {
    match raw {
        None => Ok(default),
        Some(text) => text
            .trim()
            .parse()
            .map_err(|_| ApiError::invalid(format!("{name} must be an integer"))),
    }
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
    if let Some(status) = &status {
        if status != "review" && !PUBLISH_STATUSES.contains(&status.as_str()) {
            return Err(ApiError::invalid(
                "status must be one of pending, held, published, missing, excluded, or review",
            ));
        }
    }
    let page = parse_int(query.page.as_deref(), 1, "page")?;
    if page < 1 {
        return Err(ApiError::invalid("page must be at least 1"));
    }
    let page_size = parse_int(query.page_size.as_deref(), 20, "pageSize")?;
    if !PAGE_SIZES.contains(&page_size) {
        return Err(ApiError::invalid("pageSize must be one of 10, 20, 50, or 100"));
    }

    let conn = state.db.lock();
    let tz_name = config::load_config(&conn, development(&state))?.display_timezone();
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
        if let Some(local) = timefmt::to_local(&activity.row.start_time, tz) {
            if week_start <= local && local < week_end {
                week_seconds += activity.duration_seconds.unwrap_or_default();
            }
        }
    }

    let wanted: Vec<&str> = match status.as_deref() {
        Some("review") => REVIEW_STATUSES.to_vec(),
        Some(other) => vec![other],
        None => PUBLISH_STATUSES.to_vec(),
    };
    let filtered: Vec<_> = all
        .into_iter()
        .filter(|a| status.is_none() || wanted.contains(&a.row.publish_status.as_str()))
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
    message: String,
    severity: &'static str,
    published_count: i64,
    failed_count: i64,
    blocked_count: i64,
}

impl ActionResult {
    fn success(message: impl Into<String>) -> Self {
        Self { message: message.into(), severity: "success", published_count: 0, failed_count: 0, blocked_count: 0 }
    }
}

fn require_activity(conn: &rusqlite::Connection, id: i64) -> Result<ActivityRow, ApiError> {
    db::get_activity(conn, id)?.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Activity was not found"))
}

fn is_publishable(row: &ActivityRow) -> bool {
    schedule::PUBLISHABLE_STATUSES.contains(&row.publish_status.as_str())
}

fn require_publish_connections(conn: &rusqlite::Connection) -> Result<(), ApiError> {
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
pub fn content_hash(title: &str, description: &str, activity_type: &str, garmin_data: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{title}\0{description}\0{activity_type}\0{garmin_data}"));
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub async fn edit_activity(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    payload: Result<Json<EditActivityRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let Json(payload) = payload?;
    let conn = state.db.lock();
    let activity = require_activity(&conn, id)?;
    let title = payload.title.trim();
    if title.is_empty() {
        return Err(ApiError::invalid("Activity title cannot be blank"));
    }
    let description = payload.description.trim();
    let hash = content_hash(title, description, &activity.activity_type, &activity.garmin_data);
    db::update_activity_metadata(&conn, id, title, description, &hash)?;
    drop(conn);
    state.events.publish("refresh");
    Ok(Json(ActionResult::success(format!("Saved changes to {}", activity.title))))
}

pub async fn exclude_activity(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<ActionResult> {
    let conn = state.db.lock();
    let activity = require_activity(&conn, id)?;
    if !is_publishable(&activity) {
        return Err(ApiError::conflict("Only pending, held, or missing activities can be excluded"));
    }
    db::set_publish_status(&conn, id, "excluded", None)?;
    drop(conn);
    state.events.publish("refresh");
    Ok(Json(ActionResult::success(format!("Excluded {}", activity.title))))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreActivityRequest {
    status: String,
    #[serde(default)]
    hold_reason: Option<String>,
}

pub async fn restore_activity(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    body: axum::body::Bytes,
) -> ApiResult<ActionResult> {
    let payload: Option<RestoreActivityRequest> = if body.iter().all(u8::is_ascii_whitespace) {
        None
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(Value::Null) => None,
            Ok(value) => Some(serde_json::from_value(value).map_err(|e| ApiError::invalid(e.to_string()))?),
            Err(err) => return Err(ApiError::invalid(err.to_string())),
        }
    };
    if let Some(p) = &payload {
        if !["pending", "held", "missing"].contains(&p.status.as_str()) {
            return Err(ApiError::invalid("status must be pending, held, or missing"));
        }
        if p.hold_reason.as_deref().is_some_and(|r| r != "category" && r != "backlog") {
            return Err(ApiError::invalid("holdReason must be category or backlog"));
        }
    }
    let conn = state.db.lock();
    let activity = require_activity(&conn, id)?;
    if activity.publish_status != "excluded" {
        return Err(ApiError::conflict("Activity is not excluded"));
    }
    match payload {
        Some(p) => db::set_publish_status(&conn, id, &p.status, p.hold_reason.as_deref())?,
        None => {
            let held = config::load_config(&conn, development(&state))?.held_activity_types();
            if held.contains(&activity.activity_type) {
                db::set_publish_status(&conn, id, "held", Some("category"))?;
            } else {
                db::set_publish_status(&conn, id, "pending", None)?;
            }
        }
    }
    drop(conn);
    state.events.publish("refresh");
    Ok(Json(ActionResult::success(format!("Restored {}", activity.title))))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishScheduled {
    batch_id: String,
    publish_scheduled_at: String,
    remaining_ms: i64,
    undo_window_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityIdsRequest {
    activity_ids: Vec<i64>,
}

fn unique_ids(payload: ActivityIdsRequest) -> Result<Vec<i64>, ApiError> {
    if payload.activity_ids.is_empty() {
        return Err(ApiError::invalid("activityIds must list at least one activity"));
    }
    Ok(payload.activity_ids.into_iter().collect::<BTreeSet<_>>().into_iter().collect())
}

fn schedule_publish(state: &AppState, ids: &[i64], kind: &str) -> ApiResult<PublishScheduled> {
    let activities = {
        let conn = state.db.lock();
        let activities = ids.iter().map(|id| require_activity(&conn, *id)).collect::<Result<Vec<_>, _>>()?;
        if !activities.iter().all(is_publishable) {
            return Err(ApiError::conflict(if kind == "single" {
                "Only pending, held, or missing activities can be published"
            } else {
                "Some selected activities are no longer publishable. Refresh and try again."
            }));
        }
        require_publish_connections(&conn)?;
        activities
    };
    let batch = state.scheduler.schedule(&activities, kind).map_err(|err| match err {
        ScheduleError::AlreadyScheduled => {
            ApiError::conflict("Already publishing to Strava. Refresh to see where it stands.")
        }
        ScheduleError::Db(err) => err.into(),
    })?;
    let remaining = batch.deadline - state.scheduler.now();
    Ok(Json(PublishScheduled {
        publish_scheduled_at: schedule::deadline_iso(batch.deadline),
        batch_id: batch.batch_id,
        remaining_ms: (remaining * 1000.0).round().max(0.0) as i64,
        undo_window_ms: (state.scheduler.window_seconds * 1000.0).round() as i64,
    }))
}

pub async fn schedule_publish_activity(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<PublishScheduled> {
    schedule_publish(&state, &[id], "single")
}

pub async fn schedule_publish_activities(
    State(state): State<AppState>,
    payload: Result<Json<ActivityIdsRequest>, JsonRejection>,
) -> ApiResult<PublishScheduled> {
    let ids = unique_ids(payload?.0)?;
    schedule_publish(&state, &ids, "bulk")
}

fn cancel_publish(state: &AppState, ids: &[i64]) -> ApiResult<ActionResult> {
    if state.scheduler.cancel(ids)? > 0 {
        return Ok(Json(ActionResult::success("Nothing was sent to Strava.")));
    }
    let conn = state.db.lock();
    for id in ids {
        if schedule::get(&conn, *id)?.is_some() {
            return Err(ApiError::conflict("Too late to undo: it is already being sent to Strava."));
        }
    }
    Err(ApiError::conflict("Nothing is waiting to publish."))
}

pub async fn cancel_publish_activity(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<ActionResult> {
    cancel_publish(&state, &[id])
}

pub async fn cancel_publish_activities(
    State(state): State<AppState>,
    payload: Result<Json<ActivityIdsRequest>, JsonRejection>,
) -> ApiResult<ActionResult> {
    let ids = unique_ids(payload?.0)?;
    cancel_publish(&state, &ids)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishBatch {
    batch_id: String,
    state: &'static str,
    remaining_ms: i64,
    result: Option<ActionResult>,
    error: Option<String>,
}

/// The same toast an immediate publish would return, built from what the
/// scheduler recorded.
fn batch_outcome(rows: &[schedule::ScheduleRow], titles: &HashMap<i64, String>) -> (Option<ActionResult>, Option<String>) {
    if rows[0].batch_kind == "single" {
        let row = &rows[0];
        let title = titles.get(&row.garmin_activity_id).map_or("Activity", String::as_str);
        return match row.state.as_str() {
            schedule::PUBLISHED => {
                let verb = if row.prior_status == "missing" { "Republished" } else { "Published" };
                (Some(ActionResult::success(format!("{verb} {title} to Strava"))), None)
            }
            schedule::SKIPPED => (None, Some(format!("{title} was no longer publishable, so nothing was sent."))),
            _ => (None, Some(row.message.clone().unwrap_or_else(|| "Could not publish activity".into()))),
        };
    }
    let count = |state: &str| rows.iter().filter(|r| r.state == state).count() as i64;
    let (published, failed, blocked, skipped) =
        (count(schedule::PUBLISHED), count(schedule::FAILED), count(schedule::BLOCKED), count(schedule::SKIPPED));
    if failed + blocked + skipped > 0 {
        let mut parts = vec![format!("Published {published}")];
        if failed > 0 {
            parts.push(format!("{failed} failed and stayed ready to retry"));
        }
        if blocked > 0 {
            parts.push(format!("{blocked} blocked by Hevy sync"));
        }
        if skipped > 0 {
            parts.push(format!("{skipped} no longer publishable"));
        }
        return (
            Some(ActionResult {
                message: parts.join("; "),
                severity: "warning",
                published_count: published,
                failed_count: failed,
                blocked_count: blocked,
            }),
            None,
        );
    }
    let noun = if published == 1 { "activity" } else { "activities" };
    (
        Some(ActionResult {
            published_count: published,
            ..ActionResult::success(format!("Published {published} {noun} to Strava"))
        }),
        None,
    )
}

pub async fn publish_batch(State(state): State<AppState>, Path(batch_id): Path<String>) -> ApiResult<PublishBatch> {
    let conn = state.db.lock();
    let rows = schedule::batch_rows(&conn, &batch_id)?;
    if rows.is_empty() {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "Nothing is publishing under that id."));
    }
    let remaining_ms = ((rows[0].deadline - state.scheduler.now()) * 1000.0).round().max(0.0) as i64;
    let pending = |s: &str| rows.iter().any(|r| r.state == s);
    let (batch_state, remaining_ms) = if pending(schedule::WAITING) {
        ("waiting", remaining_ms)
    } else if pending(schedule::SENDING) {
        ("sending", 0)
    } else {
        ("done", 0)
    };
    let (result, error) = if batch_state == "done" {
        let mut titles = HashMap::new();
        for row in &rows {
            if let Some(activity) = db::get_activity(&conn, row.garmin_activity_id)? {
                titles.insert(row.garmin_activity_id, activity.title);
            }
        }
        batch_outcome(&rows, &titles)
    } else {
        (None, None)
    };
    Ok(Json(PublishBatch { batch_id, state: batch_state, remaining_ms, result, error }))
}

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

pub async fn sse_events(State(state): State<AppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(state.events.subscribe())
        .filter_map(|event| event.ok())
        .map(|name| Ok(Event::default().event(name).data("")));
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("keepalive"))
}

pub async fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "Not Found")
}
