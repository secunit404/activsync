use std::sync::{Arc, Mutex};
use std::time::Duration;

use activsync::auth_guard::AuthConfig;
use activsync::db::{self, Db};
use activsync::garmin_client::{Tokens, save_tokens};
use activsync::garmin_login::SignIn;
use activsync::net::{Browser, Http, HttpError};
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

/// What the stand-in Garmin saw: path, Cookie header, body and the browser
/// each call was made to look like.
type Calls = Arc<Mutex<Vec<(String, String, String, Option<Browser>)>>>;

fn jwt(claims: Value) -> String {
    let b64 = |v: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.signature",
        b64(json!({"alg": "RS256", "typ": "JWT"})),
        b64(claims)
    )
}

#[derive(Clone, Copy)]
enum Sso {
    Ticket,
    Mfa,
    WrongPassword,
    RateLimited,
    Blocked,
    MfaAfterRateLimit,
}

#[derive(Clone, Copy)]
enum Widget {
    Ticket,
    RedirectedTicket,
    Mfa,
    Locked,
    RateLimited,
    Blocked,
}

#[derive(Clone, Copy)]
struct Garmin {
    mobile: Sso,
    widget: Widget,
    portal: Sso,
    api_rejects_token: bool,
}

/// Only the mobile route answers; the backup routes are blocked.
fn mobile(sso: Sso) -> Garmin {
    Garmin {
        mobile: sso,
        widget: Widget::Blocked,
        portal: Sso::Blocked,
        api_rejects_token: false,
    }
}

fn json_sso(
    sso: Sso,
    ticket: &str,
    browser: Option<Browser>,
    response: &mut Option<openidconnect::http::response::Builder>,
) -> (u16, String) {
    let sso = match sso {
        Sso::MfaAfterRateLimit if browser == Some(Browser::SafariIos) => Sso::RateLimited,
        Sso::MfaAfterRateLimit => Sso::Mfa,
        other => other,
    };
    let reply = match sso {
        Sso::Ticket => json!({"responseStatus": {"type": "SUCCESSFUL"}, "serviceTicketId": ticket}),
        Sso::Mfa => {
            let builder = response.take().unwrap();
            *response = Some(
                builder
                    .header(header::SET_COOKIE, "SESSION=abc; Path=/; Secure")
                    .header(header::SET_COOKIE, "GARMIN-SSO=1"),
            );
            json!({"responseStatus": {"type": "MFA_REQUIRED"}, "customerMfaInfo": {"mfaLastMethodUsed": "sms"}})
        }
        Sso::WrongPassword => json!({"responseStatus": {"type": "INVALID_USERNAME_PASSWORD"}}),
        Sso::RateLimited => return (429, "{}".into()),
        Sso::Blocked => return (403, "<html>Just a moment...</html>".into()),
        Sso::MfaAfterRateLimit => unreachable!(),
    };
    (200, reply.to_string())
}

const WIDGET_MFA_PAGE: &str = r#"<html><head><title>GARMIN Authentication Application</title></head>
<script>var customerGuid = "g-1"; var mfaMethod = "email"; var clientId = "GarminConnect"; var locale = "en";</script>
<input type="hidden" name="_csrf" value="tok-2" /></html>"#;

