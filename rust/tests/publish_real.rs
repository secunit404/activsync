use std::io::Write;
use std::sync::{Arc, Mutex};

use activsync::db::{self, Db};
use activsync::events::EventBus;
use activsync::garmin_client::{GarminClient, GarminError, Tokens};
use activsync::hevy_db;
use activsync::net::{Http, HttpError};
use activsync::poller::{BoxFuture, Clients};
use activsync::schedule::{self, PublishScheduler};
use activsync::strava_client::StravaClient;
use chrono::{TimeZone, Utc};
use openidconnect::HttpRequest;
use serde_json::json;

type Calls = Arc<Mutex<Vec<String>>>;

fn now_s() -> f64 {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
        .unwrap()
        .timestamp() as f64
}

fn zipped(fit: &[u8]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file("activity.fit", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(fit).unwrap();
    writer.finish().unwrap().into_inner()
}

fn service(calls: &Calls, route: impl Fn(&str) -> (u16, Vec<u8>) + Send + Sync + 'static) -> Http {
    let log = calls.clone();
    Arc::new(move |request: HttpRequest| {
        let key = format!("{} {}", request.method(), request.uri().path());
        log.lock().unwrap().push(key.clone());
        let (status, body) = route(&key);
        let response = openidconnect::http::Response::builder()
            .status(status)
            .body(body)
            .map_err(|e| HttpError(e.to_string()));
        Box::pin(async move { response })
    })
}

struct Fake {
    db: Db,
    garmin_http: Http,
    strava_http: Http,
    garmin_down: bool,
}

impl Clients for Fake {
    fn garmin(&self) -> BoxFuture<'_, Result<GarminClient, GarminError>> {
        let result = if self.garmin_down {
            Err(GarminError::Unavailable)
        } else {
            let tokens = Tokens {
                di_token: Some("token".into()),
                di_refresh_token: None,
                di_client_id: None,
            };
            Ok(
                GarminClient::new(self.garmin_http.clone(), Arc::new(now_s), tokens, None)
                    .without_waits(),
            )
        };
        Box::pin(async move { result })
    }

    fn strava(&self) -> StravaClient {
        StravaClient::new(
            self.strava_http.clone(),
            self.db.clone(),
            Arc::new(now_s),
            "1",
            "s",
        )
        .without_waits()
    }
}

struct Case {
    db: Db,
    scheduler: PublishScheduler,
    strava_calls: Calls,
}

fn add(db: &Db, id: i64, status: &str) {
    db.lock()
        .execute(
            "INSERT INTO activities (garmin_activity_id, activity_type, title, start_time, content_hash, publish_status, first_seen_at)
             VALUES (?1, 'running', ?2, ?3, 'h', ?4, '2026-11-10T09:00:00+00:00')",
            rusqlite::params![id, format!("Activity {id}"), format!("2026-11-10 0{}:00:00", id % 10), status],
        )
        .unwrap();
}

fn case(strava_connected: bool, garmin_down: bool) -> Case {
    let db = Db::open_in_memory().unwrap();
    if strava_connected {
        let tokens = json!({"access_token": "a", "refresh_token": "r", "expires_at": now_s() as i64 + 21600});
        db::set_config_value(&db.lock(), "strava_tokens", &tokens).unwrap();
    }
    let garmin_http = service(&Calls::default(), |key| match key {
        k if k.starts_with("GET /download-service/") => (200, zipped(b"FITDATA")),
        _ => (404, b"{}".to_vec()),
    });
    let strava_calls = Calls::default();
    let strava_http = service(&strava_calls, |key| match key {
        k if k.starts_with("GET /api/v3/athlete/activities") => (200, b"[]".to_vec()),
        "POST /api/v3/uploads" => (200, json!({"id": 9}).to_string().into_bytes()),
        "GET /api/v3/uploads/9" => (200, json!({"activity_id": 555}).to_string().into_bytes()),
        other => panic!("unexpected {other}"),
    });
    let clients = Arc::new(Fake {
        db: db.clone(),
        garmin_http,
        strava_http,
        garmin_down,
    });
    let scheduler = PublishScheduler::new(db.clone(), EventBus::new(), Arc::new(now_s), 0.0)
        .with_clients(clients);
    Case {
        db,
        scheduler,
        strava_calls,
    }
}

