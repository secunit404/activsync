use std::sync::Arc;

use activsync::auth_guard::AuthConfig;
use activsync::db::Db;
use activsync::hevy_db::{self, Template};
use activsync::{AppState, build_app, hevy_seed, schedule};
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const PYTHON_TOOLS: &str = include_str!("fixtures/hevy_tools_python.json");
const PYTHON_BACKFILL: &str = include_str!("fixtures/hevy_backfill_python.json");

const EXTRA_TEMPLATES: [(&str, &str, &str); 6] = [
    ("tpl-x-1", "Incline Bench Press (Smith Machine)", "chest"),
    ("tpl-x-2", "Zercher Squat", "quadriceps"),
    ("tpl-x-3", "Cable Fly Crossover", "chest"),
    ("tpl-x-4", "xyz", "calves"),
    ("tpl-x-5", "!!!", ""),
    ("tpl-x-6", "Seated Row Machine", "upper_back"),
];

fn app() -> (Db, Router) {
    let db = Db::open_in_memory().unwrap();
    hevy_seed::seed(&db.lock(), Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap()).unwrap();
    let state = AppState::new(
        db.clone(),
        true,
        Arc::new(|| 1_800_000_000.0),
        schedule::UNDO_WINDOW_SECONDS,
        AuthConfig::sign_in_off(),
    );
    (db, build_app(state, None))
}

async fn call(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:8382")
        .header("x-activsync-request", "1")
        .header("sec-fetch-site", "same-origin");
    let body = match body {
        Some(value) => {
            request = request.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() })
}

