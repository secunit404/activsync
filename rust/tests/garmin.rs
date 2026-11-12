use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use activsync::garmin_client::{GarminClient, GarminError, Tokens, UploadResult, save_tokens, token_file_path};
use activsync::net::{Http, HttpError, Timeout};
use activsync::schedule::Clock;
use base64::Engine;
use chrono::{TimeZone, Utc};
use openidconnect::HttpRequest;
use serde_json::{Value, json};

type Seen = Arc<Mutex<Vec<HttpRequest>>>;
type Reply = Result<(u16, Vec<u8>), &'static str>;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/garmin_python.json")).unwrap()
}

fn now() -> f64 {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap().timestamp() as f64
}

fn clock() -> Clock {
    Arc::new(now)
}

/// Garmin stand-in: `route` sees "METHOD /path?query" and the request.
fn garmin(route: impl Fn(&str, &HttpRequest) -> Reply + Send + Sync + 'static) -> (Http, Seen) {
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let http: Http = Arc::new(move |request: HttpRequest| {
        let uri = request.uri();
        let key = format!("{} {}{}", request.method(), uri.path(), uri.query().map(|q| format!("?{q}")).unwrap_or_default());
        let reply = route(&key, &request)
            .map_err(|e| HttpError(e.into()))
            .map(|(status, body)| openidconnect::http::Response::builder().status(status).body(body).unwrap());
        log.lock().unwrap().push(request);
        Box::pin(async move { reply })
    });
    (http, seen)
}

fn ok(body: Value) -> Reply {
    Ok((200, body.to_string().into_bytes()))
}

fn keys(seen: &Seen) -> Vec<String> {
    seen.lock().unwrap().iter().map(|r| format!("{} {}", r.method(), r.uri())).collect()
}

fn jwt(claims: Value) -> String {
    let b64 = |v: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
    format!("{}.{}.signature", b64(json!({"alg": "RS256", "typ": "JWT"})), b64(claims))
}

fn fresh_tokens() -> Tokens {
    Tokens {
        di_token: Some(jwt(json!({"exp": now() + 3600.0, "client_id": "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2"}))),
        di_refresh_token: Some("refresh-1".into()),
        di_client_id: Some("GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2".into()),
    }
}

/// macOS keeps temp folders behind the /var symlink, which the token store
/// rightly refuses; tests use the resolved path.
fn store() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    (dir, path)
}

fn client(http: Http) -> GarminClient {
    GarminClient::new(http, clock(), fresh_tokens(), None).without_waits()
}

fn profile_routes(key: &str) -> Option<Reply> {
    match key {
        "GET /userprofile-service/socialProfile" => Some(ok(json!({"displayName": "runner 1/x", "fullName": "R"}))),
        "GET /userprofile-service/userprofile/user-settings" => Some(ok(json!({"userData": {"measurementSystem": "metric"}}))),
        _ => None,
    }
}

#[tokio::test]
async fn a_saved_session_opens_with_the_android_app_headers() {
    let (_dir, path) = store();
    let token_dir = path.to_str().unwrap();
    save_tokens(token_dir, &fresh_tokens()).unwrap();
    let (http, seen) = garmin(|key, _| profile_routes(key).unwrap_or_else(|| ok(json!({"heart": true}))));
    let client = GarminClient::open_with(http, clock(), token_dir, Duration::ZERO).await.unwrap();
    assert_eq!(client.display_name(), Some("runner 1/x"));

    let first = seen.lock().unwrap()[0].clone();
    assert_eq!(first.uri().to_string(), "https://connectapi.garmin.com/userprofile-service/socialProfile");
    assert_eq!(first.headers()["authorization"], format!("Bearer {}", fresh_tokens().di_token.unwrap()));
    assert_eq!(first.headers()["user-agent"], "GCM-Android-5.23");
    assert_eq!(first.headers()["x-garmin-client-platform"], "Android");
    assert_eq!(first.headers()["accept"], "application/json");

    assert_eq!(client.get_daily_heart_rates("2026-11-09").await.unwrap(), json!({"heart": true}));
    assert_eq!(
        keys(&seen).last().unwrap(),
        "GET https://connectapi.garmin.com/wellness-service/wellness/dailyHeartRate/runner%201%2Fx?date=2026-11-09"
    );
    assert!(matches!(client.get_daily_heart_rates("2026-13-01").await, Err(GarminError::Invalid(_))));
}

