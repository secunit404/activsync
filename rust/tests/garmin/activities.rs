use activsync::garmin_client::{ActivityRecord, GarminError, UploadResult};
use serde_json::{Value, json};

use super::{client, fixture, garmin, keys, ok, zipped};

fn record_json(record: &ActivityRecord) -> Value {
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
        "GET /activitylist-service/activities/search/activities?start=0&limit=20" => {
            ok(batch.clone())
        }
        other => panic!("unexpected {other}"),
    });
    let records = client(http).fetch_recent_activities(7).await.unwrap();
    let expected = reference["records"].as_array().unwrap();
    assert_eq!(records.len(), expected.len());
    for (record, python) in records.iter().zip(expected) {
        let rust = record_json(record);
        for (key, value) in python.as_object().unwrap() {
            assert!(
                same(&rust[key], value),
                "{key}: rust {} vs python {value}",
                rust[key]
            );
        }
    }
    assert_eq!(keys(&seen).len(), 1, "a short page is the last one");
}

#[tokio::test]
async fn the_original_download_yields_its_fit_file() {
    let archive = zipped(&[
        ("notes.txt", b"hello"),
        ("123_ACTIVITY.FIT", b"FITDATA"),
        ("other.fit", b"NO"),
    ]);
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
    assert!(matches!(
        client.download_fit(125).await,
        Err(GarminError::NotFound(_))
    ));
    assert!(matches!(
        client.download_fit(0).await,
        Err(GarminError::Invalid(_))
    ));
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
                let expected = UploadResult {
                    upload_id: Some(expected["upload_id"].clone()),
                    activity_id: expected["activity_id"].as_i64(),
                };
                assert_eq!(result.unwrap(), expected);
            }
            None => assert_eq!(
                result.unwrap_err(),
                GarminError::UploadRejected(case["result"]["rejected"].as_str().unwrap().into())
            ),
        }
        let request = seen.lock().unwrap()[0].clone();
        assert_eq!(request.uri().path(), "/upload-service/upload");
        let body = String::from_utf8_lossy(request.body());
        assert!(body.contains(
            "Content-Disposition: form-data; name=\"file\"; filename=\"workout.fit\"\r\n\r\nFIT\r\n"
        ));
    }

    let duplicate = fixture()["errors"][1].clone();
    let (http, _) = garmin(move |_, _| Ok((409, duplicate["body"].to_string().into_bytes())));
    assert_eq!(
        client(http).upload_fit(&fit).await.unwrap_err(),
        GarminError::UploadRejected("Garmin rejected upload: API Error 409 - {'code': 202, 'content': 'Duplicate Activity.'}".into())
    );
    let (http, _) = garmin(|_, _| ok(json!({})));
    let missing = dir.path().join("gone.fit");
    assert!(
        matches!(client(http.clone()).upload_fit(&missing).await, Err(GarminError::Invalid(m)) if m.starts_with("FIT file not found"))
    );
    let csv = dir.path().join("workout.csv");
    std::fs::write(&csv, b"x").unwrap();
    assert!(matches!(
        client(http).upload_fit(&csv).await,
        Err(GarminError::Invalid(_))
    ));
}

