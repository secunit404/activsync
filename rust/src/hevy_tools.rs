use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::exercise_tables::{
    self as tables, TABLES, garmin_exercise_label, strava_exercise_label,
};
use crate::hevy_backfill::{self, Item, workout_id};
use crate::hevy_client::HevyError;
use crate::hevy_queue::{HevyWorkoutDetail, workout_detail};
use crate::routes::{ApiError, ApiResult};
use crate::view::GARMIN_ACTIVITY_URL;
use crate::{AppState, config, db, hevy_db, hevy_seed, hevy_sync, timefmt, view};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExerciseMapping {
    template_id: String,
    title: String,
    is_custom: bool,
    muscle_group: String,
    mapped: bool,
    unmapped: bool,
    suggested: bool,
    source: &'static str,
    has_standard_mapping: bool,
    standard_category: Option<i64>,
    standard_subcategory: Option<i64>,
    category: Option<i64>,
    subcategory: Option<i64>,
    category_name: Option<String>,
    subcategory_name: Option<String>,
    strava_exercise_type: Option<String>,
    strava_exercise_name: Option<String>,
    strava_source: &'static str,
    strava_standard_exercise_type: Option<String>,
}

/// Every known template with where it lands in Garmin and Strava, asking the
/// same resolvers the sync path uses so the screen cannot disagree with what
/// gets written. Unmapped first, then custom exercises, then by title.
fn mappings_view(conn: &Connection) -> rusqlite::Result<Vec<ExerciseMapping>> {
    let mut rows = Vec::new();
    for template in hevy_db::list_templates(conn)? {
        let id = Some(template.id.as_str());
        let standard = tables::standard_mapping(&template.title, id);
        let resolved = tables::lookup_exercise(conn, &template.title, id)?;
        let has_user = tables::user_mapping(conn, &template.id)?.is_some();
        let source = match (resolved.is_some(), has_user) {
            (true, true) => "user",
            (true, false) => "automatic",
            _ => "",
        };
        let suggestion = match resolved {
            None => {
                tables::suggest_mapping(&template.title, template.primary_muscle_group.as_deref())
            }
            Some(_) => None,
        };
        let pair = suggestion.or(resolved);
        let strava = tables::lookup_strava_exercise(conn, &template.title, id)?;
        let strava_default = tables::default_strava_exercise(conn, &template.title, id)?;
        rows.push(ExerciseMapping {
            mapped: resolved.is_some(),
            unmapped: resolved.is_none(),
            suggested: suggestion.is_some(),
            source,
            has_standard_mapping: standard.is_some(),
            standard_category: standard.map(|p| p.0),
            standard_subcategory: standard.map(|p| p.1),
            category: pair.map(|p| p.0),
            subcategory: pair.map(|p| p.1),
            category_name: pair
                .and_then(|(c, _)| TABLES.categories.get(&c))
                .map(|n| garmin_exercise_label(n)),
            subcategory_name: pair
                .and_then(|(c, s)| TABLES.subcategory_name(c, s))
                .map(garmin_exercise_label),
            strava_exercise_name: strava.as_ref().map(|(t, _)| strava_exercise_label(t)),
            strava_source: strava.as_ref().map_or("", |(_, s)| *s),
            strava_exercise_type: strava.map(|(t, _)| t),
            strava_standard_exercise_type: strava_default.map(|(t, _)| t),
            muscle_group: template.primary_muscle_group.unwrap_or_default(),
            is_custom: template.is_custom,
            template_id: template.id,
            title: template.title,
        });
    }
    rows.sort_by_cached_key(|r| (!r.unmapped, !r.is_custom, r.title.to_lowercase()));
    Ok(rows)
}

#[derive(Serialize)]
pub struct MappingOption {
    value: i64,
    label: String,
}

#[derive(Serialize)]
pub struct MappingCategory {
    value: i64,
    label: String,
    subcategories: Vec<MappingOption>,
}

#[derive(Serialize)]
pub struct StravaExerciseOption {
    value: String,
    label: String,
}

