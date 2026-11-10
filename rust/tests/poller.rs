use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use activsync::db::{self, Db};
use activsync::events::EventBus;
use activsync::garmin_client::{GarminClient, GarminError, Tokens};
use activsync::net::{Http, HttpError};
use activsync::poller::{BoxFuture, Clients, HevyLeg, HevyLegError, Options, Poller};
use activsync::strava_client::StravaClient;
use chrono::{DateTime, Duration, TimeZone, Utc};
use openidconnect::HttpRequest;
use serde_json::{Value, json};

type Reply = Result<(u16, Vec<u8>, Vec<(&'static str, &'static str)>), &'static str>;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap()
}

fn service(route: impl Fn(&str) -> Reply + Send + Sync + 'static, calls: Arc<Mutex<Vec<String>>>) -> Http {
    Arc::new(move |request: HttpRequest| {
        let uri = request.uri();
        let key = format!("{} {}", request.method(), uri.path());
        calls.lock().unwrap().push(key.clone());
        let reply = route(&key).map_err(|e| HttpError(e.into())).map(|(status, body, headers)| {
            let mut builder = openidconnect::http::Response::builder().status(status);
            for (name, value) in headers {
                builder = builder.header(name, value);
            }
            builder.body(body).unwrap()
        });
        Box::pin(async move { reply })
    })
}

struct Fake {
    db: Db,
    garmin_http: Http,
    strava_http: Http,
    failure: Mutex<Option<GarminError>>,
    opens: AtomicUsize,
}

impl Clients for Fake {
    fn garmin(&self) -> BoxFuture<'_, Result<GarminClient, GarminError>> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        let result = if let Some(err) = self.failure.lock().unwrap().clone() {
            Err(err)
        } else {
            let tokens = Tokens { di_token: Some("token".into()), di_refresh_token: None, di_client_id: None };
            Ok(GarminClient::new(self.garmin_http.clone(), Arc::new(|| t0().timestamp() as f64), tokens, None).without_waits())
        };
        Box::pin(async move { result })
    }

    fn strava(&self) -> StravaClient {
        StravaClient::new(self.strava_http.clone(), self.db.clone(), Arc::new(|| t0().timestamp() as f64), "1", "s").without_waits()
    }
}

struct Harness {
    db: Db,
    fake: Arc<Fake>,
    garmin_calls: Arc<Mutex<Vec<String>>>,
    strava_calls: Arc<Mutex<Vec<String>>>,
    strava_reply: Arc<Mutex<Reply>>,
    events: EventBus,
}

impl Harness {
    /// Garmin returns one running activity that started `started_ago`
    /// before t0; running is a held category, so nothing is uploaded.
    fn new(started_ago: Duration) -> Self {
        let db = Db::open_in_memory().unwrap();
        let start = (t0() - started_ago).format("%Y-%m-%d %H:%M:%S").to_string();
        let batch = json!([{"activityId": 1, "activityType": {"typeKey": "running"}, "activityName": "Run",
                            "startTimeGMT": start, "duration": 1800.0}]);
        let garmin_calls = Arc::new(Mutex::new(Vec::new()));
        let garmin_http = service(
            move |key| match key {
                "GET /activitylist-service/activities/search/activities" => Ok((200, batch.to_string().into_bytes(), vec![])),
                _ => Ok((404, b"{}".to_vec(), vec![])),
            },
            garmin_calls.clone(),
        );
        let strava_calls = Arc::new(Mutex::new(Vec::new()));
        let strava_reply: Arc<Mutex<Reply>> = Arc::new(Mutex::new(Ok((200, b"[]".to_vec(), vec![]))));
        let reply = strava_reply.clone();
        let strava_http = service(move |_| reply.lock().unwrap().clone(), strava_calls.clone());
        let fake = Arc::new(Fake { db: db.clone(), garmin_http, strava_http, failure: Mutex::new(None), opens: AtomicUsize::new(0) });
        {
            let conn = db.lock();
            db::set_config_value(&conn, "settings", &json!({"held_activity_types": ["running"]})).unwrap();
            db::set_config_value(&conn, "initial_sync_done", &json!(true)).unwrap();
            db::set_config_value(&conn, "garmin_credentials_verified", &json!(true)).unwrap();
            let tokens = json!({"access_token": "a", "refresh_token": "r", "expires_at": t0().timestamp() + 21600});
            db::set_config_value(&conn, "strava_tokens", &tokens).unwrap();
        }
        Self { db, fake, garmin_calls, strava_calls, strava_reply, events: EventBus::new() }
    }

