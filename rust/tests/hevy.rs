use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use activsync::auth_guard::AuthConfig;
use activsync::db::{self, Db};
use activsync::{AppState, build_app, config, hevy_db, hevy_seed, schedule, seed};
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const PYTHON_QUEUE: &str = r#"{"enabled": false, "inFlight": [{"hevyId": "hw-dev-midnight", "title": "Midnight session (Hevy)", "status": "waiting_watch", "error": null, "startDisplay": "2026-11-10 00:30", "needsMapping": false, "hasOpenOperation": false, "resyncable": false, "awaitingMatch": false, "matchedGarminActivityId": null, "matchedGarminTitle": null, "matchedStravaActivityId": null, "matchedStravaUrl": null, "matchScheduledAt": null, "matchPhase": null}, {"hevyId": "hw-dev-syncing", "title": "Leg day (Hevy)", "status": "awaiting_match", "error": "Choose how to apply this Hevy workout.", "startDisplay": "2026-11-10 11:30", "needsMapping": false, "hasOpenOperation": false, "resyncable": false, "awaitingMatch": true, "matchedGarminActivityId": 910002, "matchedGarminTitle": "Strength (watch)", "matchedStravaActivityId": null, "matchedStravaUrl": null, "matchScheduledAt": null, "matchPhase": null}], "problems": [{"hevyId": "hw-dev-unmapped", "title": "Ring circuit (Hevy)", "status": "needs_mapping", "error": "unmapped exercises: Bulgarian Ring Row", "startDisplay": "2026-11-10 07:00", "needsMapping": true, "hasOpenOperation": false, "resyncable": false, "awaitingMatch": false, "matchedGarminActivityId": null, "matchedGarminTitle": null, "matchedStravaActivityId": null, "matchedStravaUrl": null, "matchScheduledAt": null, "matchPhase": null}], "skipped": [], "counts": {"inFlight": 2, "problems": 1, "skipped": 0}}"#;

const PYTHON_DETAIL: &str = r#"{"hevyId": "hw-dev-syncing", "title": "Leg day (Hevy)", "startTime": "2026-11-10T10:30:00+00:00", "endTime": "2026-11-10T11:30:00+00:00", "notes": null, "exercises": [{"title": "Bench Press (Barbell)", "notes": null, "templateId": "tpl-dev-bench", "sets": [{"number": 1, "setType": "normal", "reps": 5.0, "weightKg": 80.0, "distanceMeters": null, "durationSeconds": null, "rpe": null, "customMetric": null}, {"number": 2, "setType": "normal", "reps": 5.0, "weightKg": 80.0, "distanceMeters": null, "durationSeconds": null, "rpe": null, "customMetric": null}, {"number": 3, "setType": "normal", "reps": 5.0, "weightKg": 80.0, "distanceMeters": null, "durationSeconds": null, "rpe": null, "customMetric": null}]}], "descriptionPreview": "⏱️ 60 min\n\n• Bench Press (Barbell): 3 sets - 80.0kg × 5\n\n— synced by activsync"}"#;

struct TestClock(Arc<AtomicU64>);

impl TestClock {
    fn clock(&self) -> schedule::Clock {
        let value = self.0.clone();
        Arc::new(move || f64::from_bits(value.load(Ordering::SeqCst)))
    }

    fn advance(&self, seconds: f64) {
        let now = f64::from_bits(self.0.load(Ordering::SeqCst));
        self.0.store((now + seconds).to_bits(), Ordering::SeqCst);
    }
}

struct Harness {
    state: AppState,
    app: Router,
    clock: TestClock,
}

fn harness_with(db: Db) -> Harness {
    let clock = TestClock(Arc::new(AtomicU64::new(1_800_000_000f64.to_bits())));
    let state = AppState::new(db, true, clock.clock(), schedule::UNDO_WINDOW_SECONDS, AuthConfig::sign_in_off()).with_configured_undo_window();
    let app = build_app(state.clone(), None);
    Harness { state, app, clock }
}

fn harness() -> Harness {
    let db = Db::open_in_memory().unwrap();
    seed::seed(&mut db.lock(), Utc::now(), true).unwrap();
    harness_with(db)
}

