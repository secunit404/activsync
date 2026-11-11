use activsync::auth_guard::AuthConfig;
use activsync::db::Db;
use activsync::{AppState, build_app, schedule};
use axum::body::Body;
use axum::http::{Request, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

async fn match_mode(development: bool) -> Value {
    let state = AppState::new(Db::open_in_memory().unwrap(), false, schedule::system_clock(), 0.0, AuthConfig::sign_in_off())
        .with_development(development);
    let request = Request::builder()
        .uri("/api/v1/settings")
        .header(header::HOST, "127.0.0.1:8381")
        .header("x-activsync-request", "1")
        .header("sec-fetch-site", "same-origin")
        .body(Body::empty())
        .unwrap();
    let response = build_app(state, None).oneshot(request).await.unwrap();
    let body: Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["development"], false, "only mock data counts as development on the settings page");
    body["hevy"]["matchMode"].clone()
}

#[tokio::test]
async fn real_accounts_in_manual_only_mode_default_to_reviewing_matches() {
    assert_eq!(match_mode(true).await, "review");
    assert_eq!(match_mode(false).await, "automatic");
}
