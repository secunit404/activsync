use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use activsync::db::{self, Db};
use activsync::hevy_client::{HevyClient, HevyError, retry_delay};
use activsync::net::{Http, HttpError, Timeout};
use activsync::strava_client::{StravaActivity, StravaClient, StravaError, match_closest_activity};
use chrono::{TimeZone, Utc};
use openidconnect::HttpRequest;
use serde_json::{Value, json};

type Seen = Arc<Mutex<Vec<HttpRequest>>>;
type Reply = Result<(u16, Value, Vec<(&'static str, &'static str)>), &'static str>;

/// Answers each call with the next scripted reply and keeps every request.
fn scripted(replies: Vec<Reply>) -> (Http, Seen) {
    let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let http: Http = Arc::new(move |request: HttpRequest| {
        log.lock().unwrap().push(request);
        let next = queue
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted reply for every call");
        let reply = next
            .map_err(|e| HttpError(e.into()))
            .map(|(status, body, headers)| {
                let mut builder = openidconnect::http::Response::builder().status(status);
                for (name, value) in headers {
                    builder = builder.header(name, value);
                }
                builder.body(body.to_string().into_bytes()).unwrap()
            });
        Box::pin(async move { reply })
    });
    (http, seen)
}

fn ok(body: Value) -> Reply {
    Ok((200, body, vec![]))
}

fn status(code: u16) -> Reply {
    Ok((code, json!({}), vec![]))
}

fn urls(seen: &Seen) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .map(|r| format!("{} {}", r.method(), r.uri()))
        .collect()
}

fn hevy(replies: Vec<Reply>) -> (HevyClient, Seen) {
    let (http, seen) = scripted(replies);
    (
        HevyClient::new(http, "key-123").unwrap().without_backoff(),
        seen,
    )
}

#[tokio::test]
async fn hevy_sends_the_key_and_spots_a_rejected_one() {
    let (client, seen) = hevy(vec![ok(json!({"id": "u1"})), status(401)]);
    assert_eq!(client.get_user_info().await.unwrap(), json!({"id": "u1"}));
    assert_eq!(seen.lock().unwrap()[0].headers()["api-key"], "key-123");
    assert_eq!(client.get_user_info().await, Err(HevyError::Auth));
    assert!(HevyClient::new(scripted(vec![]).0, "").is_err());
}

#[tokio::test]
async fn hevy_retries_outages_but_never_reads_them_as_deleted() {
    let (client, seen) = hevy(vec![
        status(503),
        Err("reset"),
        ok(json!({"workout": {"id": "w1"}})),
        status(404),
    ]);
    assert_eq!(
        client.get_workout("w1").await.unwrap(),
        Some(json!({"id": "w1"}))
    );
    assert_eq!(urls(&seen).len(), 3);
    assert_eq!(client.get_workout("gone").await.unwrap(), None);

    let mut down = vec![status(502); 6];
    down.push(ok(json!({})));
    let (client, seen) = hevy(down);
    let Err(HevyError::Status { code, message }) = client.get_workout("w1").await else {
        panic!("an outage past the retries is an error");
    };
    assert_eq!(code, 502);
    assert_eq!(
        message,
        "502 Server Error: Bad Gateway for url: https://api.hevyapp.com/v1/workouts/w1"
    );
    assert_eq!(
        urls(&seen).len(),
        6,
        "five retries, then the outage is reported"
    );
}

