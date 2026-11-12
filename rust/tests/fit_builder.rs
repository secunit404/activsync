use activsync::fit_builder::{
    DeviceIdentity, GENERIC_GARMIN_IDENTITY, Profile, ResolvedExercise, build_fit,
    identity_from_config, identity_from_fit, new_fallback_identity,
};
use base64::Engine;
use serde_json::{Value, json};

fn cases() -> Vec<Value> {
    let path = format!(
        "{}/tests/fixtures/fit_builder_python.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str::<Value>(&std::fs::read_to_string(path).unwrap())
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn identity(value: &Value) -> DeviceIdentity {
    DeviceIdentity {
        manufacturer: value["manufacturer"].as_i64().unwrap(),
        product: value["product"].as_i64().unwrap(),
        serial: value["serial"].as_i64().unwrap(),
    }
}

/// The first byte where two files part, for a readable failure.
fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .or_else(|| (a.len() != b.len()).then(|| a.len().min(b.len())))
}

#[test]
fn the_fit_file_is_byte_for_byte_the_python_build() {
    for case in cases() {
        let name = case["name"].as_str().unwrap();
        let resolved: Vec<ResolvedExercise> = case["exercises"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| ResolvedExercise {
                title: e["title"].as_str().unwrap().into(),
                category: e["category"].as_i64().unwrap(),
                subcategory: e["subcategory"].as_i64().unwrap(),
                sets: e["sets"].as_array().unwrap().clone(),
            })
            .collect();
        let profile = Profile {
            weight_kg: case["profile"]["weight_kg"].as_f64().unwrap(),
            birth_year: case["profile"]["birth_year"].as_i64().unwrap(),
            vo2max: case["profile"]["vo2max"].as_f64().unwrap(),
            sex: case["profile"]["sex"].as_str().unwrap().into(),
        };
        let hr = case["hr"].as_array().cloned().unwrap_or_default();
        let built = build_fit(
            &case["workout"],
            &resolved,
            &hr,
            &profile,
            identity(&case["identity"]),
        );
        let expected = &case["result"];
        match (built, expected.get("fit")) {
            (Ok((bytes, summary)), Some(fit)) => {
                let python = base64::engine::general_purpose::STANDARD
                    .decode(fit.as_str().unwrap())
                    .unwrap();
                assert_eq!(
                    first_difference(&bytes, &python),
                    None,
                    "{name}: {} vs {} bytes",
                    bytes.len(),
                    python.len()
                );
                assert_eq!(summary.to_json(), expected["summary"], "{name}");
                let found = identity_from_fit(&bytes).unwrap();
                assert_eq!(
                    json!([found.manufacturer, found.product, found.serial]),
                    expected["identity"],
                    "{name}"
                );
            }
            (Err(reason), None) => {
                assert_eq!(reason, expected["error"].as_str().unwrap(), "{name}")
            }
            (built, _) => panic!(
                "{name}: rust {:?} vs python {expected}",
                built.map(|(_, s)| s)
            ),
        }
    }
}

#[test]
fn the_device_identity_comes_from_settings_or_the_generic_fallback() {
    let stored = json!({"manufacturer": 1, "product": "4534", "serial": 77.0});
    assert_eq!(
        identity_from_config(Some(&stored)),
        DeviceIdentity {
            manufacturer: 1,
            product: 4534,
            serial: 77
        }
    );
    assert_eq!(
        identity_from_config(Some(&json!({"manufacturer": 1}))),
        GENERIC_GARMIN_IDENTITY
    );
    assert_eq!(identity_from_config(None), GENERIC_GARMIN_IDENTITY);
    let fresh = new_fallback_identity();
    assert_eq!((fresh.manufacturer, fresh.product), (1, 0));
    assert!((1..i64::from(i32::MAX)).contains(&fresh.serial));
    assert_eq!(identity_from_fit(b"not a fit file"), None);
}