#[tokio::test]
async fn a_missing_or_rejected_session_needs_a_reconnect() {
    let (_dir, path) = store();
    let token_dir = path.to_str().unwrap();
    let open = |http| GarminClient::open_with(http, clock(), token_dir, Duration::ZERO);
    let (http, seen) = garmin(|_, _| ok(json!({})));
    assert_eq!(open(http.clone()).await.err(), Some(GarminError::SessionExpired));
    std::fs::write(path.join("garmin_tokens.json"), r#"{"di_token": null, "di_refresh_token": "r"}"#).unwrap();
    assert_eq!(open(http).await.err(), Some(GarminError::SessionExpired));
    assert!(seen.lock().unwrap().is_empty(), "no session, no calls");

    save_tokens(token_dir, &fresh_tokens()).unwrap();
    let (http, seen) = garmin(|key, _| if key.starts_with("POST") { Ok((400, b"{}".to_vec())) } else { Ok((401, b"{}".to_vec())) });
    assert_eq!(open(http).await.err(), Some(GarminError::SessionExpired));
    let social = keys(&seen).iter().filter(|k| k.contains("socialProfile")).count();
    assert_eq!(social, 6, "three attempts, each retried once after a refresh");

    let (_real, real) = store();
    save_tokens(real.to_str().unwrap(), &fresh_tokens()).unwrap();
    let link = path.join("linked");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let (http, _) = garmin(|key, _| profile_routes(key).unwrap());
    let linked = GarminClient::open_with(http, clock(), link.to_str().unwrap(), Duration::ZERO).await;
    assert_eq!(linked.err(), Some(GarminError::SessionExpired), "a symlinked token store is refused");
}

#[tokio::test]
async fn an_outage_while_opening_the_session_is_not_an_expiry() {
    let (_dir, path) = store();
    let token_dir = path.to_str().unwrap();
    save_tokens(token_dir, &fresh_tokens()).unwrap();
    let open = |http| GarminClient::open_with(http, clock(), token_dir, Duration::ZERO);

    let (http, _) = garmin(|_, _| Err("connection reset"));
    assert_eq!(open(http).await.err(), Some(GarminError::Unavailable));
    let (http, _) = garmin(|_, _| Ok((503, b"{}".to_vec())));
    assert_eq!(open(http).await.err(), Some(GarminError::Unavailable));
    let (http, _) = garmin(|key, _| match key {
        "GET /userprofile-service/socialProfile" => ok(json!({"displayName": "r"})),
        _ => Ok((429, b"{}".to_vec())),
    });
    assert_eq!(open(http).await.err(), Some(GarminError::Unavailable));
    let (http, _) = garmin(|key, _| if key.starts_with("POST") { Ok((400, b"{}".to_vec())) } else { Ok((403, b"{}".to_vec())) });
    assert_eq!(open(http).await.err(), Some(GarminError::SessionExpired));
    assert_eq!(GarminError::Unavailable.to_string(), "Garmin could not be reached. ActivSync tries again at the next check.");
}

#[tokio::test]
async fn a_token_near_expiry_is_refreshed_and_saved_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, path) = store();
    let token_dir = path.to_str().unwrap();
    let expiring = Tokens { di_token: Some(jwt(json!({"exp": now() + 100.0}))), ..fresh_tokens() };
    save_tokens(token_dir, &expiring).unwrap();
    let new_token = jwt(json!({"exp": now() + 7200.0, "client_id": "GARMIN_CONNECT_MOBILE_IOS_DI"}));
    let issued = new_token.clone();
    let (http, seen) = garmin(move |key, _| match key {
        "POST /di-oauth2-service/oauth/token" => ok(json!({"access_token": issued, "refresh_token": "refresh-2"})),
        other => profile_routes(other).unwrap(),
    });
    let client = GarminClient::open_with(http, clock(), token_dir, Duration::ZERO).await.unwrap();

    let refresh = seen.lock().unwrap()[0].clone();
    assert_eq!(refresh.uri().to_string(), "https://diauth.garmin.com/di-oauth2-service/oauth/token");
    let basic = base64::engine::general_purpose::STANDARD.encode("GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2:");
    assert_eq!(refresh.headers()["authorization"], format!("Basic {basic}"));
    assert_eq!(
        String::from_utf8_lossy(refresh.body()),
        "grant_type=refresh_token&client_id=GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2&refresh_token=refresh-1"
    );
    assert_eq!(seen.lock().unwrap()[1].headers()["authorization"], format!("Bearer {new_token}"));

    let saved = Tokens {
        di_token: Some(new_token),
        di_refresh_token: Some("refresh-2".into()),
        di_client_id: Some("GARMIN_CONNECT_MOBILE_IOS_DI".into()),
    };
    assert_eq!(client.tokens(), saved);
    let file = path.join("garmin_tokens.json");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), saved.to_json());
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700);
}

