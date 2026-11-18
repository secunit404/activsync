use std::sync::{Arc, Mutex};

use activsync::db::{self, Db};
use activsync::events::EventBus;
use activsync::garmin_client::{GarminClient, GarminError, Tokens};
use activsync::hevy_db;
use activsync::hevy_match::{
    self, AUTH_MESSAGE, FAILURE_MESSAGE, HevyMatchScheduler, MAPPING_MESSAGE,
};
use activsync::hevy_sync::RealMatchApplier;
use activsync::net::{Http, HttpError};
use activsync::poller::{BoxFuture, Clients};
use activsync::strava_client::StravaClient;
use chrono::{TimeZone, Utc};
use openidconnect::HttpRequest;
use serde_json::{Value, json};

type Calls = Arc<Mutex<Vec<String>>>;

fn service(calls: &Calls, route: impl Fn(&str) -> (u16, Value) + Send + Sync + 'static) -> Http {
    let log = calls.clone();
    Arc::new(move |request: HttpRequest| {
        let key = format!("{} {}", request.method(), request.uri().path());
        log.lock().unwrap().push(key.clone());
        let (status, body) = route(&key);
        let body = if body.is_null() {
            Vec::new()
        } else {
            body.to_string().into_bytes()
        };
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
    expired: bool,
}

impl Clients for Fake {
    fn garmin(&self) -> BoxFuture<'_, Result<GarminClient, GarminError>> {
        let result = if self.expired {
            Err(GarminError::SessionExpired)
        } else {
            let tokens = Tokens {
                di_token: Some("token".into()),
                di_refresh_token: None,
                di_client_id: None,
            };
            let clock = Arc::new(|| {
                Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
                    .unwrap()
                    .timestamp() as f64
            });
            Ok(
                GarminClient::new(self.garmin_http.clone(), clock, tokens, None)
                    .without_waits()
                    .with_display_name("runner"),
            )
        };
        Box::pin(async move { result })
    }

    fn strava(&self) -> StravaClient {
        StravaClient::new(
            self.garmin_http.clone(),
            self.db.clone(),
            Arc::new(|| 0.0),
            "1",
            "s",
        )
    }
}

struct Case {
    db: Db,
    garmin_calls: Calls,
    hevy_calls: Calls,
    scheduler: HevyMatchScheduler,
}

/// A workout awaiting the owner's choice for watch activity 101.
fn case(exercise: Value, template_status: u16, expired: bool) -> Case {
    let db = Db::open_in_memory().unwrap();
    {
        let conn = db.lock();
        conn.execute(
            "INSERT INTO activities (garmin_activity_id, activity_type, title, start_time, content_hash, publish_status, first_seen_at, garmin_data)
             VALUES (101, 'strength_training', 'Strength', '2026-11-10 08:02:00', 'h', 'held', '2026-11-10T09:00:00+00:00', '{\"duration\": 3500.0}')",
            [],
        )
        .unwrap();
        let payload = json!({"id": "w1", "title": "Push Day", "start_time": "2026-11-10T08:00:00Z", "end_time": "2026-11-10T09:00:00Z",
            "updated_at": "2026-11-10T09:05:00Z", "exercises": [exercise]});
        hevy_db::upsert_workout(
            &conn,
            "w1",
            "Push Day",
            "2026-11-10T08:00:00Z",
            "2026-11-10T09:00:00Z",
            "2026-11-10T09:05:00Z",
            &payload,
        )
        .unwrap();
        hevy_db::claim_source(&conn, "w1", 101).unwrap();
        hevy_db::set_workout_status(
            &conn,
            "w1",
            "awaiting_match",
            Some("Choose how to apply this Hevy workout."),
        )
        .unwrap();
        db::set_config_value(&conn, "hevy_api_key", &json!("key")).unwrap();
        db::set_config_value(&conn, "garmin_credentials_verified", &json!(true)).unwrap();
    }
    let garmin_calls = Calls::default();
    let garmin_http = service(&garmin_calls, |key| match key {
        k if k.starts_with("GET /activitylist-service/") => (200, json!([])),
        k if k.starts_with("PUT ") => (204, Value::Null),
        k if k.starts_with("GET /activity-service/") => (200, json!({"exerciseSets": []})),
        _ => (200, json!({})),
    });
    let hevy_calls = Calls::default();
    let hevy_http = service(&hevy_calls, move |_| (template_status, json!({})));
    let events = EventBus::new();
    let clients = Arc::new(Fake {
        db: db.clone(),
        garmin_http,
        expired,
    });
    let applier = RealMatchApplier {
        db: db.clone(),
        events: events.clone(),
        clients,
        hevy_http,
        development: false,
    };
    let scheduler = HevyMatchScheduler::new(db.clone(), events, Arc::new(|| 1_000.0), 0.0)
        .with_applier(Arc::new(applier));
    Case {
        db,
        garmin_calls,
        hevy_calls,
        scheduler,
    }
}

impl Case {
    async fn choose(&self, strategy: &str) -> (String, Option<String>) {
        self.scheduler.schedule("w1", strategy).unwrap();
        assert_eq!(self.scheduler.run_due().await.unwrap(), 1);
        let decision = hevy_match::get(&self.db.lock(), "w1").unwrap().unwrap();
        (decision.state, decision.message)
    }

    fn workout(&self) -> hevy_db::Workout {
        hevy_db::get_workout(&self.db.lock(), "w1")
            .unwrap()
            .unwrap()
    }

    fn garmin(&self) -> Vec<String> {
        self.garmin_calls.lock().unwrap().clone()
    }
}

fn bench() -> Value {
    json!({"title": "Bench Press (Barbell)", "exercise_template_id": "T-BENCH", "sets": [{"type": "normal", "weight_kg": 80, "reps": 5}]})
}

fn mystery() -> Value {
    json!({"title": "Mystery Machine", "exercise_template_id": "T-MYST", "sets": [{"type": "normal", "reps": 8}]})
}

#[tokio::test]
async fn a_chosen_merge_reaches_garmin_and_reads_it_back() {
    let case = case(bench(), 404, false);
    assert_eq!(
        case.choose("merge").await,
        ("done".into(), Some("Merged Push Day.".into()))
    );
    let workout = case.workout();
    assert_eq!(
        (workout.status.as_str(), workout.garmin_activity_id),
        ("merged", Some(101))
    );
    let calls = case.garmin();
    let put = calls
        .iter()
        .position(|c| c == "PUT /activity-service/activity/101/exerciseSets")
        .expect("sets written");
    let read_back = calls
        .iter()
        .rposition(|c| c.starts_with("GET /activitylist-service/"))
        .expect("Garmin read back");
    assert!(read_back > put, "{calls:?}");
}

#[tokio::test]
async fn an_unmapped_exercise_asks_for_a_mapping() {
    let case = case(mystery(), 404, false);
    assert_eq!(
        case.choose("merge").await,
        ("failed".into(), Some(MAPPING_MESSAGE.into()))
    );
    assert_eq!(case.workout().status, "needs_mapping");
    assert!(case.garmin().iter().all(|c| !c.starts_with("PUT ")));
}

#[tokio::test]
async fn a_rejected_hevy_key_pauses_hevy() {
    let case = case(mystery(), 401, false);
    assert_eq!(
        case.choose("merge").await,
        ("failed".into(), Some(AUTH_MESSAGE.into()))
    );
    assert_eq!(
        db::get_config_value(&case.db.lock(), "hevy_auth_ok").unwrap(),
        Some(json!(false))
    );
    assert_eq!(case.workout().status, "awaiting_match");
    assert!(!case.hevy_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_expired_garmin_session_fails_the_choice_and_pauses_garmin() {
    let case = case(bench(), 404, true);
    assert_eq!(
        case.choose("describe").await,
        ("failed".into(), Some(FAILURE_MESSAGE.into()))
    );
    assert_eq!(case.workout().status, "awaiting_match");
    assert_eq!(
        db::get_config_value(&case.db.lock(), "garmin_credentials_verified").unwrap(),
        Some(json!(false))
    );
    assert!(case.garmin().is_empty());
}