#[tokio::test]
async fn hevy_counts_and_pages_defensively() {
    let (client, _) = hevy(vec![
        ok(json!({"workout_count": 42})),
        ok(json!({"workout_count": true})),
        ok(json!({"workout_count": -1})),
    ]);
    assert_eq!(client.get_workout_count().await.unwrap(), Some(42));
    assert_eq!(client.get_workout_count().await.unwrap(), None);
    assert_eq!(client.get_workout_count().await.unwrap(), None);

    let updated =
        json!({"type": "updated", "workout": {"id": "w1", "updated_at": "2026-11-10T10:00:00Z"}});
    let deleted = json!({"type": "deleted", "id": "w2", "deleted_at": "2026-11-10T11:00:00Z"});
    let (client, seen) = hevy(vec![
        ok(
            json!({"page_count": 2, "events": [updated, {"type": "updated", "workout": {"id": "w3"}}, "junk"]}),
        ),
        ok(json!({"page_count": "2", "events": [deleted]})),
    ]);
    let events = client
        .iter_events_since("2026-11-01T00:00:00Z")
        .await
        .unwrap();
    assert_eq!(events, vec![updated, deleted]);
    assert_eq!(
        urls(&seen),
        [
            "GET https://api.hevyapp.com/v1/workouts/events?since=2026-11-01T00%3A00%3A00Z&page=1&pageSize=10",
            "GET https://api.hevyapp.com/v1/workouts/events?since=2026-11-01T00%3A00%3A00Z&page=2&pageSize=10",
        ]
    );
}

const NOW: i64 = 1_800_000_450;

fn strava(replies: Vec<Reply>, tokens: Value) -> (StravaClient, Db, Seen) {
    let db = Db::open_in_memory().unwrap();
    db::set_config_value(&db.lock(), "strava_tokens", &tokens).unwrap();
    let (http, seen) = scripted(replies);
    let client =
        StravaClient::new(http, db.clone(), Arc::new(|| NOW as f64), "123", "shh").without_waits();
    (client, db, seen)
}

fn fresh() -> Value {
    json!({"access_token": "at", "refresh_token": "rt", "expires_at": NOW + 3600, "scope": "read,activity:write,activity:read_all"})
}

fn body(seen: &Seen, index: usize) -> String {
    String::from_utf8_lossy(seen.lock().unwrap()[index].body()).into_owned()
}

#[tokio::test]
async fn strava_refreshes_an_expiring_token_and_keeps_the_scope() {
    let stale = json!({"access_token": "old", "refresh_token": "rt", "expires_at": NOW + 30, "scope": "read"});
    let (client, db, seen) = strava(
        vec![
            ok(json!({"access_token": "new", "refresh_token": "rt2", "expires_at": NOW + 21600})),
            status(200),
        ],
        stale,
    );
    client
        .update_activity_metadata(77, "Run", "Easy")
        .await
        .unwrap();
    assert_eq!(
        body(&seen, 0),
        "client_id=123&client_secret=shh&grant_type=refresh_token&refresh_token=rt"
    );
    {
        let request = &seen.lock().unwrap()[1];
        assert_eq!(request.uri(), "https://www.strava.com/api/v3/activities/77");
        assert_eq!(request.headers()["authorization"], "Bearer new");
    }
    let stored = db::get_config_value(&db.lock(), "strava_tokens")
        .unwrap()
        .unwrap();
    assert_eq!(
        stored,
        json!({"access_token": "new", "refresh_token": "rt2", "expires_at": NOW + 21600, "scope": "read"})
    );

    let (client, _, _) = strava(
        vec![status(401)],
        json!({"access_token": "old", "refresh_token": "rt", "expires_at": 0}),
    );
    assert_eq!(
        client.update_activity_metadata(1, "", "").await,
        Err(StravaError::Auth("Strava refresh token was revoked".into()))
    );
    let (client, _, _) = strava(vec![], Value::Null);
    assert_eq!(
        client.update_activity_metadata(1, "", "").await,
        Err(StravaError::Auth("Strava is not connected".into()))
    );
}

#[tokio::test]
async fn strava_rate_limit_waits_for_the_quota_window() {
    let (client, _, _) = strava(
        vec![
            status(429),
            Ok((429, json!({}), vec![("retry-after", "90")])),
        ],
        fresh(),
    );
    let now = Utc.timestamp_opt(NOW, 0).unwrap();
    assert_eq!(now.format("%H:%M:%S").to_string(), "08:07:30");
    let reset = Utc.with_ymd_and_hms(2027, 1, 15, 8, 15, 0).unwrap();
    assert_eq!(
        client.update_activity_metadata(1, "a", "b").await,
        Err(StravaError::RateLimited(reset))
    );
    let told = now + chrono::Duration::seconds(90);
    assert_eq!(
        client.update_activity_metadata(1, "a", "b").await,
        Err(StravaError::RateLimited(told))
    );
}

