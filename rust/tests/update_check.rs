use std::sync::{Arc, Mutex};

use activsync::auth_guard::AuthConfig;
use activsync::db::Db;
use activsync::net::{Http, HttpError};
use activsync::update_check::{self, UpdateCache, UpdateStatus, is_newer};
use activsync::{AppState, build_app, schedule};
use axum::body::Body;
use axum::http::{Request, header};
use http_body_util::BodyExt;
use openidconnect::HttpRequest;
use serde_json::{Value, json};
use tower::ServiceExt;

fn github(answer: Arc<Mutex<(u16, Value)>>) -> Http {
    Arc::new(move |request: HttpRequest| {
        assert_eq!(request.uri().path(), "/repos/secunit404/activsync/releases/latest");
        assert_eq!(request.headers()["user-agent"], "ActivSync-update-check");
        let (status, body) = answer.lock().unwrap().clone();
        let response = openidconnect::http::Response::builder()
            .status(status)
            .body(body.to_string().into_bytes())
            .map_err(|e| HttpError(e.to_string()));
        Box::pin(async move { response })
    })
}

#[test]
fn versions_compare_as_numbers() {
    assert!(is_newer("v1.1.0", "1.0.0"));
    assert!(is_newer("1.10.0", "1.9.3"));
    assert!(is_newer("v1.2.3", "1.2"));
    assert!(!is_newer("1.0.0", "1.0.0"));
    assert!(!is_newer("0.9.0", "1.0.0"));
    assert!(!is_newer("garbage", "1.0.0"));
    assert!(!is_newer("1.0.0", "dev-branch"));
}

#[tokio::test]
async fn a_failed_check_keeps_the_last_answer() {
    let answer = Arc::new(Mutex::new((200, json!({"tag_name": " v2.0.0 "}))));
    let (http, cache) = (github(answer.clone()), UpdateCache::default());
    update_check::refresh(&http, &cache, "1.4.0").await;
    let expected = Some(UpdateStatus { latest: "v2.0.0".into(), available: true });
    assert_eq!(cache.get(), expected);
    *answer.lock().unwrap() = (503, json!({}));
    update_check::refresh(&http, &cache, "1.4.0").await;
    assert_eq!(cache.get(), expected);
}

#[tokio::test]
async fn the_app_state_reports_the_last_answer() {
    let state = AppState::new(Db::open_in_memory().unwrap(), false, schedule::system_clock(), 0.0, AuthConfig::sign_in_off());
    let http = github(Arc::new(Mutex::new((200, json!({"tag_name": "v99.0.0"})))));
    let read = |state: AppState| async move {
        let request = Request::builder()
            .uri("/api/v1/app")
            .header(header::HOST, "127.0.0.1:8381")
            .header("x-activsync-request", "1")
            .header("sec-fetch-site", "same-origin")
            .body(Body::empty())
            .unwrap();
        let response = build_app(state, None).oneshot(request).await.unwrap();
        let body: Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        body["update"].clone()
    };
    assert_eq!((read(state.clone()).await["available"].clone(), read(state.clone()).await["latest"].clone()), (json!(false), Value::Null));
    update_check::refresh(&http, &state.update, "1.0.0").await;
    let update = read(state).await;
    assert_eq!((update["available"].clone(), update["latest"].clone()), (json!(true), json!("v99.0.0")));
    assert_eq!(update["releaseUrl"], "https://github.com/secunit404/activsync/releases/latest");
}
