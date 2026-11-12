mod support;

use std::io::Write;
use std::sync::{Arc, Mutex};

use activsync::auth_guard::AuthConfig;
use activsync::db::{self, Db};
use activsync::garmin_client::{GarminClient, GarminError, Tokens};
use activsync::hevy_db;
use activsync::net::{Http, HttpError};
use activsync::poller::{BoxFuture, Clients};
use activsync::strava_client::StravaClient;
use activsync::{AppState, build_app, schedule};
use axum::Router;
use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use openidconnect::HttpRequest;
use serde_json::{Value, json};
use support::{call, send};

type Calls = Arc<Mutex<Vec<String>>>;

const SCOPE: &str = "read,activity:read_all,activity:write";

fn zipped(fit: &[u8]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file("activity.fit", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(fit).unwrap();
    writer.finish().unwrap().into_inner()
}

fn service(calls: &Calls, route: impl Fn(&str) -> (u16, Vec<u8>) + Send + Sync + 'static) -> Http {
    let log = calls.clone();
    Arc::new(move |request: HttpRequest| {
        let key = format!("{} {}", request.method(), request.uri().path());
        log.lock().unwrap().push(key.clone());
        let (status, body) = route(&key);
        let response = openidconnect::http::Response::builder()
            .status(status)
            .body(body)
            .map_err(|e| HttpError(e.to_string()));
        Box::pin(async move { response })
    })
}

fn json_body(value: Value) -> Vec<u8> {
    value.to_string().into_bytes()
}

struct Fake {
    db: Db,
    garmin_http: Http,
    strava_http: Http,
}

impl Clients for Fake {
    fn garmin(&self) -> BoxFuture<'_, Result<GarminClient, GarminError>> {
        let tokens = Tokens {
            di_token: Some("token".into()),
            di_refresh_token: None,
            di_client_id: None,
        };
        let client = GarminClient::new(
            self.garmin_http.clone(),
            Arc::new(|| Utc::now().timestamp() as f64),
            tokens,
            None,
        )
        .without_waits();
        Box::pin(async move { Ok(client) })
    }

    fn strava(&self) -> StravaClient {
        StravaClient::new(
            self.strava_http.clone(),
            self.db.clone(),
            Arc::new(|| Utc::now().timestamp() as f64),
            "1",
            "s",
        )
        .without_waits()
    }
}

struct Harness {
    db: Db,
    state: AppState,
    app: Router,
    garmin_calls: Calls,
    strava_calls: Calls,
}

fn connect(db: &Db, onboarded: bool) {
    let conn = db.lock();
    db::set_config_value(&conn, "initial_sync_done", &json!(onboarded)).unwrap();
    db::set_config_value(
        &conn,
        "garmin_credentials",
        &json!({"email": "a@example.com", "password": "p"}),
    )
    .unwrap();
    db::set_config_value(&conn, "garmin_credentials_verified", &json!(true)).unwrap();
    db::set_config_value(
        &conn,
        "strava_credentials",
        &json!({"client_id": "1", "client_secret": "s"}),
    )
    .unwrap();
    let tokens = json!({"access_token": "a", "refresh_token": "r", "expires_at": Utc::now().timestamp() + 21600, "scope": SCOPE});
    db::set_config_value(&conn, "strava_tokens", &tokens).unwrap();
}

fn mock_harness(onboarded: bool) -> Harness {
    let db = Db::open_in_memory().unwrap();
    connect(&db, onboarded);
    let state = AppState::new(
        db.clone(),
        true,
        schedule::system_clock(),
        0.0,
        AuthConfig::sign_in_off(),
    );
    Harness {
        db,
        app: build_app(state.clone(), None),
        state,
        garmin_calls: Calls::default(),
        strava_calls: Calls::default(),
    }
}

