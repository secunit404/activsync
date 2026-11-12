use activsync::auth_guard::AuthConfig;
use activsync::db::{self, Db};
use activsync::{AppState, build_app, schedule, seed};
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::Utc;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

fn app(onboarded: bool) -> (AppState, Router) {
    let db = Db::open_in_memory().unwrap();
    seed::seed(&mut db.lock(), Utc::now(), onboarded).unwrap();
    if onboarded {
        for (key, value) in [
            ("garmin_credentials", json!({"email": "dev@example.com", "password": "dev-password"})),
            ("garmin_credentials_verified", json!(true)),
            ("garmin_last_sync_at", json!(db::iso(Utc::now()))),
            ("garmin_last_sync_ok", json!(true)),
            ("strava_credentials", json!({"client_id": "12345", "client_secret": "dev-secret"})),
            ("strava_tokens", json!({"refresh_token": "dev-refresh-token"})),
        ] {
            db::set_config_value(&db.lock(), key, &value).unwrap();
        }
    }
    let state = AppState::new(db, true, schedule::system_clock(), schedule::UNDO_WINDOW_SECONDS, AuthConfig::sign_in_off());
    let app = build_app(state.clone(), None);
    (state, app)
}

async fn send(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value, Option<String>) {
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
    let response = app.clone().oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let location = response.headers().get(header::LOCATION).map(|v| v.to_str().unwrap().to_string());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    (status, json, location)
}

async fn ok(app: &Router, method: Method, uri: &str, body: Option<Value>) -> Value {
    let (status, json, _) = send(app, method, uri, body).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {json}");
    json
}

async fn step(app: &Router) -> Value {
    ok(app, Method::GET, "/api/v1/settings", None).await["setup"]["step"].clone()
}

