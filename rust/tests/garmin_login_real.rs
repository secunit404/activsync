use std::sync::{Arc, Mutex};
use std::time::Duration;

use activsync::auth_guard::AuthConfig;
use activsync::db::{self, Db};
use activsync::garmin_client::{Tokens, save_tokens};
use activsync::garmin_login::SignIn;
use activsync::net::{Http, HttpError};
use activsync::poller::RealClients;
use activsync::{AppState, build_app, schedule};
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use base64::Engine;
use chrono::Utc;
use http_body_util::BodyExt;
use openidconnect::HttpRequest;
use serde_json::{Value, json};
use tower::ServiceExt;

/// What the stand-in Garmin saw: path, Cookie header and body of each call.
type Calls = Arc<Mutex<Vec<(String, String, String)>>>;

fn jwt(claims: Value) -> String {
    let b64 = |v: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
    format!("{}.{}.signature", b64(json!({"alg": "RS256", "typ": "JWT"})), b64(claims))
}

#[derive(Clone, Copy)]
enum Sso {
    Ticket,
    Mfa,
    WrongPassword,
    RateLimited,
}

struct Garmin {
    sso: Sso,
    api_rejects_token: bool,
}

fn garmin_http(calls: &Calls, garmin: Garmin) -> Http {
    let log = calls.clone();
    Arc::new(move |request: HttpRequest| {
        let path = format!("{} {}", request.method(), request.uri().path());
        let cookie = request.headers().get(header::COOKIE).map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
        let body = String::from_utf8_lossy(request.body()).to_string();
        log.lock().unwrap().push((path.clone(), cookie, body.clone()));
        let mut response = openidconnect::http::Response::builder();
        let (status, reply) = match path.as_str() {
            "POST /mobile/api/login" => match garmin.sso {
                Sso::Ticket => (200, json!({"responseStatus": {"type": "SUCCESSFUL"}, "serviceTicketId": "ST-1"})),
                Sso::Mfa => {
                    response = response.header(header::SET_COOKIE, "SESSION=abc; Path=/; Secure").header(header::SET_COOKIE, "GARMIN-SSO=1");
                    (200, json!({"responseStatus": {"type": "MFA_REQUIRED"}, "customerMfaInfo": {"mfaLastMethodUsed": "sms"}}))
                }
                Sso::WrongPassword => (200, json!({"responseStatus": {"type": "INVALID_USERNAME_PASSWORD"}})),
                Sso::RateLimited => (429, json!({})),
            },
            "POST /mobile/api/mfa/verifyCode" | "POST /portal/api/mfa/verifyCode" if body.contains("\"123456\"") => {
                (200, json!({"responseStatus": {"type": "SUCCESSFUL"}, "serviceTicketId": "ST-2"}))
            }
            "POST /mobile/api/mfa/verifyCode" | "POST /portal/api/mfa/verifyCode" => {
                (200, json!({"responseStatus": {"type": "INVALID_MFA_CODE"}}))
            }
            "POST /di-oauth2-service/oauth/token" if body.contains("GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2") => (400, json!({})),
            "POST /di-oauth2-service/oauth/token" => (
                200,
                json!({"access_token": jwt(json!({"exp": Utc::now().timestamp() + 86400, "client_id": "FROM_JWT"})), "refresh_token": "r"}),
            ),
            "GET /userprofile-service/socialProfile" if garmin.api_rejects_token => (401, json!({})),
            "GET /userprofile-service/socialProfile" => (200, json!({"displayName": "runner"})),
            "GET /userprofile-service/userprofile/user-settings" => (200, json!({"userData": {}})),
            "GET /activity-service/activity/activityTypes" => (200, json!([{"typeKey": "running"}, {"typeKey": "all"}])),
            p if p.starts_with("GET /activitylist-service/") => (200, json!([])),
            other => panic!("unexpected Garmin call {other}"),
        };
        let response = response.status(status).body(reply.to_string().into_bytes()).map_err(|e| HttpError(e.to_string()));
        Box::pin(async move { response })
    })
}

struct Harness {
    db: Db,
    app: Router,
    calls: Calls,
    token_dir: String,
    _dir: tempfile::TempDir,
}

