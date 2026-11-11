use std::io::Write;
use std::sync::{Arc, Mutex};

use activsync::config;
use activsync::db::{self, Db};
use activsync::fit_builder::{Profile, ResolvedExercise};
use activsync::garmin_client::{GarminClient, Tokens};
use activsync::hevy_apply::{Apply, ApplyError, build_exercise_sets_payload};
use activsync::hevy_db;
use activsync::hevy_description::{generate_description, generate_title};
use activsync::net::{Http, HttpError};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{TimeZone, Utc};
use openidconnect::HttpRequest;
use rusqlite::types::ValueRef;
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/hevy_apply_python.json")).unwrap()
}

fn watch_fits() -> std::collections::HashMap<String, Vec<u8>> {
    let cases: Value = serde_json::from_str(include_str!("fixtures/fit_builder_python.json")).unwrap();
    cases
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| Some((c["name"].as_str()?.to_string(), B64.decode(c["result"]["fit"].as_str()?).ok()?)))
        .collect()
}

/// Python's side of a scripted list: each call takes the next entry and the
/// last one repeats.
fn take(script: &Value, key: &str, counter: &mut usize) -> Value {
    let items = script[key].as_array().cloned().unwrap_or_default();
    let item = items.get((*counter).min(items.len().saturating_sub(1))).cloned().unwrap_or(Value::Null);
    *counter += 1;
    item
}

#[derive(Default)]
struct Counters {
    near: usize,
    upload: usize,
    delete: usize,
}

fn zip_of(fit: &[u8]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut out);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file("201_ACTIVITY.fit", options).unwrap();
    writer.write_all(fit).unwrap();
    writer.finish().unwrap();
    out.into_inner()
}

fn uploaded_file(request: &HttpRequest) -> (String, Vec<u8>) {
    let content_type = request.headers()["content-type"].to_str().unwrap();
    let boundary = content_type.split("boundary=").nth(1).unwrap();
    let body = request.body();
    let head_end = body.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&body[..head_end]).to_string();
    let filename = head.split("filename=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
    let tail = format!("\r\n--{boundary}--\r\n").len();
    (filename, body[head_end + 4..body.len() - tail].to_vec())
}