#[derive(Serialize)]
pub struct StravaExerciseGroup {
    label: String,
    exercises: Vec<StravaExerciseOption>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HevyToolsState {
    mappings: Vec<ExerciseMapping>,
    categories: Vec<MappingCategory>,
    strava_groups: Vec<StravaExerciseGroup>,
}

pub async fn tools_state(State(state): State<AppState>) -> ApiResult<HevyToolsState> {
    let mappings = mappings_view(&state.db.lock())?;
    let categories = TABLES
        .categories
        .iter()
        .map(|(value, name)| MappingCategory {
            value: *value,
            label: garmin_exercise_label(name),
            subcategories: TABLES
                .subcategories
                .get(value)
                .into_iter()
                .flatten()
                .map(|(sub, sub_name)| MappingOption {
                    value: *sub,
                    label: garmin_exercise_label(sub_name),
                })
                .collect(),
        })
        .collect();
    let strava_groups = TABLES
        .strava_groups
        .iter()
        .map(|(label, types)| {
            let mut exercises: Vec<StravaExerciseOption> = types
                .iter()
                .map(|t| StravaExerciseOption {
                    value: t.clone(),
                    label: strava_exercise_label(t),
                })
                .collect();
            exercises.sort_by(|a, b| a.label.cmp(&b.label));
            StravaExerciseGroup {
                label: label.clone(),
                exercises,
            }
        })
        .collect();
    Ok(Json(HevyToolsState {
        mappings,
        categories,
        strava_groups,
    }))
}

#[derive(Serialize)]
pub struct ToolActionResult {
    message: String,
}

#[derive(Deserialize)]
pub struct MappingRequest {
    category: i64,
    subcategory: i64,
}

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "Exercise template not found.")
}

fn resumed(woken: usize) -> String {
    match woken {
        0 => String::new(),
        1 => " 1 waiting workout resumed.".into(),
        n => format!(" {n} waiting workouts resumed."),
    }
}

pub async fn save_mapping(
    State(state): State<AppState>,
    Path(template_id): Path<String>,
    payload: Result<Json<MappingRequest>, JsonRejection>,
) -> ApiResult<ToolActionResult> {
    let request = payload?.0;
    if !TABLES.is_valid_pair(request.category, request.subcategory) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Unknown Garmin exercise category/subcategory pair.",
        ));
    }
    let woken = {
        let conn = state.db.lock();
        if !hevy_db::template_exists(&conn, &template_id)? {
            return Err(not_found());
        }
        hevy_db::save_mapping(&conn, &template_id, request.category, request.subcategory)?;
        start_woken(
            &state,
            &conn,
            &hevy_db::wake_needs_mapping(&conn, &template_id)?,
        )?
    };
    state.events.publish("refresh");
    Ok(Json(ToolActionResult {
        message: format!("Exercise mapping saved.{}", resumed(woken)),
    }))
}

pub async fn delete_mapping(
    State(state): State<AppState>,
    Path(template_id): Path<String>,
) -> ApiResult<ToolActionResult> {
    let message = {
        let conn = state.db.lock();
        let Some(mapping) = mappings_view(&conn)?
            .into_iter()
            .find(|m| m.template_id == template_id)
        else {
            return Err(not_found());
        };
        hevy_db::delete_mapping(&conn, &template_id)?;
        if mapping.has_standard_mapping {
            let woken = start_woken(
                &state,
                &conn,
                &hevy_db::wake_needs_mapping(&conn, &template_id)?,
            )?;
            format!("Standard mapping restored.{}", resumed(woken))
        } else {
            "Exercise mapping removed.".to_string()
        }
    };
    state.events.publish("refresh");
    Ok(Json(ToolActionResult { message }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StravaMappingRequest {
    #[serde(alias = "exercise_type")]
    exercise_type: String,
}

pub async fn save_strava_mapping(
    State(state): State<AppState>,
    Path(template_id): Path<String>,
    payload: Result<Json<StravaMappingRequest>, JsonRejection>,
) -> ApiResult<ToolActionResult> {
    let request = payload?.0;
    if !TABLES.strava_types.contains(&request.exercise_type) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Unknown Strava exercise.",
        ));
    }
    {
        let conn = state.db.lock();
        if !hevy_db::template_exists(&conn, &template_id)? {
            return Err(not_found());
        }
        hevy_db::save_strava_mapping(&conn, &template_id, &request.exercise_type)?;
    }
    state.events.publish("refresh");
    Ok(Json(ToolActionResult {
        message: "Strava exercise saved.".into(),
    }))
}