fn harness(onboarded: bool, garmin: Garmin) -> Harness {
    let db = Db::open_in_memory().unwrap();
    db::set_config_value(&db.lock(), "initial_sync_done", &json!(onboarded)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let token_dir = dir.path().canonicalize().unwrap().to_string_lossy().to_string();
    let calls = Calls::default();
    let http = garmin_http(&calls, garmin);
    let clients = Arc::new(RealClients {
        db: db.clone(),
        clock: schedule::system_clock(),
        garmin_http: http.clone(),
        strava_http: Arc::new(|_| panic!("Strava is not connected")),
        garmin_token_dir: token_dir.clone(),
    });
    let sign_in = SignIn { http, clock: schedule::system_clock(), token_dir: token_dir.clone(), pause: Duration::ZERO };
    let state = AppState::new(db.clone(), false, schedule::system_clock(), 0.0, AuthConfig::sign_in_off())
        .with_clients(clients)
        .with_garmin_sign_in(sign_in);
    Harness { db, app: build_app(state, None), calls, token_dir, _dir: dir }
}

async fn send(app: &Router, method: Method, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
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

    fn calls_to(&self, path: &str) -> Vec<(String, String, String)> {
        self.calls.lock().unwrap().iter().filter(|(p, _, _)| p == path).cloned().collect()
    }

    fn saved_session(&self) -> Option<Value> {
        let file = std::path::Path::new(&self.token_dir).join("garmin_tokens.json");
        std::fs::read_to_string(file).ok().map(|text| serde_json::from_str(&text).unwrap())
    }
}

const SETUP: &str = "/api/v1/setup/garmin";

fn setup_body() -> Value {
    json!({"email": " runner@example.com ", "password": "secret", "lookbackDays": 7})
}

#[tokio::test]
async fn a_password_sign_in_keeps_the_session_and_holds_every_category() {
    let h = harness(false, Garmin { sso: Sso::Ticket, api_rejects_token: false });
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["message"].as_str(), body["mfaRequired"].as_bool()), (Some("Garmin connected."), Some(false)));
    let exchanges = h.calls_to("POST /di-oauth2-service/oauth/token");
    assert_eq!(exchanges.len(), 2, "the first app identity is refused, the next one tried");
    assert!(exchanges[1].2.contains("service_ticket=ST-1"), "{}", exchanges[1].2);
    let session = h.saved_session().expect("session saved");
    assert_eq!((session["di_client_id"].as_str(), session["di_refresh_token"].as_str()), (Some("FROM_JWT"), Some("r")));
    assert_eq!(h.config("garmin_credentials"), Some(json!({"email": "runner@example.com", "password": "secret"})));
    assert_eq!(h.config("garmin_credentials_verified"), Some(json!(true)));
    assert_eq!(h.config("settings").unwrap()["held_activity_types"], json!(["running"]));
}

#[tokio::test]
async fn a_wrong_password_saves_nothing() {
    let h = harness(false, Garmin { sso: Sso::WrongPassword, api_rejects_token: false });
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["detail"], "Could not connect to Garmin: 401 Unauthorized (Invalid Username or Password)");
    assert_eq!(h.config("garmin_credentials"), None);
    assert!(h.saved_session().is_none());
}

#[tokio::test]
async fn rate_limiting_and_a_refused_token_are_reported() {
    let h = harness(false, Garmin { sso: Sso::RateLimited, api_rejects_token: false });
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        body["detail"],
        "Could not connect to Garmin: All login strategies rate limited (429). Try again later or check your IP/network."
    );

    let h = harness(false, Garmin { sso: Sso::Ticket, api_rejects_token: true });
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        body["detail"],
        "Could not connect to Garmin: Login failed: All login strategies exhausted: mobile+requests: token rejected by API tier"
    );
    assert!(h.saved_session().is_none());
}

#[tokio::test]
async fn a_verification_code_finishes_a_reconnect_and_a_wrong_one_can_be_retried() {
    let h = harness(true, Garmin { sso: Sso::Mfa, api_rejects_token: false });
    let reconnect = json!({"email": "runner@example.com", "password": "secret"});
    let (status, body) = send(&h.app, Method::POST, "/api/v1/settings/garmin/reconnect", reconnect).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["message"].as_str(), body["mfaRequired"].as_bool()), (Some("Garmin sent a verification code."), Some(true)));
    assert_eq!(h.config("garmin_credentials"), None);

    let (status, body) = send(&h.app, Method::POST, "/api/v1/garmin/mfa", json!({"code": "000000"})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let detail = body["detail"].as_str().unwrap();
    assert!(detail.starts_with("MFA verification failed: ['https://sso.garmin.com/mobile/api/mfa/verifyCode: INVALID_MFA_CODE', "), "{detail}");
    assert!(h.saved_session().is_none());

    let (status, body) = send(&h.app, Method::POST, "/api/v1/garmin/mfa", json!({"code": "123456"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Garmin verified and connected.");
    let verify = h.calls_to("POST /mobile/api/mfa/verifyCode");
    assert_eq!(verify.last().unwrap().1, "SESSION=abc; GARMIN-SSO=1");
    assert!(verify.last().unwrap().2.contains("\"mfaMethod\":\"sms\""), "{}", verify.last().unwrap().2);
    assert!(h.saved_session().is_some());
    assert_eq!(h.config("garmin_credentials"), Some(json!({"email": "runner@example.com", "password": "secret"})));
    assert!(!h.calls_to("GET /activitylist-service/activities/search/activities").is_empty(), "catch-up ran");

    let (status, _) = send(&h.app, Method::POST, "/api/v1/garmin/mfa", json!({"code": "123456"})).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_session_garmin_still_accepts_skips_the_password() {
    let h = harness(true, Garmin { sso: Sso::WrongPassword, api_rejects_token: false });
    let tokens = Tokens { di_token: Some(jwt(json!({"exp": Utc::now().timestamp() + 86400}))), di_refresh_token: None, di_client_id: None };
    save_tokens(&h.token_dir, &tokens).unwrap();
    let reconnect = json!({"email": "runner@example.com", "password": "anything"});
    let (status, body) = send(&h.app, Method::POST, "/api/v1/settings/garmin/reconnect", reconnect).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Garmin reconnected and catch-up sync started.");
    assert!(h.calls_to("POST /mobile/api/login").is_empty());
    assert!(!h.calls_to("GET /activitylist-service/activities/search/activities").is_empty(), "catch-up ran");
}
