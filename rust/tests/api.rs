use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use activsync::auth_guard::AuthConfig;
use activsync::db::{self, Db, NewActivity};
use activsync::{AppState, build_app, config, routes, schedule, seed};
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct TestClock(Arc<AtomicU64>);

impl TestClock {
    fn new(start: f64) -> Self {
        Self(Arc::new(AtomicU64::new(start.to_bits())))
    }

    fn clock(&self) -> schedule::Clock {
        let value = self.0.clone();
        Arc::new(move || f64::from_bits(value.load(Ordering::SeqCst)))
    }

    fn advance(&self, seconds: f64) {
        let now = f64::from_bits(self.0.load(Ordering::SeqCst));
        self.0.store((now + seconds).to_bits(), Ordering::SeqCst);
    }
}

struct Harness {
    state: AppState,
    app: Router,
    clock: TestClock,
}

fn connect_services(conn: &rusqlite::Connection) {
    for (key, value) in [
        ("garmin_credentials", json!({"email": "dev@example.com", "password": "dev-password"})),
        ("garmin_credentials_verified", json!(true)),
        ("garmin_last_sync_at", json!(db::iso(Utc::now()))),
        ("garmin_last_sync_ok", json!(true)),
        ("strava_credentials", json!({"client_id": "12345", "client_secret": "dev-secret"})),
        ("strava_tokens", json!({"refresh_token": "dev-refresh-token"})),
    ] {
        db::set_config_value(conn, key, &value).unwrap();
    }
}

fn harness() -> Harness {
    let db = Db::open_in_memory().unwrap();
    seed::seed(&mut db.lock(), Utc::now(), true).unwrap();
    connect_services(&db.lock());
    let clock = TestClock::new(1_800_000_000.0);
    let state = AppState::new(db, true, clock.clock(), schedule::UNDO_WINDOW_SECONDS, AuthConfig::sign_in_off()).with_configured_undo_window();
    let app = build_app(state.clone(), None);
    Harness { state, app, clock }
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
    let json = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    (status, json)
}

async fn get(app: &Router, uri: &str) -> Value {
    let (status, body) = call(app, Method::GET, uri, None).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

/// The seeded Hevy scenarios own these activities, and one of them blocks
/// publishing until its match is chosen.
const HEVY_DEMO_IDS: i64 = 910_000;

fn first_with_status(page: &Value, status: &str) -> i64 {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["publishStatus"] == status && item["garminActivityId"].as_i64() < Some(HEVY_DEMO_IDS))
        .unwrap_or_else(|| panic!("no {status} activity"))["garminActivityId"]
        .as_i64()
        .unwrap()
}

fn plain_db_with(rows: &[(i64, &str, &str)]) -> Db {
    let db = Db::open_in_memory().unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 10, 0, 0, 0).unwrap();
    for (id, start, data) in rows {
        db::insert_activity(
            &db.lock(),
            &NewActivity {
                garmin_activity_id: *id,
                activity_type: "running",
                title: "T",
                description: if *id == 1 { "D" } else { "" },
                start_time: start,
                content_hash: "h",
                publish_status: "pending",
                garmin_data: data,
                hold_reason: None,
            },
            now,
        )
        .unwrap();
    }
    db
}

#[tokio::test]
async fn health_reports_ok() {
    let h = harness();
    assert_eq!(get(&h.app, "/health").await, json!({"status": "ok"}));
}

#[tokio::test]
async fn live_refresh_events_carry_a_data_line() {
    let h = harness();
    let request = Request::get("/api/events").header("host", "127.0.0.1:8382").body(Body::empty()).unwrap();
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body();
    h.state.events.publish("refresh");
    let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(std::str::from_utf8(&frame).unwrap(), "event: refresh\ndata:  \n\n");
}

