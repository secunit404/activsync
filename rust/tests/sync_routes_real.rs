use std::sync::{Arc, Mutex};

use activsync::auth_guard::AuthConfig;
use activsync::db::{self, Db};
use activsync::garmin_client::{GarminClient, GarminError, Tokens};
use activsync::net::{Http, HttpError};
use activsync::poller::{BoxFuture, Clients};
use activsync::strava_client::StravaClient;
use activsync::{AppState, build_app, schedule};
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use openidconnect::HttpRequest;
use serde_json::{Value, json};
use tower::ServiceExt;

type Calls = Arc<Mutex<Vec<String>>>;

fn service(
    calls: &Calls,
    route: impl Fn(&str, &str) -> (u16, Value) + Send + Sync + 'static,
) -> Http {
    let log = calls.clone();
    Arc::new(move |request: HttpRequest| {
        let key = format!("{} {}", request.method(), request.uri().path());
        log.lock().unwrap().push(key.clone());
        let (status, body) = route(&key, request.uri().query().unwrap_or_default());
        let response = openidconnect::http::Response::builder()
            .status(status)
            .body(body.to_string().into_bytes())
            .map_err(|e| HttpError(e.to_string()));
        Box::pin(async move { response })
    })
}

struct Fake {
    db: Db,
    garmin_http: Http,
    strava_http: Http,
    garmin_down: bool,
}

impl Clients for Fake {
    fn garmin(&self) -> BoxFuture<'_, Result<GarminClient, GarminError>> {
        let result = if self.garmin_down {
            Err(GarminError::SessionExpired)
        } else {
            let tokens = Tokens {
                di_token: Some("token".into()),
                di_refresh_token: None,
                di_client_id: None,
            };
            Ok(GarminClient::new(
                self.garmin_http.clone(),
                Arc::new(|| Utc::now().timestamp() as f64),
                tokens,
                None,
            )
            .without_waits())
        };
        Box::pin(async move { result })
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
    app: Router,
    garmin_calls: Calls,
    strava_calls: Calls,
}

/// Real-account mode against stand-ins: Garmin has one new strength workout
/// from an hour ago; Strava has nothing, or rejects the token.
fn harness(onboarded: bool, garmin_down: bool, strava_rejects: bool) -> Harness {
    let db = Db::open_in_memory().unwrap();
    {
        let conn = db.lock();
        db::set_config_value(&conn, "initial_sync_done", &json!(onboarded)).unwrap();
        db::set_config_value(&conn, "garmin_credentials_verified", &json!(true)).unwrap();
        let tokens = json!({"access_token": "a", "refresh_token": "r", "expires_at": Utc::now().timestamp() + 21600,
                            "scope": "read,activity:read_all,activity:write"});
        db::set_config_value(&conn, "strava_tokens", &tokens).unwrap();
    }
    let start = (Utc::now() - Duration::hours(1))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let garmin_calls = Calls::default();
    let garmin_http = service(&garmin_calls, move |key, query| match key {
        k if k.starts_with("GET /activitylist-service/") && query.contains("start=0&") => (
            200,
            json!([{"activityId": 9001, "activityType": {"typeKey": "strength_training"}, "activityName": "Gym",
                    "startTimeGMT": start, "duration": 3600.0}]),
        ),
        k if k.starts_with("GET /activitylist-service/") => (200, json!([])),
        "GET /activity-service/activity/activityTypes" => (
            200,
            json!([{"typeKey": "running"}, {"typeKey": "strength_training"}, {"typeKey": "all"}]),
        ),
        other => panic!("unexpected Garmin call {other}"),
    });
    let strava_calls = Calls::default();
    let strava_http = service(&strava_calls, move |key, _| match key {
        "GET /api/v3/athlete/activities" if strava_rejects => {
            (401, json!({"message": "Authorization Error"}))
        }
        "GET /api/v3/athlete/activities" => (200, json!([])),
        other => panic!("unexpected Strava call {other}"),
    });
    let fake = Arc::new(Fake {
        db: db.clone(),
        garmin_http,
        strava_http,
        garmin_down,
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
        app: build_app(state, None),
        garmin_calls,
        strava_calls,
    }
}

async fn post(app: &Router, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::HOST, "127.0.0.1:8381")
        .header("x-activsync-request", "1")
        .header("sec-fetch-site", "same-origin")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}

