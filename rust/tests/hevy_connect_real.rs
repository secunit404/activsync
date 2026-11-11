use std::sync::{Arc, Mutex};
use std::time::Duration;

use activsync::auth_guard::AuthConfig;
use activsync::db::{self, Db};
use activsync::garmin_client::{GarminClient, GarminError, Tokens};
use activsync::hevy_db;
use activsync::net::{Http, HttpError};
use activsync::poller::{BoxFuture, Clients};
use activsync::strava_client::StravaClient;
use activsync::{AppState, build_app, schedule};
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use openidconnect::HttpRequest;
use serde_json::{Value, json};
use tower::ServiceExt;

type Calls = Arc<Mutex<Vec<String>>>;
type Route = Box<dyn Fn(&str, &str) -> (u16, Value) + Send + Sync>;

fn service(calls: &Calls, route: Route) -> Http {
    let log = calls.clone();
    Arc::new(move |request: HttpRequest| {
        let key = format!("{} {}", request.method(), request.uri().path());
        let query = request.uri().query().unwrap_or_default().to_string();
        log.lock().unwrap().push(format!("{key}?{query}"));
        let (status, body) = route(&key, &query);
        let body = if body.is_null() { Vec::new() } else { body.to_string().into_bytes() };
        let response = openidconnect::http::Response::builder().status(status).body(body).map_err(|e| HttpError(e.to_string()));
        Box::pin(async move { response })
    })
}

struct Fake {
    db: Db,
    garmin_http: Http,
}

impl Clients for Fake {
    fn garmin(&self) -> BoxFuture<'_, Result<GarminClient, GarminError>> {
        let tokens = Tokens { di_token: Some("token".into()), di_refresh_token: None, di_client_id: None };
        let clock = Arc::new(|| Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap().timestamp() as f64);
        let client = GarminClient::new(self.garmin_http.clone(), clock, tokens, None).without_waits().with_display_name("runner");
        Box::pin(async move { Ok(client) })
    }

    fn strava(&self) -> StravaClient {
        StravaClient::new(self.garmin_http.clone(), self.db.clone(), Arc::new(|| 0.0), "1", "s")
    }
}

struct Harness {
    db: Db,
    app: Router,
    hevy_calls: Calls,
    garmin_calls: Calls,
}

/// Hevy retries 5xx with real back-off, so outages here answer 400.
/// Real-account mode with Hevy answering `hevy`, and a Garmin holding the
/// strength activity 101 recorded alongside the workout `w1`.
fn harness(onboarded: bool, hevy: Route) -> Harness {
    let db = Db::open_in_memory().unwrap();
    {
        let conn = db.lock();
        db::set_config_value(&conn, "initial_sync_done", &json!(onboarded)).unwrap();
        db::set_config_value(&conn, "garmin_credentials_verified", &json!(true)).unwrap();
        db::set_config_value(&conn, "settings", &json!({"hevy_watch_strategy": "merge"})).unwrap();
        conn.execute(
            "INSERT INTO activities (garmin_activity_id, activity_type, title, start_time, content_hash, publish_status, first_seen_at, garmin_data)
             VALUES (101, 'strength_training', 'Strength', '2026-11-10 08:00:00', 'h', 'held', '2026-11-10T09:00:00+00:00', '{\"duration\": 3600.0}')",
            [],
        )
        .unwrap();
    }
    let garmin_calls = Calls::default();
    let garmin_http = service(
        &garmin_calls,
        Box::new(|key, _| match key {
            k if k.starts_with("GET /activitylist-service/") => (200, json!([])),
            k if k.starts_with("PUT ") => (204, Value::Null),
            k if k.starts_with("GET /activity-service/") => (200, json!({"exerciseSets": []})),
            _ => (200, json!({})),
        }),
    );
    let hevy_calls = Calls::default();
    let hevy_http = service(&hevy_calls, hevy);
    let clients = Arc::new(Fake { db: db.clone(), garmin_http });
    let state = AppState::new(db.clone(), false, schedule::system_clock(), 0.0, AuthConfig::sign_in_off())
        .with_clients(clients)
        .with_hevy_http(hevy_http);
    Harness { db, app: build_app(state, None), hevy_calls, garmin_calls }
}

fn workout(id: &str, start: &str, exercise: Value) -> Value {
    json!({"id": id, "title": format!("Workout {id}"), "start_time": start, "end_time": start.replace("T08:", "T09:"),
           "updated_at": start, "exercises": [exercise]})
}

