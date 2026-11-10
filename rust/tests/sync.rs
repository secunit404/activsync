use std::collections::HashSet;
use std::io::Write;
use std::sync::{Arc, Mutex};

use activsync::db::{self, Db, NewActivity};
use activsync::garmin_client::{GarminClient, Tokens};
use activsync::net::{Http, HttpError};
use activsync::strava_client::StravaClient;
use activsync::sync::{self, Services};
use base64::Engine;
use chrono::{DateTime, Duration, TimeZone, Utc};
use openidconnect::HttpRequest;
use rusqlite::types::ValueRef;
use serde_json::{Value, json};

type Seen = Arc<Mutex<Vec<HttpRequest>>>;
type Reply = Result<(u16, Vec<u8>), &'static str>;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap()
}

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A stand-in service: `route` sees "METHOD /path?query" and the request.
fn service(route: impl Fn(&str, &HttpRequest) -> Reply + Send + Sync + 'static) -> (Http, Seen) {
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

fn fake_garmin(route: impl Fn(&str, &HttpRequest) -> Reply + Send + Sync + 'static) -> (GarminClient, Seen) {
    let (http, seen) = service(route);
    let tokens = Tokens { di_token: Some("token".into()), di_refresh_token: Some("refresh".into()), di_client_id: Some("id".into()) };
    (GarminClient::new(http, Arc::new(|| now().timestamp() as f64), tokens, None).without_waits(), seen)
}

fn fake_strava(db: &Db, route: impl Fn(&str, &HttpRequest) -> Reply + Send + Sync + 'static) -> (StravaClient, Seen) {
    let tokens = json!({"access_token": "at", "refresh_token": "rt", "expires_at": now().timestamp() + 21600});
    db::set_config_value(&db.lock(), "strava_tokens", &tokens).unwrap();
    let (http, seen) = service(route);
    (StravaClient::new(http, db.clone(), Arc::new(|| now().timestamp() as f64), "123", "shh").without_waits(), seen)
}

fn config(db: &Db) -> activsync::config::Config {
    activsync::config::load_config(&db.lock(), false).unwrap()
}

fn settings(db: &Db, values: Value) {
    db::set_config_value(&db.lock(), "settings", &values).unwrap();
}

fn gmt(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn add(db: &Db, id: i64, activity_type: &str, start: DateTime<Utc>, status: &str, garmin_data: &str) {
    let new = NewActivity {
        garmin_activity_id: id,
        activity_type,
        title: &format!("Activity {id}"),
        description: "",
        start_time: &gmt(start),
        content_hash: "x",
        publish_status: status,
        garmin_data,
        hold_reason: (status == "held").then_some("category"),
    };
    db::insert_activity(&db.lock(), &new, now()).unwrap();
}

fn hevy_workout(db: &Db, hevy_id: &str, status: &str, target: i64, payload: &Value) {
    db.lock()
        .execute(
            "INSERT INTO hevy_workouts (hevy_id, title, start_time, end_time, payload, status, garmin_activity_id, created_at, updated_at)
             VALUES (?1, 'Push', '2026-11-10T08:00:00Z', '2026-11-10T09:00:00Z', ?2, ?3, ?4, '2026-11-10', '2026-11-10')",
            rusqlite::params![hevy_id, payload.to_string(), status, target],
        )
        .unwrap();
}

fn rows(db: &Db) -> Vec<Value> {
    let conn = db.lock();
    let mut stmt = conn.prepare("SELECT * FROM activities ORDER BY garmin_activity_id").unwrap();
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_string).collect();
    stmt.query_map([], |row| {
        let mut object = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(i) => json!(i),
                ValueRef::Real(f) => json!(f),
                ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                ValueRef::Blob(_) => json!("<blob>"),
            };
            object.insert(name.clone(), value);
        }
        Ok(Value::Object(object))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn zipped(fit: &[u8]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer.start_file("activity.fit", zip::write::SimpleFileOptions::default()).unwrap();
    writer.write_all(fit).unwrap();
    writer.finish().unwrap().into_inner()
}

/// The multipart field named `name` in an upload body.
fn part(request: &HttpRequest, name: &str) -> Option<String> {
    let body = String::from_utf8_lossy(request.body()).into_owned();
    let marker = format!("name=\"{name}\"");
    let after = &body[body.find(&marker)? + marker.len()..];
    let content = &after[after.find("\r\n\r\n")? + 4..];
    Some(content[..content.find("\r\n--")?].to_string())
}

#[tokio::test]
async fn the_garmin_sync_writes_the_same_rows_as_python() {
    let reference = fixture("sync_python.json");
    let db = Db::open_in_memory().unwrap();
    let first_seen = DateTime::parse_from_rfc3339(reference["first_seen"].as_str().unwrap()).unwrap().with_timezone(&Utc);
    for row in reference["existing"].as_array().unwrap() {
        let text = |key: &str| row[key].as_str().unwrap().to_string();
        let new = NewActivity {
            garmin_activity_id: row["garmin_activity_id"].as_i64().unwrap(),
            activity_type: &text("activity_type"),
            title: &text("title"),
            description: &text("description"),
            start_time: &text("start_time"),
            content_hash: &text("content_hash"),
            publish_status: &text("publish_status"),
            garmin_data: &text("garmin_data"),
            hold_reason: row["hold_reason"].as_str(),
        };
        db::insert_activity(&db.lock(), &new, first_seen).unwrap();
    }
    settings(&db, json!({"lookback_days": 7, "held_activity_types": ["strength_training"]}));
    let batch = reference["batch"].clone();
    let (garmin, _) = fake_garmin(move |key, _| match key {
        "GET /activitylist-service/activities/search/activities?start=0&limit=20" => ok(batch.clone()),
        other => panic!("unexpected {other}"),
    });

    let stats = sync::sync_garmin(&db, &garmin, &config(&db), now(), Some(14), Some(now() - Duration::days(7))).await.unwrap();
    let expected = &reference["stats"];
    assert_eq!(
        (stats.new, stats.updated, stats.removed, stats.held_backlog),
        (expected["new"].as_i64().unwrap(), expected["updated"].as_i64().unwrap(), expected["removed"].as_i64().unwrap(), expected["held_backlog"].as_i64().unwrap())
    );
    assert_eq!(Value::from(rows(&db)), reference["rows"]);
    for (key, value) in reference["config"].as_object().unwrap() {
        assert_eq!(db::get_config_value(&db.lock(), key).unwrap().unwrap_or(Value::Null), *value, "{key}");
    }
}

#[tokio::test]
async fn a_failed_garmin_fetch_is_recorded_for_the_settings_page() {
    let db = Db::open_in_memory().unwrap();
    add(&db, 1, "running", now() - Duration::hours(1), "pending", "{}");
    db::set_config_value(&db.lock(), "garmin_credentials_verified", &json!(true)).unwrap();
    let (garmin, _) = fake_garmin(|_, _| Ok((500, b"{}".to_vec())));
    assert!(sync::sync_garmin(&db, &garmin, &config(&db), now(), None, None).await.is_err());
    let value = |key: &str| db::get_config_value(&db.lock(), key).unwrap();
    assert_eq!(value("garmin_last_sync_ok"), Some(json!(false)));
    assert_eq!(value("garmin_last_sync_error"), Some(json!("API Error 500")));
    assert_eq!(value("garmin_credentials_verified"), Some(json!(true)), "an outage is retried, not a disconnect");
    assert_eq!(value("garmin_last_sync_at"), Some(json!("2026-11-10T12:00:00+00:00")));
    assert_eq!(value("garmin_last_sync_ok_at"), None);
    assert_eq!(rows(&db).len(), 1, "nothing is removed on a failed fetch");

    let (garmin, _) = fake_garmin(|_, _| Err("connection reset"));
    assert!(sync::sync_garmin(&db, &garmin, &config(&db), now(), None, None).await.is_err());
    assert_eq!(value("garmin_credentials_verified"), Some(json!(true)));

    let (garmin, _) = fake_garmin(|key, _| if key.starts_with("POST") { Ok((400, b"{}".to_vec())) } else { Ok((401, b"{}".to_vec())) });
    assert!(sync::sync_garmin(&db, &garmin, &config(&db), now(), None, None).await.is_err());
    assert_eq!(value("garmin_credentials_verified"), Some(json!(false)), "a rejected token pauses Garmin");
}

/// Strava: one known activity starting at `twin_start`, uploads resolving to
/// activity 555.
fn strava_routes(twin_start: DateTime<Utc>) -> impl Fn(&str, &HttpRequest) -> Reply + Send + Sync + 'static {
    move |key, _| {
        if key.starts_with("GET /api/v3/athlete/activities") {
            ok(json!([{"id": 777, "start_date": twin_start.format("%Y-%m-%dT%H:%M:%SZ").to_string()}]))
        } else if key == "POST /api/v3/uploads" {
            ok(json!({"id": 9}))
        } else if key == "GET /api/v3/uploads/9" {
            ok(json!({"activity_id": 555}))
        } else {
            panic!("unexpected {key}")
        }
    }
}

#[tokio::test]
async fn the_poller_publishes_pending_rows_and_leaves_the_rest() {
    let db = Db::open_in_memory().unwrap();
    let base = now() - Duration::hours(10);
    add(&db, 201, "running", base, "pending", "{}");
    add(&db, 202, "running", base + Duration::hours(1), "pending", "{}");
    add(&db, 203, "running", base + Duration::hours(2), "pending", "{}");
    add(&db, 204, "running", base + Duration::hours(3), "held", "{}");
    add(&db, 205, "running", base + Duration::hours(4), "pending", "{}");
    hevy_workout(&db, "h1", "waiting_watch", 202, &json!({}));
    db.lock()
        .execute(
            "INSERT INTO publish_schedule (garmin_activity_id, batch_id, batch_kind, deadline, state, prior_status)
             VALUES (203, 'b', 'single', 0, 'waiting', 'pending')",
            [],
        )
        .unwrap();
    let (garmin, garmin_seen) = fake_garmin(|key, _| match key {
        "GET /download-service/files/activity/201" | "GET /download-service/files/activity/204" => Ok((200, zipped(b"FITDATA"))),
        other => panic!("unexpected {other}"),
    });
    let (strava, strava_seen) = fake_strava(&db, strava_routes(base + Duration::hours(4) + Duration::seconds(40)));
    let services = Services { db: &db, garmin: &garmin, strava: &strava };

    let stats = sync::publish_pending(&services, now(), None).await.unwrap();
    assert_eq!((stats.published, stats.failed, stats.blocked), (2, 0, 1));
    assert_eq!(stats.blocked_reasons, vec!["hevy workout h1 is waiting_watch".to_string()]);
    let status = |id| db::get_activity(&db.lock(), id).unwrap().map(|r| (r.publish_status, r.strava_activity_id));
    assert_eq!(status(201), Some(("published".into(), Some(555))));
    assert_eq!(status(205), Some(("published".into(), Some(777))), "linked to its Strava twin, not uploaded");
    assert_eq!(status(202), Some(("pending".into(), None)));
    assert_eq!(status(203), Some(("pending".into(), None)), "waiting out its undo window");
    assert_eq!(status(204), Some(("held".into(), None)));

    let uploads: Vec<HttpRequest> = strava_seen.lock().unwrap().iter().filter(|r| r.method() == "POST").cloned().collect();
    assert_eq!(uploads.len(), 1);
    assert_eq!(part(&uploads[0], "external_id").as_deref(), Some("garmin-201"));
    assert_eq!(part(&uploads[0], "data_type").as_deref(), Some("fit"));
    assert_eq!(part(&uploads[0], "name").as_deref(), Some("Activity 201"));
    assert_eq!(part(&uploads[0], "description"), None, "a blank description is left out");
    assert_eq!(part(&uploads[0], "file").as_deref(), Some("FITDATA"));
    assert_eq!(garmin_seen.lock().unwrap().len(), 1);

    let picked = HashSet::from([204]);
    let manual = sync::publish_pending(&services, now(), Some(&picked)).await.unwrap();
    assert_eq!((manual.published, manual.blocked), (1, 0));
    assert_eq!(status(204), Some(("published".into(), Some(555))));
}

#[tokio::test]
async fn a_strength_activity_waits_for_hevy_unless_published_by_hand() {
    let db = Db::open_in_memory().unwrap();
    settings(&db, json!({"hevy_enabled": true, "hevy_poll_interval_minutes": 5}));
    let start = now() - Duration::hours(2);
    add(&db, 301, "strength_training", start, "pending", r#"{"duration": 3600.0}"#);
    let (garmin, _) = fake_garmin(|_, _| Ok((200, zipped(b"FIT"))));
    let (strava, _) = fake_strava(&db, strava_routes(now() - Duration::days(3)));
    let services = Services { db: &db, garmin: &garmin, strava: &strava };

    let stats = sync::publish_pending(&services, now(), None).await.unwrap();
    assert_eq!(stats.blocked_reasons, vec!["settlement hold".to_string()]);

    let late_enough = start + Duration::minutes(60 + 5 + 5);
    db::set_config_value(&db.lock(), "hevy_last_success_at", &json!(db::iso(late_enough - Duration::seconds(1)))).unwrap();
    assert_eq!(sync::publish_pending(&services, now(), None).await.unwrap().blocked, 1);
    db::set_config_value(&db.lock(), "hevy_last_success_at", &json!(db::iso(late_enough))).unwrap();
    assert_eq!(sync::publish_pending(&services, now(), None).await.unwrap().published, 1);

    add(&db, 302, "strength_training", start, "pending", "not json");
    db::set_config_value(&db.lock(), "hevy_last_success_at", &json!(db::iso(now()))).unwrap();
    assert_eq!(sync::publish_pending(&services, now(), None).await.unwrap().blocked, 1, "no end time, no release");
    sync::publish_now(&services, 302, now()).await.unwrap();
    assert!(db::get_activity(&db.lock(), 302).unwrap().unwrap().is_published());
}

#[tokio::test]
async fn hevy_sets_go_to_strava_by_name_and_fall_back_to_the_fit() {
    let cases = fixture("strava_strength_python.json");
    let timed = &cases[0];
    let fit = base64::engine::general_purpose::STANDARD.decode(timed["fit"].as_str().unwrap()).unwrap();
    let db = Db::open_in_memory().unwrap();
    settings(&db, json!({"display_timezone": "Europe/Stockholm"}));
    add(&db, 401, "strength_training", now() - Duration::hours(5), "pending", "{}");
    add(&db, 402, "strength_training", now() - Duration::hours(3), "pending", "{}");
    hevy_workout(&db, "h-merged", "merged", 401, &timed["workout"]);
    hevy_workout(&db, "h-described", "described", 402, &timed["workout"]);
    let archive = zipped(&fit);
    let (garmin, _) = fake_garmin(move |_, _| Ok((200, archive.clone())));
    let (strava, seen) = fake_strava(&db, strava_routes(now() - Duration::days(3)));
    let services = Services { db: &db, garmin: &garmin, strava: &strava };

    sync::publish_now(&services, 401, now()).await.unwrap();
    sync::publish_now(&services, 402, now()).await.unwrap();
    let uploads: Vec<HttpRequest> = seen.lock().unwrap().iter().filter(|r| r.method() == "POST").cloned().collect();
    assert_eq!(part(&uploads[0], "data_type").as_deref(), Some("json"));
    let document: Value = serde_json::from_str(&part(&uploads[0], "file").unwrap()).unwrap();
    assert_eq!(document, timed["result"]["ok"]);
    assert_eq!(part(&uploads[1], "data_type").as_deref(), Some("fit"), "a described workout keeps the watch's sets");

    let db = Db::open_in_memory().unwrap();
    add(&db, 403, "strength_training", now() - Duration::hours(5), "pending", "{}");
    hevy_workout(&db, "h-replaced", "replaced", 403, &timed["workout"]);
    let archive = zipped(&fit);
    let (garmin, _) = fake_garmin(move |_, _| Ok((200, archive.clone())));
    let uploads_seen = Arc::new(Mutex::new(0));
    let counter = uploads_seen.clone();
    let routes = strava_routes(now() - Duration::days(3));
    let (strava, seen) = fake_strava(&db, move |key, request| {
        if key == "POST /api/v3/uploads" {
            let mut count = counter.lock().unwrap();
            *count += 1;
            if *count == 1 {
                return Ok((400, b"{\"error\": \"unknown exercise\"}".to_vec()));
            }
        }
        routes(key, request)
    });
    let services = Services { db: &db, garmin: &garmin, strava: &strava };
    sync::publish_now(&services, 403, now()).await.unwrap();
    let kinds: Vec<Option<String>> =
        seen.lock().unwrap().iter().filter(|r| r.method() == "POST").map(|r| part(r, "data_type")).collect();
    assert_eq!(kinds, vec![Some("json".into()), Some("fit".into())]);
}

#[test]
fn the_strength_document_matches_python() {
    let db = Db::open_in_memory().unwrap();
    for case in fixture("strava_strength_python.json").as_array().unwrap() {
        let fit = base64::engine::general_purpose::STANDARD.decode(case["fit"].as_str().unwrap()).unwrap();
        let built = activsync::strava_strength::build_strength_upload(&db.lock(), &case["workout"], &fit, case["tz"].as_str());
        let name = case["name"].as_str().unwrap();
        match (&built, case["result"].get("ok")) {
            (Ok(document), Some(expected)) => assert_eq!(document, expected, "{name}"),
            (Err(reason), None) => {
                let expected = case["result"].get("miss").or_else(|| case["result"].get("invalid")).unwrap();
                assert_eq!(reason, expected.as_str().unwrap(), "{name}");
            }
            _ => panic!("{name}: rust {built:?} vs python {}", case["result"]),
        }
    }
}

#[tokio::test]
async fn one_strava_window_links_twins_and_flags_deletions() {
    let db = Db::open_in_memory().unwrap();
    let base = now() - Duration::hours(6);
    add(&db, 501, "running", base, "pending", "{}");
    add(&db, 502, "running", base + Duration::minutes(2), "held", "{}");
    add(&db, 503, "running", base - Duration::days(1), "published", "{}");
    add(&db, 504, "running", base - Duration::days(2), "published", "{}");
    add(&db, 505, "running", base - Duration::days(30), "published", "{}");
    for (id, strava_id) in [(503, 900), (504, 901), (505, 902)] {
        db::set_published(&db.lock(), id, strava_id, now()).unwrap();
    }
    let twin = (base + Duration::minutes(1)).format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let (strava, seen) = fake_strava(&db, move |key, _| {
        assert!(key.starts_with("GET /api/v3/athlete/activities"), "{key}");
        ok(json!([{"id": 901, "start_date": "2026-11-08T06:00:00Z"}, {"id": 800, "start_date": twin}]))
    });
    let stats = sync::check_strava_status(&db, &strava, &config(&db), now(), None).await.unwrap();
    assert_eq!((stats.linked_existing, stats.flagged_missing), (1, 1));
    let status = |id| db::get_activity(&db.lock(), id).unwrap().map(|r| (r.publish_status, r.strava_activity_id));
    assert_eq!(status(501), Some(("published".into(), Some(800))), "pending rows are matched before held ones");
    assert_eq!(status(502), Some(("held".into(), None)), "the twin was already claimed");
    assert_eq!(status(503).unwrap().0, "missing");
    assert_eq!(status(504).unwrap().0, "published");
    assert_eq!(status(505).unwrap().0, "published", "outside the window is unknown, not deleted");

    let query = seen.lock().unwrap()[0].uri().query().unwrap().to_string();
    let after = (now() - Duration::days(7) - Duration::minutes(30)).timestamp();
    let before = (now() + Duration::minutes(30)).timestamp();
    assert_eq!(query, format!("after={after}&before={before}&per_page=200&page=1"));

    let empty = Db::open_in_memory().unwrap();
    let (strava, seen) = fake_strava(&empty, |_, _| panic!("no rows, no request"));
    sync::check_strava_status(&empty, &strava, &config(&empty), now(), None).await.unwrap();
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn catch_up_windows_cover_the_outage_within_bounds() {
    let db = Db::open_in_memory().unwrap();
    let cfg = config(&db);
    let conn = db.lock();
    assert_eq!(sync::catch_up_lookback_days(&conn, &cfg, now(), Some(None)).unwrap(), 7);
    assert_eq!(sync::catch_up_lookback_days(&conn, &cfg, now(), None).unwrap(), 7);
    let ago = |days: i64, hours: i64| Some(Some(db::iso(now() - Duration::days(days) - Duration::hours(hours))));
    assert_eq!(sync::catch_up_lookback_days(&conn, &cfg, now(), ago(20, 5)).unwrap(), 21);
    assert_eq!(sync::catch_up_lookback_days(&conn, &cfg, now(), ago(2, 0)).unwrap(), 7);
    assert_eq!(sync::catch_up_lookback_days(&conn, &cfg, now(), ago(400, 0)).unwrap(), 90);
    db::set_config_value(&conn, "garmin_last_sync_ok_at", &json!(db::iso(now() - Duration::days(12)))).unwrap();
    assert_eq!(sync::catch_up_lookback_days(&conn, &cfg, now(), None).unwrap(), 13);

    assert_eq!(sync::strava_status_lookback_days(&conn, &cfg, now()).unwrap(), 7);
    drop(conn);
    add(&db, 1, "running", now() - Duration::days(30) - Duration::hours(1), "held", "{}");
    add(&db, 2, "running", now() - Duration::days(50), "excluded", "{}");
    assert_eq!(sync::strava_status_lookback_days(&db.lock(), &cfg, now()).unwrap(), 31);
}

#[test]
fn holds_follow_the_category_rules() {
    let db = Db::open_in_memory().unwrap();
    add(&db, 1, "running", now(), "held", "{}");
    add(&db, 2, "yoga", now(), "held", "{}");
    add(&db, 3, "running", now(), "pending", "{}");
    db::set_publish_status(&db.lock(), 3, "held", Some(sync::HOLD_BACKLOG)).unwrap();
    let conn = db.lock();
    assert_eq!(sync::reconcile_held_activities(&conn, &["yoga".to_string()]).unwrap(), 1);
    let status = |id| db::get_activity(&conn, id).unwrap().map(|r| (r.publish_status, r.hold_reason));
    assert_eq!(status(1), Some(("pending".into(), None)));
    assert_eq!(status(2), Some(("held".into(), Some("category".into()))));
    assert_eq!(status(3), Some(("held".into(), Some("backlog".into()))), "only the user lifts a backlog hold");

    sync::exclude(&conn, 3).unwrap();
    assert_eq!(status(3), Some(("excluded".into(), None)));
    sync::restore_status(&conn, 3, "held", Some(sync::HOLD_BACKLOG)).unwrap();
    assert_eq!(status(3), Some(("held".into(), Some("backlog".into()))));
    sync::exclude(&conn, 2).unwrap();
    drop(conn);
    settings(&db, json!({"held_activity_types": ["yoga"]}));
    let cfg = config(&db);
    let conn = db.lock();
    sync::unexclude(&conn, 2, &cfg).unwrap();
    sync::exclude(&conn, 1).unwrap();
    sync::unexclude(&conn, 1, &cfg).unwrap();
    let status = |id| db::get_activity(&conn, id).unwrap().map(|r| (r.publish_status, r.hold_reason));
    assert_eq!(status(2), Some(("held".into(), Some("category".into()))));
    assert_eq!(status(1), Some(("pending".into(), None)));
}

#[tokio::test]
async fn edits_reach_garmin_and_a_published_strava_copy() {
    let db = Db::open_in_memory().unwrap();
    add(&db, 601, "running", now(), "pending", r#"{"distance": 5.0}"#);
    add(&db, 602, "running", now(), "pending", "{}");
    db::set_published(&db.lock(), 602, 9000, now()).unwrap();
    let (garmin, garmin_seen) = fake_garmin(|_, _| Ok((204, Vec::new())));
    let (strava, strava_seen) = fake_strava(&db, |key, _| {
        assert_eq!(key, "PUT /api/v3/activities/9000");
        ok(json!({}))
    });
    let services = Services { db: &db, garmin: &garmin, strava: &strava };

    sync::edit_activity_metadata(&services, 601, "  Tempo  ", " fast ").await.unwrap();
    let row = db::get_activity(&db.lock(), 601).unwrap().unwrap();
    assert_eq!((row.title.as_str(), row.description.as_str()), ("Tempo", "fast"));
    assert_eq!(row.content_hash, sync::content_hash("Tempo", "fast", "running", r#"{"distance": 5.0}"#));
    assert!(strava_seen.lock().unwrap().is_empty());
    assert_eq!(garmin_seen.lock().unwrap().len(), 2);

    sync::edit_activity_metadata(&services, 602, "Long", "").await.unwrap();
    assert_eq!(strava_seen.lock().unwrap().len(), 1);
    assert!(sync::edit_activity_metadata(&services, 602, "   ", "").await.is_err());
    assert!(sync::edit_activity_metadata(&services, 999, "x", "").await.is_err());
}
