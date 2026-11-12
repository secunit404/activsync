use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};

use activsync::auth_core::{self, HashCost};
use activsync::auth_guard::{AuthConfig, ForwardedTrust, ProxyAuth};
use activsync::db::Db;
use activsync::{AppState, build_app, schedule, seed};
use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use chrono::Utc;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple";

struct Client {
    app: Router,
    state: AppState,
    cookies: Vec<(String, String)>,
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    json: Value,
    client_addr: Option<String>,
}

fn client_with(auth: AuthConfig, peer: &str) -> Client {
    let db = Db::open_in_memory().unwrap();
    seed::seed(&mut db.lock(), Utc::now(), true).unwrap();
    let state = AppState::new(db, true, schedule::system_clock(), schedule::UNDO_WINDOW_SECONDS, auth);
    let peer: SocketAddr = format!("{peer}:50000").parse().unwrap();
    let app = build_app(state.clone(), None).layer(MockConnectInfo(peer));
    Client { app, state, cookies: Vec::new() }
}

fn client() -> Client {
    client_with(AuthConfig::for_tests(), "127.0.0.1")
}

impl Client {
    async fn raw(&mut self, request: axum::http::request::Builder, body: Option<Value>) -> Reply {
        let cookie_header: Vec<String> = self.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let mut request = request;
        if !cookie_header.is_empty() {
            request = request.header("cookie", cookie_header.join("; "));
        }
        let body = match body {
            Some(value) => {
                request = request.header("content-type", "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        let response = self.app.clone().oneshot(request.body(body).unwrap()).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let client_addr = response.extensions().get::<activsync::logging::ClientAddr>().map(|a| a.0.clone());
        for set in headers.get_all("set-cookie") {
            let text = set.to_str().unwrap();
            let (pair, attributes) = text.split_once(';').unwrap_or((text, ""));
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(k, _)| k != name);
            if !attributes.contains("Max-Age=0") {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
        Reply { status, headers, json, client_addr }
    }

    async fn send(&mut self, method: Method, uri: &str, body: Option<Value>) -> Reply {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:8382")
            .header("x-activsync-request", "1")
            .header("sec-fetch-site", "same-origin")
            .header("user-agent", "test-browser");
        self.raw(request, body).await
    }

    async fn claim(&mut self) -> Reply {
        let code = self.state.setup_code.current().expect("unclaimed install has a code");
        self.send(Method::POST, "/api/v1/auth/claim", Some(json!({"setupCode": code, "password": PASSWORD}))).await
    }
}

#[tokio::test]
async fn unknown_hosts_are_refused_except_for_health() {
    let mut c = client();
    let evil = Request::get("/api/v1/auth/status").header("host", "evil.example");
    let reply = c.raw(evil, None).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.json, "Unknown host — set ACTIVSYNC_PUBLIC_URL or add it to ACTIVSYNC_ALLOWED_HOSTS");
    let health = c.raw(Request::get("/health").header("host", "evil.example"), None).await;
    assert_eq!(health.status, StatusCode::OK);
}

#[tokio::test]
async fn signed_out_requests_are_turned_away() {
    let mut c = client();
    let api = c.send(Method::GET, "/api/v1/app", None).await;
    assert_eq!((api.status, api.json.clone()), (StatusCode::UNAUTHORIZED, json!({"detail": "Not signed in"})));
    let page = c.send(Method::GET, "/strava/callback?code=x", None).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    assert_eq!(page.headers["location"], "/login?next=%2Fstrava%2Fcallback%3Fcode%3Dx");
    let events = c.send(Method::GET, "/api/events", None).await;
    assert_eq!(events.status, StatusCode::UNAUTHORIZED);
    let status = c.send(Method::GET, "/api/v1/auth/status", None).await;
    assert_eq!(status.json, json!({
        "authenticated": false, "method": null, "user": null, "ownerClaimed": false,
        "authDisabled": false, "login": {"oidc": null}
    }));
}

#[tokio::test]
async fn claim_checks_code_and_password_then_signs_in() {
    let mut c = client();
    let short = c.send(Method::POST, "/api/v1/auth/claim", Some(json!({"setupCode": "x", "password": "short"}))).await;
    assert_eq!(short.status, StatusCode::BAD_REQUEST);
    assert!(short.json["detail"].as_str().unwrap().starts_with("Use at least 15 characters."));
    let wrong = c.send(Method::POST, "/api/v1/auth/claim", Some(json!({"setupCode": "AAAA-AAAA-AAAA-AAAA", "password": PASSWORD}))).await;
    assert_eq!(wrong.status, StatusCode::FORBIDDEN);
    assert_eq!(wrong.json["detail"], "That setup code is not right. Check the server log for the current one.");

    let claimed = c.claim().await;
    assert_eq!(claimed.status, StatusCode::OK, "{}", claimed.json);
    assert_eq!(claimed.json["authenticated"], true);
    assert_eq!(claimed.json["method"], "password");
    assert_eq!(claimed.json["ownerClaimed"], true);
    let cookies: Vec<&str> = claimed.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap()).collect();
    assert!(cookies.iter().any(|c| c.starts_with("activsync_session=") && c.contains("HttpOnly") && c.contains("SameSite=lax") && c.contains("Max-Age=2592000")));
    assert!(cookies.iter().any(|c| c.starts_with("activsync_device=")));
    assert!(cookies.iter().all(|c| !c.contains("Secure")), "plain http gets no Secure flag");
    assert!(c.state.setup_code.current().is_none(), "code is spent");

    assert_eq!(c.send(Method::GET, "/api/v1/app", None).await.json["setup"]["complete"], true);
    let again = c.send(Method::POST, "/api/v1/auth/claim", Some(json!({"setupCode": "x", "password": PASSWORD}))).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn login_logout_and_wrong_password() {
    let mut owner = client();
    owner.claim().await;
    let logout = owner.send(Method::POST, "/api/v1/auth/logout", None).await;
    assert_eq!(logout.status, StatusCode::NO_CONTENT);
    assert_eq!(owner.send(Method::GET, "/api/v1/app", None).await.status, StatusCode::UNAUTHORIZED);

    let wrong = owner.send(Method::POST, "/api/v1/auth/login", Some(json!({"password": "not the password at all"}))).await;
    assert_eq!((wrong.status, wrong.json["detail"].clone()), (StatusCode::UNAUTHORIZED, json!("Wrong password.")));
    let ok = owner.send(Method::POST, "/api/v1/auth/login", Some(json!({"password": PASSWORD}))).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.json);
    assert_eq!(owner.send(Method::GET, "/api/v1/app", None).await.status, StatusCode::OK);
}

#[tokio::test]
async fn cross_site_writes_are_blocked() {
    let mut c = client();
    c.claim().await;
    let base = || Request::builder().method(Method::POST).uri("/api/v1/setup/hevy/skip").header("host", "127.0.0.1:8382");
    let blocked = json!({"detail": "This request was blocked as cross-site."});

    let no_header = c.raw(base().header("sec-fetch-site", "same-origin"), None).await;
    assert_eq!((no_header.status, no_header.json), (StatusCode::FORBIDDEN, blocked.clone()));
    let cross = c.raw(base().header("x-activsync-request", "1").header("sec-fetch-site", "cross-site"), None).await;
    assert_eq!(cross.status, StatusCode::FORBIDDEN);
    let other_origin = c.raw(base().header("x-activsync-request", "1").header("origin", "http://evil.example"), None).await;
    assert_eq!(other_origin.status, StatusCode::FORBIDDEN);
    let opaque = c.raw(base().header("x-activsync-request", "1").header("origin", "null"), None).await;
    assert_eq!(opaque.status, StatusCode::FORBIDDEN);
    let nothing = c.raw(base().header("x-activsync-request", "1"), None).await;
    assert_eq!(nothing.status, StatusCode::FORBIDDEN);

    let same_origin = c.raw(base().header("x-activsync-request", "1").header("origin", "http://127.0.0.1:8382"), None).await;
    assert_eq!(same_origin.status, StatusCode::CONFLICT, "passes the guard; setup is already complete");
    let referer = c.raw(base().header("x-activsync-request", "1").header("referer", "http://127.0.0.1:8382/settings"), None).await;
    assert_eq!(referer.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn anonymous_bodies_are_capped() {
    let mut c = client();
    let huge = json!({"password": "x".repeat(20_000)});
    let reply = c.send(Method::POST, "/api/v1/auth/login", Some(huge)).await;
    assert_eq!((reply.status, reply.json), (StatusCode::PAYLOAD_TOO_LARGE, json!({"detail": "That request is too large."})));
}

#[tokio::test]
async fn every_response_carries_security_headers() {
    let mut c = client();
    for reply in [c.send(Method::GET, "/api/v1/auth/status", None).await, c.send(Method::GET, "/api/v1/app", None).await] {
        assert!(reply.headers["content-security-policy"].to_str().unwrap().starts_with("default-src 'self'; script-src 'self'"));
        assert_eq!(reply.headers["x-content-type-options"], "nosniff");
        assert_eq!(reply.headers["referrer-policy"], "same-origin");
        assert!(!reply.headers.contains_key("strict-transport-security"));
    }
}

#[tokio::test]
async fn https_from_a_trusted_proxy_gets_secure_cookies_and_hsts() {
    let mut c = client();
    let code = c.state.setup_code.current().unwrap();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/auth/claim")
        .header("host", "activsync.localhost")
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-for", "203.0.113.7")
        .header("x-activsync-request", "1")
        .header("origin", "https://activsync.localhost");
    let reply = c.raw(request, Some(json!({"setupCode": code, "password": PASSWORD}))).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let cookies: Vec<&str> = reply.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap()).collect();
    assert!(cookies.iter().any(|c| c.starts_with("__Host-activsync_session=") && c.contains("Secure")));
    assert_eq!(reply.headers["strict-transport-security"], "max-age=31536000");

    let sessions = c.raw(
        Request::get("/api/v1/auth/sessions").header("host", "activsync.localhost").header("x-forwarded-proto", "https").header("x-forwarded-for", "203.0.113.7"),
        None,
    ).await;
    assert_eq!(sessions.json[0]["ip"], "203.0.113.7", "client address comes from the trusted proxy");
}

#[tokio::test]
async fn forwarded_headers_from_untrusted_peers_are_ignored() {
    let mut auth = AuthConfig::for_tests();
    auth.forwarded = ForwardedTrust::parse("10.9.9.9");
    let mut c = client_with(auth, "192.0.2.50");
    c.claim().await;
    let sessions = c.send(Method::GET, "/api/v1/auth/sessions", None).await;
    assert_eq!(sessions.json[0]["ip"], "192.0.2.50");
}

#[tokio::test]
async fn sessions_list_revoke_and_password_change() {
    let mut first = client();
    first.claim().await;
    let mut second = Client { app: first.app.clone(), state: first.state.clone(), cookies: Vec::new() };
    second.send(Method::POST, "/api/v1/auth/login", Some(json!({"password": PASSWORD}))).await;

    let listed = first.send(Method::GET, "/api/v1/auth/sessions", None).await.json;
    let rows = listed.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().filter(|r| r["current"] == true).count(), 1);
    assert_eq!(rows[0]["userAgent"], "test-browser");
    assert!(rows[0]["createdAt"].as_str().unwrap().ends_with("+00:00"));

    let missing = first.send(Method::DELETE, "/api/v1/auth/sessions/nope", None).await;
    assert_eq!((missing.status, missing.json["detail"].clone()), (StatusCode::NOT_FOUND, json!("That session has already ended.")));

    let wrong = first.send(Method::PUT, "/api/v1/auth/password", Some(json!({"current": "nope", "new": "a brand new long passphrase"}))).await;
    assert_eq!((wrong.status, wrong.json["detail"].clone()), (StatusCode::BAD_REQUEST, json!("Wrong password.")));
    let changed = first.send(Method::PUT, "/api/v1/auth/password", Some(json!({"current": PASSWORD, "new": "a brand new long passphrase"}))).await;
    assert_eq!(changed.json["message"], "Password changed. Other sessions were signed out.");
    assert_eq!(second.send(Method::GET, "/api/v1/app", None).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(first.send(Method::GET, "/api/v1/app", None).await.status, StatusCode::OK);

    let mut third = Client { app: first.app.clone(), state: first.state.clone(), cookies: Vec::new() };
    third.send(Method::POST, "/api/v1/auth/login", Some(json!({"password": "a brand new long passphrase"}))).await;
    let others = first.send(Method::DELETE, "/api/v1/auth/sessions", None).await;
    assert_eq!(others.json["message"], "Signed out 1 other session(s).");
    assert_eq!(third.send(Method::GET, "/api/v1/app", None).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn too_many_attempts_from_one_address_wait() {
    let mut c = client();
    for _ in 0..10 {
        c.send(Method::POST, "/api/v1/auth/claim", Some(json!({"setupCode": "x", "password": PASSWORD}))).await;
    }
    let reply = c.send(Method::POST, "/api/v1/auth/claim", Some(json!({"setupCode": "x", "password": PASSWORD}))).await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(reply.json["detail"].as_str().unwrap().starts_with("Too many attempts. Try again in "));
    assert!(reply.headers.contains_key("retry-after"));
}

fn proxy_auth() -> AuthConfig {
    let env = |name: &str| match name {
        "ACTIVSYNC_AUTH_PROXY_HEADER" => Some("Remote-User".to_string()),
        "ACTIVSYNC_AUTH_PROXY_TRUSTED" => Some("172.18.0.10/32".to_string()),
        "ACTIVSYNC_AUTH_PROXY_USER" => Some("owner".to_string()),
        _ => None,
    };
    AuthConfig { proxy: ProxyAuth::from_env(env, &[]).unwrap(), ..AuthConfig::for_tests() }
}

#[tokio::test]
async fn proxy_sign_in_trusts_only_its_own_address() {
    let mut proxied = client_with(proxy_auth(), "172.18.0.10");
    let as_owner = || Request::get("/api/v1/auth/status").header("host", "127.0.0.1:8382").header("remote-user", "owner");
    let status = proxied.raw(as_owner(), None).await.json;
    assert_eq!((status["method"].clone(), status["user"].clone()), (json!("proxy"), json!("owner")));

    let app = proxied.raw(Request::get("/api/v1/app").header("host", "127.0.0.1:8382").header("remote-user", "owner"), None).await;
    assert_eq!(app.json["setup"]["step"], "account", "an unclaimed install still claims first");

    let stranger = proxied.raw(Request::get("/api/v1/auth/status").header("host", "127.0.0.1:8382").header("remote-user", "mallory"), None).await;
    assert_eq!(stranger.status, StatusCode::UNAUTHORIZED);

    let settings = proxied.raw(Request::get("/api/v1/auth/settings").header("host", "127.0.0.1:8382").header("remote-user", "owner"), None).await;
    assert_eq!(settings.json, json!({"proxy": {"header": "Remote-User", "trusted": ["172.18.0.10/32"], "user": "owner"}}));

    let logout = proxied.raw(
        Request::post("/api/v1/auth/logout").header("host", "127.0.0.1:8382").header("remote-user", "owner").header("x-activsync-request", "1").header("sec-fetch-site", "same-origin"),
        None,
    ).await;
    assert_eq!(logout.json["detail"], "You are signed in through your proxy; sign out there.");

    let mut bypass = client_with(proxy_auth(), "192.0.2.1");
    let ignored = bypass.raw(as_owner(), None).await.json;
    assert_eq!(ignored["authenticated"], false, "header from a peer that is not the proxy");
}

#[tokio::test]
async fn public_url_pins_the_host_and_forces_https() {
    let auth = AuthConfig { public_url: Some("https://activsync.example.com".into()), ..AuthConfig::for_tests() };
    let mut c = client_with(auth, "127.0.0.1");
    let other = c.raw(Request::get("/api/v1/auth/status").header("host", "127.0.0.1:8382"), None).await;
    assert_eq!(other.json, "Unknown host — ActivSync answers only on the host in ACTIVSYNC_PUBLIC_URL");
    let ok = c.raw(Request::get("/api/v1/auth/status").header("host", "activsync.example.com:443"), None).await;
    assert_eq!(ok.status, StatusCode::OK);
    assert_eq!(ok.headers["strict-transport-security"], "max-age=31536000");
}

#[tokio::test]
async fn dev_unclaim_issues_a_new_code() {
    let mut c = client();
    c.claim().await;
    let reply = c.send(Method::POST, "/api/v1/dev/unclaim", None).await;
    let code = reply.json["setupCode"].as_str().unwrap().to_string();
    assert_eq!(c.state.setup_code.current(), Some(code));
    assert_eq!(c.send(Method::GET, "/api/v1/auth/status", None).await.json["ownerClaimed"], false);
}

async fn claim_through_proxy(c: &mut Client, headers: &[(&str, &str)]) -> Reply {
    let code = c.state.setup_code.current().unwrap();
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/auth/claim")
        .header("host", "activsync.localhost")
        .header("x-activsync-request", "1")
        .header("origin", "https://activsync.localhost");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    c.raw(request, Some(json!({"setupCode": code, "password": PASSWORD}))).await
}

#[tokio::test]
async fn forwarded_for_lines_are_joined_and_ports_dropped() {
    let mut c = client();
    let reply = claim_through_proxy(
        &mut c,
        &[("x-forwarded-proto", "https"), ("x-forwarded-for", "1.1.1.1"), ("x-forwarded-for", "203.0.113.7:4444")],
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    assert_eq!(reply.client_addr.as_deref(), Some("203.0.113.7:4444"));
    let sessions = c.raw(
        Request::get("/api/v1/auth/sessions")
            .header("host", "activsync.localhost")
            .header("x-forwarded-proto", "https")
            .header("x-forwarded-for", "1.1.1.1")
            .header("x-forwarded-for", "203.0.113.7:4444"),
        None,
    ).await;
    assert_eq!(sessions.json[0]["ip"], "203.0.113.7");
}

#[tokio::test]
async fn an_empty_forwarded_for_keeps_the_socket_peer() {
    let mut c = client();
    let reply = claim_through_proxy(&mut c, &[("x-forwarded-proto", "https"), ("x-forwarded-for", "")]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    assert_eq!(reply.client_addr.as_deref(), Some("127.0.0.1:50000"));
}

#[tokio::test]
async fn forwarded_proto_is_case_sensitive() {
    let mut c = client();
    let reply = claim_through_proxy(&mut c, &[("x-forwarded-proto", "HTTPS")]).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "scheme stays http, so an https Origin does not match");
    assert!(!reply.headers.contains_key("strict-transport-security"));

    let wss = claim_through_proxy(&mut c, &[("x-forwarded-proto", "wss")]).await;
    assert_eq!(wss.status, StatusCode::FORBIDDEN, "wss is taken as the scheme, which is not https");
    assert!(!wss.headers.contains_key("strict-transport-security"));
}

#[tokio::test]
async fn access_log_shows_the_forwarded_client() {
    let mut c = client();
    let plain = c.send(Method::GET, "/api/v1/auth/status", None).await;
    assert_eq!(plain.client_addr.as_deref(), Some("127.0.0.1:50000"));
    let no_port = c.raw(
        Request::get("/api/v1/auth/status").header("host", "127.0.0.1:8382").header("x-forwarded-for", "203.0.113.9"),
        None,
    ).await;
    assert_eq!(no_port.client_addr.as_deref(), Some("203.0.113.9:0"));
    let refused = c.raw(Request::get("/api/v1/auth/status").header("host", "evil.example").header("x-forwarded-for", "203.0.113.9:80"), None).await;
    assert_eq!((refused.status, refused.client_addr.as_deref()), (StatusCode::BAD_REQUEST, Some("203.0.113.9:80")));

    let mut v6 = client_with(AuthConfig::for_tests(), "[::1]");
    let reply = v6.send(Method::GET, "/api/v1/auth/status", None).await;
    assert_eq!(reply.client_addr.as_deref(), Some("::1:50000"), "uvicorn writes IPv6 hosts without brackets");
}

#[tokio::test]
async fn forwarded_allow_ips_with_host_bits_trusts_no_one() {
    let mut auth = AuthConfig::for_tests();
    auth.forwarded = ForwardedTrust::parse("172.18.0.5/16");
    let mut c = client_with(auth, "172.18.0.5");
    let reply = c.raw(
        Request::get("/api/v1/auth/status").header("host", "127.0.0.1:8382").header("x-forwarded-for", "203.0.113.9"),
        None,
    ).await;
    assert_eq!(reply.client_addr.as_deref(), Some("172.18.0.5:50000"));
}

#[tokio::test]
async fn anonymous_body_cap_comes_before_the_csrf_check() {
    let mut c = client();
    let request = Request::post("/api/v1/auth/login").header("host", "127.0.0.1:8382");
    let reply = c.raw(request, Some(json!({"password": "x".repeat(20_000)}))).await;
    assert_eq!((reply.status, reply.json), (StatusCode::PAYLOAD_TOO_LARGE, json!({"detail": "That request is too large."})));
}

#[tokio::test]
async fn a_proxy_header_with_non_ascii_bytes_is_a_mismatch() {
    let mut proxied = client_with(proxy_auth(), "172.18.0.10");
    let odd = || Request::get("/api/v1/auth/status").header("host", "127.0.0.1:8382").header("remote-user", HeaderValue::from_bytes(b"caf\xe9").unwrap());
    let reply = proxied.raw(odd(), None).await;
    assert_eq!((reply.status, reply.json), (StatusCode::UNAUTHORIZED, json!({"detail": "Not signed in"})));

    let mut bypass = client_with(proxy_auth(), "192.0.2.1");
    assert_eq!(bypass.raw(odd(), None).await.status, StatusCode::OK, "an untrusted peer's header is ignored");
}

#[tokio::test]
async fn password_checks_leave_the_database_free() {
    let mut c = client();
    c.claim().await;
    let slow = HashCost { memory_kib: 65536, time: 12, parallelism: 4, hash_len: 32, salt_len: 16 };
    let hash = auth_core::hash_password(PASSWORD, slow).unwrap();
    auth_core::set_password_hash(&c.state.db.lock(), &hash, 0.0).unwrap();
    let mut other = Client { app: c.app.clone(), state: c.state.clone(), cookies: Vec::new() };
    let login_done = AtomicBool::new(false);
    let (login, finished_first) = tokio::join!(
        async {
            let reply = other.send(Method::POST, "/api/v1/auth/login", Some(json!({"password": PASSWORD}))).await;
            login_done.store(true, Ordering::SeqCst);
            reply
        },
        async {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let status = c.send(Method::GET, "/api/v1/auth/status", None).await;
            assert_eq!(status.status, StatusCode::OK);
            !login_done.load(Ordering::SeqCst)
        }
    );
    assert_eq!(login.status, StatusCode::OK);
    assert!(finished_first, "other requests were served while the password was being verified");
}

#[test]
fn a_password_changed_during_a_check_voids_a_match_against_the_old_one() {
    let db = Db::open_in_memory().unwrap();
    let conn = db.lock();
    let cost = HashCost::cheapest();
    auth_core::set_password_hash(&conn, &auth_core::hash_password("old password here", cost).unwrap(), 0.0).unwrap();
    let reserved = auth_core::reserve_check(&conn, 1.0, None).unwrap().unwrap();
    let verdict = auth_core::verify_stored(reserved.stored.as_deref(), "old password here", cost);
    assert!(verdict.verified);
    let replaced = auth_core::hash_password("new password here", cost).unwrap();
    auth_core::set_password_hash(&conn, &replaced, 2.0).unwrap();
    assert!(!auth_core::settle_check(&conn, None, &reserved, &verdict).unwrap());
    assert!(auth_core::verify_password(&conn, "new password here", cost).unwrap());
}