    fn poller(&self, hevy: Option<Arc<dyn HevyLeg>>) -> Poller {
        Poller::new(self.db.clone(), self.events.clone(), self.fake.clone(), hevy, Options::default())
    }

    fn garmin_fetches(&self) -> usize {
        self.garmin_calls.lock().unwrap().iter().filter(|k| k.contains("activitylist")).count()
    }

    fn strava_reads(&self) -> usize {
        self.strava_calls.lock().unwrap().iter().filter(|k| k.contains("athlete/activities")).count()
    }

    fn config(&self, key: &str) -> Option<Value> {
        db::get_config_value(&self.db.lock(), key).unwrap()
    }
}

#[tokio::test]
async fn each_leg_runs_on_its_own_interval_once_setup_is_done() {
    let h = Harness::new(Duration::hours(10));
    db::set_config_value(&h.db.lock(), "initial_sync_done", &json!(false)).unwrap();
    let mut poller = h.poller(None);
    poller.loop_once(t0()).await.unwrap();
    assert_eq!(h.fake.opens.load(Ordering::SeqCst), 0, "the setup wizard runs its own first sync");

    db::set_config_value(&h.db.lock(), "initial_sync_done", &json!(true)).unwrap();
    let mut refreshes = h.events.subscribe();
    poller.loop_once(t0()).await.unwrap();
    assert_eq!((h.garmin_fetches(), h.strava_reads()), (1, 1), "a Garmin change runs Strava in the same tick");
    assert_eq!(refreshes.try_recv().unwrap(), "refresh");
    assert_eq!(db::get_activity(&h.db.lock(), 1).unwrap().unwrap().publish_status, "held");

    poller.loop_once(t0() + Duration::seconds(60)).await.unwrap();
    assert_eq!((h.garmin_fetches(), h.strava_reads()), (1, 1));
    poller.loop_once(t0() + Duration::minutes(5)).await.unwrap();
    assert_eq!((h.garmin_fetches(), h.strava_reads()), (1, 2), "Strava every 5 minutes");
    poller.loop_once(t0() + Duration::minutes(19)).await.unwrap();
    assert_eq!(h.garmin_fetches(), 1);
    poller.loop_once(t0() + Duration::minutes(20)).await.unwrap();
    assert_eq!((h.garmin_fetches(), h.strava_reads()), (2, 3), "Garmin every 20 minutes");
    assert!(refreshes.try_recv().is_err(), "nothing changed, no refresh");
}

#[tokio::test]
async fn a_reconnected_side_is_polled_on_the_next_tick() {
    let h = Harness::new(Duration::hours(10));
    db::set_config_value(&h.db.lock(), "garmin_credentials_verified", &json!(false)).unwrap();
    let mut poller = h.poller(None);
    poller.loop_once(t0()).await.unwrap();
    assert_eq!((h.garmin_fetches(), h.strava_reads()), (0, 0), "Strava waits for Garmin too");
    db::set_config_value(&h.db.lock(), "garmin_credentials_verified", &json!(true)).unwrap();
    poller.loop_once(t0() + Duration::seconds(60)).await.unwrap();
    assert_eq!(h.garmin_fetches(), 1);
}

#[tokio::test]
async fn a_strava_rate_limit_pauses_strava_until_the_quota_resets() {
    let h = Harness::new(Duration::hours(10));
    *h.strava_reply.lock().unwrap() = Ok((429, b"{}".to_vec(), vec![("retry-after", "600")]));
    let mut poller = h.poller(None);
    poller.loop_once(t0()).await.unwrap();
    assert_eq!(h.strava_reads(), 1);
    *h.strava_reply.lock().unwrap() = Ok((200, b"[]".to_vec(), vec![]));
    poller.loop_once(t0() + Duration::minutes(5)).await.unwrap();
    poller.loop_once(t0() + Duration::minutes(9)).await.unwrap();
    assert_eq!(h.strava_reads(), 1, "no calls while the quota is spent");
    poller.loop_once(t0() + Duration::minutes(10)).await.unwrap();
    assert_eq!(h.strava_reads(), 2);
}