fn activities(n: usize, start: i64) -> Value {
    Value::Array(
        (0..n)
            .map(|i| json!({"id": start + i as i64, "start_date": "2027-01-15T07:00:00Z"}))
            .collect(),
    )
}

#[tokio::test]
async fn strava_window_reads_page_to_the_end_or_say_so() {
    let (client, _, seen) = strava(
        vec![ok(activities(200, 1)), ok(activities(3, 1000))],
        fresh(),
    );
    let after = Utc.with_ymd_and_hms(2027, 1, 8, 0, 0, 0).unwrap();
    let before = Utc.with_ymd_and_hms(2027, 1, 15, 0, 0, 0).unwrap();
    assert_eq!(
        client
            .list_activities_between(after, before)
            .await
            .unwrap()
            .len(),
        203
    );
    assert_eq!(
        urls(&seen)[1],
        "GET https://www.strava.com/api/v3/athlete/activities?after=1799366400&before=1799971200&per_page=200&page=2"
    );

    let (client, _, _) = strava((0..10).map(|_| ok(activities(200, 1))).collect(), fresh());
    assert!(matches!(
        client.list_activities_between(after, before).await,
        Err(StravaError::WindowIncomplete(_))
    ));
}

#[test]
fn closest_activity_within_tolerance() {
    let start = Utc.with_ymd_and_hms(2027, 1, 15, 7, 0, 0).unwrap();
    let at = |id, seconds| StravaActivity {
        id,
        start_date: start + chrono::Duration::seconds(seconds),
    };
    assert_eq!(
        match_closest_activity(&[at(1, 240), at(2, -60), at(3, 30)], start, 5),
        Some(3)
    );
    assert_eq!(match_closest_activity(&[at(1, 301)], start, 5), None);
}

#[tokio::test]
async fn strava_upload_waits_for_processing() {
    let (client, _, seen) = strava(
        vec![
            ok(json!({"id": 555})),
            ok(json!({"activity_id": null, "error": null})),
            ok(json!({"activity_id": 9001})),
        ],
        fresh(),
    );
    assert_eq!(
        client
            .publish(42, b"FITDATA", Some("Morning run"), None)
            .await
            .unwrap(),
        9001
    );
    let upload = body(&seen, 0);
    for expected in [
        "name=\"external_id\"\r\n\r\ngarmin-42",
        "name=\"data_type\"\r\n\r\nfit",
        "name=\"name\"\r\n\r\nMorning run",
        "filename=\"42.fit\"",
        "FITDATA",
    ] {
        assert!(upload.contains(expected), "{expected} in upload");
    }
    assert!(!upload.contains("name=\"description\""));
    assert_eq!(
        urls(&seen)[2],
        "GET https://www.strava.com/api/v3/uploads/555"
    );

    let (client, _, _) = strava(
        vec![Ok((400, json!({"message": "bad file"}), vec![]))],
        fresh(),
    );
    assert_eq!(
        client.publish(42, b"x", None, None).await,
        Err(StravaError::Rejected(
            "upload rejected (400): {\"message\":\"bad file\"}".into()
        ))
    );
    let (client, _, _) = strava(
        vec![
            ok(json!({"id": 1})),
            ok(json!({"error": "duplicate of activity 7"})),
        ],
        fresh(),
    );
    assert_eq!(
        client.publish(42, b"x", None, None).await,
        Err(StravaError::Rejected("duplicate of activity 7".into()))
    );
    let mut slow = vec![ok(json!({"id": 1}))];
    slow.extend((0..10).map(|_| ok(json!({"activity_id": null}))));
    let (client, _, _) = strava(slow, fresh());
    assert_eq!(
        client.publish(42, b"x", None, None).await,
        Err(StravaError::Upload(
            "upload 1 did not resolve in time".into()
        ))
    );
}

