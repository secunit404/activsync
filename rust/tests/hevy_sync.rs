use std::sync::{Arc, Mutex};

use activsync::config;
use activsync::db::{self, Db};
use activsync::fit_builder::Profile;
use activsync::garmin_client::{GarminClient, Tokens};
use activsync::hevy_apply::Apply;
use activsync::hevy_client::HevyClient;
use activsync::hevy_db::{self, OpField, Template};
use activsync::hevy_sync::{self, CURSOR_KEY};
use activsync::net::{Http, HttpError};
use activsync::poller::HevyLegError;
use activsync::strava_client::StravaClient;
use chrono::{DateTime, Utc};
use openidconnect::HttpRequest;
use rusqlite::types::ValueRef;
use serde_json::{Value, json};

type Calls = Arc<Mutex<Vec<Value>>>;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/hevy_sync_python.json")).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(fixture()["now"].as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc)
}

fn reply(status: u16, body: Value) -> (u16, Vec<u8>) {
    (
        status,
        if body.is_null() {
            Vec::new()
        } else {
            body.to_string().into_bytes()
        },
    )
}

/// A stand-in service; `route` sees the request and logs calls in the
/// Python fakes' terms.
fn service(
    calls: &Calls,
    route: impl Fn(&HttpRequest, &mut dyn FnMut(Value)) -> (u16, Vec<u8>) + Send + Sync + 'static,
) -> Http {
    let log = calls.clone();
    Arc::new(move |request: HttpRequest| {
        let (status, body) = route(&request, &mut |call| log.lock().unwrap().push(call));
        let response = openidconnect::http::Response::builder()
            .status(status)
            .body(body)
            .map_err(|e| HttpError(e.to_string()));
        Box::pin(async move { response })
    })
}