/// Garmin stand-in following the scenario's script and logging each call in
/// the Python fake's terms.
fn fake_garmin(script: Value, fits: &std::collections::HashMap<String, Vec<u8>>) -> (GarminClient, Arc<Mutex<Vec<Value>>>) {
    let calls: Arc<Mutex<Vec<Value>>> = Arc::default();
    let log = calls.clone();
    let counters = Arc::new(Mutex::new(Counters::default()));
    let fit = script["fit"].as_str().map(|name| zip_of(&fits[name]));
    let http: Http = Arc::new(move |request: HttpRequest| {
        let path = request.uri().path().to_string();
        let query = request.uri().query().unwrap_or_default().to_string();
        let method = request.method().as_str().to_string();
        let id = || path.split('/').filter_map(|p| p.parse::<i64>().ok()).next_back().unwrap_or_default();
        let mut counters = counters.lock().unwrap();
        let call = |entry: Value| log.lock().unwrap().push(entry);
        let reply: (u16, Vec<u8>) = match (method.as_str(), path.as_str()) {
            ("GET", p) if p.ends_with("/exerciseSets") => {
                call(json!(["get_exercise_sets", id()]));
                (200, script.get("sets").cloned().unwrap_or(json!({"exerciseSets": []})).to_string().into_bytes())
            }
            ("PUT", p) if p.ends_with("/exerciseSets") => {
                let body: Value = serde_json::from_slice(request.body()).unwrap();
                call(json!(["put_exercise_sets", id(), body]));
                match script["put"].as_str() {
                    Some("subcategory") => (400, br#"{"message": "Invalid sub-category"}"#.to_vec()),
                    Some("gone") => (404, b"{}".to_vec()),
                    _ => (204, Vec::new()),
                }
            }
            ("PUT", _) => {
                let body: Value = serde_json::from_slice(request.body()).unwrap();
                match body.get("activityName") {
                    Some(title) => call(json!(["set_title", id(), title])),
                    None => call(json!(["set_description", id(), body["description"]])),
                }
                if script["title"] == "gone" { (404, b"{}".to_vec()) } else { (204, Vec::new()) }
            }
            ("GET", p) if p.starts_with("/download-service/") => {
                call(json!(["download_fit", id()]));
                (200, fit.clone().unwrap())
            }
            ("GET", p) if p.starts_with("/activitylist-service/") => {
                if query.contains("start=0&") {
                    call(json!(["near"]));
                    (200, take(&script, "near", &mut counters.near).to_string().into_bytes())
                } else {
                    (200, b"[]".to_vec())
                }
            }
            ("POST", _) => {
                let (filename, data) = uploaded_file(&request);
                call(json!(["upload_fit", filename, B64.encode(data)]));
                let step = take(&script, "upload", &mut counters.upload);
                if step.get("rejected").is_some() {
                    (409, b"{}".to_vec())
                } else if step.get("error").is_some() {
                    (500, b"{}".to_vec())
                } else {
                    let ok = &step["ok"];
                    let successes = if ok["activity_id"].is_null() { json!([]) } else { json!([{"internalId": ok["activity_id"]}]) };
                    let body = json!({"detailedImportResult": {"uploadId": ok["upload_id"], "successes": successes, "failures": []}});
                    (200, body.to_string().into_bytes())
                }
            }
            ("DELETE", _) => {
                call(json!(["delete_activity", id()]));
                match take(&script, "delete", &mut counters.delete).as_str() {
                    Some("404") => (404, b"{}".to_vec()),
                    Some("BOOM") => (500, b"{}".to_vec()),
                    _ => (204, Vec::new()),
                }
            }
            ("GET", p) if p.starts_with("/wellness-service/") => {
                let date = query.trim_start_matches("date=").to_string();
                call(json!(["daily", date]));
                (200, script["daily"].get(&date).cloned().unwrap_or(json!({})).to_string().into_bytes())
            }
            other => panic!("unexpected Garmin call {other:?}"),
        };
        let response = openidconnect::http::Response::builder().status(reply.0).body(reply.1).map_err(|e| HttpError(e.to_string()));
        Box::pin(async move { response })
    });
    let tokens = Tokens { di_token: Some("t".into()), di_refresh_token: None, di_client_id: None };
    let clock = Arc::new(|| Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap().timestamp() as f64);
    (GarminClient::new(http, clock, tokens, None).without_waits().with_display_name("runner"), calls)
}

fn rows(conn: &rusqlite::Connection, sql: &str) -> Vec<Value> {
    let mut stmt = conn.prepare(sql).unwrap();
    let names: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
    stmt.query_map([], |row| {
        let mut map = serde_json::Map::new();
        for (i, name) in names.iter().enumerate() {
            let value = match row.get_ref(i)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(v) => json!(v),
                ValueRef::Real(v) => json!(v),
                ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                ValueRef::Blob(b) => json!(b.len()),
            };
            map.insert(name.clone(), value);
        }
        Ok(Value::Object(map))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn state(conn: &rusqlite::Connection) -> Value {
    let workout = rows(conn, "SELECT status, error, garmin_activity_id, applied_strategy, garmin_applied_updated_at, source_garmin_activity_id FROM hevy_workouts WHERE hevy_id = 'w1'");
    let mut operations = rows(conn, "SELECT kind, phase, next_step, source_activity_id, target_activity_id, upload_id, pre_upload_ids, generated_calories, generated_avg_hr, attempt_count, delete_attempt_count, last_error FROM hevy_operations WHERE hevy_id = 'w1' ORDER BY id");
    for op in &mut operations {
        op["pre_upload_ids"] = serde_json::from_str(op["pre_upload_ids"].as_str().unwrap()).unwrap();
    }
    let mut backups = rows(conn, "SELECT garmin_activity_id, hevy_id, original_sets, original_fit FROM merge_backups ORDER BY garmin_activity_id");
    for backup in &mut backups {
        backup["original_sets"] = serde_json::from_str(backup["original_sets"].as_str().unwrap()).unwrap();
    }
    let settings = db::get_config_value(conn, "settings").unwrap().unwrap_or(json!({}));
    let mut identity = settings.get("hevy_device_identity").cloned().unwrap_or(Value::Null);
    if identity["product"] == 0 && identity["serial"] != 55 {
        identity["serial"] = json!("random");
    }
    json!({"workout": workout[0], "operations": operations, "backups": backups, "identity": identity})
}

/// The Python fake fails with "BOOM" where the real client words its own
/// error, so only the text before it has to agree.
fn same(rust: &Value, python: &Value) -> bool {
    match (rust, python) {
        (Value::String(r), Value::String(p)) if p.contains("BOOM") => r.starts_with(p.split("BOOM").next().unwrap()),
        (Value::Object(r), Value::Object(p)) => r.len() == p.len() && p.iter().all(|(k, v)| r.get(k).is_some_and(|rv| same(rv, v))),
        (Value::Array(r), Value::Array(p)) => r.len() == p.len() && r.iter().zip(p).all(|(a, b)| same(a, b)),
        _ => rust == python,
    }
}

fn setup(scenario: &Value) -> Db {
    let db = Db::open_in_memory().unwrap();
    let conn = db.lock();
    for a in scenario["activities"].as_array().unwrap() {
        conn.execute(
            "INSERT INTO activities (garmin_activity_id, activity_type, title, start_time, content_hash, publish_status, strava_activity_id, first_seen_at, garmin_data)
             VALUES (?1, ?2, 'x', ?3, 'h', 'pending', ?4, '2026-11-01T00:00:00+00:00', ?5)",
            rusqlite::params![
                a["garmin_activity_id"].as_i64(),
                a["activity_type"].as_str(),
                a["start_time"].as_str(),
                a["strava_activity_id"].as_i64(),
                a["garmin_data"].as_str()
            ],
        )
        .unwrap();
    }
    for mapping in scenario["mappings"].as_array().into_iter().flatten() {
        hevy_db::save_mapping(&conn, mapping[0].as_str().unwrap(), mapping[1].as_i64().unwrap(), mapping[2].as_i64().unwrap()).unwrap();
    }
    if let Some(settings) = scenario.get("settings") {
        db::set_config_value(&conn, "settings", settings).unwrap();
    }
    let w = &scenario["workout"];
    let field = |k: &str| w[k].as_str().unwrap();
    hevy_db::upsert_workout(&conn, "w1", field("title"), field("start_time"), field("end_time"), field("updated_at"), w).unwrap();
    let source = scenario["source"].as_i64();
    if let Some(source) = source {
        hevy_db::claim_source(&conn, "w1", source).unwrap();
    }
    if scenario["blocker"] == true {
        hevy_db::upsert_workout(&conn, "w0", "other", field("start_time"), field("end_time"), field("updated_at"), w).unwrap();
        hevy_db::open_operation(&conn, "w0", "replace", source, &[]).unwrap();
    }
    drop(conn);
    db
}

fn profile(value: &Value) -> Profile {
    Profile {
        weight_kg: value["weight_kg"].as_f64().unwrap(),
        birth_year: value["birth_year"].as_i64().unwrap(),
        vo2max: value["vo2max"].as_f64().unwrap(),
        sex: value["sex"].as_str().unwrap().into(),
    }
}

#[tokio::test]
async fn every_strategy_runs_like_python() {
    let reference = fixture();
    let fits = watch_fits();
    let profile = profile(&reference["profile"]);
    for scenario in reference["scenarios"].as_array().unwrap() {
        let name = scenario["name"].as_str().unwrap();
        let db = setup(scenario);
        let (garmin, calls) = fake_garmin(scenario["garmin"].clone(), &fits);
        let mut raised = Value::Null;
        for tick in 0..scenario["ticks"].as_u64().unwrap_or(1) {
            let cfg = config::load_config(&db.lock(), false).unwrap();
            let apply = Apply { db: &db, garmin: &garmin, cfg: &cfg, profile: &profile };
            let row = hevy_db::get_workout(&db.lock(), "w1").unwrap().unwrap();
            let outcome = if tick == 0 {
                match scenario["action"].as_str().unwrap() {
                    "merge" => apply.execute_merge(&row).await,
                    "describe" => apply.execute_describe(&row).await,
                    "replace" => apply.execute_replace(&row).await,
                    _ => apply.execute_passive(&row).await,
                }
            } else if hevy_db::get_open_operation(&db.lock(), "w1").unwrap().is_none() {
                break;
            } else {
                apply.advance_operation(&row).await
            };
            if let Err(err) = outcome {
                raised = json!(match err {
                    ApplyError::MappingMiss { .. } => "MappingMiss".to_string(),
                    other => format!("{other:?}"),
                });
                break;
            }
        }
        assert_eq!(raised, scenario["raised"], "{name}");

        let mut seen = calls.lock().unwrap().clone();
        let mut expected = scenario["calls"].as_array().unwrap().clone();
        let random_identity = scenario["state"]["identity"]["serial"] == "random";
        for call in seen.iter_mut().chain(expected.iter_mut()) {
            if call[0] == "upload_fit" && random_identity {
                call[2] = Value::Null;
            }
        }
        assert_eq!(seen.len(), expected.len(), "{name}: {:?}", seen.iter().map(|c| c[0].clone()).collect::<Vec<_>>());
        for (index, (rust, python)) in seen.iter().zip(&expected).enumerate() {
            assert_eq!(rust, python, "{name}: call {index}");
        }
        let actual = state(&db.lock());
        assert!(same(&actual, &scenario["state"]), "{name}:\nrust   {actual}\npython {}", scenario["state"]);
    }
}

fn resolved(items: &Value) -> Vec<ResolvedExercise> {
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|e| ResolvedExercise {
            title: e[0].as_str().unwrap().into(),
            category: e[1].as_i64().unwrap(),
            subcategory: e[2].as_i64().unwrap(),
            sets: e[3].as_array().unwrap().clone(),
        })
        .collect()
}

#[test]
fn the_exercise_sets_payload_matches_python() {
    for case in fixture()["payloads"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let built = build_exercise_sets_payload(&resolved(&case["resolved"]), case["start"].as_str().unwrap(), case["duration"].as_f64().unwrap());
        match built {
            Ok(payload) => assert_eq!(payload, case["payload"], "{name}"),
            Err(error) => assert_eq!(error, case["error"].as_str().unwrap(), "{name}"),
        }
    }
}

#[test]
fn titles_and_descriptions_match_python() {
    for case in fixture()["texts"].as_array().unwrap() {
        let template = case["template"].as_str();
        if let Some(title) = case.get("title") {
            assert_eq!(generate_title(&case["workout"], template).unwrap(), title.as_str().unwrap(), "{case}");
        } else {
            let text = generate_description(&case["workout"], case["calories"].as_i64(), case["avg_hr"].as_i64(), template).unwrap();
            assert_eq!(text, case["description"].as_str().unwrap(), "{case}");
        }
    }
}

#[test]
fn applying_runs_on_the_poller_threads() {
    fn assert_send<T: Send>(_: &T) {}
    let db = Db::open_in_memory().unwrap();
    let (garmin, _) = fake_garmin(json!({}), &Default::default());
    let cfg = config::load_config(&db.lock(), false).unwrap();
    let profile = profile(&fixture()["profile"]);
    let apply = Apply { db: &db, garmin: &garmin, cfg: &cfg, profile: &profile };
    let row = hevy_db::Workout {
        hevy_id: "w".into(),
        title: String::new(),
        start_time: String::new(),
        end_time: String::new(),
        source_updated_at: None,
        payload: "{}".into(),
        status: "syncing".into(),
        applied_strategy: None,
        source_garmin_activity_id: None,
        garmin_activity_id: None,
        error: None,
        garmin_applied_updated_at: None,
        strava_applied_updated_at: None,
    };
    assert_send(&apply.execute_merge(&row));
    assert_send(&apply.execute_describe(&row));
    assert_send(&apply.execute_replace(&row));
    assert_send(&apply.execute_passive(&row));
}
