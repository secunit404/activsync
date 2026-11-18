use activsync::auth_guard::{AuthConfig, ProxyAuth};
use activsync::proxy_headers::ForwardedTrust;
use axum::http::{HeaderValue, Method, Request, StatusCode};
use serde_json::json;

use super::{Client, PASSWORD, Reply, client, client_with};

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
    let reply = c
        .raw(
            request,
            Some(json!({"setupCode": code, "password": PASSWORD})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    let cookies: Vec<&str> = reply
        .headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect();
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with("__Host-activsync_session=") && c.contains("Secure"))
    );
    assert_eq!(
        reply.headers["strict-transport-security"],
        "max-age=31536000"
    );

    let sessions = c
        .raw(
            Request::get("/api/v1/auth/sessions")
                .header("host", "activsync.localhost")
                .header("x-forwarded-proto", "https")
                .header("x-forwarded-for", "203.0.113.7"),
            None,
        )
        .await;
    assert_eq!(
        sessions.json[0]["ip"], "203.0.113.7",
        "client address comes from the trusted proxy"
    );
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

fn proxy_auth() -> AuthConfig {
    let env = |name: &str| match name {
        "ACTIVSYNC_AUTH_PROXY_HEADER" => Some("Remote-User".to_string()),
        "ACTIVSYNC_AUTH_PROXY_TRUSTED" => Some("172.18.0.10/32".to_string()),
        "ACTIVSYNC_AUTH_PROXY_USER" => Some("owner".to_string()),
        _ => None,
    };
    AuthConfig {
        proxy: ProxyAuth::from_env(env, &[]).unwrap(),
        ..AuthConfig::for_tests()
    }
}

#[tokio::test]
async fn proxy_sign_in_trusts_only_its_own_address() {
    let mut proxied = client_with(proxy_auth(), "172.18.0.10");
    let as_owner = || {
        Request::get("/api/v1/auth/status")
            .header("host", "127.0.0.1:8382")
            .header("remote-user", "owner")
    };
    let status = proxied.raw(as_owner(), None).await.json;
    assert_eq!(
        (status["method"].clone(), status["user"].clone()),
        (json!("proxy"), json!("owner"))
    );

    let app = proxied
        .raw(
            Request::get("/api/v1/app")
                .header("host", "127.0.0.1:8382")
                .header("remote-user", "owner"),
            None,
        )
        .await;
    assert_eq!(
        app.json["setup"]["step"], "account",
        "an unclaimed install still claims first"
    );

    let stranger = proxied
        .raw(
            Request::get("/api/v1/auth/status")
                .header("host", "127.0.0.1:8382")
                .header("remote-user", "mallory"),
            None,
        )
        .await;
    assert_eq!(stranger.status, StatusCode::UNAUTHORIZED);

    let settings = proxied
        .raw(
            Request::get("/api/v1/auth/settings")
                .header("host", "127.0.0.1:8382")
                .header("remote-user", "owner"),
            None,
        )
        .await;
    assert_eq!(
        settings.json,
        json!({"proxy": {"header": "Remote-User", "trusted": ["172.18.0.10/32"], "user": "owner"}})
    );

    let logout = proxied
        .raw(
            Request::post("/api/v1/auth/logout")
                .header("host", "127.0.0.1:8382")
                .header("remote-user", "owner")
                .header("x-activsync-request", "1")
                .header("sec-fetch-site", "same-origin"),
            None,
        )
        .await;
    assert_eq!(
        logout.json["detail"],
        "You are signed in through your reverse proxy. Sign out there."
    );

    let mut bypass = client_with(proxy_auth(), "192.0.2.1");
    let ignored = bypass.raw(as_owner(), None).await.json;
    assert_eq!(
        ignored["authenticated"], false,
        "header from a peer that is not the proxy"
    );
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
    c.raw(
        request,
        Some(json!({"setupCode": code, "password": PASSWORD})),
    )
    .await
}