fn garmin_http(calls: &Calls, garmin: Garmin) -> Http {
    let log = calls.clone();
    Arc::new(move |request: HttpRequest| {
        let path = format!("{} {}", request.method(), request.uri().path());
        let query = request.uri().query().unwrap_or_default().to_string();
        let cookie = request
            .headers()
            .get(header::COOKIE)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let body = String::from_utf8_lossy(request.body()).to_string();
        let browser = request.extensions().get::<Browser>().copied();
        log.lock()
            .unwrap()
            .push((path.clone(), cookie, body.clone(), browser));
        if path.contains(" /portal/") || path.contains(" /mobile/") {
            assert!(
                !request.headers().contains_key(header::USER_AGENT),
                "the User-Agent is left to the browser the request presents as"
            );
        }
        let mut response = Some(openidconnect::http::Response::builder());
        let json = |value: Value| (200, value.to_string());
        let html = |text: &str| (200, text.to_string());
        let (status, reply) = match path.as_str() {
            "POST /mobile/api/login" => json_sso(garmin.mobile, "ST-1", browser, &mut response),
            "POST /portal/api/login" => json_sso(garmin.portal, "ST-P", browser, &mut response),
            "GET /portal/sso/en-US/sign-in" => html("<html>sign in</html>"),
            "GET /sso/embed" => match garmin.widget {
                Widget::RateLimited => (429, String::new()),
                Widget::Blocked => (403, String::new()),
                _ if query.contains("hop=2") => {
                    response = response.map(|r| r.header(header::SET_COOKIE, "CF=2; Path=/"));
                    html("<html>widget</html>")
                }
                _ => {
                    response = response.map(|r| {
                        r.header(header::SET_COOKIE, "EMBED=1; Path=/")
                            .header(header::LOCATION, "/sso/embed?hop=2")
                    });
                    (302, String::new())
                }
            },
            "GET /sso/signin" if query.contains("done=1") => html(
                r#"<title>Success</title><a href="https://sso.garmin.com/sso/embed?ticket=ST-R">go</a>"#,
            ),
            "GET /sso/signin" => {
                html(r#"<form><input type="hidden" name="_csrf" value="tok-1" /></form>"#)
            }
            "POST /sso/signin" => match garmin.widget {
                Widget::Ticket => html(
                    r#"<title>Success</title><a href="https://sso.garmin.com/sso/embed?ticket=ST-W">go</a>"#,
                ),
                Widget::RedirectedTicket => {
                    response = response.map(|r| {
                        r.header(header::SET_COOKIE, "SSO=3; Path=/")
                            .header(header::LOCATION, "/sso/signin?done=1")
                    });
                    (302, String::new())
                }
                Widget::Mfa => html(WIDGET_MFA_PAGE),
                Widget::Locked => html("<title>Account Locked</title>"),
                Widget::RateLimited | Widget::Blocked => {
                    panic!("widget sign-in posted after a refused page")
                }
            },
            "POST /sso/verifyMFA/mfaCode" => json(json!({})),
            "POST /sso/verifyMFA/loginEnterMfaCode" if body.contains("mfa-code=123456") => {
                response = response.map(|r| r.header(header::LOCATION, "/sso/verifyMFA/done"));
                (303, String::new())
            }
            "GET /sso/verifyMFA/done" => html(
                r#"<title>Success</title><a href="https://sso.garmin.com/sso/embed?ticket=ST-WM">go</a>"#,
            ),
            "POST /sso/verifyMFA/loginEnterMfaCode" => html("<title>Invalid code</title>"),
            "POST /mobile/api/mfa/verifyCode" | "POST /portal/api/mfa/verifyCode"
                if body.contains("\"123456\"") =>
            {
                json(json!({"responseStatus": {"type": "SUCCESSFUL"}, "serviceTicketId": "ST-2"}))
            }
            "POST /mobile/api/mfa/verifyCode" | "POST /portal/api/mfa/verifyCode" => {
                json(json!({"responseStatus": {"type": "INVALID_MFA_CODE"}}))
            }
            "POST /di-oauth2-service/oauth/token"
                if body.contains("GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2") =>
            {
                (400, "{}".into())
            }
            "POST /di-oauth2-service/oauth/token" => json(
                json!({"access_token": jwt(json!({"exp": Utc::now().timestamp() + 86400, "client_id": "FROM_JWT"})), "refresh_token": "r"}),
            ),
            "GET /userprofile-service/socialProfile" if garmin.api_rejects_token => {
                (401, "{}".into())
            }
            "GET /userprofile-service/socialProfile" => json(json!({"displayName": "runner"})),
            "GET /userprofile-service/userprofile/user-settings" => json(json!({"userData": {}})),
            "GET /activity-service/activity/activityTypes" => {
                json(json!([{"typeKey": "running"}, {"typeKey": "all"}]))
            }
            p if p.starts_with("GET /activitylist-service/") => json(json!([])),
            other => panic!("unexpected Garmin call {other}"),
        };
        let response = response
            .unwrap()
            .status(status)
            .body(reply.into_bytes())
            .map_err(|e| HttpError(e.to_string()));
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
    let token_dir = dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let calls = Calls::default();
    let http = garmin_http(&calls, garmin);
    let clients = Arc::new(RealClients {
        db: db.clone(),
        clock: schedule::system_clock(),
        garmin_http: http.clone(),
        strava_http: Arc::new(|_| panic!("Strava is not connected")),
        garmin_token_dir: token_dir.clone(),
    });
    let sign_in = SignIn {
        http,
        clock: schedule::system_clock(),
        token_dir: token_dir.clone(),
        pause: Duration::ZERO,
    };
    let state = AppState::new(
        db.clone(),
        false,
        schedule::system_clock(),
        0.0,
        AuthConfig::sign_in_off(),
    )
    .with_clients(clients)
    .with_garmin_sign_in(sign_in);
    Harness {
        db,
        app: build_app(state, None),
        calls,
        token_dir,
        _dir: dir,
    }
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
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

impl Harness {
    fn config(&self, key: &str) -> Option<Value> {
        db::get_config_value(&self.db.lock(), key).unwrap()
    }

    fn calls_to(&self, path: &str) -> Vec<(String, String, String, Option<Browser>)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, ..)| p == path)
            .cloned()
            .collect()
    }

    fn browsers_for(&self, path: &str) -> Vec<Option<Browser>> {
        self.calls_to(path)
            .into_iter()
            .map(|(.., browser)| browser)
            .collect()
    }

    fn saved_session(&self) -> Option<Value> {
        let file = std::path::Path::new(&self.token_dir).join("garmin_tokens.json");
        std::fs::read_to_string(file)
            .ok()
            .map(|text| serde_json::from_str(&text).unwrap())
    }
}

const SETUP: &str = "/api/v1/setup/garmin";

fn setup_body() -> Value {
    json!({"email": " runner@example.com ", "password": "secret", "lookbackDays": 7})
}

#[tokio::test]
async fn a_password_sign_in_keeps_the_session_and_holds_every_category() {
    let h = harness(false, mobile(Sso::Ticket));
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (body["message"].as_str(), body["mfaRequired"].as_bool()),
        (Some("Garmin connected."), Some(false))
    );
    let exchanges = h.calls_to("POST /di-oauth2-service/oauth/token");
    assert_eq!(
        exchanges.len(),
        2,
        "the first app identity is refused, the next one tried"
    );
    assert!(
        exchanges[1].2.contains("service_ticket=ST-1"),
        "{}",
        exchanges[1].2
    );
    let session = h.saved_session().expect("session saved");
    assert_eq!(
        (
            session["di_client_id"].as_str(),
            session["di_refresh_token"].as_str()
        ),
        (Some("FROM_JWT"), Some("r"))
    );
    assert_eq!(
        h.config("garmin_credentials"),
        Some(json!({"email": "runner@example.com", "password": "secret"}))
    );
    assert_eq!(h.config("garmin_credentials_verified"), Some(json!(true)));
    assert_eq!(
        h.config("settings").unwrap()["held_activity_types"],
        json!(["running"])
    );
}