#[tokio::test]
async fn wizard_walks_garmin_mfa_strava_hevy_and_first_sync() {
    let (state, app) = app(false);
    assert_eq!(step(&app).await, "garmin");

    let (status, body, _) =
        send(&app, Method::POST, "/api/v1/setup/garmin", Some(json!({"email": "me@example.com"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], "Enter your Garmin email and password.");

    let mfa = ok(&app, Method::POST, "/api/v1/setup/garmin", Some(json!({
        "email": " me@example.com ", "password": "mfa", "lookbackDays": 14, "detectedTimezone": "Asia/Tokyo"
    }))).await;
    assert_eq!(mfa, json!({"message": "Garmin sent a verification code.", "setupStep": "garmin", "mfaRequired": true}));
    assert_eq!(ok(&app, Method::GET, "/api/v1/settings", None).await["setup"]["mfaRequired"], true);

    let (status, body, _) = send(&app, Method::POST, "/api/v1/garmin/mfa", Some(json!({"code": "000000"}))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["detail"], "Invalid MFA code (dev mock rejects 000000)");

    let verified = ok(&app, Method::POST, "/api/v1/garmin/mfa", Some(json!({"code": "123456"}))).await;
    assert_eq!(verified["setupStep"], "strava");
    let settings = ok(&app, Method::GET, "/api/v1/settings", None).await;
    assert_eq!(settings["credentials"]["garminEmail"], "me@example.com");
    assert_eq!(settings["preferences"]["lookbackDays"], 14);
    assert_eq!(settings["preferences"]["displayTimezone"], "Asia/Tokyo");
    assert_eq!(settings["connections"]["garmin"]["status"], "Not yet synced");
    assert!(settings["activityTypes"].as_array().unwrap().iter().all(|t| t["autosync"] == false), "first run holds every category");

    let (status, _, _) = send(&app, Method::POST, "/api/v1/settings/strava/authorize", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    ok(&app, Method::PUT, "/api/v1/settings/strava-credentials", Some(json!({"clientId": "123", "clientSecret": "s3cret"}))).await;
    let url = ok(&app, Method::POST, "/api/v1/settings/strava/authorize", None).await["url"].as_str().unwrap().to_string();
    assert!(url.starts_with("http://127.0.0.1:8382/strava/callback?code=dev-mock-code&state="), "{url}");
    let callback = url.trim_start_matches("http://127.0.0.1:8382");
    let (status, _, location) = send(&app, Method::GET, callback, None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location.as_deref(), Some("/setup"));
    assert_eq!(step(&app).await, "hevy");

    let (status, body, _) = send(&app, Method::POST, "/api/v1/setup/hevy", Some(json!({"apiKey": "  "}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], "Enter your Hevy API key.");
    let hevy = ok(&app, Method::POST, "/api/v1/setup/hevy", Some(json!({"apiKey": "key"}))).await;
    assert_eq!(hevy["setupStep"], "syncing");
    assert_eq!(ok(&app, Method::GET, "/api/v1/settings", None).await["hevy"]["enabled"], true);

    let done = ok(&app, Method::POST, "/api/v1/setup/initial-sync", None).await;
    assert_eq!(done, json!({"message": "Setup complete. Your activities are ready.", "setupStep": null, "mfaRequired": false}));
    let (status, body, _) = send(&app, Method::POST, "/api/v1/setup/initial-sync", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["detail"], "Setup is already complete.");
    assert_eq!(ok(&app, Method::GET, "/api/v1/app", None).await["setup"]["complete"], true);
    assert!(db::get_config_value(&state.db.lock(), "garmin_last_sync_ok").unwrap() == Some(json!(true)));
}

#[tokio::test]
async fn hevy_can_be_skipped_in_setup() {
    let (state, app) = app(false);
    {
        let conn = state.db.lock();
        db::set_config_value(&conn, "garmin_credentials_verified", &json!(true)).unwrap();
        db::set_config_value(&conn, "strava_tokens", &json!({"refresh_token": "x"})).unwrap();
    }
    assert_eq!(step(&app).await, "hevy");
    let skipped = ok(&app, Method::POST, "/api/v1/setup/hevy/skip", None).await;
    assert_eq!(skipped["setupStep"], "syncing");
}

#[tokio::test]
async fn strava_callback_rejects_bad_state_and_missing_scope() {
    let (state, app) = app(true);
    let (_, _, location) = send(&app, Method::GET, "/strava/callback?code=x&state=forged", None).await;
    assert_eq!(
        location.as_deref(),
        Some("/settings?stravaError=That+Strava+response+did+not+match+the+connection+request%2C+so+it+was+ignored.+Start+the+connection+again.")
    );
    let (_, _, location) = send(&app, Method::GET, "/strava/callback?error=access_denied", None).await;
    assert!(location.unwrap().starts_with("/settings?stravaError=Strava+authorization+was+declined"));

    db::set_config_value(&state.db.lock(), "strava_oauth_state", &json!("abc")).unwrap();
    let (_, _, location) = send(&app, Method::GET, "/strava/callback?code=x&state=abc&scope=read", None).await;
    assert!(location.unwrap().starts_with("/settings?stravaError=Strava+didn%27t+grant+upload+access"));
    let connections = ok(&app, Method::GET, "/api/v1/settings", None).await["connections"].clone();
    assert_eq!(connections["strava"]["status"], "Reconnect needed — Strava did not grant upload access");

    db::set_config_value(&state.db.lock(), "strava_oauth_state", &json!("abc")).unwrap();
    let (_, _, location) = send(&app, Method::GET, "/strava/callback?code=x&state=abc&scope=read,activity:read_all,activity:write", None).await;
    assert_eq!(location.as_deref(), Some("/settings"));
}

#[tokio::test]
async fn settings_state_matches_python_shape() {
    let (_, app) = app(true);
    let s = ok(&app, Method::GET, "/api/v1/settings", None).await;
    let mut keys: Vec<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["activityTypes", "connections", "credentials", "development", "hevy", "preferences", "release", "setup", "timezones", "update", "version"]);
    assert_eq!(s["setup"], json!({"complete": true, "step": null, "mfaRequired": false}));
    assert_eq!(s["credentials"], json!({
        "garminEmail": "dev@example.com", "garminPasswordSaved": true,
        "stravaClientId": "12345", "stravaClientSecretSaved": true
    }));
    assert_eq!(s["preferences"], json!({
        "displayTimezone": "Europe/Stockholm", "garminPollIntervalMinutes": 20,
        "stravaPollIntervalMinutes": 5, "lookbackDays": 7, "undoDelaySeconds": 5
    }));
    assert_eq!(s["timezones"].as_array().unwrap().len(), 27);

    let types = s["activityTypes"].as_array().unwrap();
    assert_eq!(types.len(), 152);
    assert_eq!(types[0], json!({"typeKey": "american_football", "label": "American Football", "autosync": true}));
    let find = |key: &str| types.iter().find(|t| t["typeKey"] == key).unwrap().clone();
    assert_eq!(find("atv_v2")["label"], "Atv V2");
    assert_eq!(find("e_bike_fitness")["label"], "E Bike Fitness");
    assert_eq!(find("cycling")["autosync"], false, "seeded as a held category");

    assert_eq!(s["hevy"], json!({
        "connected": false, "status": "Not connected", "apiKeySaved": false, "enabled": false,
        "watchStrategy": "replace", "matchMode": "review", "titleTemplate": "{clean_title}",
        "descriptionTemplate": "{duration}\n{calories}\n{avg_hr}\n\n{exercises}\n\n{marker}",
        "summaryOnStructured": true, "graceMinutes": 120, "pollIntervalMinutes": 5,
        "identity": {"manufacturer": null, "product": null, "serial": null},
        "identityDisplay": "not yet detected — set automatically from your watch",
        "profileOverride": {"weightKg": null, "birthYear": null, "vo2max": null, "sex": null},
        "profileBaseline": {"weightKg": 80.0, "birthYear": 1990, "vo2max": 45.0, "sex": "male"},
        "profileFromGarmin": false
    }));
}

#[tokio::test]
async fn undo_delay_defaults_to_five_and_rejects_other_values() {
    let (_, app) = app(true);
    assert_eq!(ok(&app, Method::GET, "/api/v1/settings", None).await["preferences"]["undoDelaySeconds"], 5);
    let payload = json!({"displayTimezone": "Europe/Oslo", "garminPollIntervalMinutes": 30, "stravaPollIntervalMinutes": 8, "lookbackDays": 14});

    for bad in [json!(1), json!(-1), json!(60), json!("5")] {
        let mut request = payload.clone();
        request["undoDelaySeconds"] = bad;
        let (status, _, _) = send(&app, Method::PUT, "/api/v1/settings/preferences", Some(request)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let (status, _, _) = send(&app, Method::PUT, "/api/v1/settings/preferences", Some(payload.clone())).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(ok(&app, Method::GET, "/api/v1/settings", None).await["preferences"]["undoDelaySeconds"], 5);

    for good in [0, 3, 5, 10] {
        let mut request = payload.clone();
        request["undoDelaySeconds"] = json!(good);
        ok(&app, Method::PUT, "/api/v1/settings/preferences", Some(request)).await;
        assert_eq!(ok(&app, Method::GET, "/api/v1/settings", None).await["preferences"]["undoDelaySeconds"], good);
    }
}

#[tokio::test]
async fn preferences_save_and_validate_timezone() {
    let (_, app) = app(true);
    let prefs = json!({"displayTimezone": "Mars/Base", "garminPollIntervalMinutes": 30, "stravaPollIntervalMinutes": 10, "lookbackDays": 3, "undoDelaySeconds": 10});
    let (status, body, _) = send(&app, Method::PUT, "/api/v1/settings/preferences", Some(prefs.clone())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], "Unknown timezone: Mars/Base");

    let mut good = prefs.clone();
    good["displayTimezone"] = json!("America/New_York");
    let saved = ok(&app, Method::PUT, "/api/v1/settings/preferences", Some(good.clone())).await;
    assert_eq!(saved["message"], "Preferences saved.");
    assert_eq!(ok(&app, Method::GET, "/api/v1/settings", None).await["preferences"], good);
    assert_eq!(ok(&app, Method::GET, "/api/v1/app", None).await["pollIntervals"], json!({"garminMinutes": 30, "stravaMinutes": 10}));

    let mut zero = good;
    zero["lookbackDays"] = json!(0);
    let (status, _, _) = send(&app, Method::PUT, "/api/v1/settings/preferences", Some(zero)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn autosync_categories_release_category_holds_but_not_backlog() {
    let (state, app) = app(true);
    let settings = ok(&app, Method::GET, "/api/v1/settings", None).await;
    let all: Vec<String> = settings["activityTypes"].as_array().unwrap().iter().map(|t| t["typeKey"].as_str().unwrap().to_string()).collect();

    let (status, body, _) = send(&app, Method::PUT, "/api/v1/settings/activity-types", Some(json!({"autosyncTypes": ["teleporting"]}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], "Unknown activity type.");

    ok(&app, Method::PUT, "/api/v1/settings/activity-types", Some(json!({"autosyncTypes": all}))).await;
    let rows = db::list_activities(&state.db.lock(), false).unwrap();
    let held: Vec<_> = rows.iter().filter(|r| r.publish_status == "held").collect();
    assert!(!held.is_empty());
    assert!(held.iter().all(|r| r.hold_reason.as_deref() == Some("backlog")));
    assert!(ok(&app, Method::GET, "/api/v1/settings", None).await["activityTypes"].as_array().unwrap().iter().all(|t| t["autosync"] == true));

    db::set_config_value(&state.db.lock(), "garmin_credentials_verified", &json!(false)).unwrap();
    let (status, body, _) = send(&app, Method::POST, "/api/v1/settings/activity-types/refresh", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["detail"], "Connect Garmin before refreshing activity types.");
}

#[tokio::test]
async fn manual_sync_reports_per_service() {
    let (state, app) = app(true);
    assert_eq!(ok(&app, Method::POST, "/api/v1/settings/sync/garmin", None).await["message"], "Garmin synced.");
    assert_eq!(ok(&app, Method::POST, "/api/v1/settings/sync/strava", None).await["message"], "Strava synced.");
    ok(&app, Method::DELETE, "/api/v1/settings/strava", None).await;
    let (status, body, _) = send(&app, Method::POST, "/api/v1/settings/sync/strava", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["detail"], "Strava is disconnected. Reconnect Strava to sync.");
    assert_eq!(db::get_config_value(&state.db.lock(), "strava_tokens").unwrap(), Some(Value::Null));
}

#[tokio::test]
async fn reconnect_garmin_reuses_saved_password() {
    let (state, app) = app(true);
    db::set_config_value(&state.db.lock(), "garmin_credentials_verified", &json!(false)).unwrap();
    let done = ok(&app, Method::POST, "/api/v1/settings/garmin/reconnect", Some(json!({"email": ""}))).await;
    assert_eq!(done["message"], "Garmin reconnected and catch-up sync started.");
    let app_state = ok(&app, Method::GET, "/api/v1/app", None).await;
    assert_eq!(app_state["connections"]["garmin"]["connected"], true);
    assert_eq!(app_state["connections"]["garmin"]["status"], "Connected");

    ok(&app, Method::POST, "/api/v1/settings/garmin/reconnect", Some(json!({"email": "me@x.com", "password": "mfa"}))).await;
    let cancelled = ok(&app, Method::DELETE, "/api/v1/garmin/mfa", None).await;
    assert_eq!(cancelled["message"], "Garmin verification cancelled.");
    let (status, _, _) = send(&app, Method::POST, "/api/v1/garmin/mfa", Some(json!({"code": "1"}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn strava_credentials_change_drops_old_tokens() {
    let (state, app) = app(true);
    ok(&app, Method::PUT, "/api/v1/settings/strava-credentials", Some(json!({"clientId": "12345"}))).await;
    assert_eq!(db::get_config_value(&state.db.lock(), "strava_tokens").unwrap().unwrap()["refresh_token"], "dev-refresh-token");
    ok(&app, Method::PUT, "/api/v1/settings/strava-credentials", Some(json!({"clientId": "999"}))).await;
    assert_eq!(db::get_config_value(&state.db.lock(), "strava_tokens").unwrap(), Some(Value::Null));
    assert_eq!(ok(&app, Method::GET, "/api/v1/settings", None).await["credentials"]["stravaClientSecretSaved"], true);
}

fn hevy_body() -> Value {
    json!({
        "enabled": true, "watchStrategy": "merge", "matchMode": "automatic",
        "titleTemplate": "{clean_title} (Hevy)", "descriptionTemplate": "{duration}\n{exercises}",
        "summaryOnStructured": false, "graceMinutes": 30, "pollIntervalMinutes": 10,
        "identity": {"manufacturer": 1, "product": 4315, "serial": 123},
        "profileOverride": {"weightKg": 72.5, "birthYear": null, "vo2max": null, "sex": "female"}
    })
}

#[tokio::test]
async fn hevy_settings_save_and_reject_bad_input() {
    let (state, app) = app(true);
    ok(&app, Method::POST, "/api/v1/settings/hevy/credentials", Some(json!({"apiKey": "k"}))).await;
    ok(&app, Method::PUT, "/api/v1/settings/hevy", Some(hevy_body())).await;
    let hevy = ok(&app, Method::GET, "/api/v1/settings", None).await["hevy"].clone();
    assert_eq!(hevy["connected"], true);
    assert_eq!(hevy["watchStrategy"], "merge");
    assert_eq!(hevy["titleTemplate"], "{clean_title} (Hevy)");
    assert_eq!(hevy["identity"], json!({"manufacturer": 1, "product": 4315, "serial": 123}));
    assert_eq!(hevy["profileOverride"], json!({"weightKg": 72.5, "birthYear": null, "vo2max": null, "sex": "female"}));
    let stored = db::get_config_value(&state.db.lock(), "settings").unwrap().unwrap();
    assert_eq!(stored["profile_override"], json!({"weight_kg": 72.5, "sex": "female"}), "stored the way Python stores it");

    let mut partial = hevy_body();
    partial["identity"] = json!({"manufacturer": 1});
    let (status, body, _) = send(&app, Method::PUT, "/api/v1/settings/hevy", Some(partial)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], "Enter all three device identity values, or leave all three blank.");

    let mut bad_template = hevy_body();
    bad_template["titleTemplate"] = json!("{nope}");
    let (status, body, _) = send(&app, Method::PUT, "/api/v1/settings/hevy", Some(bad_template)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], "Unknown title placeholder: {nope}.");

    let mut bad_strategy = hevy_body();
    bad_strategy["watchStrategy"] = json!("yolo");
    let (status, _, _) = send(&app, Method::PUT, "/api/v1/settings/hevy", Some(bad_strategy)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let mut cleared = hevy_body();
    cleared["identity"] = json!({});
    cleared["profileOverride"] = json!({});
    ok(&app, Method::PUT, "/api/v1/settings/hevy", Some(cleared)).await;
    let stored = db::get_config_value(&state.db.lock(), "settings").unwrap().unwrap();
    assert!(stored.get("profile_override").is_none());
    assert_eq!(stored["hevy_device_identity_override"], Value::Null);

    ok(&app, Method::DELETE, "/api/v1/settings/hevy", None).await;
    let hevy = ok(&app, Method::GET, "/api/v1/settings", None).await["hevy"].clone();
    assert_eq!((hevy["connected"].clone(), hevy["enabled"].clone()), (json!(false), json!(false)));
}

#[tokio::test]
async fn device_options_and_access_section_load() {
    let (_, app) = app(true);
    let options = ok(&app, Method::GET, "/api/v1/settings/hevy/device-options", None).await;
    assert_eq!(options["manufacturers"], json!([{"value": 1, "label": "Garmin"}]));
    assert!(options["products"].as_array().unwrap().len() > 100);
    assert_eq!(ok(&app, Method::GET, "/api/v1/auth/settings", None).await, json!({"proxy": null}));
    assert_eq!(ok(&app, Method::GET, "/api/v1/auth/sessions", None).await, json!([]));
    let oidc = ok(&app, Method::GET, "/api/v1/auth/oidc", None).await;
    assert_eq!(oidc["configured"], false);
    assert_eq!(oidc["redirectUri"], "http://127.0.0.1:8382/auth/oidc/callback");
}

#[tokio::test]
async fn dev_onboarding_flag_reopens_the_wizard() {
    let (_, app) = app(true);
    ok(&app, Method::POST, "/api/v1/dev/onboarding-state", Some(json!({"complete": false}))).await;
    assert_eq!(ok(&app, Method::GET, "/api/v1/app", None).await["setup"]["step"], "hevy");
}

#[test]
fn default_undo_delay_is_five_seconds() {
    let db = Db::open_in_memory().unwrap();
    assert_eq!(activsync::config::load_config(&db.lock(), false).unwrap().int("undo_delay_seconds"), 5);
}