#[tokio::test]
async fn exercise_sets_and_edits_tell_a_missing_activity_from_an_outage() {
    let (http, seen) = garmin(|key, _| match key {
        "PUT /activity-service/activity/5/exerciseSets" => {
            ok(json!({"message": "Invalid sub-category ARM_CURL"})).map(|(_, b)| (400, b))
        }
        "PUT /activity-service/activity/6/exerciseSets" | "PUT /activity-service/activity/9" => {
            Ok((404, b"{}".to_vec()))
        }
        "PUT /activity-service/activity/7/exerciseSets" => {
            Err("connection refused for /activity/404/")
        }
        "GET /activity-service/activity/8/exerciseSets" => ok(json!({"exerciseSets": []})),
        "PUT /activity-service/activity/77" | "DELETE /activity-service/activity/77" => {
            Ok((204, Vec::new()))
        }
        other => panic!("unexpected {other}"),
    });
    let client = client(http);
    let payload = json!({"exerciseSets": [{"setType": "ACTIVE"}]});
    assert!(
        matches!(client.put_exercise_sets(5, &payload).await, Err(GarminError::SubcategoryRejected(m)) if m.contains("ARM_CURL"))
    );
    assert_eq!(
        seen.lock().unwrap()[0].headers()["content-type"],
        "application/json"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(seen.lock().unwrap()[0].body()).unwrap(),
        payload
    );
    assert_eq!(
        client.put_exercise_sets(6, &payload).await,
        Err(GarminError::ActivityGone(6))
    );
    assert!(matches!(
        client.put_exercise_sets(7, &payload).await,
        Err(GarminError::Network(_))
    ));
    assert_eq!(
        client.get_exercise_sets(8).await.unwrap(),
        json!({"exerciseSets": []})
    );
    assert_eq!(
        client.set_title(9, "x").await,
        Err(GarminError::ActivityGone(9))
    );

    seen.lock().unwrap().clear();
    client
        .update_activity_metadata(77, "Leg day", "Squats")
        .await
        .unwrap();
    client.delete_activity(77).await.unwrap();
    let bodies: Vec<Value> = seen
        .lock()
        .unwrap()
        .iter()
        .take(2)
        .map(|r| serde_json::from_slice(r.body()).unwrap())
        .collect();
    assert_eq!(
        bodies,
        vec![
            json!({"activityId": "77", "activityName": "Leg day"}),
            json!({"activityId": "77", "description": "Squats"}),
        ]
    );
    assert_eq!(
        keys(&seen)[2],
        "DELETE https://connectapi.garmin.com/activity-service/activity/77"
    );
}

#[tokio::test]
async fn the_profile_and_type_list_match_python() {
    let reference = fixture();
    for case in reference["profiles"].as_array().unwrap() {
        let (settings, metrics) = (case["settings"].clone(), case["metrics"].clone());
        let (http, _) = garmin(move |key, _| match key {
            "GET /userprofile-service/userprofile/user-settings" => ok(settings.clone()),
            "GET /metrics-service/metrics/maxmet/daily/2026-11-10/2026-11-10" => {
                ok(metrics.clone())
            }
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
    assert_eq!(
        (profile.weight_kg, profile.vo2max),
        (None, None),
        "a failed read is not fatal"
    );

    let raw = reference["raw_types"].clone();
    let (http, _) = garmin(move |_, _| ok(raw.clone()));
    assert_eq!(
        Value::from(client(http).fetch_activity_types().await.unwrap()),
        reference["types"]
    );
}

#[tokio::test]
async fn snapshots_near_a_start_cover_three_days_and_never_hide_an_outage() {
    let (http, seen) = garmin(|key, _| match key {
        "GET /activitylist-service/activities/search/activities?startDate=2026-11-09&start=0&limit=20&endDate=2026-11-11" => {
            ok(json!([{"activityId": 1}, {"activityId": "2"}, {"activityName": "no id"}]))
        }
        "GET /activitylist-service/activities/search/activities?startDate=2026-11-09&start=20&limit=20&endDate=2026-11-11" => {
            ok(json!([]))
        }
        other => panic!("unexpected {other}"),
    });
    assert_eq!(
        client(http)
            .list_activity_ids_near("2026-11-10 06:30:00")
            .await
            .unwrap(),
        vec![1, 2]
    );
    assert_eq!(keys(&seen).len(), 2);

    let (http, _) = garmin(|_, _| Ok((503, b"{}".to_vec())));
    assert!(
        client(http.clone())
            .list_activity_ids_near("2026-11-10T06:30:00Z")
            .await
            .is_err()
    );
    assert!(matches!(
        client(http).list_activities_near("soon").await,
        Err(GarminError::Invalid(_))
    ));
}
