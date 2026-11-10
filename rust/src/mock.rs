//! No-network stand-ins for Garmin, Strava and Hevy, matching what the Python
//! app's `dev_mock.py` answers so the dev UI walks the same paths.

use serde_json::{Value, json};

/// A Garmin password of "mfa" asks for a verification code.
pub const MFA_TRIGGER_PASSWORD: &str = "mfa";
/// The one verification code the mock rejects.
pub const MFA_REJECT_CODE: &str = "000000";

const GARMIN_ACTIVITY_TYPE_KEYS: &[&str] = &[
    "american_football", "apnea_diving", "apnea_hunting", "archery", "assistance", "atv_v2",
    "auto_racing", "backcountry_skiing", "backcountry_skiing_snowboarding_ws",
    "backcountry_snowboarding", "badminton", "baseball", "basketball", "biketoruntransition_v2",
    "bmx", "boating_v2", "bouldering", "boxing", "breathwork", "casual_walking", "ccr_diving",
    "cricket", "cross_country_indoor_skiing", "cross_country_skiing_ws", "cycling", "cyclocross",
    "dance", "disc_golf", "diving", "downhill_biking", "driving_general", "e_bike_fitness",
    "e_bike_mountain", "e_enduro_mtb", "e_sport", "elliptical", "enduro_mtb", "field_hockey",
    "fishing_v2", "fitness_equipment", "floor_climbing", "flying", "gauge_diving", "golf",
    "gravel_cycling", "hand_cycling", "hang_gliding", "hiit", "hiking", "horseback_riding",
    "hunting", "hunting_fishing", "ice_hockey", "incident_detected", "indoor_cardio",
    "indoor_climbing", "indoor_cycling", "indoor_hand_cycling", "indoor_rowing", "indoor_running",
    "inline_skating", "jump_rope", "kayaking_v2", "kiteboarding_v2", "lacrosse", "lap_swimming",
    "meditation", "mixed_martial_arts", "mobility", "motocross_v2", "motorcycling_v2",
    "mountain_biking", "mountaineering", "multi_gas_diving", "multi_sport", "obstacle_run",
    "offshore_grinding_v2", "onshore_grinding_v2", "open_water_swimming", "other", "overland",
    "paddelball", "paddling_v2", "para_sports", "pickleball", "pilates", "platform_tennis",
    "pool_apnea", "racket_sports", "racquetball", "rc_drone", "recumbent_cycling", "resort_skiing",
    "resort_skiing_snowboarding_ws", "resort_snowboarding", "road_biking", "rock_climbing",
    "rowing_v2", "rucking", "rugby", "running", "runtobiketransition_v2", "safety", "sailing_v2",
    "single_gas_diving", "skate_skiing_ws", "skating_ws", "sky_diving", "snorkeling",
    "snow_shoe_ws", "snowmobiling_ws", "soccer", "softball", "speed_walking", "squash",
    "stair_climbing", "stand_up_paddleboarding_v2", "steps", "stop_watch", "street_running",
    "strength_training", "surfing_v2", "swimming", "swimtobiketransition_v2", "table_tennis",
    "team_sports", "tennis_v2", "track_cycling", "track_running", "trail_running", "transition_v2",
    "treadmill_running", "ultimate_disc", "ultra_run", "virtual_ride", "virtual_run", "volleyball",
    "wakeboarding_v2", "wakesurfing", "walking", "water_sports", "water_tubing", "waterskiing",
    "wheelchair_push_run", "wheelchair_push_walk", "whitewater_rafting_kayaking",
    "whitewater_rafting_v2", "wind_kite_surfing", "windsurfing_v2", "wingsuit_flying",
    "winter_sports", "yoga",
];

/// Python's `str.title()`: a letter is upper-cased when the character before
/// it is not a letter, so "atv v2" becomes "Atv V2".
pub fn python_title(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_is_letter = false;
    for ch in text.chars() {
        if ch.is_alphabetic() {
            if previous_is_letter {
                out.extend(ch.to_lowercase());
            } else {
                out.extend(ch.to_uppercase());
            }
            previous_is_letter = true;
        } else {
            out.push(ch);
            previous_is_letter = false;
        }
    }
    out
}

/// The taxonomy the way the real Garmin client reports it: labels derived
/// from the key, sorted by label.
pub fn garmin_activity_types() -> Vec<Value> {
    let mut types: Vec<(String, &str)> = GARMIN_ACTIVITY_TYPE_KEYS
        .iter()
        .map(|key| (python_title(&key.replace('_', " ")), *key))
        .collect();
    types.sort();
    types.into_iter().map(|(label, key)| json!({"type_key": key, "label": label})).collect()
}

pub enum GarminLogin {
    Connected,
    MfaRequired,
}

pub fn begin_garmin_login(_email: &str, password: &str) -> GarminLogin {
    if password == MFA_TRIGGER_PASSWORD {
        GarminLogin::MfaRequired
    } else {
        GarminLogin::Connected
    }
}

pub fn complete_garmin_login(code: &str) -> Result<(), String> {
    if code == MFA_REJECT_CODE {
        Err("Invalid MFA code (dev mock rejects 000000)".into())
    } else {
        Ok(())
    }
}

/// Skips Strava entirely: bounces straight back to our own callback with a
/// placeholder code, echoing the state as the real Strava would.
pub fn strava_authorize_url(redirect_uri: &str, state: &str) -> String {
    let separator = if redirect_uri.contains('?') { '&' } else { '?' };
    format!("{redirect_uri}{separator}code=dev-mock-code&state={state}")
}

pub fn strava_tokens(granted_scope: Option<&str>, now_epoch: i64) -> Value {
    let mut tokens = json!({
        "access_token": "dev-mock-access-token",
        "refresh_token": "dev-mock-refresh-token",
        "expires_at": now_epoch + 6 * 3600,
    });
    if let Some(scope) = granted_scope {
        tokens["scope"] = json!(scope);
    }
    tokens
}