fn real_harness(garmin_ok: bool, strava_upload: u16) -> Harness {
    let db = Db::open_in_memory().unwrap();
    connect(&db, true);
    let garmin_calls = Calls::default();
    let garmin_http = service(&garmin_calls, move |key| match key {
        k if k.starts_with("GET /download-service/") => (200, zipped(b"FITDATA")),
        k if k.starts_with("PUT /") && garmin_ok => (204, Vec::new()),
        _ => (500, b"{}".to_vec()),
    });
    let strava_calls = Calls::default();
    let strava_http = service(&strava_calls, move |key| match key {
        k if k.starts_with("GET /api/v3/athlete/activities") => (200, b"[]".to_vec()),
        "POST /api/v3/uploads" if strava_upload == 200 => (200, json_body(json!({"id": 9}))),
        "POST /api/v3/uploads" => (
            strava_upload,
            json_body(json!({"message": "Authorization Error"})),
        ),
        "GET /api/v3/uploads/9" => (200, json_body(json!({"activity_id": 555}))),
        k if k.starts_with("PUT /api/v3/activities/") => (200, b"{}".to_vec()),
        _ => (200, b"{}".to_vec()),
    });
    let fake = Arc::new(Fake {
        db: db.clone(),
        garmin_http,
        strava_http,
    });
    let state = AppState::new(
        db.clone(),
        false,
        schedule::system_clock(),
        0.0,
        AuthConfig::sign_in_off(),
    )
    .with_clients(fake);
    Harness {
        db,
        app: build_app(state.clone(), None),
        state,
        garmin_calls,
        strava_calls,
    }
}

impl Harness {
    fn add(&self, id: i64, status: &str, start: &str) {
        self.db
            .lock()
            .execute(
                "INSERT INTO activities (garmin_activity_id, activity_type, title, start_time, content_hash, publish_status, first_seen_at)
                 VALUES (?1, 'running', ?2, ?3, 'h', ?4, '2026-11-10T09:00:00+00:00')",
                rusqlite::params![id, format!("Activity {id}"), start, status],
            )
            .unwrap();
    }

    fn recent(&self, minutes_ago: i64) -> String {
        (Utc::now() - Duration::minutes(minutes_ago))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    }

    fn activity(&self, id: i64) -> Option<db::ActivityRow> {
        db::get_activity(&self.db.lock(), id).unwrap()
    }

    fn config(&self, key: &str) -> Option<Value> {
        db::get_config_value(&self.db.lock(), key).unwrap()
    }

    fn calls(calls: &Calls, prefix: &str) -> usize {
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.starts_with(prefix))
            .count()
    }
}

#[tokio::test]
async fn the_app_state_reports_development_for_mock_data_only() {
    let h = mock_harness(true);
    let (_, body) = call(&h.app, Method::GET, "/api/v1/app", None).await;
    assert_eq!(body["development"], true);

    let db = Db::open_in_memory().unwrap();
    let state = AppState::new(
        db,
        false,
        schedule::system_clock(),
        0.0,
        AuthConfig::sign_in_off(),
    )
    .with_development(true);
    let (_, body) = call(&build_app(state, None), Method::GET, "/api/v1/app", None).await;
    assert_eq!(body["development"], false);
}