fn query(request: &HttpRequest, key: &str) -> String {
    let url = openidconnect::url::Url::parse(&request.uri().to_string()).unwrap();
    url.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

fn last_id(request: &HttpRequest) -> i64 {
    request
        .uri()
        .path()
        .split('/')
        .filter_map(|p| p.parse::<i64>().ok())
        .next_back()
        .unwrap_or_default()
}

fn fake_garmin(script: Value, calls: &Calls) -> GarminClient {
    let near_index = Mutex::new(0usize);
    let http = service(calls, move |request, call| {
        let path = request.uri().path();
        let id = last_id(request);
        let body = || serde_json::from_slice::<Value>(request.body()).unwrap_or(Value::Null);
        match (request.method().as_str(), path) {
            ("GET", p) if p.ends_with("/exerciseSets") => {
                call(json!(["get_exercise_sets", id]));
                reply(200, json!({"exerciseSets": []}))
            }
            ("PUT", p) if p.ends_with("/exerciseSets") => {
                call(json!(["put_exercise_sets", id, body()]));
                match script["put"].as_str() {
                    Some("gone") => reply(404, json!({})),
                    Some("BOOM") => reply(500, json!({})),
                    _ => reply(204, Value::Null),
                }
            }
            ("PUT", _) => {
                let body = body();
                match body.get("activityName") {
                    Some(title) => call(json!(["set_title", id, title])),
                    None => call(json!(["set_description", id, body["description"]])),
                }
                reply(204, Value::Null)
            }
            ("GET", p) if p.starts_with("/activitylist-service/") => {
                if query(request, "start") != "0" {
                    return reply(200, json!([]));
                }
                call(json!(["near"]));
                let mut index = near_index.lock().unwrap();
                let pages = script["near"]
                    .as_array()
                    .cloned()
                    .unwrap_or_else(|| vec![json!([])]);
                let page = pages[(*index).min(pages.len() - 1)].clone();
                *index += 1;
                reply(200, page)
            }
            ("POST", _) => {
                let head = String::from_utf8_lossy(request.body()).to_string();
                let filename = head
                    .split("filename=\"")
                    .nth(1)
                    .unwrap()
                    .split('"')
                    .next()
                    .unwrap()
                    .to_string();
                call(json!(["upload_fit", filename]));
                let ok = &script["upload"];
                reply(
                    200,
                    json!({"detailedImportResult": {"uploadId": ok["upload_id"], "successes": [{"internalId": ok["activity_id"]}], "failures": []}}),
                )
            }
            ("GET", p) if p.starts_with("/wellness-service/") => {
                call(json!(["daily", query(request, "date")]));
                reply(200, json!({}))
            }
            other => panic!("unexpected Garmin call {other:?}"),
        }
    });
    let tokens = Tokens {
        di_token: Some("t".into()),
        di_refresh_token: None,
        di_client_id: None,
    };
    GarminClient::new(http, Arc::new(|| now().timestamp() as f64), tokens, None)
        .without_waits()
        .with_display_name("runner")
}

fn fake_hevy(script: Value, calls: &Calls) -> HevyClient {
    let http = service(calls, move |request, call| {
        let path = request.uri().path();
        if path.ends_with("/workouts/events") {
            call(json!(["events", query(request, "since")]));
            return reply(
                200,
                json!({"events": script["events"].as_array().cloned().unwrap_or_default(), "page_count": 1}),
            );
        }
        let id = path.rsplit('/').next().unwrap().to_string();
        call(json!(["template", id]));
        match script["templates"].get(&id) {
            Some(Value::String(s)) if s == "AUTH" => reply(401, json!({})),
            Some(Value::String(s)) if s == "BOOM" => reply(500, json!({})),
            Some(template) if template.is_object() => reply(200, template.clone()),
            _ => reply(404, json!({})),
        }
    });
    HevyClient::with_base_url(http, "key", "https://hevy.test")
        .unwrap()
        .without_backoff()
}

fn fake_strava(db: &Db, script: Value, calls: &Calls) -> StravaClient {
    let tokens = json!({"access_token": "at", "refresh_token": "rt", "expires_at": now().timestamp() + 21600});
    db::set_config_value(&db.lock(), "strava_tokens", &tokens).unwrap();
    let http = service(calls, move |request, call| {
        let id = last_id(request);
        let body: Value = serde_json::from_slice(request.body()).unwrap();
        call(json!([
            "strava_update",
            id,
            body["name"],
            body["description"]
        ]));
        if script["fail"] == id {
            reply(500, json!({}))
        } else {
            reply(200, json!({"id": id}))
        }
    });
    StravaClient::new(
        http,
        db.clone(),
        Arc::new(|| now().timestamp() as f64),
        "123",
        "shh",
    )
    .without_waits()
}

/// Retries repeat a call the Python fake logs once.
fn collapse(calls: &Calls) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for call in calls.lock().unwrap().iter() {
        if out.last() != Some(call) {
            out.push(call.clone());
        }
    }
    out
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
    let mut templates = rows(
        conn,
        "SELECT exercise_template_id, title, primary_muscle_group, secondary_muscle_groups, equipment_category, is_custom FROM exercise_templates ORDER BY exercise_template_id",
    );
    for template in &mut templates {
        template["secondary_muscle_groups"] =
            serde_json::from_str(template["secondary_muscle_groups"].as_str().unwrap()).unwrap();
    }
    json!({
        "workouts": rows(conn, "SELECT hevy_id, title, status, error, source_garmin_activity_id, garmin_activity_id, applied_strategy, garmin_applied_updated_at, strava_applied_updated_at, source_updated_at, locked_until, lease_owner FROM hevy_workouts ORDER BY hevy_id"),
        "operations": rows(conn, "SELECT hevy_id, kind, phase, attempt_count FROM hevy_operations ORDER BY id"),
        "seen": rows(conn, "SELECT * FROM hevy_events_seen ORDER BY event_type, workout_id, event_timestamp"),
        "templates": templates,
        "cursor": db::get_config_value(conn, CURSOR_KEY).unwrap(),
    })
}

/// The Python fakes fail with "BOOM" where the real clients word their own
/// errors, so only the text before it has to agree.
fn same(rust: &Value, python: &Value) -> bool {
    match (rust, python) {
        (Value::String(r), Value::String(p)) if p.contains("BOOM") => {
            r.starts_with(p.split("BOOM").next().unwrap())
        }
        (Value::Object(r), Value::Object(p)) => {
            r.len() == p.len()
                && p.iter()
                    .all(|(k, v)| r.get(k).is_some_and(|rv| same(rv, v)))
        }
        (Value::Array(r), Value::Array(p)) => {
            r.len() == p.len() && r.iter().zip(p).all(|(a, b)| same(a, b))
        }
        _ => rust == python,
    }
}

