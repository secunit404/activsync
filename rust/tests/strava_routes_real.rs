use std::sync::{Arc, Mutex};

use activsync::auth_guard::AuthConfig;
use activsync::db::{self, Db};
use activsync::garmin_client::{Tokens, save_tokens};
use activsync::net::{Http, HttpError};
use activsync::poller::RealClients;
use activsync::{AppState, build_app, schedule};
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use base64::Engine;
use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use openidconnect::HttpRequest;
use serde_json::{Value, json};
use tower::ServiceExt;

type Calls = Arc<Mutex<Vec<String>>>;

const SCOPE: &str = "read,activity:read_all,activity:write";

fn service(
    calls: &Calls,
    route: impl Fn(&str, &HttpRequest) -> (u16, Value) + Send + Sync + 'static,
) -> Http {
    let log = calls.clone();
    Arc::new(move |request: HttpRequest| {
        let key = format!("{} {}", request.method(), request.uri().path());
        log.lock().unwrap().push(key.clone());
        let (status, body) = route(&key, &request);
        let response = openidconnect::http::Response::builder()
            .status(status)
            .body(body.to_string().into_bytes())
            .map_err(|e| HttpError(e.to_string()));
        Box::pin(async move { response })
    })
}

fn jwt(claims: Value) -> String {
    let b64 = |v: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.signature",
        b64(json!({"alg": "RS256", "typ": "JWT"})),
        b64(claims)
    )
}

struct Harness {
    db: Db,
    app: Router,
    host: &'static str,
    garmin_calls: Calls,
    token_requests: Arc<Mutex<Vec<String>>>,
    _dir: tempfile::TempDir,
}

fn harness(onboarded: bool, token_status: u16, public_url: Option<&str>) -> Harness {
    let db = Db::open_in_memory().unwrap();
    {
        let conn = db.lock();
        db::set_config_value(&conn, "initial_sync_done", &json!(onboarded)).unwrap();
        db::set_config_value(&conn, "setup_hevy_done", &json!(true)).unwrap();
        db::set_config_value(&conn, "garmin_credentials_verified", &json!(true)).unwrap();
        db::set_config_value(
            &conn,
            "strava_credentials",
            &json!({"client_id": "4242", "client_secret": "shh"}),
        )
        .unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let token_dir = dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let tokens = Tokens {
        di_token: Some(jwt(json!({"exp": Utc::now().timestamp() + 86400}))),
        di_refresh_token: None,
        di_client_id: None,
    };
    save_tokens(&token_dir, &tokens).unwrap();

    let start = (Utc::now() - Duration::hours(1))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let garmin_calls = Calls::default();
    let garmin_http = service(&garmin_calls, move |key, request| match key {
        "GET /userprofile-service/socialProfile" => (200, json!({"displayName": "runner"})),
        "GET /userprofile-service/userprofile/user-settings" => (200, json!({"userData": {}})),
        k if k.starts_with("GET /activitylist-service/")
            && request
                .uri()
                .query()
                .unwrap_or_default()
                .contains("start=0&") =>
        {
            (
                200,
                json!([{"activityId": 9001, "activityType": {"typeKey": "running"}, "activityName": "Run", "startTimeGMT": start, "duration": 1800.0}]),
            )
        }
        k if k.starts_with("GET /activitylist-service/") => (200, json!([])),
        _ => (200, json!({})),
    });
    let token_requests = Arc::new(Mutex::new(Vec::new()));
    let seen = token_requests.clone();
    let strava_http = service(&Calls::default(), move |key, request| match key {
        "POST /oauth/token" => {
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(request.body()).to_string());
            let tokens = json!({"access_token": "fresh", "refresh_token": "fresh-r", "expires_at": Utc::now().timestamp() + 21600});
            (
                token_status,
                if token_status == 200 {
                    tokens
                } else {
                    json!({"message": "Bad Request"})
                },
            )
        }
        k if k.starts_with("GET /api/v3/athlete/activities") => (200, json!([])),
        other => panic!("unexpected Strava call {other}"),
    });
    let clients = Arc::new(RealClients {
        db: db.clone(),
        clock: schedule::system_clock(),
        garmin_http,
        strava_http,
        garmin_token_dir: token_dir,
    });
    let auth = AuthConfig {
        public_url: public_url.map(str::to_string),
        ..AuthConfig::sign_in_off()
    };
    let state =
        AppState::new(db.clone(), false, schedule::system_clock(), 0.0, auth).with_clients(clients);
    let host = if public_url.is_some() {
        "sync.example.com"
    } else {
        "127.0.0.1:8381"
    };
    Harness {
        db,
        app: build_app(state, None),
        host,
        garmin_calls,
        token_requests,
        _dir: dir,
    }
}

