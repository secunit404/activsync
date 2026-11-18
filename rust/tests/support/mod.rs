// Each test crate compiles its own copy of this module and uses only some of it.
#![allow(dead_code)]

use activsync::db;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::Utc;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

/// A same-origin API request: the status, the JSON body (`Null` when empty)
/// and any `Location` header.
pub async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, Option<String>) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "127.0.0.1:8382")
        .header("x-activsync-request", "1")
        .header("sec-fetch-site", "same-origin");
    let body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json, location)
}

pub async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, body, _) = send(app, method, uri, body).await;
    (status, body)
}

/// Garmin and Strava connected with the dev mock's credentials.
pub fn connect_services(conn: &rusqlite::Connection) {
    for (key, value) in [
        (
            "garmin_credentials",
            json!({"email": "dev@example.com", "password": "dev-password"}),
        ),
        ("garmin_credentials_verified", json!(true)),
        ("garmin_last_sync_at", json!(db::iso(Utc::now()))),
        ("garmin_last_sync_ok", json!(true)),
        (
            "strava_credentials",
            json!({"client_id": "12345", "client_secret": "dev-secret"}),
        ),
        (
            "strava_tokens",
            json!({"refresh_token": "dev-refresh-token"}),
        ),
    ] {
        db::set_config_value(conn, key, &value).unwrap();
    }
}
