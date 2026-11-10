use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use activsync::db::{self, Db};
use activsync::garmin_client::{GarminClient, Tokens};
use activsync::hevy_profile;
use activsync::hr_sources::{extract_fit_hr, merge_hr_sources, passive_hr_for_window};
use activsync::net::{Http, HttpError};
use base64::Engine;
use chrono::{DateTime, TimeZone, Utc};
use openidconnect::HttpRequest;
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap()
}

fn garmin(route: impl Fn(&str, &str) -> Result<Value, &'static str> + Send + Sync + 'static) -> (GarminClient, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let http: Http = Arc::new(move |request: HttpRequest| {
        counter.fetch_add(1, Ordering::SeqCst);
        let reply = route(request.uri().path(), request.uri().query().unwrap_or_default())
            .map(|body| openidconnect::http::Response::builder().status(200).body(body.to_string().into_bytes()).unwrap())
            .map_err(|e| HttpError(e.into()));
        Box::pin(async move { reply })
    });
    let tokens = Tokens { di_token: Some("t".into()), di_refresh_token: None, di_client_id: None };
    (GarminClient::new(http, Arc::new(|| now().timestamp() as f64), tokens, None).without_waits(), calls)
}

#[test]
fn watch_heart_rate_matches_python_even_from_a_truncated_file() {
    let reference = fixture("hr_profile_python.json");
    let fits: std::collections::HashMap<String, Vec<u8>> = fixture("fit_builder_python.json")
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| {
            let fit = c["result"]["fit"].as_str()?;
            Some((c["name"].as_str()?.to_string(), base64::engine::general_purpose::STANDARD.decode(fit).ok()?))
        })
        .collect();
    for case in reference["extract"].as_array().unwrap() {
        let fit = &fits[case["name"].as_str().unwrap()];
        let cut = case["cut"].as_u64().unwrap() as usize;
        let samples = extract_fit_hr(&fit[..fit.len() - cut], case["start_ms"].as_i64().unwrap());
        assert_eq!(Value::from(samples), case["samples"], "{} cut {cut}", case["name"]);
    }
    assert!(extract_fit_hr(b"garbage", 0).is_empty());
}

#[tokio::test]
async fn daily_heart_rate_covers_both_days_of_a_late_session() {
    let reference = fixture("hr_profile_python.json");
    let passive = reference["passive"].clone();
    let days = passive["days"].clone();
    let (client, _) = garmin(move |path, query| {
        assert!(path.starts_with("/wellness-service/wellness/dailyHeartRate/"));
        Ok(days[query.trim_start_matches("date=")].clone())
    });
    let client = client.with_display_name("runner");
    let at = |key: &str| DateTime::parse_from_rfc3339(passive[key].as_str().unwrap()).unwrap().with_timezone(&Utc);
    let samples = passive_hr_for_window(&client, at("start"), at("end")).await;
    assert_eq!(Value::from(samples), passive["samples"]);

    let (failing, _) = garmin(|_, _| Err("down"));
    assert!(passive_hr_for_window(&failing.with_display_name("r"), at("start"), at("end")).await.is_empty());
}

#[test]
fn the_merge_prefers_the_watch_per_ten_seconds() {
    let merge = fixture("hr_profile_python.json")["merge"].clone();
    let list = |key: &str| merge[key].as_array().unwrap().clone();
    assert_eq!(Value::from(merge_hr_sources(&list("primary"), &list("secondary"))), merge["merged"]);
    assert_eq!(Value::from(merge_hr_sources(&list("primary"), &[])), merge["primary_only"]);
    let mut reversed = list("secondary");
    reversed.reverse();
    assert_eq!(Value::from(merge_hr_sources(&[], &reversed)), merge["secondary_only"]);
}

/// Garmin answers in its own units: grams, a birth date, upper-case sex.
fn garmin_profile(fetched: &Value) -> Value {
    let mut user = serde_json::Map::new();
    if let Some(weight) = fetched["weight_kg"].as_f64() {
        user.insert("weight".into(), json!(weight * 1000.0));
    }
    if let Some(year) = fetched["birth_year"].as_i64() {
        user.insert("birthDate".into(), json!(format!("{year}-03-14")));
    }
    if let Some(sex) = fetched["sex"].as_str() {
        user.insert("gender".into(), json!(sex.to_uppercase()));
    }
    json!({"userData": user})
}

#[tokio::test]
async fn the_profile_falls_back_like_python() {
    for case in fixture("hr_profile_python.json")["profiles"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let db = Db::open_in_memory().unwrap();
        if !case["cache"].is_null() {
            db::set_config_value(&db.lock(), hevy_profile::CACHE_KEY, &case["cache"]).unwrap();
        }
        if !case["settings"].is_null() {
            db::set_config_value(&db.lock(), "settings", &case["settings"]).unwrap();
        }
        let fetched = case["fetched"].clone();
        let empty = fetched.as_object().unwrap().values().all(Value::is_null);
        let settings = garmin_profile(&fetched);
        let (client, calls) = garmin(move |path, _| {
            if empty {
                return Err("down");
            }
            match path {
                "/userprofile-service/userprofile/user-settings" => Ok(settings.clone()),
                _ => Ok(json!([])),
            }
        });
        let profile = hevy_profile::get_profile(&db, &client, now()).await;
        let expected = &case["profile"];
        assert_eq!(profile.weight_kg, expected["weight_kg"].as_f64().unwrap(), "{name}");
        assert_eq!(profile.birth_year, expected["birth_year"].as_i64().unwrap(), "{name}");
        assert_eq!(profile.vo2max, expected["vo2max"].as_f64().unwrap(), "{name}");
        assert_eq!(profile.sex, expected["sex"].as_str().unwrap(), "{name}");
        assert_eq!(db::get_config_value(&db.lock(), hevy_profile::CACHE_KEY).unwrap(), Some(case["stored"].clone()).filter(|v| !v.is_null()), "{name}");
        assert_eq!(calls.load(Ordering::SeqCst) > 0, case["fetch_calls"].as_u64().unwrap() > 0, "{name}");
    }
}