#[tokio::test]
async fn strava_disconnect_forgets_tokens_even_offline() {
    let (client, db, seen) = strava(vec![Err("offline")], fresh());
    client.disconnect().await;
    let request = &seen.lock().unwrap()[0];
    assert_eq!(request.uri(), "https://www.strava.com/oauth/revoke");
    assert_eq!(request.headers()["authorization"], "Basic MTIzOnNoaA==");
    assert_eq!(
        db::get_config_value(&db.lock(), "strava_tokens").unwrap(),
        Some(Value::Null)
    );
}

#[tokio::test]
async fn strava_connect_stores_the_granted_scope() {
    let (client, db, seen) = strava(
        vec![ok(
            json!({"access_token": "a", "refresh_token": "r", "expires_at": 5, "athlete": {}}),
        )],
        Value::Null,
    );
    client
        .exchange_code("the-code", Some("read,activity:write"))
        .await
        .unwrap();
    assert_eq!(
        body(&seen, 0),
        "client_id=123&client_secret=shh&code=the-code&grant_type=authorization_code"
    );
    let stored = db::get_config_value(&db.lock(), "strava_tokens")
        .unwrap()
        .unwrap();
    assert_eq!(
        stored,
        json!({"access_token": "a", "refresh_token": "r", "expires_at": 5, "scope": "read,activity:write"})
    );
    assert_eq!(
        client.authorize_url("http://127.0.0.1:8382/strava/callback", "st"),
        "https://www.strava.com/oauth/authorize?client_id=123&redirect_uri=http://127.0.0.1:8382/strava/callback&response_type=code&approval_prompt=auto&scope=read,activity:write,activity:read_all&state=st"
    );
}

fn timeouts(seen: &Seen) -> Vec<Option<Duration>> {
    seen.lock()
        .unwrap()
        .iter()
        .map(|r| r.extensions().get::<Timeout>().map(|t| t.0))
        .collect()
}

#[tokio::test]
async fn hevy_words_failures_as_requests_does() {
    let (client, seen) = hevy(vec![status(404), status(418), ok(json!({}))]);
    assert_eq!(
        client
            .get_workouts_page(1, 10)
            .await
            .unwrap_err()
            .to_string(),
        "404 Client Error: Not Found for url: https://api.hevyapp.com/v1/workouts?page=1&pageSize=10"
    );
    assert_eq!(
        client.get_user_info().await.unwrap_err().to_string(),
        "418 Client Error: I'm a teapot for url: https://api.hevyapp.com/v1/user/info"
    );
    assert_eq!(timeouts(&seen), vec![Some(Duration::from_secs(30)); 2]);

    let (client, _) = hevy(vec![status(500); 6]);
    assert_eq!(
        client
            .get_workouts_page(1, 10)
            .await
            .unwrap_err()
            .to_string(),
        "500 Server Error: Internal Server Error for url: https://api.hevyapp.com/v1/workouts?page=1&pageSize=10"
    );

    let (client, seen) = hevy(vec![Err("connection refused"); 6]);
    assert_eq!(
        client
            .get_workouts_page(1, 10)
            .await
            .unwrap_err()
            .to_string(),
        "HTTPSConnectionPool(host='api.hevyapp.com', port=443): Max retries exceeded with url: /v1/workouts?page=1&pageSize=10 (Caused by connection refused)"
    );
    assert_eq!(urls(&seen).len(), 6);

    let (client, _) = hevy(vec![Err("error sending request: operation timed out"); 6]);
    assert_eq!(
        client.get_user_info().await.unwrap_err().to_string(),
        "HTTPSConnectionPool(host='api.hevyapp.com', port=443): Max retries exceeded with url: /v1/user/info \
         (Caused by ReadTimeoutError(\"HTTPSConnectionPool(host='api.hevyapp.com', port=443): Read timed out. (read timeout=30)\"))"
    );

    let (http, _) = scripted(vec![Ok((200, json!({}), vec![]))]);
    let empty = HevyClient::with_base_url(
        Arc::new(move |request| {
            let reply = http(request);
            Box::pin(async move {
                let mut response = reply.await?;
                *response.body_mut() = b"<html>".to_vec();
                Ok(response)
            })
        }),
        "key",
        "http://localhost:9",
    )
    .unwrap();
    assert_eq!(
        empty.get_user_info().await.unwrap_err().to_string(),
        "Expecting value: line 1 column 1 (char 0)"
    );
}

