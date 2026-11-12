mod support;

use activsync::auth_guard::AuthConfig;
use activsync::db::Db;
use activsync::{AppState, build_app, schedule};
use axum::http::Method;
use serde_json::Value;
use support::call;

async fn match_mode(development: bool) -> Value {
    let state = AppState::new(
        Db::open_in_memory().unwrap(),
        false,
        schedule::system_clock(),
        0.0,
        AuthConfig::sign_in_off(),
    )
    .with_development(development);
    let (_, body) = call(
        &build_app(state, None),
        Method::GET,
        "/api/v1/settings",
        None,
    )
    .await;
    assert_eq!(
        body["development"], false,
        "only mock data counts as development on the settings page"
    );
    body["hevy"]["matchMode"].clone()
}

#[tokio::test]
async fn real_accounts_in_manual_only_mode_default_to_reviewing_matches() {
    assert_eq!(match_mode(true).await, "review");
    assert_eq!(match_mode(false).await, "automatic");
}