#[tokio::test]
async fn a_wrong_password_saves_nothing() {
    let h = harness(false, mobile(Sso::WrongPassword));
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        body["detail"],
        "Could not connect to Garmin: 401 Unauthorized (Invalid Username or Password)"
    );
    assert_eq!(h.config("garmin_credentials"), None);
    assert!(h.saved_session().is_none());
}

#[tokio::test]
async fn rate_limiting_and_a_refused_token_are_reported() {
    let h = harness(
        false,
        Garmin {
            mobile: Sso::RateLimited,
            widget: Widget::RateLimited,
            portal: Sso::RateLimited,
            api_rejects_token: false,
        },
    );
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        body["detail"],
        "Could not connect to Garmin: All login strategies rate limited (429). Try again later or check your IP/network."
    );

    let h = harness(
        false,
        Garmin {
            mobile: Sso::Ticket,
            widget: Widget::Ticket,
            portal: Sso::Ticket,
            api_rejects_token: true,
        },
    );
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        body["detail"],
        "Could not connect to Garmin: Login failed: All login strategies exhausted: portal: token rejected by API tier"
    );
    assert!(h.saved_session().is_none());
}

#[tokio::test]
async fn a_verification_code_finishes_a_reconnect_and_a_wrong_one_can_be_retried() {
    let h = harness(true, mobile(Sso::Mfa));
    let reconnect = json!({"email": "runner@example.com", "password": "secret"});
    let (status, body) = send(
        &h.app,
        Method::POST,
        "/api/v1/settings/garmin/reconnect",
        reconnect,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (body["message"].as_str(), body["mfaRequired"].as_bool()),
        (Some("Garmin sent a verification code."), Some(true))
    );
    assert_eq!(h.config("garmin_credentials"), None);

    let (status, body) = send(
        &h.app,
        Method::POST,
        "/api/v1/garmin/mfa",
        json!({"code": "000000"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let detail = body["detail"].as_str().unwrap();
    assert!(detail.starts_with("MFA verification failed: ['https://sso.garmin.com/mobile/api/mfa/verifyCode: INVALID_MFA_CODE', "), "{detail}");
    assert!(h.saved_session().is_none());

    let (status, body) = send(
        &h.app,
        Method::POST,
        "/api/v1/garmin/mfa",
        json!({"code": "123456"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Garmin verified and connected.");
    let verify = h.calls_to("POST /mobile/api/mfa/verifyCode");
    assert_eq!(verify.last().unwrap().1, "SESSION=abc; GARMIN-SSO=1");
    assert!(
        verify.last().unwrap().2.contains("\"mfaMethod\":\"sms\""),
        "{}",
        verify.last().unwrap().2
    );
    assert!(h.saved_session().is_some());
    assert_eq!(
        h.config("garmin_credentials"),
        Some(json!({"email": "runner@example.com", "password": "secret"}))
    );
    assert!(
        !h.calls_to("GET /activitylist-service/activities/search/activities")
            .is_empty(),
        "catch-up ran"
    );

    let (status, _) = send(
        &h.app,
        Method::POST,
        "/api/v1/garmin/mfa",
        json!({"code": "123456"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_session_garmin_still_accepts_skips_the_password() {
    let h = harness(true, mobile(Sso::WrongPassword));
    let tokens = Tokens {
        di_token: Some(jwt(json!({"exp": Utc::now().timestamp() + 86400}))),
        di_refresh_token: None,
        di_client_id: None,
    };
    save_tokens(&h.token_dir, &tokens).unwrap();
    let reconnect = json!({"email": "runner@example.com", "password": "anything"});
    let (status, body) = send(
        &h.app,
        Method::POST,
        "/api/v1/settings/garmin/reconnect",
        reconnect,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["message"],
        "Garmin reconnected and catch-up sync started."
    );
    assert!(h.calls_to("POST /mobile/api/login").is_empty());
    assert!(
        !h.calls_to("GET /activitylist-service/activities/search/activities")
            .is_empty(),
        "catch-up ran"
    );
}

fn exchange_bodies(h: &Harness) -> Vec<String> {
    h.calls_to("POST /di-oauth2-service/oauth/token")
        .into_iter()
        .map(|(_, _, body, _)| body)
        .collect()
}

#[tokio::test]
async fn when_the_mobile_route_is_blocked_the_widget_signs_in() {
    let h = harness(
        false,
        Garmin {
            widget: Widget::Ticket,
            ..mobile(Sso::Blocked)
        },
    );
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Garmin connected.");
    let post = h.calls_to("POST /sso/signin").pop().unwrap();
    assert_eq!(
        post.1, "EMBED=1; CF=2",
        "cookies from both hops of the redirect"
    );
    assert!(
        post.2.contains("_csrf=tok-1") && post.2.contains("username=runner%40example.com"),
        "{}",
        post.2
    );
    let exchange = exchange_bodies(&h).pop().unwrap();
    assert!(
        exchange.contains("service_ticket=ST-W")
            && exchange.contains("service_url=https%3A%2F%2Fsso.garmin.com%2Fsso%2Fembed"),
        "{exchange}"
    );
    assert!(h.saved_session().is_some());
    assert!(h.calls_to("POST /portal/api/login").is_empty());
}

#[tokio::test]
async fn the_widget_follows_the_redirect_after_the_password_post() {
    let h = harness(
        false,
        Garmin {
            widget: Widget::RedirectedTicket,
            ..mobile(Sso::Blocked)
        },
    );
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let landing = h.calls_to("GET /sso/signin").pop().unwrap();
    assert_eq!(
        landing.1, "EMBED=1; CF=2; SSO=3",
        "the redirect's cookie goes along"
    );
    assert_eq!(landing.2, "", "the form is not sent again after a 302");
    assert!(
        exchange_bodies(&h)
            .pop()
            .unwrap()
            .contains("service_ticket=ST-R")
    );
    assert!(h.saved_session().is_some());
}

#[tokio::test]
async fn a_locked_account_on_the_widget_ends_the_chain() {
    let h = harness(
        false,
        Garmin {
            widget: Widget::Locked,
            portal: Sso::Ticket,
            ..mobile(Sso::Blocked)
        },
    );
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        body["detail"],
        "Could not connect to Garmin: Widget authentication failed: 'Account Locked'"
    );
    assert!(h.calls_to("POST /portal/api/login").is_empty());
}

#[tokio::test]
async fn the_portal_is_the_last_resort_and_its_code_goes_to_the_portal_first() {
    let h = harness(
        false,
        Garmin {
            portal: Sso::Mfa,
            ..mobile(Sso::Blocked)
        },
    );
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mfaRequired"], true);
    assert_eq!(h.calls_to("GET /portal/sso/en-US/sign-in").len(), 1);
    let (status, body) = send(
        &h.app,
        Method::POST,
        "/api/v1/garmin/mfa",
        json!({"code": "123456"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(h.calls_to("POST /portal/api/mfa/verifyCode").len(), 1);
    assert!(h.calls_to("POST /mobile/api/mfa/verifyCode").is_empty());
    let exchange = exchange_bodies(&h).pop().unwrap();
    assert!(
        exchange.contains("service_url=https%3A%2F%2Fconnect.garmin.com%2Fapp"),
        "{exchange}"
    );
    assert!(h.saved_session().is_some());
}

#[tokio::test]
async fn a_widget_code_is_requested_and_can_be_retried() {
    let h = harness(
        true,
        Garmin {
            widget: Widget::Mfa,
            ..mobile(Sso::Blocked)
        },
    );
    let reconnect = json!({"email": "runner@example.com", "password": "secret"});
    let (status, body) = send(
        &h.app,
        Method::POST,
        "/api/v1/settings/garmin/reconnect",
        reconnect,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mfaRequired"], true);
    let request = h
        .calls_to("POST /sso/verifyMFA/mfaCode")
        .pop()
        .expect("code requested");
    assert!(
        request.2.contains("\"customerGuid\":\"g-1\""),
        "{}",
        request.2
    );

    let (status, body) = send(
        &h.app,
        Method::POST,
        "/api/v1/garmin/mfa",
        json!({"code": "000000"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["detail"], "Widget MFA failed: Invalid code");

    let (status, body) = send(
        &h.app,
        Method::POST,
        "/api/v1/garmin/mfa",
        json!({"code": "123456"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let verify = h
        .calls_to("POST /sso/verifyMFA/loginEnterMfaCode")
        .pop()
        .unwrap();
    assert!(verify.2.contains("_csrf=tok-2"), "{}", verify.2);
    assert_eq!(verify.1, "EMBED=1; CF=2");
    assert!(
        exchange_bodies(&h)
            .pop()
            .unwrap()
            .contains("service_ticket=ST-WM")
    );
    assert!(h.saved_session().is_some());
}

#[tokio::test]
async fn each_route_tries_the_library_browsers_in_turn() {
    let h = harness(
        false,
        Garmin {
            mobile: Sso::RateLimited,
            widget: Widget::RateLimited,
            portal: Sso::RateLimited,
            api_rejects_token: false,
        },
    );
    let (status, _) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let mobile = [Browser::SafariIos, Browser::Safari, Browser::Chrome120].map(Some);
    assert_eq!(h.browsers_for("POST /mobile/api/login"), mobile);
    assert_eq!(h.browsers_for("GET /sso/embed"), [Some(Browser::Chrome)]);
    let portal = [
        Browser::Safari,
        Browser::SafariIos,
        Browser::Chrome120,
        Browser::Edge101,
        Browser::Chrome,
    ]
    .map(Some);
    assert_eq!(h.browsers_for("GET /portal/sso/en-US/sign-in"), portal);
    assert_eq!(h.browsers_for("POST /portal/api/login"), portal);
}

#[tokio::test]
async fn the_code_goes_in_as_the_browser_that_was_asked_for_it() {
    let h = harness(false, mobile(Sso::MfaAfterRateLimit));
    let (status, body) = send(&h.app, Method::POST, SETUP, setup_body()).await;
    assert_eq!(
        (status, body["mfaRequired"].as_bool()),
        (StatusCode::OK, Some(true)),
        "{body}"
    );
    assert_eq!(
        h.browsers_for("POST /mobile/api/login"),
        [Some(Browser::SafariIos), Some(Browser::Safari)]
    );

    let (status, body) = send(
        &h.app,
        Method::POST,
        "/api/v1/garmin/mfa",
        json!({"code": "123456"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        h.browsers_for("POST /mobile/api/mfa/verifyCode"),
        [Some(Browser::Safari)]
    );
    assert_eq!(
        h.browsers_for("POST /di-oauth2-service/oauth/token"),
        [Some(Browser::Chrome); 2],
        "the ticket exchange looks like Chrome"
    );
    assert_eq!(
        h.browsers_for("GET /userprofile-service/socialProfile")[0],
        None,
        "the API check goes out plain"
    );
}