fn bench() -> Value {
    json!({"title": "Bench Press (Barbell)", "exercise_template_id": "T-BENCH", "sets": [{"type": "normal", "weight_kg": 80, "reps": 5}]})
}

/// `count` workouts on consecutive days from 2026-10-01, newest first, ten
/// to a page and no `page_count`, the way Hevy sometimes answers.
fn history(count: usize) -> Vec<Value> {
    (0..count)
        .rev()
        .map(|day| {
            let start = (Utc.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).unwrap() + chrono::Duration::days(day as i64)).to_rfc3339();
            workout(&format!("h{day}"), &start.replace("+00:00", "Z"), bench())
        })
        .collect()
}

fn page_of(query: &str) -> usize {
    query.split('&').find_map(|p| p.strip_prefix("page=")).unwrap().parse().unwrap()
}

fn history_route(workouts: Vec<Value>, count_status: u16) -> Route {
    let total = workouts.len();
    Box::new(move |key, query| match key {
        k if k.ends_with("/workouts/count") => (count_status, json!({"workout_count": total})),
        k if k.ends_with("/workouts") => {
            let page = page_of(query);
            (200, json!({"page": page, "workouts": workouts.iter().skip((page - 1) * 10).take(10).collect::<Vec<_>>()}))
        }
        _ => (404, json!({})),
    })
}