#[tokio::test]
async fn hevy_retries_follow_urllib3() {
    let now = Utc.with_ymd_and_hms(2027, 1, 15, 8, 0, 0).unwrap();
    let delay = |retry, header| retry_delay(retry, 2.0, header, now).as_secs_f64();
    assert_eq!(
        [
            delay(1, None),
            delay(2, None),
            delay(3, None),
            delay(5, None),
            delay(9, None)
        ],
        [0.0, 4.0, 8.0, 32.0, 120.0]
    );
    assert_eq!(delay(1, Some("7")), 7.0);
    assert_eq!(delay(4, Some(" 7 ")), 7.0);
    assert_eq!(
        delay(3, Some("0")),
        8.0,
        "a Retry-After of nothing falls back to the backoff"
    );
    assert_eq!(delay(3, Some("soon")), 8.0);
    assert_eq!(delay(1, Some("99999999")), 21600.0);
    assert_eq!(delay(1, Some("Fri, 15 Jan 2027 08:01:30 GMT")), 90.0);
    assert_eq!(
        delay(2, Some("Fri, 15 Jan 2027 07:00:00 GMT")),
        4.0,
        "a date in the past asks for nothing"
    );

    let told = |status| Ok((status, json!({}), vec![("retry-after", "1")]));
    let (client, seen) = hevy(vec![told(413), told(500), ok(json!({"id": "u1"}))]);
    assert_eq!(client.get_user_info().await.unwrap(), json!({"id": "u1"}));
    assert_eq!(
        urls(&seen).len(),
        3,
        "a 413 is retried only when Hevy says when"
    );

    let (client, seen) = hevy(vec![status(413), ok(json!({}))]);
    assert!(matches!(
        client.get_user_info().await,
        Err(HevyError::Status { code: 413, .. })
    ));
    assert_eq!(urls(&seen).len(), 1);
}

#[tokio::test]
async fn strava_calls_carry_the_timeouts_python_sets() {
    let stale = json!({"access_token": "old", "refresh_token": "rt", "expires_at": 0});
    let after = Utc.with_ymd_and_hms(2027, 1, 8, 0, 0, 0).unwrap();
    let before = Utc.with_ymd_and_hms(2027, 1, 15, 0, 0, 0).unwrap();
    let token = || ok(json!({"access_token": "a", "refresh_token": "r", "expires_at": NOW + 3600}));
    let secs = |n| Some(Duration::from_secs(n));

    let (client, _, seen) = strava(vec![token(), status(200), ok(json!([]))], stale.clone());
    client.update_activity_metadata(1, "a", "b").await.unwrap();
    client.list_activities_between(after, before).await.unwrap();
    assert_eq!(
        timeouts(&seen),
        vec![secs(30), secs(30), secs(30)],
        "token refresh, PUT, list"
    );

    let (client, _, seen) = strava(
        vec![ok(json!({"id": 5})), ok(json!({"activity_id": 9}))],
        fresh(),
    );
    client.publish(42, b"x", None, None).await.unwrap();
    assert_eq!(
        timeouts(&seen),
        vec![secs(60), secs(30)],
        "upload, status poll"
    );

    let (client, _, seen) = strava(vec![token()], Value::Null);
    client.exchange_code("c", None).await.unwrap();
    assert_eq!(timeouts(&seen), vec![secs(30)]);

    let (client, _, seen) = strava(vec![status(200)], fresh());
    client.disconnect().await;
    assert_eq!(timeouts(&seen), vec![secs(10)]);
}