#[tokio::test]
async fn a_page_beyond_64_bits_clamps_to_the_last_page() {
    let h = mock_harness(true);
    h.add(1, "pending", "2026-11-10 08:00:00");
    let (status, body) = call(
        &h.app,
        Method::GET,
        "/api/v1/activities?page=99999999999999999999999",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["pagination"]["page"], 1);
    let (status, body) = call(
        &h.app,
        Method::GET,
        "/api/v1/activities?pageSize=99999999999999999999999",
        None,
    )
    .await;
    assert_eq!(
        (status, body["detail"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("pageSize must be one of 10, 20, 50, or 100")
        )
    );
    let (status, body) = call(
        &h.app,
        Method::GET,
        "/api/v1/activities?page=-99999999999999999999999",
        None,
    )
    .await;
    assert_eq!(
        (status, body["detail"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("page must be at least 1")
        )
    );
    let (status, body) = call(&h.app, Method::GET, "/api/v1/activities?page=x", None).await;
    assert_eq!(
        (status, body["detail"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("page must be an integer")
        )
    );
}

#[tokio::test]
async fn restore_reads_hold_reason_in_either_spelling() {
    let h = mock_harness(true);
    h.add(1, "excluded", "2026-11-10 08:00:00");
    let (status, body) = call(
        &h.app,
        Method::POST,
        "/api/v1/activities/1/restore",
        Some(json!({"status": "held", "hold_reason": "backlog"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = h.activity(1).unwrap();
    assert_eq!(
        (row.publish_status.as_str(), row.hold_reason.as_deref()),
        ("held", Some("backlog"))
    );

    h.add(2, "excluded", "2026-11-10 08:00:00");
    let both = json!({"status": "held", "holdReason": "category", "hold_reason": "backlog"});
    call(
        &h.app,
        Method::POST,
        "/api/v1/activities/2/restore",
        Some(both),
    )
    .await;
    assert_eq!(
        h.activity(2).unwrap().hold_reason.as_deref(),
        Some("category")
    );

    h.add(3, "excluded", "2026-11-10 08:00:00");
    let (status, body) = call(
        &h.app,
        Method::POST,
        "/api/v1/activities/3/restore",
        Some(json!({"status": "held", "hold_reason": "nope"})),
    )
    .await;
    assert_eq!(
        (status, body["detail"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("holdReason must be category or backlog")
        )
    );
}

#[tokio::test]
async fn mock_edits_land_on_the_mock_garmin_and_survive_a_sync() {
    let h = mock_harness(true);
    h.add(1, "pending", &h.recent(60));
    let (status, body) = call(
        &h.app,
        Method::PUT,
        "/api/v1/activities/1",
        Some(json!({"title": "  New title ", "description": " words "})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Saved changes to Activity 1");
    let row = h.activity(1).unwrap();
    assert_eq!(
        (row.title.as_str(), row.description.as_str()),
        ("New title", "words")
    );
    assert_eq!(
        row.content_hash,
        activsync::routes::content_hash("New title", "words", "running", "{}")
    );
    let remote = h.config("dev_garmin_remote_activities").unwrap();
    assert_eq!(remote["1"]["title"], "New title");

    call(&h.app, Method::POST, "/api/v1/settings/sync/garmin", None).await;
    let row = h.activity(1).unwrap();
    assert_eq!(
        (row.title.as_str(), row.content_hash.as_str()),
        (
            "New title",
            activsync::routes::content_hash("New title", "words", "running", "{}").as_str()
        )
    );
}

#[tokio::test]
async fn syncing_garmin_in_mock_mode_reads_the_mock_garmin() {
    let h = mock_harness(true);
    h.add(1, "pending", &h.recent(60));
    h.add(2, "pending", &h.recent(90));
    let start = h.recent(30);
    let remote = json!({"950001": {"garmin_activity_id": 950001, "activity_type": "strength_training", "title": "Uploaded",
        "description": "", "start_time": start, "duration": 3600.0}});
    db::set_config_value(&h.db.lock(), "dev_garmin_remote_activities", &remote).unwrap();
    db::set_config_value(&h.db.lock(), "dev_garmin_deleted_activity_ids", &json!([2])).unwrap();

    let (status, body) = call(&h.app, Method::POST, "/api/v1/settings/sync/garmin", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let uploaded = h.activity(950001).unwrap();
    assert_eq!(
        (uploaded.title.as_str(), uploaded.publish_status.as_str()),
        ("Uploaded", "pending")
    );
    assert_eq!(uploaded.garmin_data, "{\"duration\": 3600.0}");
    assert!(h.activity(1).is_some());
    assert!(h.activity(2).is_none());
    assert_eq!(h.config("garmin_last_sync_ok"), Some(json!(true)));
}

#[tokio::test]
async fn syncing_strava_in_mock_mode_links_a_twin_the_mock_strava_reports() {
    let h = mock_harness(true);
    h.add(1, "published", &h.recent(60));
    h.add(2, "pending", &h.recent(62));
    db::set_published(&h.db.lock(), 1, 7001, Utc::now()).unwrap();
    let (status, body) = call(&h.app, Method::POST, "/api/v1/settings/sync/strava", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let linked = h.activity(2).unwrap();
    assert_eq!(
        (linked.publish_status.as_str(), linked.strava_activity_id),
        ("published", Some(7001))
    );
    assert_eq!(h.activity(1).unwrap().publish_status, "published");
}

#[tokio::test]
async fn reconnecting_garmin_in_mock_mode_catches_up_and_reports_it() {
    let h = mock_harness(true);
    let remote = json!({"950001": {"garmin_activity_id": 950001, "activity_type": "running", "title": "During the outage",
        "description": "", "start_time": h.recent(30), "duration": 60.0}});
    db::set_config_value(&h.db.lock(), "dev_garmin_remote_activities", &remote).unwrap();
    let reconnect = json!({"email": "me@example.com", "password": "secret"});
    let (status, body) = call(
        &h.app,
        Method::POST,
        "/api/v1/settings/garmin/reconnect",
        Some(reconnect),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(h.activity(950001).is_some());
    let report = h.config("catch_up_report").unwrap();
    assert_eq!(
        (report["new"].clone(), report["linked"].clone()),
        (json!(1), json!(0))
    );
}

#[tokio::test]
async fn the_initial_sync_in_mock_mode_reads_the_mock_garmin() {
    let h = mock_harness(false);
    h.add(1, "pending", &h.recent(60));
    let remote = json!({"950001": {"garmin_activity_id": 950001, "activity_type": "strength_training", "title": "Uploaded",
        "description": "", "start_time": h.recent(30), "duration": 60.0}});
    db::set_config_value(&h.db.lock(), "dev_garmin_remote_activities", &remote).unwrap();
    let (status, body) = call(&h.app, Method::POST, "/api/v1/setup/initial-sync", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(h.config("initial_sync_done"), Some(json!(true)));
    assert!(h.activity(950001).is_some());
}

#[tokio::test]
async fn publishing_now_in_mock_mode_marks_the_activity_published() {
    let h = mock_harness(true);
    h.add(1, "held", &h.recent(60));
    h.add(2, "missing", &h.recent(120));
    h.add(3, "published", &h.recent(180));
    h.add(4, "pending", &h.recent(240));
    let mut events = h.state.events.subscribe();

    let (status, body) = call(&h.app, Method::POST, "/api/v1/activities/1/publish", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Published Activity 1 to Strava");
    assert_eq!(h.activity(1).unwrap().strava_activity_id, Some(9_000_001));
    assert_eq!(events.try_recv(), Ok("refresh"));

    let (_, body) = call(&h.app, Method::POST, "/api/v1/activities/2/publish", None).await;
    assert_eq!(body["message"], "Republished Activity 2 to Strava");

    let (status, body) = call(&h.app, Method::POST, "/api/v1/activities/3/publish", None).await;
    assert_eq!(
        (status, body["detail"].as_str()),
        (
            StatusCode::CONFLICT,
            Some("Only pending, held, or missing activities can be published")
        )
    );
    let (status, body) = call(&h.app, Method::POST, "/api/v1/activities/77/publish", None).await;
    assert_eq!(
        (status, body["detail"].as_str()),
        (StatusCode::NOT_FOUND, Some("Activity was not found"))
    );

    let (status, body) = call(
        &h.app,
        Method::POST,
        "/api/v1/activities/publish",
        Some(json!({"activityIds": [3, 4]})),
    )
    .await;
    assert_eq!(
        (status, body["detail"].as_str()),
        (
            StatusCode::CONFLICT,
            Some("Some selected activities are no longer publishable. Refresh and try again.")
        )
    );
    let (status, _) = call(
        &h.app,
        Method::POST,
        "/api/v1/activities/publish",
        Some(json!({"activityIds": []})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, body) = call(
        &h.app,
        Method::POST,
        "/api/v1/activities/publish",
        Some(json!({"activityIds": [4]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (body["message"].as_str(), body["publishedCount"].as_i64()),
        (Some("Published 1 activity to Strava"), Some(1))
    );
}

#[tokio::test]
async fn a_bulk_publish_reports_what_hevy_blocked() {
    let h = mock_harness(true);
    h.add(1, "pending", &h.recent(60));
    h.add(2, "pending", &h.recent(120));
    {
        let conn = h.db.lock();
        hevy_db::upsert_workout(
            &conn,
            "h1",
            "Legs",
            "2026-11-10T02:00:00Z",
            "2026-11-10T03:00:00Z",
            "2026-11-10T03:05:00Z",
            &json!({}),
        )
        .unwrap();
        hevy_db::claim_source(&conn, "h1", 2).unwrap();
    }
    let (status, body) = call(
        &h.app,
        Method::POST,
        "/api/v1/activities/publish",
        Some(json!({"activityIds": [1, 2]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Published 1; 1 blocked by Hevy sync");
    assert_eq!(
        (body["severity"].as_str(), body["blockedCount"].as_i64()),
        (Some("warning"), Some(1))
    );

    let (status, body) = call(&h.app, Method::POST, "/api/v1/activities/2/publish", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .starts_with("Activity was blocked by Hevy sync: hevy workout h1 is "),
        "{body}"
    );
}

#[tokio::test]
async fn publishing_needs_both_services_connected() {
    let h = mock_harness(true);
    h.add(1, "pending", &h.recent(60));
    db::set_config_value(&h.db.lock(), "strava_tokens", &Value::Null).unwrap();
    let (status, body) = call(&h.app, Method::POST, "/api/v1/activities/1/publish", None).await;
    assert_eq!(
        (status, body["detail"].as_str()),
        (
            StatusCode::CONFLICT,
            Some("Both Garmin and Strava must be connected to publish. Reconnect to continue.")
        )
    );
}

#[tokio::test]
async fn editing_pushes_to_garmin_and_to_strava_when_published() {
    let h = real_harness(true, 200);
    h.add(1, "pending", &h.recent(60));
    h.add(2, "published", &h.recent(120));
    db::set_published(&h.db.lock(), 2, 555, Utc::now()).unwrap();

    let (status, body) = call(
        &h.app,
        Method::PUT,
        "/api/v1/activities/1",
        Some(json!({"title": "Leg day", "description": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(Harness::calls(&h.garmin_calls, "PUT /"), 2);
    assert_eq!(Harness::calls(&h.strava_calls, "PUT /"), 0);
    assert_eq!(h.activity(1).unwrap().title, "Leg day");

    call(
        &h.app,
        Method::PUT,
        "/api/v1/activities/2",
        Some(json!({"title": "Easy run", "description": "d"})),
    )
    .await;
    assert_eq!(Harness::calls(&h.garmin_calls, "PUT /"), 4);
    assert_eq!(
        Harness::calls(&h.strava_calls, "PUT /api/v3/activities/555"),
        1
    );
}

#[tokio::test]
async fn an_edit_garmin_refuses_is_a_502_and_changes_nothing_locally() {
    let h = real_harness(false, 200);
    h.add(1, "pending", &h.recent(60));
    let (status, body) = call(
        &h.app,
        Method::PUT,
        "/api/v1/activities/1",
        Some(json!({"title": "Leg day"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .starts_with("Could not save activity: "),
        "{body}"
    );
    assert_eq!(h.activity(1).unwrap().title, "Activity 1");

    let (status, body) = call(
        &h.app,
        Method::PUT,
        "/api/v1/activities/1",
        Some(json!({"title": "  "})),
    )
    .await;
    assert_eq!(
        (status, body["detail"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("Activity title cannot be blank")
        )
    );
}

#[tokio::test]
async fn publishing_now_uploads_to_strava() {
    let h = real_harness(true, 200);
    h.add(1, "pending", &h.recent(60));
    h.add(2, "held", &h.recent(120));
    let (status, body) = call(&h.app, Method::POST, "/api/v1/activities/1/publish", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Published Activity 1 to Strava");
    assert_eq!(h.activity(1).unwrap().strava_activity_id, Some(555));

    let (status, body) = call(
        &h.app,
        Method::POST,
        "/api/v1/activities/publish",
        Some(json!({"activityIds": [2]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Published 1 activity to Strava");
    assert_eq!(Harness::calls(&h.strava_calls, "POST /api/v3/uploads"), 2);
}

#[tokio::test]
async fn a_publish_strava_rejects_is_a_409_for_one_and_a_failure_count_for_many() {
    let h = real_harness(true, 401);
    h.add(1, "pending", &h.recent(60));
    h.add(2, "pending", &h.recent(120));
    let (status, body) = call(&h.app, Method::POST, "/api/v1/activities/1/publish", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["detail"].as_str().is_some_and(|d| !d.is_empty()),
        "{body}"
    );
    assert_eq!(h.activity(1).unwrap().publish_status, "pending");

    let (status, body) = call(
        &h.app,
        Method::POST,
        "/api/v1/activities/publish",
        Some(json!({"activityIds": [1, 2]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Published 0; 2 failed and can be retried");
    assert_eq!(
        (body["severity"].as_str(), body["failedCount"].as_i64()),
        (Some("warning"), Some(2))
    );
}

#[tokio::test]
async fn a_publish_garmin_cannot_serve_is_a_502() {
    let h = real_harness(true, 200);
    h.add(1, "pending", &h.recent(60));
    let broken = service(&Calls::default(), |_| (500, b"{}".to_vec()));
    let fake = Arc::new(Fake {
        db: h.db.clone(),
        garmin_http: broken,
        strava_http: service(&Calls::default(), |_| (200, b"[]".to_vec())),
    });
    let app = build_app(h.state.clone().with_clients(fake), None);
    let (status, body) = call(&app, Method::POST, "/api/v1/activities/1/publish", None).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .starts_with("Could not publish activity: "),
        "{body}"
    );
}

#[tokio::test]
async fn disconnecting_strava_revokes_it_there_and_forgets_the_tokens() {
    let h = real_harness(true, 200);
    let (status, body) = call(&h.app, Method::DELETE, "/api/v1/settings/strava", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["message"],
        "Strava disconnected. Publishing is paused."
    );
    assert_eq!(Harness::calls(&h.strava_calls, "POST /oauth/revoke"), 1);
    assert_eq!(h.config("strava_tokens"), Some(Value::Null));

    let mock = mock_harness(true);
    let (status, _) = call(&mock.app, Method::DELETE, "/api/v1/settings/strava", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mock.config("strava_tokens"), Some(Value::Null));
}

#[tokio::test]
async fn connecting_hevy_in_mock_mode_caches_the_mock_templates() {
    let h = mock_harness(true);
    let (status, body) = call(
        &h.app,
        Method::POST,
        "/api/v1/settings/hevy/credentials",
        Some(json!({"apiKey": "key"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let conn = h.db.lock();
    for id in [
        "tpl-dev-bench",
        "tpl-dev-rdl",
        "tpl-dev-pushdown",
        "tpl-dev-custom",
    ] {
        assert!(hevy_db::template_exists(&conn, id).unwrap(), "{id}");
    }
}

async fn strava_callback(h: &Harness, query: &str) -> String {
    let uri = format!("/strava/callback?{query}");
    let (status, _, location) = send(&h.app, Method::GET, &uri, None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    location.unwrap()
}

#[tokio::test]
async fn the_strava_callback_in_setup_sends_no_refresh_and_ignores_an_empty_error() {
    let h = mock_harness(false);
    db::set_config_value(&h.db.lock(), "strava_oauth_state", &json!("tok")).unwrap();
    let mut events = h.state.events.subscribe();
    let location = strava_callback(
        &h,
        &format!(
            "error=&code=abc&state=tok&scope={}",
            SCOPE.replace(',', "%2C")
        ),
    )
    .await;
    assert_eq!(location, "/setup");
    assert!(events.try_recv().is_err());
    assert!(h.config("strava_tokens").is_some());

    let location = strava_callback(&h, "error=&state=tok").await;
    assert!(
        location.starts_with("/setup?stravaError=Strava+did+not+return+an+authorization."),
        "{location}"
    );
}