#[tokio::test]
async fn an_expired_garmin_session_pauses_garmin_and_tells_the_page() {
    let h = Harness::new(Duration::hours(10));
    *h.fake.failure.lock().unwrap() = Some(GarminError::SessionExpired);
    let mut refreshes = h.events.subscribe();
    let mut poller = h.poller(None);
    poller.loop_once(t0()).await.unwrap();
    assert_eq!(h.config("garmin_credentials_verified"), Some(json!(false)));
    assert_eq!(h.config("garmin_last_sync_ok"), Some(json!(false)));
    assert_eq!(h.config("garmin_last_sync_error"), Some(json!("Garmin session expired; reconnect Garmin to resume sync.")));
    assert_eq!(refreshes.try_recv().unwrap(), "refresh");

    let opens = h.fake.opens.load(Ordering::SeqCst);
    poller.loop_once(t0() + Duration::minutes(30)).await.unwrap();
    assert_eq!(h.fake.opens.load(Ordering::SeqCst), opens, "paused until the user reconnects");
}

#[tokio::test]
async fn garmin_is_polled_faster_right_after_a_session() {
    let h = Harness::new(Duration::minutes(90));
    let mut poller = h.poller(None);
    poller.loop_once(t0()).await.unwrap();
    poller.loop_once(t0() + Duration::minutes(4)).await.unwrap();
    assert_eq!(h.garmin_fetches(), 1);
    poller.loop_once(t0() + Duration::minutes(5)).await.unwrap();
    assert_eq!(h.garmin_fetches(), 2, "the run ended 60 minutes ago");
}

struct FakeLeg {
    reply: Mutex<Result<bool, HevyLegError>>,
    runs: AtomicUsize,
    saw_strava: AtomicBool,
}

impl HevyLeg for FakeLeg {
    fn run<'a>(
        &'a self,
        _db: &'a Db,
        _garmin: &'a GarminClient,
        strava: Option<&'a StravaClient>,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<bool, HevyLegError>> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.saw_strava.store(strava.is_some(), Ordering::SeqCst);
        let reply = self.reply.lock().unwrap().clone();
        Box::pin(async move { reply })
    }
}

#[tokio::test]
async fn a_hevy_change_makes_garmin_due_and_a_rejected_key_pauses_hevy() {
    let h = Harness::new(Duration::hours(10));
    db::set_config_value(&h.db.lock(), "settings", &json!({"hevy_enabled": true, "held_activity_types": ["running"]})).unwrap();
    db::set_config_value(&h.db.lock(), "hevy_api_key", &json!("key")).unwrap();
    let leg = Arc::new(FakeLeg { reply: Mutex::new(Ok(false)), runs: AtomicUsize::new(0), saw_strava: AtomicBool::new(false) });
    let mut poller = h.poller(Some(leg.clone()));

    poller.loop_once(t0()).await.unwrap();
    assert_eq!((leg.runs.load(Ordering::SeqCst), h.garmin_fetches()), (1, 1));
    assert!(leg.saw_strava.load(Ordering::SeqCst));
    assert_eq!(h.config("hevy_last_success_at"), Some(json!("2026-11-10T12:00:00+00:00")));

    *leg.reply.lock().unwrap() = Ok(true);
    poller.loop_once(t0() + Duration::minutes(5)).await.unwrap();
    assert_eq!((leg.runs.load(Ordering::SeqCst), h.garmin_fetches()), (2, 2), "Garmin fetched now, not in 20 minutes");

    *leg.reply.lock().unwrap() = Err(HevyLegError::Auth);
    poller.loop_once(t0() + Duration::minutes(10)).await.unwrap();
    assert_eq!(h.config("hevy_auth_ok"), Some(json!(false)));
    assert_eq!(h.config("hevy_last_success_at"), Some(json!("2026-11-10T12:05:00+00:00")));
    poller.loop_once(t0() + Duration::minutes(15)).await.unwrap();
    assert_eq!(leg.runs.load(Ordering::SeqCst), 3, "paused until the key is updated");
}

#[tokio::test]
async fn a_garmin_outage_is_shown_and_retried_without_a_reconnect() {
    let h = Harness::new(Duration::hours(10));
    *h.fake.failure.lock().unwrap() = Some(GarminError::Unavailable);
    let mut poller = h.poller(None);
    poller.loop_once(t0()).await.unwrap();
    assert_eq!(h.config("garmin_credentials_verified"), Some(json!(true)));
    assert_eq!(h.config("garmin_last_sync_ok"), Some(json!(false)));
    assert_eq!(h.config("garmin_last_sync_error"), Some(json!("Garmin could not be reached; retrying on the next poll.")));

    *h.fake.failure.lock().unwrap() = None;
    poller.loop_once(t0() + Duration::minutes(20)).await.unwrap();
    assert_eq!(h.garmin_fetches(), 1, "polling resumed on its own");
    assert_eq!(h.config("garmin_last_sync_ok"), Some(json!(true)));
}