#[tokio::test]
async fn a_401_is_answered_with_one_refresh_and_retry() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let (http, seen) = garmin(move |key, request| match key {
        "POST /di-oauth2-service/oauth/token" => ok(json!({"access_token": "new.token.x"})),
        _ if request.headers()["authorization"] == "Bearer new.token.x" => ok(json!([{"typeKey": "running"}])),
        _ => {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok((401, b"{}".to_vec()))
        }
    });
    let client = client(http);
    let types = client.fetch_activity_types().await.unwrap();
    assert_eq!(types, vec![json!({"type_key": "running", "label": "Running"})]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(keys(&seen).len(), 3);
    assert_eq!(client.tokens().di_refresh_token.as_deref(), Some("refresh-1"), "kept when Garmin sends none");
}

#[tokio::test]
async fn error_text_matches_the_python_library() {
    for case in fixture()["errors"].as_array().unwrap() {
        let body = case["body"].as_str().map_or_else(|| case["body"].to_string(), str::to_string);
        let status = case["status"].as_u64().unwrap() as u16;
        let (http, _) = garmin(move |_, _| Ok((status, body.clone().into_bytes())));
        let err = client(http.clone()).delete_activity(5).await.unwrap_err();
        assert_eq!(err.to_string(), case["message"].as_str().unwrap());
        assert_eq!(matches!(err, GarminError::NotFound(_)), case["not_found"].as_bool().unwrap());

        let message = case["message"].as_str().unwrap();
        let wrapped = client(http).fetch_activity_types().await.unwrap_err();
        let expected = match status {
            404 => format!("API call client error (404): {message}"),
            400..=499 => format!("API call client error ({status}): {message}"),
            _ => format!("API call HTTP error: {message}"),
        };
        assert_eq!(wrapped.to_string(), expected);
        assert_eq!(matches!(wrapped, GarminError::NotFound(_)), case["not_found"].as_bool().unwrap());
    }
}

fn count(seen: &Seen, prefix: &str) -> usize {
    keys(seen).iter().filter(|k| k.starts_with(prefix)).count()
}

fn timeouts(seen: &Seen) -> Vec<Option<Duration>> {
    seen.lock().unwrap().iter().map(|r| r.extensions().get::<Timeout>().map(|t| t.0)).collect()
}