impl Case {
    async fn publish(&self, ids: &[i64]) -> Vec<(String, Option<String>)> {
        let rows: Vec<_> = ids
            .iter()
            .map(|id| db::get_activity(&self.db.lock(), *id).unwrap().unwrap())
            .collect();
        self.scheduler.schedule(&rows, "manual").unwrap();
        assert_eq!(self.scheduler.run_due().await.unwrap(), ids.len());
        ids.iter()
            .map(|id| {
                let row = schedule::get(&self.db.lock(), *id).unwrap().unwrap();
                (row.state, row.message)
            })
            .collect()
    }

    fn uploads(&self) -> usize {
        self.strava_calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == "POST /api/v3/uploads")
            .count()
    }
}

#[tokio::test]
async fn a_scheduled_publish_uploads_to_strava() {
    let case = case(true, false);
    add(&case.db, 201, "held");
    assert_eq!(case.publish(&[201]).await, vec![("published".into(), None)]);
    let activity = db::get_activity(&case.db.lock(), 201).unwrap().unwrap();
    assert_eq!(
        (
            activity.publish_status.as_str(),
            activity.strava_activity_id
        ),
        ("published", Some(555))
    );
    assert_eq!(case.uploads(), 1);
}

#[tokio::test]
async fn hevy_work_blocks_and_a_later_exclusion_skips() {
    let case = case(true, false);
    add(&case.db, 201, "pending");
    add(&case.db, 202, "pending");
    {
        let conn = case.db.lock();
        hevy_db::upsert_workout(
            &conn,
            "h1",
            "Legs",
            "2026-11-10T02:00:00Z",
            "2026-11-10T03:00:00Z",
            "2026-11-10T03:05:00Z",
            &json!({}),
        )
        .unwrap();
        hevy_db::claim_source(&conn, "h1", 202).unwrap();
    }
    let rows: Vec<_> = [201, 202]
        .iter()
        .map(|id| db::get_activity(&case.db.lock(), *id).unwrap().unwrap())
        .collect();
    case.scheduler.schedule(&rows, "manual").unwrap();
    case.db
        .lock()
        .execute(
            "UPDATE activities SET publish_status = 'excluded' WHERE garmin_activity_id = 201",
            [],
        )
        .unwrap();
    assert_eq!(case.scheduler.run_due().await.unwrap(), 2);
    let state = |id| {
        let row = schedule::get(&case.db.lock(), id).unwrap().unwrap();
        (row.state, row.message)
    };
    assert_eq!(state(201), ("skipped".into(), None));
    assert_eq!(
        state(202),
        (
            "blocked".into(),
            Some("Activity was blocked by Hevy sync: hevy workout h1 is waiting_watch.".into())
        )
    );
    assert_eq!(case.uploads(), 0);
}

#[tokio::test]
async fn a_disconnected_strava_fails_with_its_own_words() {
    let case = case(false, false);
    add(&case.db, 201, "pending");
    let outcome = case.publish(&[201]).await;
    assert_eq!(outcome[0].0, "failed");
    let message = outcome[0].1.clone().unwrap();
    assert!(
        !message.starts_with("Could not publish activity"),
        "{message}"
    );
    assert!(message.contains("Strava"), "{message}");
}

#[tokio::test]
async fn garmin_down_fails_the_whole_batch() {
    let case = case(true, true);
    add(&case.db, 201, "pending");
    add(&case.db, 202, "held");
    let message = format!("Could not publish activity: {}", GarminError::Unavailable);
    assert_eq!(
        case.publish(&[201, 202]).await,
        vec![
            ("failed".into(), Some(message.clone())),
            ("failed".into(), Some(message))
        ]
    );
    assert_eq!(case.uploads(), 0);
}