async fn call(app: &Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(if uri.contains("/mappings/") { Method::PUT } else { Method::POST })
        .uri(uri)
        .header(header::HOST, "127.0.0.1:8381")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-activsync-request", "1")
        .header("sec-fetch-site", "same-origin")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

impl Harness {
    fn config(&self, key: &str) -> Option<Value> {
        db::get_config_value(&self.db.lock(), key).unwrap()
    }

    fn hevy(&self, path: &str) -> Vec<String> {
        self.hevy_calls.lock().unwrap().iter().filter(|c| c.contains(path)).cloned().collect()
    }

    fn connect_key(&self) {
        db::set_config_value(&self.db.lock(), "hevy_api_key", &json!("key")).unwrap();
    }

    async fn wait_for_status(&self, hevy_id: &str, status: &str) {
        for _ in 0..200 {
            if hevy_db::get_workout(&self.db.lock(), hevy_id).unwrap().is_some_and(|w| w.status == status) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let found = hevy_db::get_workout(&self.db.lock(), hevy_id).unwrap().map(|w| w.status);
        panic!("{hevy_id} never reached {status}: {found:?}");
    }
}

#[tokio::test]
async fn a_rejected_key_is_not_saved() {
    let h = harness(true, Box::new(|_, _| (401, json!({"error": "Unauthorized"}))));
    let (status, body) = call(&h.app, "/api/v1/settings/hevy/credentials", json!({"apiKey": " bad "})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["detail"], "Hevy rejected that API key — check it and try again.");
    assert_eq!(h.config("hevy_api_key"), None);
    assert_eq!(h.hevy("/exercise_templates"), Vec::<String>::new());
}

#[tokio::test]
async fn an_unreachable_hevy_saves_nothing() {
    let h = harness(true, Box::new(|_, _| (400, json!({}))));
    let (status, body) = call(&h.app, "/api/v1/settings/hevy/credentials", json!({"apiKey": "key"})).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body["detail"].as_str().unwrap().starts_with("Could not reach Hevy to validate the key: "), "{body}");
    assert_eq!(h.config("hevy_api_key"), None);
}

#[tokio::test]
async fn a_good_key_in_setup_is_saved_and_caches_templates_best_effort() {
    let h = harness(
        false,
        Box::new(|key, query| match key {
            k if k.ends_with("/user/info") => (200, json!({"data": {"name": "lifter"}})),
            k if k.ends_with("/exercise_templates") && page_of(query) == 1 => (
                200,
                json!({"page_count": 2, "exercise_templates": [
                    {"id": "T-ROW", "title": "Row", "primary_muscle_group": "upper_back", "equipment": "barbell"},
                    {"title": "No id"},
                ]}),
            ),
            _ => (400, json!({})),
        }),
    );
    let (status, body) = call(&h.app, "/api/v1/setup/hevy", json!({"apiKey": " key "})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Hevy connected.");
    assert_eq!(h.config("hevy_api_key"), Some(json!("key")));
    assert_eq!(h.config("hevy_auth_ok"), Some(json!(true)));
    assert!(h.config("hevy_events_cursor").is_some());
    assert_eq!(h.config("setup_hevy_done"), Some(json!(true)));
    assert_eq!(h.config("settings").unwrap()["hevy_enabled"], json!(true));
    assert!(hevy_db::template_exists(&h.db.lock(), "T-ROW").unwrap());
    assert_eq!(h.hevy("/exercise_templates").len(), 2);
}

#[tokio::test]
async fn backfill_pages_through_history_bounded_by_the_account_total() {
    let h = harness(true, history_route(history(25), 200));
    h.connect_key();
    let (status, body) = call(&h.app, "/api/v1/settings/hevy/backfill/preview", json!({"since": "2026-09-01"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().unwrap().len(), 25);
    assert_eq!(body["message"], "25 workouts found, of 25 on your Hevy account. Nothing has been written yet.");
    assert_eq!(h.hevy("/workouts?").len(), 3);
    assert!(hevy_db::get_workout(&h.db.lock(), "h0").unwrap().is_none());
}

#[tokio::test]
async fn backfill_stops_at_the_start_date() {
    let h = harness(true, history_route(history(25), 200));
    h.connect_key();
    let (status, body) = call(&h.app, "/api/v1/settings/hevy/backfill/preview", json!({"since": "2026-10-20"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().unwrap().len(), 6);
    assert_eq!(h.hevy("/workouts?").len(), 1);
}

#[tokio::test]
async fn without_an_account_total_backfill_reads_one_page() {
    let h = harness(true, history_route(history(25), 400));
    h.connect_key();
    let (status, body) = call(&h.app, "/api/v1/settings/hevy/backfill/preview", json!({"since": "2026-09-01"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "10 workouts found. Nothing has been written yet.");
    assert_eq!(h.hevy("/workouts?").len(), 1);
}

#[tokio::test]
async fn backfill_reports_a_rejected_key_and_an_outage() {
    let h = harness(true, Box::new(|key, _| (if key.ends_with("/count") { 200 } else { 401 }, json!({"workout_count": 3}))));
    h.connect_key();
    let (status, body) = call(&h.app, "/api/v1/settings/hevy/backfill/preview", json!({"since": "2026-09-01"})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["detail"], "Hevy rejected the stored API key — reconnect Hevy.");

    let h = harness(true, Box::new(|_, _| (400, json!({}))));
    h.connect_key();
    let (status, body) = call(&h.app, "/api/v1/settings/hevy/backfill/run", json!({"since": "2026-09-01"})).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body["detail"].as_str().unwrap().starts_with("Could not fetch workouts from Hevy: "), "{body}");
}

#[tokio::test]
async fn an_imported_workout_with_its_watch_twin_is_merged_straight_away() {
    let w1 = workout("w1", "2026-11-10T08:00:00Z", bench());
    let h = harness(true, history_route(vec![w1], 200));
    h.connect_key();
    let (status, body) = call(&h.app, "/api/v1/settings/hevy/backfill/run", json!({"since": "2026-11-01"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"][0]["action"], "merge", "{body}");
    h.wait_for_status("w1", "merged").await;
    let calls = h.garmin_calls.lock().unwrap().clone();
    assert!(calls.iter().any(|c| c.starts_with("PUT /activity-service/activity/101/exerciseSets")), "{calls:?}");
}

#[tokio::test]
async fn a_saved_mapping_starts_the_workout_it_unblocks() {
    let exercise = json!({"title": "Mystery Machine", "exercise_template_id": "T-MYST", "sets": [{"type": "normal", "reps": 8}]});
    let h = harness(true, Box::new(|_, _| (404, json!({}))));
    h.connect_key();
    {
        let conn = h.db.lock();
        let payload = workout("w1", "2026-11-10T08:00:00Z", exercise);
        hevy_db::upsert_workout(&conn, "w1", "Workout w1", "2026-11-10T08:00:00Z", "2026-11-10T09:00:00Z", "2026-11-10T08:00:00Z", &payload)
            .unwrap();
        hevy_db::set_workout_status(&conn, "w1", "needs_mapping", Some("Map the missing exercises: Mystery Machine.")).unwrap();
        conn.execute(
            "INSERT INTO exercise_templates (exercise_template_id, title, secondary_muscle_groups, is_custom, fetched_at)
             VALUES ('T-MYST', 'Mystery Machine', '[]', 1, '2026-11-10T08:00:00+00:00')",
            [],
        )
        .unwrap();
    }
    let (status, body) = call(&h.app, "/api/v1/settings/hevy/mappings/T-MYST", json!({"category": 23, "subcategory": 0})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Exercise mapping saved. 1 waiting workout resumed.");
    h.wait_for_status("w1", "merged").await;
}