#[tokio::test]
async fn reads_retry_a_5xx_or_a_network_failure_three_times_like_the_library() {
    let (http, seen) = garmin(|_, _| Ok((503, br#"{"message": "busy"}"#.to_vec())));
    let err = client(http).fetch_activity_types().await.unwrap_err();
    assert_eq!(err.to_string(), "API call HTTP error: API Error 503 - busy");
    assert!(err.is_transient());
    assert_eq!(keys(&seen).len(), 4, "one call and three retries");

    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let (http, _) = garmin(move |_, _| {
        if counter.fetch_add(1, Ordering::SeqCst) < 2 { Ok((500, b"{}".to_vec())) } else { ok(json!([{"typeKey": "running"}])) }
    });
    assert_eq!(client(http).fetch_activity_types().await.unwrap().len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    let (http, seen) = garmin(|_, _| Err("connection reset"));
    let err = client(http).fetch_activity_types().await.unwrap_err();
    assert_eq!(err, GarminError::Network("Connection error: connection reset".into()));
    assert!(err.is_transient());
    assert_eq!(keys(&seen).len(), 4);

    let (http, seen) = garmin(|_, _| Ok((503, b"{}".to_vec())));
    let err = client(http).download_fit(5).await.unwrap_err();
    assert_eq!(err.to_string(), "Download HTTP error: API Error 503");
    assert_eq!(keys(&seen).len(), 4);

    for (status, expected) in [
        (400, "API call client error (400): API Error 400"),
        (403, "API call client error (403): API Error 403"),
        (404, "API call client error (404): API Error 404"),
    ] {
        let (http, seen) = garmin(move |_, _| Ok((status, b"{}".to_vec())));
        let err = client(http).fetch_activity_types().await.unwrap_err();
        assert_eq!(err.to_string(), expected);
        assert!(!err.is_transient());
        assert_eq!(keys(&seen).len(), 1, "a 4xx is not retried");
    }
}

#[tokio::test]
async fn a_rejected_token_is_worded_as_an_authentication_failure() {
    let (http, seen) = garmin(|_, _| Ok((401, b"{}".to_vec())));
    let err = client(http).fetch_activity_types().await.unwrap_err();
    assert_eq!(err.to_string(), "Authentication failed: API Error 401");
    assert!(!err.is_transient());
    assert_eq!(count(&seen, "GET"), 2, "one retry after the refresh, no more");
}

#[tokio::test]
async fn calls_the_library_does_not_wrap_are_sent_once() {
    let (http, seen) = garmin(|_, _| Ok((503, b"{}".to_vec())));
    let plain = client(http);
    assert_eq!(plain.delete_activity(5).await.unwrap_err().to_string(), "API Error 503");
    assert_eq!(plain.set_title(5, "x").await.unwrap_err().to_string(), "API Error 503");
    assert_eq!(plain.put_exercise_sets(5, &json!({})).await.unwrap_err().to_string(), "API Error 503");
    assert_eq!(keys(&seen).len(), 3);

    let (http, seen) = garmin(|_, _| Err("connection reset"));
    assert_eq!(client(http).delete_activity(5).await, Err(GarminError::Network("connection reset".into())));
    assert_eq!(keys(&seen).len(), 1);

    let (http, seen) = garmin(|_, _| Ok((429, b"{}".to_vec())));
    let (_dir, path) = store();
    let fit = path.join("w.fit");
    std::fs::write(&fit, b"FIT").unwrap();
    let err = client(http).upload_fit(&fit).await.unwrap_err();
    assert_eq!(err.to_string(), "API Error 429");
    assert!(err.is_transient());
    assert_eq!(keys(&seen).len(), 1, "the library words an upload's 429 as a plain error, which the rate limiter leaves alone");

    let (_dir, path) = store();
    let token_dir = path.to_str().unwrap();
    save_tokens(token_dir, &fresh_tokens()).unwrap();
    let (http, seen) = garmin(|_, _| Ok((503, b"{}".to_vec())));
    assert_eq!(GarminClient::open_with(http, clock(), token_dir, Duration::ZERO).await.err(), Some(GarminError::Unavailable));
    assert_eq!(count(&seen, "GET"), 3, "the sign-in read makes its own three attempts and no more");
}

#[tokio::test]
async fn a_rate_limit_is_waited_out_twice_before_it_is_given_up() {
    let (http, seen) = garmin(|_, _| Ok((429, b"{}".to_vec())));
    let slow = client(http).with_waits(Duration::ZERO, Duration::from_millis(20), Duration::ZERO);
    let started = std::time::Instant::now();
    let err = slow.fetch_recent_activities(7).await.unwrap_err();
    assert_eq!(err.to_string(), "Rate limit exceeded: API Error 429");
    assert!(err.is_transient());
    assert_eq!(keys(&seen).len(), 3);
    assert!(started.elapsed() >= Duration::from_millis(60), "20 ms, then 40 ms");

    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let (http, _) = garmin(move |_, _| {
        if counter.fetch_add(1, Ordering::SeqCst) == 0 { Ok((429, b"{}".to_vec())) } else { ok(json!([])) }
    });
    assert_eq!(client(http).fetch_recent_activities(7).await.unwrap(), vec![]);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_5xx_waits_for_a_jittered_backoff() {
    let (http, seen) = garmin(|_, _| Ok((500, b"{}".to_vec())));
    let backing_off = client(http).with_waits(Duration::ZERO, Duration::ZERO, Duration::from_millis(10));
    let started = std::time::Instant::now();
    assert!(backing_off.fetch_activity_types().await.is_err());
    assert_eq!(keys(&seen).len(), 4);
    assert!(started.elapsed() >= Duration::from_millis(35), "at least half of 10, 20 and 40 ms");
}

#[tokio::test]
async fn every_call_carries_the_timeout_the_library_sets() {
    let (_dir, path) = store();
    let token_dir = path.to_str().unwrap();
    let expiring = Tokens { di_token: Some(jwt(json!({"exp": now() + 100.0}))), ..fresh_tokens() };
    save_tokens(token_dir, &expiring).unwrap();
    let issued = jwt(json!({"exp": now() + 7200.0}));
    let (http, seen) = garmin(move |key, _| match key {
        "POST /di-oauth2-service/oauth/token" => ok(json!({"access_token": issued})),
        "GET /download-service/files/activity/5" => Ok((200, zipped(&[("a.fit", b"F")]))),
        "POST /upload-service/upload" => ok(json!({})),
        "GET /userprofile-service/socialProfile" | "GET /userprofile-service/userprofile/user-settings" => profile_routes(key).unwrap(),
        _ => ok(json!([])),
    });
    let opened = GarminClient::open_with(http, clock(), token_dir, Duration::ZERO).await.unwrap();
    let fit = path.join("w.fit");
    std::fs::write(&fit, b"FIT").unwrap();
    opened.fetch_activity_types().await.unwrap();
    opened.download_fit(5).await.unwrap();
    opened.upload_fit(&fit).await.unwrap();
    opened.set_title(5, "t").await.unwrap();

    let secs = |n| Some(Duration::from_secs(n));
    assert_eq!(timeouts(&seen), vec![secs(30), secs(15), secs(15), secs(15), secs(15), secs(15), secs(15)]);
    assert_eq!(seen.lock().unwrap()[0].uri().path(), "/di-oauth2-service/oauth/token");
}

fn record_json(record: &activsync::garmin_client::ActivityRecord) -> Value {
    json!({
        "garmin_activity_id": record.garmin_activity_id, "activity_type": record.activity_type, "title": record.title,
        "description": record.description, "start_time": record.start_time, "distance": record.distance,
        "duration": record.duration, "moving_duration": record.moving_duration, "elapsed_duration": record.elapsed_duration,
        "elevation_gain": record.elevation_gain, "elevation_loss": record.elevation_loss, "calories": record.calories,
        "avg_hr": record.avg_hr, "max_hr": record.max_hr, "avg_power": record.avg_power, "max_power": record.max_power,
        "norm_power": record.norm_power, "aerobic_te": record.aerobic_te, "anaerobic_te": record.anaerobic_te,
        "training_load": record.training_load, "avg_cadence": record.avg_cadence, "max_cadence": record.max_cadence,
        "total_sets": record.total_sets, "total_reps": record.total_reps, "total_volume": record.total_volume,
    })
}

/// Python keeps Garmin's ints as ints; Rust reads metrics as floats.
fn same(rust: &Value, python: &Value) -> bool {
    match (rust.as_f64(), python.as_f64()) {
        (Some(a), Some(b)) => a == b,
        _ => rust == python,
    }
}

#[tokio::test]
async fn recent_activities_page_until_the_lookback_like_python() {
    let reference = fixture();
    let batch = reference["batch"].clone();
    let (http, seen) = garmin(move |key, _| match key {
        "GET /activitylist-service/activities/search/activities?start=0&limit=20" => ok(batch.clone()),
        other => panic!("unexpected {other}"),
    });
    let records = client(http).fetch_recent_activities(7).await.unwrap();
    let expected = reference["records"].as_array().unwrap();
    assert_eq!(records.len(), expected.len());
    for (record, python) in records.iter().zip(expected) {
        let rust = record_json(record);
        for (key, value) in python.as_object().unwrap() {
            assert!(same(&rust[key], value), "{key}: rust {} vs python {value}", rust[key]);
        }
    }
    assert_eq!(keys(&seen).len(), 1, "a short page is the last one");
}

fn zipped(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in members {
        writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[tokio::test]
async fn the_original_download_yields_its_fit_file() {
    let archive = zipped(&[("notes.txt", b"hello"), ("123_ACTIVITY.FIT", b"FITDATA"), ("other.fit", b"NO")]);
    let (http, seen) = garmin(move |key, _| match key {
        "GET /download-service/files/activity/123" => Ok((200, archive.clone())),
        "GET /download-service/files/activity/124" => Ok((200, zipped(&[("notes.txt", b"x")]))),
        _ => Ok((404, b"{}".to_vec())),
    });
    let client = client(http);
    assert_eq!(client.download_fit(123).await.unwrap(), b"FITDATA");
    assert_eq!(seen.lock().unwrap()[0].headers()["accept"], "*/*");
    assert_eq!(
        client.download_fit(124).await.unwrap_err().to_string(),
        "unreadable Garmin response: no .fit file in original download for activity 124"
    );
    assert!(matches!(client.download_fit(125).await, Err(GarminError::NotFound(_))));
    assert!(matches!(client.download_fit(0).await, Err(GarminError::Invalid(_))));
}

#[tokio::test]
async fn uploads_read_garmins_import_result_like_python() {
    let dir = tempfile::tempdir().unwrap();
    let fit = dir.path().join("workout.fit");
    std::fs::write(&fit, b"FIT").unwrap();
    for case in fixture()["uploads"].as_array().unwrap() {
        let response = case["response"].clone();
        let (http, seen) = garmin(move |_, _| ok(response.clone()));
        let result = client(http).upload_fit(&fit).await;
        match case["result"].get("ok") {
            Some(expected) => {
                let expected = UploadResult { upload_id: Some(expected["upload_id"].clone()), activity_id: expected["activity_id"].as_i64() };
                assert_eq!(result.unwrap(), expected);
            }
            None => assert_eq!(result.unwrap_err(), GarminError::UploadRejected(case["result"]["rejected"].as_str().unwrap().into())),
        }
        let request = seen.lock().unwrap()[0].clone();
        assert_eq!(request.uri().path(), "/upload-service/upload");
        let body = String::from_utf8_lossy(request.body());
        assert!(body.contains("Content-Disposition: form-data; name=\"file\"; filename=\"workout.fit\"\r\n\r\nFIT\r\n"));
    }

    let duplicate = fixture()["errors"][1].clone();
    let (http, _) = garmin(move |_, _| Ok((409, duplicate["body"].to_string().into_bytes())));
    assert_eq!(
        client(http).upload_fit(&fit).await.unwrap_err(),
        GarminError::UploadRejected("Garmin rejected upload: API Error 409 - {'code': 202, 'content': 'Duplicate Activity.'}".into())
    );
    let (http, _) = garmin(|_, _| ok(json!({})));
    let missing = dir.path().join("gone.fit");
    assert!(matches!(client(http.clone()).upload_fit(&missing).await, Err(GarminError::Invalid(m)) if m.starts_with("FIT file not found")));
    let csv = dir.path().join("workout.csv");
    std::fs::write(&csv, b"x").unwrap();
    assert!(matches!(client(http).upload_fit(&csv).await, Err(GarminError::Invalid(_))));
}

#[tokio::test]
async fn exercise_sets_and_edits_tell_a_missing_activity_from_an_outage() {
    let (http, seen) = garmin(|key, _| match key {
        "PUT /activity-service/activity/5/exerciseSets" => ok(json!({"message": "Invalid sub-category ARM_CURL"})).map(|(_, b)| (400, b)),
        "PUT /activity-service/activity/6/exerciseSets" | "PUT /activity-service/activity/9" => Ok((404, b"{}".to_vec())),
        "PUT /activity-service/activity/7/exerciseSets" => Err("connection refused for /activity/404/"),
        "GET /activity-service/activity/8/exerciseSets" => ok(json!({"exerciseSets": []})),
        "PUT /activity-service/activity/77" | "DELETE /activity-service/activity/77" => Ok((204, Vec::new())),
        other => panic!("unexpected {other}"),
    });
    let client = client(http);
    let payload = json!({"exerciseSets": [{"setType": "ACTIVE"}]});
    assert!(matches!(client.put_exercise_sets(5, &payload).await, Err(GarminError::SubcategoryRejected(m)) if m.contains("ARM_CURL")));
    assert_eq!(seen.lock().unwrap()[0].headers()["content-type"], "application/json");
    assert_eq!(serde_json::from_slice::<Value>(seen.lock().unwrap()[0].body()).unwrap(), payload);
    assert_eq!(client.put_exercise_sets(6, &payload).await, Err(GarminError::ActivityGone(6)));
    assert!(matches!(client.put_exercise_sets(7, &payload).await, Err(GarminError::Network(_))));
    assert_eq!(client.get_exercise_sets(8).await.unwrap(), json!({"exerciseSets": []}));
    assert_eq!(client.set_title(9, "x").await, Err(GarminError::ActivityGone(9)));

    seen.lock().unwrap().clear();
    client.update_activity_metadata(77, "Leg day", "Squats").await.unwrap();
    client.delete_activity(77).await.unwrap();
    let bodies: Vec<Value> =
        seen.lock().unwrap().iter().take(2).map(|r| serde_json::from_slice(r.body()).unwrap()).collect();
    assert_eq!(bodies, vec![
        json!({"activityId": "77", "activityName": "Leg day"}),
        json!({"activityId": "77", "description": "Squats"}),
    ]);
    assert_eq!(keys(&seen)[2], "DELETE https://connectapi.garmin.com/activity-service/activity/77");
}

#[tokio::test]
async fn the_profile_and_type_list_match_python() {
    let reference = fixture();
    for case in reference["profiles"].as_array().unwrap() {
        let (settings, metrics) = (case["settings"].clone(), case["metrics"].clone());
        let (http, _) = garmin(move |key, _| match key {
            "GET /userprofile-service/userprofile/user-settings" => ok(settings.clone()),
            "GET /metrics-service/metrics/maxmet/daily/2026-11-10/2026-11-10" => ok(metrics.clone()),
            other => panic!("unexpected {other}"),
        });
        let profile = client(http).fetch_user_profile().await;
        let expected = &case["expected"];
        assert_eq!(profile.weight_kg, expected["weight_kg"].as_f64());
        assert_eq!(profile.birth_year, expected["birth_year"].as_i64());
        assert_eq!(profile.sex.as_deref(), expected["sex"].as_str());
        assert_eq!(profile.vo2max, expected["vo2max"].as_f64());
    }

    let (http, _) = garmin(|_, _| Err("down"));
    let profile = client(http).fetch_user_profile().await;
    assert_eq!((profile.weight_kg, profile.vo2max), (None, None), "a failed read is not fatal");

    let raw = reference["raw_types"].clone();
    let (http, _) = garmin(move |_, _| ok(raw.clone()));
    assert_eq!(Value::from(client(http).fetch_activity_types().await.unwrap()), reference["types"]);
}

#[tokio::test]
async fn snapshots_near_a_start_cover_three_days_and_never_hide_an_outage() {
    let (http, seen) = garmin(|key, _| match key {
        "GET /activitylist-service/activities/search/activities?startDate=2026-11-09&start=0&limit=20&endDate=2026-11-11" => {
            ok(json!([{"activityId": 1}, {"activityId": "2"}, {"activityName": "no id"}]))
        }
        "GET /activitylist-service/activities/search/activities?startDate=2026-11-09&start=20&limit=20&endDate=2026-11-11" => ok(json!([])),
        other => panic!("unexpected {other}"),
    });
    assert_eq!(client(http).list_activity_ids_near("2026-11-10 06:30:00").await.unwrap(), vec![1, 2]);
    assert_eq!(keys(&seen).len(), 2);

    let (http, _) = garmin(|_, _| Ok((503, b"{}".to_vec())));
    assert!(client(http.clone()).list_activity_ids_near("2026-11-10T06:30:00Z").await.is_err());
    assert!(matches!(client(http).list_activities_near("soon").await, Err(GarminError::Invalid(_))));
}

#[tokio::test]
async fn token_files_are_shared_with_the_python_library() {
    let tokens = Tokens { di_token: Some("a".into()), di_refresh_token: Some("b".into()), di_client_id: None };
    assert_eq!(tokens.to_json(), fixture()["dumps"].as_str().unwrap());

    let (_dir, path) = store();
    let base = path.to_str().unwrap();
    assert_eq!(token_file_path(base).unwrap(), path.join("garmin_tokens.json"));
    assert_eq!(token_file_path(&format!("{base}/mine.JSON")).unwrap(), path.join("mine.JSON"));
    assert_eq!(token_file_path(&format!("{base}/store")).unwrap(), path.join("store/garmin_tokens.json"));
    assert!(token_file_path("/var/x").is_err() || !std::path::Path::new("/var").is_symlink());
    assert!(token_file_path("~someone/tokens").is_err());

    let (http, _) = garmin(|_, _| Err("no route"));
    let unnamed = client(http).get_daily_heart_rates("2026-11-09").await;
    assert!(matches!(unnamed, Err(GarminError::Api(m)) if m.starts_with("Display name is not set")));

    save_tokens(base, &fresh_tokens()).unwrap();
    let (http, seen) = garmin(|key, _| match key {
        "GET /userprofile-service/socialProfile" => ok(json!({"displayName": ".."})),
        other => profile_routes(other).unwrap(),
    });
    let dotted = GarminClient::open_with(http, clock(), base, Duration::ZERO).await.unwrap();
    assert!(matches!(dotted.get_daily_heart_rates("2026-11-09").await, Err(GarminError::Invalid(_))));
    assert_eq!(keys(&seen).len(), 2, "a `..` display name never reaches Garmin");
}