async fn send(
    app: &Router,
    host: &str,
    method: Method,
    uri: &str,
) -> (StatusCode, Value, Option<String>) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, host)
        .header("x-activsync-request", "1")
        .header("sec-fetch-site", "same-origin")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        location,
    )
}

impl Harness {
    async fn authorize(&self) -> String {
        let (status, body, _) = send(
            &self.app,
            self.host,
            Method::POST,
            "/api/v1/settings/strava/authorize",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["url"].as_str().unwrap().to_string()
    }

    fn state_token(&self) -> String {
        db::get_config_value(&self.db.lock(), "strava_oauth_state")
            .unwrap()
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn callback(&self, scope: &str) -> String {
        let uri = format!(
            "/strava/callback?code=abc&state={}&scope={}",
            self.state_token(),
            scope.replace(',', "%2C")
        );
        let (status, _, location) = send(&self.app, self.host, Method::GET, &uri).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        location.unwrap()
    }

    fn config(&self, key: &str) -> Option<Value> {
        db::get_config_value(&self.db.lock(), key).unwrap()
    }
}

#[tokio::test]
async fn the_link_uses_the_public_url_and_the_saved_client_id() {
    let h = harness(true, 200, Some("https://sync.example.com"));
    let url = h.authorize().await;
    assert!(
        url.starts_with("https://www.strava.com/oauth/authorize?client_id=4242&"),
        "{url}"
    );
    assert!(
        url.contains("redirect_uri=https://sync.example.com/strava/callback&"),
        "{url}"
    );
    assert!(
        url.ends_with(&format!("&state={}", h.state_token())),
        "{url}"
    );
}

#[tokio::test]
async fn reconnecting_exchanges_the_code_and_catches_up() {
    let h = harness(true, 200, None);
    let url = h.authorize().await;
    assert!(
        url.contains("redirect_uri=http://127.0.0.1:8381/strava/callback&"),
        "{url}"
    );
    assert_eq!(h.callback(SCOPE).await, "/settings");
    let tokens = h.config("strava_tokens").unwrap();
    assert_eq!(
        (tokens["access_token"].as_str(), tokens["scope"].as_str()),
        (Some("fresh"), Some(SCOPE))
    );
    let exchange = h.token_requests.lock().unwrap()[0].clone();
    assert!(
        exchange.contains("code=abc")
            && exchange.contains("client_id=4242")
            && exchange.contains("grant_type=authorization_code"),
        "{exchange}"
    );
    assert_eq!(h.config("strava_oauth_state"), Some(Value::Null));
    let report = h.config("catch_up_report").unwrap();
    assert_eq!(report["new"], 1, "{report}");
    assert!(db::get_activity(&h.db.lock(), 9001).unwrap().is_some());
}

#[tokio::test]
async fn during_setup_the_callback_returns_to_the_wizard_without_syncing() {
    let h = harness(false, 200, None);
    h.authorize().await;
    assert_eq!(h.callback(SCOPE).await, "/setup");
    assert!(h.config("strava_tokens").is_some());
    assert!(h.garmin_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_refused_exchange_connects_nothing() {
    let h = harness(true, 400, None);
    h.authorize().await;
    let location = h.callback(SCOPE).await;
    assert!(
        location.starts_with("/settings?stravaError=Could+not+complete+the+Strava+connection."),
        "{location}"
    );
    assert_eq!(h.config("strava_tokens"), None);
}

#[tokio::test]
async fn read_only_access_is_sent_back_to_connect_again() {
    let h = harness(true, 200, None);
    h.authorize().await;
    let location = h.callback("read").await;
    assert!(
        location.starts_with("/settings?stravaError=Strava+didn%27t+grant+upload+access."),
        "{location}"
    );
    assert!(h.garmin_calls.lock().unwrap().is_empty());
}