async fn call(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:8382")
        .header("x-activsync-request", "1")
        .header("sec-fetch-site", "same-origin");
    let body = match body {
        Some(value) => {
            request = request.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    (status, json)
}

async fn ok(app: &Router, method: Method, uri: &str, body: Option<Value>) -> Value {
    let (status, body) = call(app, method, uri, body).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

async fn refused(app: &Router, uri: &str, body: Option<Value>) -> (StatusCode, String) {
    let (status, body) = call(app, Method::POST, uri, body).await;
    (status, body["detail"].as_str().unwrap_or_default().to_string())
}

fn status_of(h: &Harness, hevy_id: &str) -> String {
    hevy_db::get_workout(&h.state.db.lock(), hevy_id).unwrap().unwrap().status
}

#[tokio::test]
async fn queue_and_detail_match_python() {
    let db = Db::open_in_memory().unwrap();
    hevy_seed::seed(&db.lock(), Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap()).unwrap();
    let h = harness_with(db);
    let python: Value = serde_json::from_str(PYTHON_QUEUE).unwrap();
    assert_eq!(ok(&h.app, Method::GET, "/api/v1/hevy/queue", None).await, python);
    let python: Value = serde_json::from_str(PYTHON_DETAIL).unwrap();
    assert_eq!(ok(&h.app, Method::GET, "/api/v1/hevy/hw-dev-syncing", None).await, python);

    let (status, body) = call(&h.app, Method::GET, "/api/v1/hevy/nope", None).await;
    assert_eq!((status, body["detail"].as_str()), (StatusCode::NOT_FOUND, Some("Unknown Hevy workout.")));
}

fn set_undo_delay(h: &Harness, seconds: i64) {
    config::edit(&h.state.db.lock(), false, |cfg| cfg.set("undo_delay_seconds", json!(seconds))).unwrap();
}

#[tokio::test]
async fn the_configured_undo_delay_sets_the_window_and_deadline() {
    for delay in [0, 3, 5, 10] {
        let h = harness();
        set_undo_delay(&h, delay);

        let body = ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-syncing/match", Some(json!({"strategy": "merge"}))).await;

        assert_eq!(body["undoWindowMs"], delay * 1000);
        assert_eq!(body["remainingMs"], delay * 1000);
        assert_eq!(body["deadline"], schedule::deadline_iso(1_800_000_000.0 + delay as f64));
    }
}

#[tokio::test]
async fn a_zero_undo_delay_applies_on_the_next_pass() {
    let h = harness();
    set_undo_delay(&h, 0);
    ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-syncing/match", Some(json!({"strategy": "merge"}))).await;

    assert_eq!(h.state.match_scheduler.run_due().await.unwrap(), 1);
}

#[tokio::test]
async fn match_waits_out_undo_window_then_applies() {
    let h = harness();
    let scheduled = ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-syncing/match", Some(json!({"strategy": "merge"}))).await;
    assert_eq!(scheduled["strategy"], "merge");
    assert_eq!((scheduled["remainingMs"].clone(), scheduled["undoWindowMs"].clone()), (json!(5000), json!(5000)));

    let queue = ok(&h.app, Method::GET, "/api/v1/hevy/queue", None).await;
    let item = queue["inFlight"].as_array().unwrap().iter().find(|i| i["hevyId"] == "hw-dev-syncing").unwrap().clone();
    assert_eq!((item["matchPhase"].clone(), item["matchScheduledAt"].clone()), (json!("waiting"), scheduled["deadline"].clone()));

    let (status, detail) = refused(&h.app, "/api/v1/hevy/hw-dev-syncing/match", Some(json!({"strategy": "describe"}))).await;
    assert_eq!((status, detail.as_str()), (StatusCode::CONFLICT, "This workout is already being matched."));

    assert_eq!(h.state.match_scheduler.run_due().await.unwrap(), 0, "not due yet");
    assert_eq!(ok(&h.app, Method::GET, "/api/v1/hevy/hw-dev-syncing/match/status", None).await["state"], "waiting");
    h.clock.advance(5.0);
    assert_eq!(h.state.match_scheduler.run_due().await.unwrap(), 1);
    assert_eq!(
        ok(&h.app, Method::GET, "/api/v1/hevy/hw-dev-syncing/match/status", None).await,
        json!({"state": "done", "message": "Merged Leg day (Hevy)."})
    );
    let workout = hevy_db::get_workout(&h.state.db.lock(), hevy_seed::SYNCING_ID).unwrap().unwrap();
    assert_eq!(workout.status, "merged");
    assert_eq!(workout.garmin_activity_id, Some(hevy_seed::SYNCING_ACTIVITY_ID));
    assert_eq!(workout.applied_strategy.as_deref(), Some("merge"));
    assert_eq!(db::blocking_workout_for_activity(&h.state.db.lock(), hevy_seed::SYNCING_ACTIVITY_ID).unwrap(), None);

    let (status, detail) = refused(&h.app, "/api/v1/hevy/hw-dev-syncing/match/cancel", None).await;
    assert_eq!((status, detail.as_str()), (StatusCode::NOT_FOUND, "No Hevy match is waiting to be undone."));
    let (status, detail) = refused(&h.app, "/api/v1/hevy/hw-dev-syncing/match", Some(json!({"strategy": "merge"}))).await;
    assert_eq!((status, detail.as_str()), (StatusCode::CONFLICT, "This workout is not waiting for a match decision."));
}

#[tokio::test]
async fn undo_inside_window_changes_nothing() {
    let h = harness();
    ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-syncing/match", Some(json!({"strategy": "describe"}))).await;
    assert_eq!(ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-syncing/match/cancel", None).await, json!({"cancelled": true}));
    h.clock.advance(10.0);
    assert_eq!(h.state.match_scheduler.run_due().await.unwrap(), 0);
    assert_eq!(status_of(&h, hevy_seed::SYNCING_ID), "awaiting_match");
    let (status, body) = call(&h.app, Method::GET, "/api/v1/hevy/hw-dev-syncing/match/status", None).await;
    assert_eq!((status, body["detail"].as_str()), (StatusCode::NOT_FOUND, Some("No Hevy match decision for that workout.")));
}

#[tokio::test]
async fn skipped_during_window_finishes_as_skipped() {
    let h = harness();
    ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-syncing/match", Some(json!({"strategy": "merge"}))).await;
    h.clock.advance(5.0);
    hevy_db::set_workout_status(&h.state.db.lock(), hevy_seed::SYNCING_ID, "skipped", None).unwrap();
    assert_eq!(h.state.match_scheduler.run_due().await.unwrap(), 1);
    assert_eq!(
        ok(&h.app, Method::GET, "/api/v1/hevy/hw-dev-syncing/match/status", None).await,
        json!({"state": "skipped", "message": null})
    );
}

#[tokio::test]
async fn replace_refused_once_source_is_on_strava() {
    let h = harness();
    db::set_published(&h.state.db.lock(), hevy_seed::SYNCING_ACTIVITY_ID, 42, Utc::now()).unwrap();
    let (status, detail) = refused(&h.app, "/api/v1/hevy/hw-dev-syncing/match", Some(json!({"strategy": "replace"}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(detail, "Replace is unavailable because this activity is already on Strava. Choose Merge or Description only.");
    let queue = ok(&h.app, Method::GET, "/api/v1/hevy/queue", None).await;
    let item = &queue["inFlight"][1];
    assert_eq!((item["matchedStravaActivityId"].clone(), item["matchedStravaUrl"].clone()),
               (json!(42), json!("https://www.strava.com/activities/42")));

    let (status, _) = call(&h.app, Method::POST, "/api/v1/hevy/hw-dev-syncing/match", Some(json!({"strategy": "wipe"}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn skip_and_restore() {
    let h = harness();
    let body = ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-syncing/skip", None).await;
    assert_eq!(body, json!({"message": "Skipped Leg day (Hevy)."}));
    let workout = hevy_db::get_workout(&h.state.db.lock(), hevy_seed::SYNCING_ID).unwrap().unwrap();
    assert_eq!((workout.status.as_str(), workout.source_garmin_activity_id), ("skipped", None));

    let queue = ok(&h.app, Method::GET, "/api/v1/hevy/queue", None).await;
    assert_eq!(queue["counts"], json!({"inFlight": 1, "problems": 1, "skipped": 1}));
    assert_eq!(queue["skipped"][0]["hevyId"], "hw-dev-syncing");

    let (status, detail) = refused(&h.app, "/api/v1/hevy/hw-dev-syncing/skip", None).await;
    assert_eq!((status, detail.as_str()), (StatusCode::CONFLICT, "Cannot skip a skipped workout."));
    let (status, detail) = refused(&h.app, "/api/v1/hevy/hw-dev-merged/skip", None).await;
    assert_eq!((status, detail.as_str()), (StatusCode::CONFLICT, "Cannot skip a merged workout."));

    let body = ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-syncing/unskip", None).await;
    assert_eq!(body, json!({"message": "Restored Leg day (Hevy) to the sync queue."}));
    assert_eq!(status_of(&h, hevy_seed::SYNCING_ID), "waiting_watch");
    let (status, detail) = refused(&h.app, "/api/v1/hevy/hw-dev-syncing/unskip", None).await;
    assert_eq!((status, detail.as_str()), (StatusCode::CONFLICT, "Workout is waiting_watch, not skipped."));
}

#[tokio::test]
async fn skip_cancels_a_waiting_match() {
    let h = harness();
    ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-syncing/match", Some(json!({"strategy": "merge"}))).await;
    ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-syncing/skip", None).await;
    h.clock.advance(5.0);
    assert_eq!(h.state.match_scheduler.run_due().await.unwrap(), 0);
    assert_eq!(status_of(&h, hevy_seed::SYNCING_ID), "skipped");
}

#[tokio::test]
async fn retry_requeues_or_resumes_the_operation() {
    let h = harness();
    let body = ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-unmapped/retry", None).await;
    assert_eq!(body, json!({"message": "Retry queued for Ring circuit (Hevy)."}));
    let workout = hevy_db::get_workout(&h.state.db.lock(), hevy_seed::UNMAPPED_ID).unwrap().unwrap();
    assert_eq!((workout.status.as_str(), workout.error), ("waiting_watch", None));
    let (status, detail) = refused(&h.app, "/api/v1/hevy/hw-dev-unmapped/retry", None).await;
    assert_eq!((status, detail.as_str()), (StatusCode::CONFLICT, "Nothing to retry — workout is waiting_watch."));

    {
        let conn = h.state.db.lock();
        hevy_db::set_workout_status(&conn, hevy_seed::MIDNIGHT_ID, "needs_review", Some("stuck")).unwrap();
        conn.execute(
            "INSERT INTO hevy_operations (hevy_id, kind, phase, upload_id, attempt_count, created_at, updated_at)
             VALUES ('hw-dev-midnight', 'replace', 'needs_review', 'up-1', 4, 'now', 'now')",
            [],
        )
        .unwrap();
    }
    let queue = ok(&h.app, Method::GET, "/api/v1/hevy/queue", None).await;
    assert_eq!(queue["problems"].as_array().unwrap().iter().find(|i| i["hevyId"] == "hw-dev-midnight").unwrap()["hasOpenOperation"], true);
    let (status, detail) = refused(&h.app, "/api/v1/hevy/hw-dev-midnight/skip", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(detail, "An operation is in progress for this workout — resolve or retry it instead of skipping.");

    ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-midnight/retry", None).await;
    assert_eq!(status_of(&h, hevy_seed::MIDNIGHT_ID), "syncing");
    let (phase, attempts): (String, i64) = h.state.db.lock()
        .query_row("SELECT phase, attempt_count FROM hevy_operations WHERE hevy_id = 'hw-dev-midnight'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((phase.as_str(), attempts), ("submission_unknown", 0));
}

#[tokio::test]
async fn resync_fresh_only_after_a_garmin_delete() {
    let h = harness();
    let (status, detail) = refused(&h.app, "/api/v1/hevy/hw-dev-merged/resync-fresh", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(detail, "Re-sync as fresh is only available when the linked activity was deleted on Garmin.");

    hevy_db::set_workout_status(&h.state.db.lock(), hevy_seed::MERGED_ID, "needs_review", Some("Activity 910001 was deleted on Garmin.")).unwrap();
    let queue = ok(&h.app, Method::GET, "/api/v1/hevy/queue", None).await;
    assert_eq!(queue["problems"].as_array().unwrap().iter().find(|i| i["hevyId"] == "hw-dev-merged").unwrap()["resyncable"], true);
    let body = ok(&h.app, Method::POST, "/api/v1/hevy/hw-dev-merged/resync-fresh", None).await;
    assert_eq!(body, json!({"message": "Fresh sync queued for Push day (Hevy)."}));
    let workout = hevy_db::get_workout(&h.state.db.lock(), hevy_seed::MERGED_ID).unwrap().unwrap();
    assert_eq!((workout.status.as_str(), workout.garmin_activity_id, workout.applied_strategy), ("waiting_watch", None, None));
}

#[tokio::test]
async fn busy_workout_refuses_actions() {
    let h = harness();
    let token = hevy_db::acquire_lease(&h.state.db.lock(), hevy_seed::UNMAPPED_ID, Utc::now()).unwrap();
    assert!(token.is_some());
    let (status, detail) = refused(&h.app, "/api/v1/hevy/hw-dev-unmapped/retry", None).await;
    assert_eq!((status, detail.as_str()), (StatusCode::CONFLICT, "Workout is being processed right now — try again in a moment."));
    let (status, detail) = refused(&h.app, "/api/v1/hevy/nope/retry", None).await;
    assert_eq!((status, detail.as_str()), (StatusCode::NOT_FOUND, "Unknown Hevy workout."));
}
