use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::db::truthy;

const LEEWAY_SECONDS: f64 = 120.0;

pub(super) struct Expected<'a> {
    pub issuer: &'a str,
    pub client_id: &'a str,
    pub nonce: &'a str,
    pub access_token: &'a str,
}

fn decode_segment(jwt: &str, index: usize) -> Option<Value> {
    let segment = jwt.split('.').nth(index)?;
    let bytes = URL_SAFE_NO_PAD.decode(segment.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(super) fn check(jwt: &str, expected: &Expected<'_>, now: f64) -> Result<(), String> {
    let header = decode_segment(jwt, 0).unwrap_or(Value::Null);
    let payload = decode_segment(jwt, 1).unwrap_or(Value::Null);
    authlib_problem(&header, &payload, expected, now)
}

fn half_hash(text: &str, alg: &str) -> Option<String> {
    let digest = match alg {
        "EdDSA" => Sha512::digest(text.as_bytes()).to_vec(),
        _ => match alg.get(2..)? {
            "256" => Sha256::digest(text.as_bytes()).to_vec(),
            "384" => Sha384::digest(text.as_bytes()).to_vec(),
            "512" => Sha512::digest(text.as_bytes()).to_vec(),
            _ => return None,
        },
    };
    Some(URL_SAFE_NO_PAD.encode(&digest[..digest.len() / 2]))
}

/// The ID token claim checks Authlib makes once the signature holds (its
/// `CodeIDToken.validate` with the app's `claims_options`): the client must be
/// among the audiences but others may be too; `azp` is required when the sole
/// audience is not the client, or there are several, and must then be the
/// client; time claims get two minutes of leeway.
fn authlib_problem(
    header: &Value,
    claims: &Value,
    expected: &Expected<'_>,
    now: f64,
) -> Result<(), String> {
    let get = |name: &str| claims.get(name).filter(|v| !v.is_null());
    for name in ["iss", "sub", "aud", "exp", "iat"] {
        if get(name).is_none() {
            return Err(format!("missing claim: {name}"));
        }
    }
    let invalid = |name: &str| Err(format!("invalid claim: {name}"));
    match get("iss").and_then(Value::as_str) {
        Some(iss) if !iss.is_empty() && iss == expected.issuer => {}
        _ => return invalid("iss"),
    }
    if get("sub").and_then(Value::as_str).is_none() {
        return invalid("sub");
    }
    let audiences: Vec<&str> = match get("aud") {
        Some(Value::String(aud)) => vec![aud.as_str()],
        Some(Value::Array(list)) => {
            match list.iter().map(Value::as_str).collect::<Option<Vec<_>>>() {
                Some(list) => list,
                None => return invalid("aud"),
            }
        }
        _ => return invalid("aud"),
    };
    if !audiences.contains(&expected.client_id) {
        return invalid("aud");
    }
    let number =
        |name: &str| get(name).map(|v| v.as_f64().ok_or_else(|| format!("invalid claim: {name}")));
    if number("exp")
        .transpose()?
        .is_some_and(|exp| exp < now - LEEWAY_SECONDS)
    {
        return Err("expired token".into());
    }
    if number("nbf")
        .transpose()?
        .is_some_and(|nbf| nbf > now + LEEWAY_SECONDS)
    {
        return Err("invalid claim: nbf".into());
    }
    if number("iat")
        .transpose()?
        .is_some_and(|iat| iat > now + LEEWAY_SECONDS)
    {
        return Err("invalid claim: iat".into());
    }
    if truthy(claims.get("auth_time")) && !claims["auth_time"].is_number() {
        return invalid("auth_time");
    }
    if !expected.nonce.is_empty() {
        match get("nonce") {
            None => return Err("missing claim: nonce".into()),
            Some(nonce) if nonce.as_str() != Some(expected.nonce) => return invalid("nonce"),
            Some(_) => {}
        }
    }
    if truthy(claims.get("amr")) && !claims["amr"].is_array() {
        return invalid("amr");
    }
    let azp = claims.get("azp").filter(|_| truthy(claims.get("azp")));
    let sole_client = audiences.len() == 1 && audiences[0] == expected.client_id;
    if !sole_client && azp.is_none() {
        return Err("missing claim: azp".into());
    }
    if azp.is_some_and(|azp| azp.as_str() != Some(expected.client_id)) {
        return invalid("azp");
    }
    if let Some(at_hash) = claims
        .get("at_hash")
        .filter(|_| truthy(claims.get("at_hash")))
        && !expected.access_token.is_empty()
    {
        let alg = header
            .get("alg")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if half_hash(expected.access_token, alg).as_deref() != at_hash.as_str() {
            return invalid("at_hash");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const NOW: f64 = 1_000_000.0;
    const ISSUER: &str = "https://id.example.test/";

    fn verdict(patch: Value) -> bool {
        let mut claims = json!({
            "iss": ISSUER, "sub": "s", "aud": "activsync", "exp": NOW + 300.0, "iat": NOW, "nonce": "n1",
        });
        for (key, value) in patch.as_object().unwrap() {
            if value.is_null() {
                claims.as_object_mut().unwrap().remove(key);
            } else {
                claims[key] = value.clone();
            }
        }
        let expected = Expected {
            issuer: ISSUER,
            client_id: "activsync",
            nonce: "n1",
            access_token: "at",
        };
        authlib_problem(&json!({"alg": "RS256"}), &claims, &expected, NOW).is_ok()
    }

    #[test]
    fn id_token_claims_follow_authlib() {
        let cases = [
            ("plain", json!({}), true),
            ("aud_list_one", json!({"aud": ["activsync"]}), true),
            (
                "aud_two_no_azp",
                json!({"aud": ["activsync", "other"]}),
                false,
            ),
            (
                "aud_two_azp",
                json!({"aud": ["activsync", "other"], "azp": "activsync"}),
                true,
            ),
            (
                "aud_two_wrong_azp",
                json!({"aud": ["activsync", "other"], "azp": "other"}),
                false,
            ),
            ("aud_other_only", json!({"aud": ["other"]}), false),
            (
                "aud_other_azp_client",
                json!({"aud": ["other"], "azp": "activsync"}),
                false,
            ),
            ("azp_other_single_aud", json!({"azp": "other"}), false),
            ("azp_client", json!({"azp": "activsync"}), true),
            ("aud_empty_list", json!({"aud": []}), false),
            ("exp_within_leeway", json!({"exp": NOW - 100.0}), true),
            ("exp_past_leeway", json!({"exp": NOW - 121.0}), false),
            ("exp_edge", json!({"exp": NOW - 120.0}), true),
            ("iat_within_leeway", json!({"iat": NOW + 100.0}), true),
            ("iat_future", json!({"iat": NOW + 121.0}), false),
            ("nbf_future", json!({"nbf": NOW + 121.0}), false),
            ("nbf_ok", json!({"nbf": NOW + 100.0}), true),
            ("amr_string", json!({"amr": "pwd"}), false),
            ("amr_list", json!({"amr": ["pwd"]}), true),
            ("auth_time_string", json!({"auth_time": "x"}), false),
            ("auth_time_number", json!({"auth_time": 5}), true),
            (
                "at_hash_ok",
                json!({"at_hash": "sda5G2fCr6XjIpiNlGJjjQ"}),
                true,
            ),
            ("at_hash_bad", json!({"at_hash": "AAAA"}), false),
            ("nonce_wrong", json!({"nonce": "other"}), false),
            ("nonce_missing", json!({"nonce": null}), false),
            ("iss_other", json!({"iss": "https://evil.example/"}), false),
            ("iss_blank", json!({"iss": ""}), false),
        ];
        for (name, patch, expected) in cases {
            assert_eq!(verdict(patch), expected, "{name}");
        }
    }

    #[test]
    fn half_hash_matches_authlib() {
        assert_eq!(
            half_hash("at", "RS256").as_deref(),
            Some("sda5G2fCr6XjIpiNlGJjjQ")
        );
        assert_eq!(half_hash("at", "ES256K"), None);
        assert_eq!(half_hash("at", "HS1"), None);
    }
}