pub async fn delete_strava_mapping(
    State(state): State<AppState>,
    Path(template_id): Path<String>,
) -> ApiResult<ToolActionResult> {
    {
        let conn = state.db.lock();
        if !hevy_db::template_exists(&conn, &template_id)? {
            return Err(not_found());
        }
        hevy_db::delete_strava_mapping(&conn, &template_id)?;
    }
    state.events.publish("refresh");
    Ok(Json(ToolActionResult {
        message: "Standard Strava exercise restored.".into(),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillRequest {
    since: String,
    #[serde(default, alias = "hevy_ids")]
    hevy_ids: Option<Vec<String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillItem {
    hevy_id: String,
    title: String,
    start_time: String,
    action: String,
    twin_activity_id: Option<i64>,
    garmin_url: Option<String>,
    strava_activity_id: Option<i64>,
    strava_url: Option<String>,
    missing_template_ids: Vec<String>,
    workout: HevyWorkoutDetail,
}

#[derive(Serialize)]
pub struct BackfillResult {
    message: String,
    since: String,
    ran: bool,
    linked: usize,
    items: Vec<BackfillItem>,
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn backfill_item(item: &Item, template: Option<&str>) -> Result<BackfillItem, ApiError> {
    let workout = &item.workout;
    let text = |key: &str| workout.get(key).and_then(Value::as_str).unwrap_or_default();
    let id = text("id");
    let twin = item.twin.as_ref();
    Ok(BackfillItem {
        hevy_id: id.to_string(),
        title: [text("title"), id]
            .into_iter()
            .find(|t| !t.is_empty())
            .unwrap_or("Untitled workout")
            .to_string(),
        start_time: text("start_time").to_string(),
        action: item.action.replace(' ', "_"),
        twin_activity_id: twin.map(|t| t.garmin_activity_id),
        garmin_url: twin.map(|t| format!("{GARMIN_ACTIVITY_URL}{}", t.garmin_activity_id)),
        strava_activity_id: twin
            .filter(|t| t.publish_status == "published")
            .and_then(|t| t.strava_activity_id),
        strava_url: twin.and_then(view::strava_url_for),
        missing_template_ids: item.missing_template_ids.clone(),
        workout: workout_detail(
            workout,
            (id, text("title"), text("start_time"), text("end_time")),
            template,
        )?,
    })
}

struct Backfill {
    raw: Vec<Item>,
    items: Vec<BackfillItem>,
}

/// The workouts since the typed date, read from Hevy before the database is
/// locked, with the account total when Hevy gave one.
async fn fetch_backfill(
    state: &AppState,
    since: &str,
) -> Result<(Vec<Value>, Option<i64>), ApiError> {
    let api_key = db::get_config_value(&state.db.lock(), "hevy_api_key")?;
    let Some(api_key) = api_key.filter(|key| db::truthy(Some(key))) else {
        return Err(ApiError::conflict(
            "Connect Hevy before backfilling workouts.",
        ));
    };
    let Some(since) = timefmt::parse_timestamp(since) else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Enter the backfill start date as YYYY-MM-DD.",
        ));
    };
    let Some(client) = state.real_hevy(api_key.as_str().unwrap_or_default()) else {
        let account = hevy_seed::account_workouts(scheduler_now(state));
        let total = account.len() as i64;
        return Ok((hevy_backfill::workouts_since(account, since), Some(total)));
    };
    let fetch_failed = |err: &dyn std::fmt::Display| {
        crate::log_warn!("hevy_tools", "hevy backfill fetch failed: {err}");
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("Could not fetch workouts from Hevy: {err}"),
        )
    };
    let client = client.map_err(|err| fetch_failed(&err))?;
    let total = hevy_backfill::account_workout_count(&client).await;
    match hevy_backfill::fetch_since(&client, since, total).await {
        Ok(workouts) => Ok((workouts, total)),
        Err(HevyError::Auth) => Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Hevy rejected the saved API key. Reconnect Hevy in Settings.",
        )),
        Err(err) => Err(fetch_failed(&err)),
    }
}

fn scheduler_now(state: &AppState) -> DateTime<Utc> {
    timefmt::from_epoch(state.scheduler.now())
}