impl Harness {
    fn activity_ids(&self) -> Vec<i64> {
        db::list_activities(&self.db.lock(), false)
            .unwrap()
            .iter()
            .map(|a| a.garmin_activity_id)
            .collect()
    }

    fn config(&self, key: &str) -> Option<Value> {
        db::get_config_value(&self.db.lock(), key).unwrap()
    }

    fn strava_reads(&self) -> usize {
        self.strava_calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.contains("athlete/activities"))
            .count()
    }
}

#[tokio::test]
async fn sync_now_reads_garmin_then_checks_strava() {
    let h = harness(true, false, false);
    let (status, body) = post(&h.app, "/api/v1/settings/sync/garmin").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Garmin synced.");
    assert_eq!(h.activity_ids(), vec![9001]);
    assert_eq!(h.config("garmin_last_sync_ok"), Some(json!(true)));
    assert_eq!(h.strava_reads(), 1);
}

#[tokio::test]
async fn sync_now_for_strava_reports_a_rejected_token() {
    let h = harness(true, false, true);
    let start = (Utc::now() - Duration::hours(2))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    h.db.lock()
        .execute(
            "INSERT INTO activities (garmin_activity_id, activity_type, title, start_time, content_hash, publish_status, first_seen_at)
             VALUES (77, 'running', 'Run', ?1, 'h', 'pending', ?1)",
            [start],
        )
        .unwrap();
    let (status, body) = post(&h.app, "/api/v1/settings/sync/strava").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["detail"].as_str().unwrap().contains("Strava"),
        "{body}"
    );
    assert!(h.garmin_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sync_now_with_garmin_signed_out_is_a_server_error() {
    let h = harness(true, true, false);
    let (status, _) = post(&h.app, "/api/v1/settings/sync/garmin").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(h.activity_ids().is_empty());
}

#[tokio::test]
async fn the_first_sync_holds_every_category_and_finishes_setup() {
    let h = harness(false, false, false);
    let (status, body) = post(&h.app, "/api/v1/setup/initial-sync").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["message"],
        "Setup complete. Your activities are ready."
    );
    assert_eq!(h.config("initial_sync_done"), Some(json!(true)));
    let types = h.config("garmin_activity_types").unwrap();
    assert_eq!(
        types,
        json!([{"type_key": "running", "label": "Running"}, {"type_key": "strength_training", "label": "Strength Training"}])
    );
    let settings = h.config("settings").unwrap();
    assert_eq!(
        settings["held_activity_types"],
        json!(["running", "strength_training"])
    );
    let activity = db::get_activity(&h.db.lock(), 9001).unwrap().unwrap();
    assert_eq!(activity.publish_status, "held");
}

#[tokio::test]
async fn a_failed_first_sync_keeps_setup_open() {
    let h = harness(false, true, false);
    let (status, body) = post(&h.app, "/api/v1/setup/initial-sync").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .starts_with("First sync failed: "),
        "{body}"
    );
    assert_eq!(h.config("initial_sync_done"), Some(json!(false)));
}

#[tokio::test]
async fn refreshing_categories_keeps_the_current_choices() {
    let h = harness(true, false, false);
    db::set_config_value(
        &h.db.lock(),
        "settings",
        &json!({"held_activity_types": ["running"]}),
    )
    .unwrap();
    let (status, body) = post(&h.app, "/api/v1/settings/activity-types/refresh").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        h.config("garmin_activity_types")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        h.config("settings").unwrap()["held_activity_types"],
        json!(["running"])
    );
}