fn setup(scenario: &Value) -> Db {
    let db = Db::open_in_memory().unwrap();
    let conn = db.lock();
    let text = |v: &Value| v.as_str().map(str::to_string);
    for a in scenario["activities"].as_array().unwrap() {
        conn.execute(
            "INSERT INTO activities (garmin_activity_id, activity_type, title, description, start_time, content_hash, publish_status, strava_activity_id, first_seen_at, garmin_data)
             VALUES (?1, ?2, 'x', ?3, ?4, 'h', ?5, ?6, '2026-11-01T00:00:00+00:00', ?7)",
            rusqlite::params![
                a["garmin_activity_id"].as_i64(),
                text(&a["activity_type"]),
                text(&a["description"]),
                text(&a["start_time"]),
                text(&a["publish_status"]),
                a["strava_activity_id"].as_i64(),
                text(&a["garmin_data"])
            ],
        )
        .unwrap();
    }
    for w in scenario["workouts"].as_array().unwrap() {
        let p = &w["payload"];
        let field = |k: &str| p[k].as_str().unwrap();
        hevy_db::upsert_workout(
            &conn,
            field("id"),
            field("title"),
            field("start_time"),
            field("end_time"),
            field("updated_at"),
            p,
        )
        .unwrap();
        conn.execute(
            "UPDATE hevy_workouts SET status = ?1, error = ?2, source_garmin_activity_id = ?3, garmin_activity_id = ?4, applied_strategy = ?5,
               garmin_applied_updated_at = ?6, strava_applied_updated_at = ?7 WHERE hevy_id = ?8",
            rusqlite::params![
                text(&w["status"]),
                text(&w["error"]),
                w["source"].as_i64(),
                w["target"].as_i64(),
                text(&w["strategy"]),
                text(&w["garmin_applied"]),
                text(&w["strava_applied"]),
                field("id")
            ],
        )
        .unwrap();
    }
    for t in scenario["templates"].as_array().into_iter().flatten() {
        let template = Template {
            id: t[0].as_str().unwrap(),
            title: t[1].as_str().unwrap(),
            primary_muscle_group: None,
            secondary_muscle_groups: &[],
            equipment_category: None,
            is_custom: false,
        };
        hevy_db::upsert_template(&conn, &template).unwrap();
    }
    for s in scenario["seen"].as_array().into_iter().flatten() {
        hevy_db::record_event_seen(
            &conn,
            s[0].as_str().unwrap(),
            s[1].as_str().unwrap(),
            s[2].as_str().unwrap(),
        )
        .unwrap();
    }
    if let Some(settings) = scenario.get("settings") {
        db::set_config_value(&conn, "settings", settings).unwrap();
    }
    if let Some(cursor) = scenario.get("cursor") {
        db::set_config_value(&conn, CURSOR_KEY, cursor).unwrap();
    }
    if let Some(op) = scenario.get("operation") {
        let pre: Vec<i64> = op["pre"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_i64)
            .collect();
        let id =
            hevy_db::open_operation(&conn, "w1", op["kind"].as_str().unwrap(), Some(101), &pre)
                .unwrap()
                .unwrap();
        hevy_db::update_operation(
            &conn,
            id,
            &[
                OpField::Phase("submission_unknown"),
                OpField::NextStep("resolve"),
            ],
        )
        .unwrap();
    }
    drop(conn);
    db
}