fn build_backfill(
    state: &AppState,
    conn: &Connection,
    workouts: Vec<Value>,
) -> Result<Backfill, ApiError> {
    let cfg = config::load_config(conn, state.development)?;
    let raw = hevy_backfill::preview_items(conn, workouts, &cfg, scheduler_now(state))?;
    let template = db::get_config_value(conn, "hevy_description_template")?;
    let template = template.as_ref().and_then(Value::as_str);
    let items = raw
        .iter()
        .map(|item| backfill_item(item, template))
        .collect::<Result<_, _>>()?;
    Ok(Backfill { raw, items })
}

/// Hands workouts to the Hevy pipeline now rather than at the next Hevy
/// poll; the pipeline still owns every safety gate. Real accounts only.
fn start_workouts(state: &AppState, hevy_ids: Vec<String>) {
    let (Some(clients), Some(http)) = (state.clients.clone(), state.hevy_http.clone()) else {
        return;
    };
    if hevy_ids.is_empty() {
        return;
    }
    let (db, events, development) = (state.db.clone(), state.events.clone(), state.development);
    tokio::spawn(async move {
        for hevy_id in hevy_ids {
            if let Err(err) =
                hevy_sync::process_now(&db, &events, clients.as_ref(), &http, development, &hevy_id)
                    .await
            {
                crate::log_warn!(
                    "hevy_tools",
                    "starting Hevy workout {hevy_id} failed: {err}"
                );
            }
        }
    });
}

/// Mapping saves resume parked workouts; those now waiting for their watch
/// activity start straight away.
fn start_woken(state: &AppState, conn: &Connection, woken: &[String]) -> rusqlite::Result<usize> {
    let mut waiting = Vec::new();
    for hevy_id in woken {
        if hevy_db::get_workout(conn, hevy_id)?.is_some_and(|w| w.status == "waiting_watch") {
            waiting.push(hevy_id.clone());
        }
    }
    start_workouts(state, waiting);
    Ok(woken.len())
}

pub async fn preview_backfill(
    State(state): State<AppState>,
    payload: Result<Json<BackfillRequest>, JsonRejection>,
) -> ApiResult<BackfillResult> {
    let request = payload?.0;
    let (workouts, total) = fetch_backfill(&state, &request.since).await?;
    let backfill = build_backfill(&state, &state.db.lock(), workouts)?;
    let count = backfill.items.len();
    let message = if count == 0 {
        format!("No Hevy workouts found since {}.", request.since)
    } else {
        let scale = match total.filter(|total| *total != 0) {
            Some(total) => format!(", of {total} on your Hevy account"),
            None => String::new(),
        };
        format!(
            "{count} workout{} found{scale}. Nothing has been imported yet.",
            plural(count)
        )
    };
    Ok(Json(BackfillResult {
        message,
        since: request.since,
        ran: false,
        linked: 0,
        items: backfill.items,
    }))
}

/// Imports the preview, or only the listed workouts when `hevyIds` is sent
/// (an empty list imports nothing). Ids outside the preview and workouts
/// still waiting for a mapping are never imported.
pub async fn run_backfill(
    State(state): State<AppState>,
    payload: Result<Json<BackfillRequest>, JsonRejection>,
) -> ApiResult<BackfillResult> {
    let request = payload?.0;
    let (workouts, _) = fetch_backfill(&state, &request.since).await?;
    let (backfill, selected, matched) = {
        let conn = state.db.lock();
        let backfill = build_backfill(&state, &conn, workouts)?;
        let selected: Vec<&Item> = match &request.hevy_ids {
            None => backfill.raw.iter().collect(),
            Some(ids) => backfill
                .raw
                .iter()
                .filter(|item| {
                    ids.iter().any(|id| id == workout_id(&item.workout))
                        && item.action != "needs_mapping"
                })
                .collect(),
        };
        let matched = hevy_backfill::run_items(&conn, &selected)?;
        let applying = selected
            .iter()
            .filter(|item| {
                item.twin.is_some()
                    && matches!(item.action.as_str(), "merge" | "replace" | "describe")
            })
            .map(|item| workout_id(&item.workout).to_string())
            .collect();
        start_workouts(&state, applying);
        let selected = selected.len();
        (backfill, selected, matched)
    };
    state.events.publish("refresh");
    Ok(Json(BackfillResult {
        message: format!(
            "Backfill complete: {selected} workout{} imported, {matched} matched to existing Garmin activities.",
            plural(selected)
        ),
        since: request.since,
        ran: true,
        linked: matched,
        items: backfill.items,
    }))
}