#[tokio::test]
async fn forwarded_for_lines_are_joined_and_ports_dropped() {
    let mut c = client();
    let reply = claim_through_proxy(
        &mut c,
        &[
            ("x-forwarded-proto", "https"),
            ("x-forwarded-for", "1.1.1.1"),
            ("x-forwarded-for", "203.0.113.7:4444"),
        ],
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    assert_eq!(reply.client_addr.as_deref(), Some("203.0.113.7:4444"));
    let sessions = c
        .raw(
            Request::get("/api/v1/auth/sessions")
                .header("host", "activsync.localhost")
                .header("x-forwarded-proto", "https")
                .header("x-forwarded-for", "1.1.1.1")
                .header("x-forwarded-for", "203.0.113.7:4444"),
            None,
        )
        .await;
    assert_eq!(sessions.json[0]["ip"], "203.0.113.7");
}

#[tokio::test]
async fn an_empty_forwarded_for_keeps_the_socket_peer() {
    let mut c = client();
    let reply = claim_through_proxy(
        &mut c,
        &[("x-forwarded-proto", "https"), ("x-forwarded-for", "")],
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.json);
    assert_eq!(reply.client_addr.as_deref(), Some("127.0.0.1:50000"));
}

#[tokio::test]
async fn forwarded_proto_is_case_sensitive() {
    let mut c = client();
    let reply = claim_through_proxy(&mut c, &[("x-forwarded-proto", "HTTPS")]).await;
    assert_eq!(
        reply.status,
        StatusCode::FORBIDDEN,
        "scheme stays http, so an https Origin does not match"
    );
    assert!(!reply.headers.contains_key("strict-transport-security"));

    let wss = claim_through_proxy(&mut c, &[("x-forwarded-proto", "wss")]).await;
    assert_eq!(
        wss.status,
        StatusCode::FORBIDDEN,
        "wss is taken as the scheme, which is not https"
    );
    assert!(!wss.headers.contains_key("strict-transport-security"));
}

#[tokio::test]
async fn access_log_shows_the_forwarded_client() {
    let mut c = client();
    let plain = c.send(Method::GET, "/api/v1/auth/status", None).await;
    assert_eq!(plain.client_addr.as_deref(), Some("127.0.0.1:50000"));
    let no_port = c
        .raw(
            Request::get("/api/v1/auth/status")
                .header("host", "127.0.0.1:8382")
                .header("x-forwarded-for", "203.0.113.9"),
            None,
        )
        .await;
    assert_eq!(no_port.client_addr.as_deref(), Some("203.0.113.9:0"));
    let refused = c
        .raw(
            Request::get("/api/v1/auth/status")
                .header("host", "evil.example")
                .header("x-forwarded-for", "203.0.113.9:80"),
            None,
        )
        .await;
    assert_eq!(
        (refused.status, refused.client_addr.as_deref()),
        (StatusCode::BAD_REQUEST, Some("203.0.113.9:80"))
    );

    let mut v6 = client_with(AuthConfig::for_tests(), "[::1]");
    let reply = v6.send(Method::GET, "/api/v1/auth/status", None).await;
    assert_eq!(
        reply.client_addr.as_deref(),
        Some("::1:50000"),
        "uvicorn writes IPv6 hosts without brackets"
    );
}

#[tokio::test]
async fn forwarded_allow_ips_with_host_bits_trusts_no_one() {
    let mut auth = AuthConfig::for_tests();
    auth.forwarded = ForwardedTrust::parse("172.18.0.5/16");
    let mut c = client_with(auth, "172.18.0.5");
    let reply = c
        .raw(
            Request::get("/api/v1/auth/status")
                .header("host", "127.0.0.1:8382")
                .header("x-forwarded-for", "203.0.113.9"),
            None,
        )
        .await;
    assert_eq!(reply.client_addr.as_deref(), Some("172.18.0.5:50000"));
}

#[tokio::test]
async fn a_proxy_header_with_non_ascii_bytes_is_a_mismatch() {
    let mut proxied = client_with(proxy_auth(), "172.18.0.10");
    let odd = || {
        Request::get("/api/v1/auth/status")
            .header("host", "127.0.0.1:8382")
            .header("remote-user", HeaderValue::from_bytes(b"caf\xe9").unwrap())
    };
    let reply = proxied.raw(odd(), None).await;
    assert_eq!(
        (reply.status, reply.json),
        (StatusCode::UNAUTHORIZED, json!({"detail": "Not signed in"}))
    );

    let mut bypass = client_with(proxy_auth(), "192.0.2.1");
    assert_eq!(
        bypass.raw(odd(), None).await.status,
        StatusCode::OK,
        "an untrusted peer's header is ignored"
    );
}
