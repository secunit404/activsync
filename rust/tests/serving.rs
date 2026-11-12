mod support;

use activsync::auth_guard::AuthConfig;
use activsync::db::Db;
use activsync::{AppState, build_app, schedule};
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use support::call;
use tower::ServiceExt;

struct Server {
    state: AppState,
    app: Router,
}

fn server() -> Server {
    let state = AppState::new(
        Db::open_in_memory().unwrap(),
        true,
        schedule::system_clock(),
        0.0,
        AuthConfig::sign_in_off(),
    );
    Server {
        app: build_app(state.clone(), None),
        state,
    }
}

#[tokio::test]
async fn the_browser_tab_and_home_screen_icons_need_no_sign_in() {
    let auth = AuthConfig {
        disabled: false,
        ..AuthConfig::sign_in_off()
    };
    let state = AppState::new(
        Db::open_in_memory().unwrap(),
        false,
        schedule::system_clock(),
        0.0,
        auth,
    );
    let app = build_app(state, None);
    for (path, file) in [
        (
            "/favicon.ico",
            &include_bytes!("../../src/activsync/static/favicon.png")[..],
        ),
        (
            "/apple-touch-icon.png",
            &include_bytes!("../../src/activsync/static/apple-touch-icon.png")[..],
        ),
    ] {
        let request = Request::builder()
            .uri(path)
            .header(header::HOST, "127.0.0.1:8381")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(
            &response.into_body().collect().await.unwrap().to_bytes()[..],
            file,
            "{path}"
        );
    }
}

#[tokio::test]
async fn health_reports_ok() {
    let (status, body) = call(&server().app, Method::GET, "/health", None).await;
    assert_eq!((status, body), (StatusCode::OK, json!({"status": "ok"})));
}

#[tokio::test]
async fn live_refresh_events_carry_a_data_line() {
    let h = server();
    let request = Request::get("/api/events")
        .header("host", "127.0.0.1:8382")
        .body(Body::empty())
        .unwrap();
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body();
    h.state.events.publish("refresh");
    let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(
        std::str::from_utf8(&frame).unwrap(),
        "event: refresh\ndata:  \n\n"
    );
}

#[tokio::test]
async fn unknown_api_paths_are_404_json() {
    let h = server();
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
    let h = server();
    let app = build_app(h.state.clone(), Some(dir.path()));
    let fetch = |uri: &'static str| {
        let app = app.clone();
        async move {
            let request = Request::get(uri)
                .header("host", "127.0.0.1:8382")
                .body(Body::empty())
                .unwrap();
            let (parts, body) = app.oneshot(request).await.unwrap().into_parts();
            (parts, body.collect().await.unwrap().to_bytes())
        }
    };

    for uri in ["/", "/activities", "/settings/hevy", "/robots.txt"] {
        let (parts, body) = fetch(uri).await;
        assert_eq!(parts.status, StatusCode::OK, "{uri}");
        assert_eq!(body, "<html>spa</html>".as_bytes(), "{uri}");
        assert_eq!(
            parts.headers[header::CONTENT_TYPE],
            "text/html; charset=utf-8",
            "{uri}"
        );
        assert_eq!(parts.headers[header::CACHE_CONTROL], "no-cache", "{uri}");
    }

    let (parts, body) = fetch("/assets/app.js").await;
    assert_eq!(parts.status, StatusCode::OK);
    assert_eq!(body, "console.log(1)".as_bytes());
    assert!(
        !parts.headers.contains_key(header::CACHE_CONTROL),
        "hashed assets are not revalidated on every load"
    );
    assert!(
        parts.headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .ends_with("; charset=utf-8")
    );

    for uri in [
        "/api",
        "/api/",
        "/api/nothing",
        "/assets",
        "/assets/missing.js",
        "/strava/other",
        "/auth",
        "/auth/other",
    ] {
        let (parts, body) = fetch(uri).await;
        assert_eq!(parts.status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"detail": "Not Found"}),
            "{uri}"
        );
        assert!(!parts.headers.contains_key(header::CACHE_CONTROL), "{uri}");
    }
}

#[tokio::test]
async fn every_unknown_path_is_404_without_a_frontend_build() {
    let h = server();
    let request = Request::get("/activities")
        .header("host", "127.0.0.1:8382")
        .body(Body::empty())
        .unwrap();
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_path_id_that_is_not_an_integer_gets_a_json_422() {
    let h = server();
    for (method, uri) in [
        (Method::PUT, "/api/v1/activities/abc"),
        (Method::POST, "/api/v1/activities/abc/exclude"),
        (Method::POST, "/api/v1/activities/1.5/publish"),
        (Method::POST, "/api/v1/activities/abc/publish/schedule"),
    ] {
        let (status, body) = call(&h.app, method, uri, Some(json!({"title": "t"}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
        assert_eq!(
            body["detail"], "Input should be a valid integer, unable to parse string as an integer",
            "{uri}"
        );
    }
    let (status, body) = call(
        &h.app,
        Method::POST,
        "/api/v1/activities/99999999999999999999999/exclude",
        None,
    )
    .await;
    assert_eq!(
        (status, body["detail"].as_str()),
        (StatusCode::NOT_FOUND, Some("Activity was not found"))
    );
}

#[tokio::test]
async fn a_wrong_method_on_a_known_path_gets_a_json_405() {
    let h = server();
    for (method, uri) in [
        (Method::GET, "/api/v1/activities/1/exclude"),
        (Method::DELETE, "/api/v1/activities"),
        (Method::POST, "/api/v1/app"),
        (Method::GET, "/api/v1/activities/publish"),
    ] {
        let (status, body) = call(&h.app, method, uri, None).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{uri}");
        assert_eq!(body, json!({"detail": "Method Not Allowed"}), "{uri}");
    }
    let (status, body) = call(&h.app, Method::GET, "/api/v1/nowhere", None).await;
    assert_eq!(
        (status, body["detail"].as_str()),
        (StatusCode::NOT_FOUND, Some("Not Found"))
    );
    let request = Request::get("/api/v1/activities/1/exclude")
        .header(header::HOST, "127.0.0.1:8381")
        .body(Body::empty())
        .unwrap();
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response
            .headers()
            .get(header::ALLOW)
            .map(|v| v.to_str().unwrap()),
        Some("POST")
    );
}