async fn ok(app: &Router, method: Method, uri: &str, body: Option<Value>) -> Value {
    let (status, body) = call(app, method, uri, body).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

fn digest(value: &Value) -> String {
    hex(&Sha256::digest(value.to_string().as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn mapping<'a>(tools: &'a Value, template_id: &str) -> &'a Value {
    tools["mappings"].as_array().unwrap().iter().find(|m| m["templateId"] == template_id).unwrap()
}

#[tokio::test]
async fn mapping_screen_matches_python() {
    let (db, app) = app();
    {
        let conn = db.lock();
        for (id, title, muscle) in EXTRA_TEMPLATES {
            let template = Template {
                id,
                title,
                primary_muscle_group: Some(muscle),
                secondary_muscle_groups: &[],
                equipment_category: Some("other"),
                is_custom: true,
            };
            hevy_db::upsert_template(&conn, &template).unwrap();
        }
        conn.execute("UPDATE exercise_templates SET primary_muscle_group = NULL WHERE primary_muscle_group = ''", [])
            .unwrap();
        hevy_db::save_strava_mapping(&conn, "tpl-dev-squat", "BARBELL_BENCH_PRESS").unwrap();
    }
    let python: Value = serde_json::from_str(PYTHON_TOOLS).unwrap();
    let tools = ok(&app, Method::GET, "/api/v1/settings/hevy/tools", None).await;
    assert_eq!(tools["mappings"], python["mappings"]);
    let categories = tools["categories"].as_array().unwrap();
    assert_eq!(json!([categories.len(), digest(&tools["categories"])]), python["categories"]);
    let groups = tools["stravaGroups"].as_array().unwrap();
    assert_eq!(json!([groups.len(), digest(&tools["stravaGroups"])]), python["stravaGroups"]);
}

#[tokio::test]
async fn saving_a_mapping_resumes_waiting_workouts() {
    let (db, app) = app();
    let uri = "/api/v1/settings/hevy/mappings/tpl-dev-custom";
    let (status, body) = call(&app, Method::PUT, uri, Some(json!({"category": 9999, "subcategory": 0}))).await;
    assert_eq!((status, body["detail"].as_str()), (StatusCode::BAD_REQUEST, Some("Unknown Garmin exercise category/subcategory pair.")));
    let (status, body) =
        call(&app, Method::PUT, "/api/v1/settings/hevy/mappings/nope", Some(json!({"category": 23, "subcategory": 0}))).await;
    assert_eq!((status, body["detail"].as_str()), (StatusCode::NOT_FOUND, Some("Exercise template not found.")));

    let body = ok(&app, Method::PUT, uri, Some(json!({"category": 23, "subcategory": 0}))).await;
    assert_eq!(body["message"], "Exercise mapping saved. 1 waiting workout resumed.");
    let workout = hevy_db::get_workout(&db.lock(), hevy_seed::UNMAPPED_ID).unwrap().unwrap();
    assert_eq!((workout.status.as_str(), workout.error), ("waiting_watch", None));

    let tools = ok(&app, Method::GET, "/api/v1/settings/hevy/tools", None).await;
    let custom = mapping(&tools, "tpl-dev-custom");
    assert_eq!((custom["source"].clone(), custom["categoryName"].clone()), (json!("user"), json!("Row")));
    assert_eq!(custom["stravaExerciseType"], "ROW_GENERIC");

    let body = ok(&app, Method::PUT, uri, Some(json!({"category": 23, "subcategory": 0}))).await;
    assert_eq!(body["message"], "Exercise mapping saved.");
    let body = ok(&app, Method::DELETE, uri, None).await;
    assert_eq!(body["message"], "Exercise mapping removed.");
    assert_eq!(mapping(&ok(&app, Method::GET, "/api/v1/settings/hevy/tools", None).await, "tpl-dev-custom")["unmapped"], true);
}

#[tokio::test]
async fn removing_an_override_restores_the_standard_pair() {
    let (_db, app) = app();
    let body = ok(&app, Method::DELETE, "/api/v1/settings/hevy/mappings/tpl-dev-bench", None).await;
    assert_eq!(body["message"], "Standard mapping restored.");
    let (status, _) = call(&app, Method::DELETE, "/api/v1/settings/hevy/mappings/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn strava_exercise_override_and_reset() {
    let (_db, app) = app();
    let uri = "/api/v1/settings/hevy/mappings/tpl-dev-bench/strava";
    let (status, body) = call(&app, Method::PUT, uri, Some(json!({"exerciseType": "MADE_UP"}))).await;
    assert_eq!((status, body["detail"].as_str()), (StatusCode::BAD_REQUEST, Some("Unknown Strava exercise.")));
    let (status, _) =
        call(&app, Method::PUT, "/api/v1/settings/hevy/mappings/nope/strava", Some(json!({"exerciseType": "SKI_ERG"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    assert_eq!(ok(&app, Method::PUT, uri, Some(json!({"exerciseType": "SKI_ERG"}))).await["message"], "Strava exercise saved.");
    let bench = mapping(&ok(&app, Method::GET, "/api/v1/settings/hevy/tools", None).await, "tpl-dev-bench").clone();
    assert_eq!(
        (bench["stravaExerciseType"].clone(), bench["stravaSource"].clone(), bench["stravaStandardExerciseType"].clone()),
        (json!("SKI_ERG"), json!("user"), json!("BARBELL_BENCH_PRESS"))
    );
    assert_eq!(ok(&app, Method::DELETE, uri, None).await["message"], "Standard Strava exercise restored.");
    let bench = mapping(&ok(&app, Method::GET, "/api/v1/settings/hevy/tools", None).await, "tpl-dev-bench").clone();
    assert_eq!(bench["stravaSource"], "garmin");
}

fn garmin_time(workout: &Value, minutes: i64) -> String {
    let start = chrono::DateTime::parse_from_rfc3339(workout["start_time"].as_str().unwrap()).unwrap();
    (start + chrono::Duration::minutes(minutes)).format("%Y-%m-%d %H:%M:%S").to_string()
}

#[tokio::test]
async fn history_import_matches_python() {
    let now = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
    let db = Db::open_in_memory().unwrap();
    {
        let conn = db.lock();
        hevy_seed::seed(&conn, now).unwrap();
        let settings = json!({"hevy_match_mode": "automatic", "hevy_watch_strategy": "merge", "hevy_grace_minutes": 120});
        activsync::db::set_config_value(&conn, "settings", &settings).unwrap();
        activsync::db::set_config_value(&conn, "hevy_api_key", &json!("k")).unwrap();
        let account = hevy_seed::account_workouts(now);
        let find = |id: &str| account.iter().find(|w| w["id"] == id).unwrap();
        for (id, title, start) in [
            (920001, "Twin", garmin_time(find("hw-dev-backfill-importable"), 0)),
            (920002, "Overlap", garmin_time(find("hw-dev-backfill-unmapped"), 5)),
        ] {
            let activity = activsync::db::NewActivity {
                garmin_activity_id: id,
                activity_type: "strength_training",
                title,
                description: "",
                start_time: &start,
                content_hash: &format!("dev-{id}"),
                publish_status: "pending",
                garmin_data: r#"{"duration": 3600}"#,
                hold_reason: None,
            };
            activsync::db::insert_activity(&conn, &activity, now).unwrap();
        }
        hevy_db::set_workout_status(&conn, hevy_seed::MERGED_ID, "skipped", None).unwrap();
    }
    let state = AppState::new(db.clone(), true, Arc::new(|| 1_800_000_000.0), 5.0, AuthConfig::sign_in_off());
    let app = build_app(state, None);
    let python: Value = serde_json::from_str(PYTHON_BACKFILL).unwrap();
    let uri = "/api/v1/settings/hevy/backfill";

    let preview = ok(&app, Method::POST, &format!("{uri}/preview"), Some(json!({"since": "2026-01-01"}))).await;
    assert_eq!(preview, python["preview"]);
    let recent = ok(&app, Method::POST, &format!("{uri}/preview"), Some(json!({"since": "2027-01-14T00:00:00Z"}))).await;
    assert_eq!(recent["message"], python["recent"]);

    let ids = ["hw-dev-backfill-importable", "hw-dev-backfill-unmapped", "hw-dev-backfill-unmappable", "hw-dev-merged", "bogus"];
    let run = ok(&app, Method::POST, &format!("{uri}/run"), Some(json!({"since": "2026-01-01", "hevyIds": ids}))).await;
    assert_eq!(json!({"message": run["message"], "linked": run["linked"], "ran": run["ran"]}), python["run"]);

    let conn = db.lock();
    let mut stmt = conn
        .prepare("SELECT hevy_id, status, error, source_garmin_activity_id FROM hevy_workouts ORDER BY hevy_id")
        .unwrap();
    let rows: Vec<Value> = stmt
        .query_map([], |r| {
            Ok(json!([r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<i64>>(3)?]))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(Value::Array(rows), python["rows"]);
}

#[tokio::test]
async fn history_import_needs_hevy_and_a_date() {
    let (db, app) = app();
    let uri = "/api/v1/settings/hevy/backfill/preview";
    let (status, body) = call(&app, Method::POST, uri, Some(json!({"since": "2026-01-01"}))).await;
    assert_eq!((status, body["detail"].as_str()), (StatusCode::CONFLICT, Some("Connect Hevy before backfilling workouts.")));
    activsync::db::set_config_value(&db.lock(), "hevy_api_key", &json!("k")).unwrap();
    let (status, body) = call(&app, Method::POST, uri, Some(json!({"since": "last week"}))).await;
    assert_eq!((status, body["detail"].as_str()), (StatusCode::BAD_REQUEST, Some("Enter the backfill start date as YYYY-MM-DD.")));
    let (status, body) = call(&app, Method::POST, uri, Some(json!({"since": "2030-01-01"}))).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::OK, Some("No Hevy workouts found since 2030-01-01.")));

    let body = json!({"since": "2026-01-01", "hevyIds": []});
    let run = ok(&app, Method::POST, "/api/v1/settings/hevy/backfill/run", Some(body)).await;
    assert_eq!(run["message"], "Backfill complete: 0 workouts imported, 0 matched to existing Garmin activities.");
}