#[tokio::test]
async fn the_hevy_leg_runs_like_python() {
    let reference = fixture();
    let p = &reference["profile"];
    let profile = Profile {
        weight_kg: p["weight_kg"].as_f64().unwrap(),
        birth_year: p["birth_year"].as_i64().unwrap(),
        vo2max: p["vo2max"].as_f64().unwrap(),
        sex: p["sex"].as_str().unwrap().into(),
    };
    for scenario in reference["scenarios"].as_array().unwrap() {
        let name = scenario["name"].as_str().unwrap();
        let db = setup(scenario);
        let (garmin_calls, hevy_calls, strava_calls) =
            (Calls::default(), Calls::default(), Calls::default());
        let garmin = fake_garmin(
            scenario.get("garmin").cloned().unwrap_or(json!({})),
            &garmin_calls,
        );
        let hevy = fake_hevy(
            scenario.get("hevy").cloned().unwrap_or(json!({})),
            &hevy_calls,
        );
        let strava = fake_strava(
            &db,
            scenario.get("strava").cloned().unwrap_or(json!({})),
            &strava_calls,
        );
        let cfg = config::load_config(&db.lock(), false).unwrap();
        let apply = Apply {
            db: &db,
            garmin: &garmin,
            cfg: &cfg,
            profile: &profile,
        };
        let (changed, raised) =
            match hevy_sync::run_hevy_leg(&apply, &hevy, Some(&strava), now()).await {
                Ok(changed) => (json!(changed), Value::Null),
                Err(HevyLegError::Auth) => (Value::Null, json!("HevyAuthError")),
                Err(other) => panic!("{name}: {other:?}"),
            };
        assert_eq!(raised, scenario["raised"], "{name}");
        assert_eq!(changed, scenario["changed"], "{name}");
        for (seen, key) in [
            (&garmin_calls, "garmin_calls"),
            (&hevy_calls, "hevy_calls"),
            (&strava_calls, "strava_calls"),
        ] {
            let seen = Value::from(collapse(seen));
            assert!(
                same(&seen, &scenario[key]),
                "{name} {key}:\nrust   {seen}\npython {}",
                scenario[key]
            );
        }
        let actual = state(&db.lock());
        assert!(
            same(&actual, &scenario["state"]),
            "{name}:\nrust   {actual}\npython {}",
            scenario["state"]
        );
    }
}

#[tokio::test]
async fn a_match_choice_applies_like_python() {
    let reference = fixture();
    let p = &reference["profile"];
    let profile = Profile {
        weight_kg: p["weight_kg"].as_f64().unwrap(),
        birth_year: p["birth_year"].as_i64().unwrap(),
        vo2max: p["vo2max"].as_f64().unwrap(),
        sex: p["sex"].as_str().unwrap().into(),
    };
    for choice in reference["choices"].as_array().unwrap() {
        let name = choice["name"].as_str().unwrap();
        let mut scenario = json!({"activities": choice["activities"], "workouts": []});
        if !choice["status"].is_null() {
            let exercises = choice.get("exercises").cloned().unwrap_or_else(|| {
                json!([{"title": "Bench Press (Barbell)", "exercise_template_id": "T-BENCH", "sets": [{"type": "normal", "weight_kg": 80, "reps": 5}]}])
            });
            let payload = json!({"id": "w1", "title": "Workout w1", "start_time": "2026-11-10T08:00:00Z", "end_time": "2026-11-10T09:00:00Z",
                "updated_at": "2026-11-10T09:05:00Z", "exercises": exercises});
            scenario["workouts"] = json!([{"payload": payload, "status": choice["status"], "source": choice["source"]}]);
        }
        let db = setup(&scenario);
        let calls = Calls::default();
        let garmin = fake_garmin(choice.get("garmin").cloned().unwrap_or(json!({})), &calls);
        let hevy = fake_hevy(json!({}), &Calls::default());
        let cfg = config::load_config(&db.lock(), false).unwrap();
        let apply = Apply {
            db: &db,
            garmin: &garmin,
            cfg: &cfg,
            profile: &profile,
        };
        let result = match hevy_sync::apply_match_choice(
            &apply,
            &hevy,
            "w1",
            choice["strategy"].as_str().unwrap(),
            now(),
        )
        .await
        {
            Ok(row) => json!({"status": row.unwrap().status}),
            Err(err) => json!({"message": err.to_string()}),
        };
        let mut expected = choice["result"].clone();
        if let Some(map) = expected.as_object_mut() {
            map.remove("raised");
        }
        assert!(
            same(&result, &expected),
            "{name}: rust {result} python {expected}"
        );
        let after = rows(
            &db.lock(),
            "SELECT status, error, garmin_activity_id, lease_owner FROM hevy_workouts",
        );
        assert!(
            same(&Value::from(after.clone()), &choice["after"]),
            "{name}: rust {after:?} python {}",
            choice["after"]
        );
        let names: Vec<Value> = collapse(&calls).iter().map(|c| c[0].clone()).collect();
        assert_eq!(Value::from(names), choice["garmin_calls"], "{name}");
    }
}