#[tokio::test]
async fn activity_detail_matches_python_formatting() {
    // Expected strings were produced by the Python app's view.activities_view
    // for the same rows, so both servers render a row identically.
    let db = plain_db_with(&[
        (1, "2026-07-09 09:00:00", r#"{"duration": 2520, "moving_duration": 2430, "elapsed_duration": 2700, "distance": 7850, "elevation_gain": 120, "elevation_loss": 110, "calories": 420, "avg_hr": 142, "max_hr": 168}"#),
        (2, "2026-03-29 00:30:00", r#"{"duration": 3725.6, "distance": 10012.345, "aerobic_te": 3.25, "anaerobic_te": 0.05, "training_load": 87.5, "avg_power": 211.9, "max_power": 0, "avg_cadence": 171.4, "total_sets": 15.0, "total_reps": 75, "total_volume": 4250.5, "calories": 0, "avg_hr": 0}"#),
        (3, "2026-12-31 23:59:59", r#"{"duration": 5, "distance": 5}"#),
    ]);
    let state = AppState::new(db, true, schedule::system_clock(), 5.0, AuthConfig::sign_in_off());
    let app = build_app(state, None);
    let page = get(&app, "/api/v1/activities?sort=oldest").await;
    let items = page["items"].as_array().unwrap();
    let ids: Vec<i64> = items.iter().map(|i| i["garminActivityId"].as_i64().unwrap()).collect();
    assert_eq!(ids, [2, 1, 3]);

    let second = &items[0];
    assert_eq!((second["startDateDisplay"].as_str(), second["startMonthYearDisplay"].as_str(), second["startClockDisplay"].as_str()),
               (Some("29 Mar"), Some("March 2026"), Some("01:30")));
    assert_eq!(second["detail"], json!({
        "description": null, "distance": "10.01 km", "duration": "1h 02m 05s", "movingTime": "",
        "elapsedTime": "", "pace": "6:12 /km", "speed": "9.7 km/h", "elevGain": "", "elevLoss": "",
        "calories": "", "avgHr": "0 bpm", "maxHr": "", "avgPower": "211 W", "maxPower": "",
        "normPower": "", "aerobicTe": "3.2", "anaerobicTe": "0.1", "trainingLoad": "88",
        "avgCadence": "171 spm", "maxCadence": "", "totalSets": "15.0", "totalReps": "75",
        "totalVolume": "4250 kg"
    }));

    let first = &items[1];
    assert_eq!((first["startDateDisplay"].as_str(), first["startClockDisplay"].as_str()), (Some("9 Jul"), Some("11:00")));
    assert_eq!(first["detail"], json!({
        "description": "D", "distance": "7.85 km", "duration": "42m 00s", "movingTime": "40m 30s",
        "elapsedTime": "45m 00s", "pace": "5:21 /km", "speed": "11.2 km/h", "elevGain": "120 m",
        "elevLoss": "110 m", "calories": "420", "avgHr": "142 bpm", "maxHr": "168 bpm", "avgPower": "",
        "maxPower": "", "normPower": "", "aerobicTe": "", "anaerobicTe": "", "trainingLoad": "",
        "avgCadence": "", "maxCadence": "", "totalSets": "", "totalReps": "", "totalVolume": ""
    }));
    assert_eq!(first["garminUrl"], "https://connect.garmin.com/modern/activity/1");
    assert_eq!(first["stravaUrl"], Value::Null);

    let third = &items[2];
    assert_eq!((third["startDateDisplay"].as_str(), third["startMonthYearDisplay"].as_str(), third["startClockDisplay"].as_str()),
               (Some("1 Jan"), Some("January 2027"), Some("00:59")));
    assert_eq!((third["detail"]["distance"].as_str(), third["detail"]["duration"].as_str(), third["detail"]["pace"].as_str(), third["detail"]["speed"].as_str()),
               (Some("0.01 km"), Some("0m 05s"), Some(""), Some("3.6 km/h")));
}

#[test]
fn coarse_duration_and_content_hash_match_python() {
    assert_eq!(activsync::view::fmt_duration_coarse(27720.9), "7h 42m");
    assert_eq!(activsync::view::fmt_duration_coarse(59.0), "0m");
    assert_eq!(
        routes::content_hash("Morning run", "Easy loop", "running", r#"{"duration": 2520}"#),
        "2789c07ea16f8a14772618696976c4b9c96dce2ac55906a97dadd8f128f124b1"
    );
}

#[tokio::test]
async fn seeded_list_counts_and_paginates() {
    let h = harness();
    let page = get(&h.app, "/api/v1/activities").await;
    let counts = &page["counts"];
    let total: i64 = ["pending", "held", "published", "missing", "excluded"]
        .iter()
        .map(|k| counts[k].as_i64().unwrap())
        .sum();
    assert_eq!(total, 39);
    assert_eq!(page["pagination"], json!({
        "page": 1, "pageSize": 20, "pageCount": 2, "totalCount": 39, "firstItem": 1, "lastItem": 20
    }));
    assert_eq!(page["sort"], "newest");
    assert_eq!(page["status"], Value::Null);

    let starts: Vec<&str> = page["items"].as_array().unwrap().iter().map(|i| i["startTime"].as_str().unwrap()).collect();
    assert!(starts.windows(2).all(|w| w[0] >= w[1]), "newest first");

    let last = get(&h.app, "/api/v1/activities?page=9&pageSize=10").await;
    assert_eq!(last["pagination"]["page"], 4);
    assert_eq!(last["pagination"]["firstItem"], 31);
    assert_eq!(last["pagination"]["lastItem"], 39);
}

#[tokio::test]
async fn review_filter_keeps_held_and_missing() {
    let h = harness();
    let page = get(&h.app, "/api/v1/activities?status=review&pageSize=100").await;
    assert_eq!(page["status"], "review");
    let items = page["items"].as_array().unwrap();
    assert!(!items.is_empty());
    assert!(items.iter().all(|i| i["publishStatus"] == "held" || i["publishStatus"] == "missing"));
    let reasons: Vec<&Value> = items.iter().map(|i| &i["holdReason"]).collect();
    assert!(reasons.contains(&&json!("backlog")) && reasons.contains(&&json!("category")));
}

#[tokio::test]
async fn bad_query_values_are_rejected() {
    let h = harness();
    for uri in [
        "/api/v1/activities?pageSize=25",
        "/api/v1/activities?pageSize=abc",
        "/api/v1/activities?page=0",
        "/api/v1/activities?sort=sideways",
        "/api/v1/activities?status=deleted",
    ] {
        let (status, body) = call(&h.app, Method::GET, uri, None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
        assert!(body["detail"].is_string());
    }
}

#[tokio::test]
async fn app_state_reports_onboarded_mock_install() {
    let h = harness();
    let state = get(&h.app, "/api/v1/app").await;
    assert_eq!(state["name"], "ActivSync");
    assert_eq!(state["development"], true);
    assert_eq!(state["setup"], json!({"complete": true, "step": null}));
    assert_eq!(state["connections"]["garmin"]["connected"], true);
    assert_eq!(state["connections"]["garmin"]["email"], "dev@example.com");
    assert_eq!(state["connections"]["strava"]["status"], "Connected");
    assert_eq!(state["connections"]["broken"], json!([]));
    assert_eq!(state["hevy"], json!({"enabled": false, "connected": false, "status": "Not connected"}));
    assert_eq!(state["pollIntervals"], json!({"garminMinutes": 20, "stravaMinutes": 5}));
    assert_eq!(state["catchUpReport"], Value::Null);
    assert!(state["reviewCount"].as_i64().unwrap() > 0);
    assert_eq!(state["update"]["available"], false);
    assert!(!state["version"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn auth_status_says_sign_in_is_off() {
    let h = harness();
    let status = get(&h.app, "/api/v1/auth/status").await;
    assert_eq!(status["authenticated"], true);
    assert_eq!(status["method"], "disabled");
    assert_eq!(status["authDisabled"], true);
}

#[tokio::test]
async fn catch_up_report_can_be_dismissed() {
    let h = harness();
    db::set_config_value(&h.state.db.lock(), "catch_up_report", &json!({"new": 2, "held": 1, "linked": 0, "days": 9})).unwrap();
    assert_eq!(get(&h.app, "/api/v1/app").await["catchUpReport"]["new"], 2);
    let (status, _) = call(&h.app, Method::DELETE, "/api/v1/catch-up-report", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(get(&h.app, "/api/v1/app").await["catchUpReport"], Value::Null);
}

#[tokio::test]
async fn edit_saves_trimmed_title_and_rehashes() {
    let h = harness();
    let page = get(&h.app, "/api/v1/activities").await;
    let id = first_with_status(&page, "pending");
    let (status, body) = call(&h.app, Method::PUT, &format!("/api/v1/activities/{id}"),
                              Some(json!({"title": "  New title ", "description": " Notes "}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["message"].as_str().unwrap().starts_with("Saved changes to "));
    let row = db::get_activity(&h.state.db.lock(), id).unwrap().unwrap();
    assert_eq!((row.title.as_str(), row.description.as_str()), ("New title", "Notes"));
    assert_eq!(row.content_hash, routes::content_hash("New title", "Notes", &row.activity_type, &row.garmin_data));

    let (status, body) = call(&h.app, Method::PUT, &format!("/api/v1/activities/{id}"), Some(json!({"title": "   "}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["detail"], "Activity title cannot be blank");

    let (status, _) = call(&h.app, Method::PUT, "/api/v1/activities/1", Some(json!({"title": "x"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn exclude_then_restore_returns_row_where_it_was() {
    let h = harness();
    let page = get(&h.app, "/api/v1/activities?status=held").await;
    let id = page["items"][0]["garminActivityId"].as_i64().unwrap();
    let reason = page["items"][0]["holdReason"].clone();

    let (status, body) = call(&h.app, Method::POST, &format!("/api/v1/activities/{id}/exclude"), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(db::get_activity(&h.state.db.lock(), id).unwrap().unwrap().publish_status, "excluded");

    let (status, _) = call(&h.app, Method::POST, &format!("/api/v1/activities/{id}/exclude"), None).await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, body) = call(&h.app, Method::POST, &format!("/api/v1/activities/{id}/restore"),
                              Some(json!({"status": "held", "holdReason": reason}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = db::get_activity(&h.state.db.lock(), id).unwrap().unwrap();
    assert_eq!(row.publish_status, "held");
    assert_eq!(json!(row.hold_reason), reason);

    let (status, body) = call(&h.app, Method::POST, &format!("/api/v1/activities/{id}/restore"), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["detail"], "Activity is not excluded");
}

#[tokio::test]
async fn restore_without_body_follows_held_categories() {
    let h = harness();
    let page = get(&h.app, "/api/v1/activities?status=excluded").await;
    let id = page["items"][0]["garminActivityId"].as_i64().unwrap();
    let (status, _) = call(&h.app, Method::POST, &format!("/api/v1/activities/{id}/restore"), None).await;
    assert_eq!(status, StatusCode::OK);
    let row = db::get_activity(&h.state.db.lock(), id).unwrap().unwrap();
    assert_eq!(row.publish_status, "pending", "walking and yoga are not held categories");
}

fn set_undo_delay(h: &Harness, seconds: i64) {
    config::edit(&h.state.db.lock(), false, |cfg| cfg.set("undo_delay_seconds", json!(seconds))).unwrap();
}

#[tokio::test]
async fn the_configured_undo_delay_sets_the_window_and_deadline() {
    for delay in [0, 3, 5, 10] {
        let h = harness();
        set_undo_delay(&h, delay);
        let id = first_with_status(&get(&h.app, "/api/v1/activities?status=pending").await, "pending");

        let (status, body) = call(&h.app, Method::POST, &format!("/api/v1/activities/{id}/publish/schedule"), None).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["undoWindowMs"], delay * 1000);
        assert_eq!(body["remainingMs"], delay * 1000);
        assert_eq!(body["publishScheduledAt"], schedule::deadline_iso(1_800_000_000.0 + delay as f64));
    }
}

#[tokio::test]
async fn a_zero_undo_delay_sends_on_the_next_pass() {
    let h = harness();
    set_undo_delay(&h, 0);
    let id = first_with_status(&get(&h.app, "/api/v1/activities?status=pending").await, "pending");

    let (_, scheduled) = call(&h.app, Method::POST, &format!("/api/v1/activities/{id}/publish/schedule"), None).await;
    let batch = scheduled["batchId"].as_str().unwrap().to_string();

    assert_eq!(h.state.scheduler.run_due().await.unwrap(), 1);
    assert_eq!(get(&h.app, &format!("/api/v1/activities/publish/batches/{batch}")).await["state"], "done");
}

#[tokio::test]
async fn publish_waits_out_undo_window_then_sends() {
    let h = harness();
    let id = first_with_status(&get(&h.app, "/api/v1/activities?status=pending").await, "pending");

    let (status, scheduled) = call(&h.app, Method::POST, &format!("/api/v1/activities/{id}/publish/schedule"), None).await;
    assert_eq!(status, StatusCode::OK, "{scheduled}");
    assert_eq!(scheduled["remainingMs"], 5000);
    assert_eq!(scheduled["undoWindowMs"], 5000);
    let batch = scheduled["batchId"].as_str().unwrap().to_string();

    let listed = get(&h.app, "/api/v1/activities?status=pending&pageSize=100").await;
    let item = listed["items"].as_array().unwrap().iter().find(|i| i["garminActivityId"] == id).unwrap().clone();
    assert_eq!(item["publishPhase"], "waiting");
    assert_eq!(item["publishScheduledAt"], scheduled["publishScheduledAt"]);

    let (status, _) = call(&h.app, Method::POST, &format!("/api/v1/activities/{id}/publish/schedule"), None).await;
    assert_eq!(status, StatusCode::CONFLICT);

    assert_eq!(h.state.scheduler.run_due().await.unwrap(), 0, "not due yet");
    assert_eq!(get(&h.app, &format!("/api/v1/activities/publish/batches/{batch}")).await["state"], "waiting");

    h.clock.advance(5.0);
    assert_eq!(h.state.scheduler.run_due().await.unwrap(), 1);
    let done = get(&h.app, &format!("/api/v1/activities/publish/batches/{batch}")).await;
    assert_eq!(done["state"], "done");
    let row = db::get_activity(&h.state.db.lock(), id).unwrap().unwrap();
    assert!(row.is_published());
    assert_eq!(done["result"]["message"], format!("Published {} to Strava", row.title));

    let after = get(&h.app, "/api/v1/activities?status=published&pageSize=100").await;
    let item = after["items"].as_array().unwrap().iter().find(|i| i["garminActivityId"] == id).unwrap().clone();
    assert_eq!(item["stravaUrl"], format!("https://www.strava.com/activities/{}", row.strava_activity_id.unwrap()));
    assert_eq!(item["publishPhase"], Value::Null);
}

#[tokio::test]
async fn undo_inside_window_sends_nothing() {
    let h = harness();
    let pending = get(&h.app, "/api/v1/activities?status=pending&pageSize=100").await;
    let ids: Vec<i64> = pending["items"].as_array().unwrap().iter().take(3).map(|i| i["garminActivityId"].as_i64().unwrap()).collect();

    let (status, scheduled) = call(&h.app, Method::POST, "/api/v1/activities/publish/schedule", Some(json!({"activityIds": ids}))).await;
    assert_eq!(status, StatusCode::OK, "{scheduled}");

    let (status, body) = call(&h.app, Method::POST, "/api/v1/activities/publish/cancel", Some(json!({"activityIds": ids}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["message"], "Nothing was sent to Strava.");

    h.clock.advance(10.0);
    assert_eq!(h.state.scheduler.run_due().await.unwrap(), 0);
    for id in &ids {
        assert_eq!(db::get_activity(&h.state.db.lock(), *id).unwrap().unwrap().publish_status, "pending");
    }
    let (status, body) = call(&h.app, Method::POST, &format!("/api/v1/activities/{}/publish/cancel", ids[0]), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["detail"], "Nothing is waiting to publish.");

    let (status, _) = call(&h.app, Method::GET, &format!("/api/v1/activities/publish/batches/{}", scheduled["batchId"].as_str().unwrap()), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bulk_publish_reports_hevy_block_and_skips() {
    let h = harness();
    let pending = get(&h.app, "/api/v1/activities?status=pending&pageSize=100").await;
    let ids: Vec<i64> = pending["items"].as_array().unwrap().iter()
        .filter(|i| i["garminActivityId"].as_i64() < Some(HEVY_DEMO_IDS))
        .take(3)
        .map(|i| i["garminActivityId"].as_i64().unwrap())
        .collect();
    {
        let conn = h.state.db.lock();
        conn.execute(
            "INSERT INTO hevy_workouts (hevy_id, start_time, end_time, status, source_garmin_activity_id, created_at, updated_at)
             VALUES ('w1', 'x', 'y', 'awaiting_match', ?1, 'now', 'now')",
            [ids[0]],
        ).unwrap();
    }
    let (_, scheduled) = call(&h.app, Method::POST, "/api/v1/activities/publish/schedule", Some(json!({"activityIds": ids}))).await;
    let batch = scheduled["batchId"].as_str().unwrap().to_string();
    db::set_publish_status(&h.state.db.lock(), ids[1], "excluded", None).unwrap();

    h.clock.advance(6.0);
    assert_eq!(h.state.scheduler.run_due().await.unwrap(), 3);
    let done = get(&h.app, &format!("/api/v1/activities/publish/batches/{batch}")).await;
    assert_eq!(done["result"], json!({
        "message": "Published 1; 1 blocked by Hevy sync; 1 no longer publishable",
        "severity": "warning", "publishedCount": 1, "failedCount": 0, "blockedCount": 1
    }));
}

#[tokio::test]
async fn publish_needs_both_services_connected() {
    let h = harness();
    db::set_config_value(&h.state.db.lock(), "strava_tokens", &Value::Null).unwrap();
    let id = first_with_status(&get(&h.app, "/api/v1/activities?status=pending").await, "pending");
    let (status, body) = call(&h.app, Method::POST, &format!("/api/v1/activities/{id}/publish/schedule"), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["detail"], "Both Garmin and Strava must be connected to publish. Reconnect to continue.");
    assert_eq!(get(&h.app, "/api/v1/app").await["connections"]["broken"], json!(["strava"]));
}

#[tokio::test]
async fn hevy_merge_shows_badge() {
    let h = harness();
    let page = get(&h.app, "/api/v1/activities?pageSize=100").await;
    let badges: Vec<(i64, &str)> = page["items"].as_array().unwrap().iter()
        .filter_map(|i| Some((i["garminActivityId"].as_i64()?, i["hevyBadge"].as_str()?)))
        .collect();
    assert_eq!(badges, [(910001, "merge"), (910050, "passive")]);
}

#[tokio::test]
async fn unknown_api_paths_are_404_json() {
    let h = harness();
    let (status, body) = call(&h.app, Method::GET, "/api/v1/nothing/here", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["detail"], "Not Found");
}

#[tokio::test]
async fn spa_paths_serve_index_html() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>spa</html>").unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/app.js"), "console.log(1)").unwrap();
    let h = harness();
    let app = build_app(h.state.clone(), Some(dir.path()));
    let fetch = |uri: &'static str| {
        let app = app.clone();
        async move {
            let request = Request::get(uri).header("host", "127.0.0.1:8382").body(Body::empty()).unwrap();
            let (parts, body) = app.oneshot(request).await.unwrap().into_parts();
            (parts, body.collect().await.unwrap().to_bytes())
        }
    };

    for uri in ["/", "/activities", "/settings/hevy", "/robots.txt"] {
        let (parts, body) = fetch(uri).await;
        assert_eq!(parts.status, StatusCode::OK, "{uri}");
        assert_eq!(body, "<html>spa</html>".as_bytes(), "{uri}");
        assert_eq!(parts.headers[header::CONTENT_TYPE], "text/html; charset=utf-8", "{uri}");
        assert_eq!(parts.headers[header::CACHE_CONTROL], "no-cache", "{uri}");
    }

    let (parts, body) = fetch("/assets/app.js").await;
    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(body, "console.log(1)".as_bytes());
    assert!(!parts.headers.contains_key(header::CACHE_CONTROL), "hashed assets are not revalidated on every load");
    assert!(parts.headers[header::CONTENT_TYPE].to_str().unwrap().ends_with("; charset=utf-8"));

    for uri in ["/api", "/api/", "/api/nothing", "/assets", "/assets/missing.js", "/strava/other", "/auth", "/auth/other"] {
        let (parts, body) = fetch(uri).await;
        assert_eq!(parts.status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), json!({"detail": "Not Found"}), "{uri}");
        assert!(!parts.headers.contains_key(header::CACHE_CONTROL), "{uri}");
    }
}

#[tokio::test]
async fn every_unknown_path_is_404_without_a_frontend_build() {
    let h = harness();
    let request = Request::get("/activities").header("host", "127.0.0.1:8382").body(Body::empty()).unwrap();
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn seed_refuses_a_database_with_real_activities() {
    let db = plain_db_with(&[(123, "2026-07-09 09:00:00", "{}")]);
    assert!(!seed::seed(&mut db.lock(), Utc::now(), true).unwrap());
    let rows = db::list_activities(&db.lock(), false).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].garmin_activity_id, 123);
}

#[test]
fn reseeding_the_same_version_keeps_edits() {
    let db = Db::open_in_memory().unwrap();
    seed::seed(&mut db.lock(), Utc::now(), true).unwrap();
    db::update_activity_metadata(&db.lock(), 900001, "Kept", "", "dev-900001").unwrap();
    seed::seed(&mut db.lock(), Utc::now(), true).unwrap();
    assert_eq!(db::get_activity(&db.lock(), 900001).unwrap().unwrap().title, "Kept");
}

#[test]
fn deadline_iso_matches_python_isoformat() {
    assert_eq!(schedule::deadline_iso(1_800_000_005.0), "2027-01-15T08:00:05+00:00");
    assert_eq!(schedule::deadline_iso(1_800_000_005.25), "2027-01-15T08:00:05.250000+00:00");
}
