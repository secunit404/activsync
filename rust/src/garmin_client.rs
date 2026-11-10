//! Garmin Connect after sign-in, as the Python app's `garmin_client.py` over
//! garminconnect 0.3.16: the saved DI token opens a session, every call goes
//! to connectapi with the Android app's headers, and a token near expiry or a
//! 401 is answered with one refresh. Signing in with a password (the SSO
//! strategy chain and MFA) is not here; it still needs the Python library.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, NaiveDate, Utc};
use openidconnect::{HttpResponse, http};
use serde_json::{Number, Value, json};

use crate::net::{self, Http, Part};
use crate::schedule::Clock;

pub const CONNECTAPI_URL: &str = "https://connectapi.garmin.com";
pub const DI_TOKEN_URL: &str = "https://diauth.garmin.com/di-oauth2-service/oauth/token";
pub const TOKEN_FILE_NAME: &str = "garmin_tokens.json";

const NATIVE_API_USER_AGENT: &str = "GCM-Android-5.23";
const NATIVE_X_GARMIN_USER_AGENT: &str =
    "com.garmin.android.apps.connectmobile/5.23; ; Google/sdk_gphone64_arm64/google; Android/33; Dalvik/2.1.0";
/// Refresh this long before the token's `exp`, as the library does.
const REFRESH_MARGIN_SECONDS: f64 = 900.0;
const PAGE_SIZE: usize = 20;
const MAX_PAGINATED_REQUESTS: usize = 2000;

const ACTIVITIES_PATH: &str = "/activitylist-service/activities/search/activities";
const ACTIVITY_PATH: &str = "/activity-service/activity";
const FIT_DOWNLOAD_PATH: &str = "/download-service/files/activity";
const UPLOAD_PATH: &str = "/upload-service/upload";
const SOCIAL_PROFILE_PATH: &str = "/userprofile-service/socialProfile";
const USER_SETTINGS_PATH: &str = "/userprofile-service/userprofile/user-settings";
const MAX_METRICS_PATH: &str = "/metrics-service/metrics/maxmet/daily";
const DAILY_HEART_RATE_PATH: &str = "/wellness-service/wellness/dailyHeartRate";
const ACTIVITY_TYPES_PATH: &str = "/activity-service/activity/activityTypes";
const UPLOAD_FORMATS: [&str; 3] = ["FIT", "GPX", "TCX"];

#[derive(Debug, Clone, PartialEq)]
pub enum GarminError {
    /// The saved session is missing or Garmin no longer accepts it.
    SessionExpired,
    /// A definitive 404 on an activity call.
    ActivityGone(i64),
    /// A 404 anywhere else.
    NotFound(String),
    /// Garmin definitively refused a FIT upload.
    UploadRejected(String),
    /// Garmin 400-rejected an exercise-sets payload over a (category,
    /// subcategory) pair. The PUT is atomic, so the whole payload failed.
    SubcategoryRejected(String),
    /// Any other answer of 400 or above, worded as the library words it.
    Api(String),
    Network(String),
    Malformed(String),
    Invalid(String),
}

impl std::fmt::Display for GarminError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionExpired => f.write_str("Garmin session expired; reconnect Garmin to resume sync."),
            Self::ActivityGone(id) => write!(f, "activity {id} not found"),
            Self::NotFound(msg) | Self::UploadRejected(msg) | Self::SubcategoryRejected(msg) | Self::Api(msg) => {
                f.write_str(msg)
            }
            Self::Network(err) => write!(f, "could not reach Garmin: {err}"),
            Self::Malformed(err) => write!(f, "unreadable Garmin response: {err}"),
            Self::Invalid(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for GarminError {}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActivityRecord {
    pub garmin_activity_id: i64,
    pub activity_type: String,
    pub title: String,
    pub description: String,
    pub start_time: String,
    pub distance: Option<Number>,
    pub duration: Option<Number>,
    pub moving_duration: Option<Number>,
    pub elapsed_duration: Option<Number>,
    pub elevation_gain: Option<Number>,
    pub elevation_loss: Option<Number>,
    pub calories: Option<Number>,
    pub avg_hr: Option<Number>,
    pub max_hr: Option<Number>,
    pub avg_power: Option<Number>,
    pub max_power: Option<Number>,
    pub norm_power: Option<Number>,
    pub aerobic_te: Option<Number>,
    pub anaerobic_te: Option<Number>,
    pub training_load: Option<Number>,
    pub avg_cadence: Option<Number>,
    pub max_cadence: Option<Number>,
    pub total_sets: Option<Number>,
    pub total_reps: Option<Number>,
    pub total_volume: Option<Number>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UploadResult {
    pub upload_id: Option<Value>,
    /// `None` while Garmin is still processing the import; resolving it is
    /// the operation journal's job, never a retry loop here.
    pub activity_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserProfile {
    pub weight_kg: Option<f64>,
    pub birth_year: Option<i64>,
    pub sex: Option<String>,
    pub vo2max: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tokens {
    pub di_token: Option<String>,
    pub di_refresh_token: Option<String>,
    pub di_client_id: Option<String>,
}

impl Tokens {
    /// Byte-for-byte what the library's `dumps()` writes, so either server
    /// reads the other's token file.
    pub fn to_json(&self) -> String {
        let field = |v: &Option<String>| serde_json::to_string(v).unwrap_or_else(|_| "null".into());
        format!(
            "{{\"di_token\": {}, \"di_refresh_token\": {}, \"di_client_id\": {}}}",
            field(&self.di_token),
            field(&self.di_refresh_token),
            field(&self.di_client_id)
        )
    }

    fn from_json(text: &str) -> Option<Self> {
        let data: Value = serde_json::from_str(text).ok()?;
        let text_of = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_string);
        let tokens = Self { di_token: text_of("di_token"), di_refresh_token: text_of("di_refresh_token"), di_client_id: text_of("di_client_id") };
        tokens.di_token.as_deref().is_some_and(|t| !t.is_empty()).then_some(tokens)
    }
}

/// The token file a directory or `.json` path names. Refuses `~otheruser`
/// and a symlink anywhere in the path, so a planted link cannot redirect the
/// refresh token elsewhere.
pub fn token_file_path(path: &str) -> Result<PathBuf, GarminError> {
    let mut chars = path.chars();
    if chars.next() == Some('~') && chars.next().is_some_and(|c| c != '/' && c != '\\') {
        return Err(GarminError::Invalid(format!("Token path must not reference another user's home directory: {path:?}")));
    }
    let expanded = match (path.strip_prefix('~'), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest.trim_start_matches('/')),
        _ => PathBuf::from(path),
    };
    for check in expanded.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        if std::fs::symlink_metadata(check).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(GarminError::Invalid(format!("Token path must not be a symlink: {path:?}")));
        }
    }
    let is_json = expanded.extension().is_some_and(|e| e.to_string_lossy().to_lowercase() == "json");
    Ok(if expanded.is_dir() || !is_json { expanded.join(TOKEN_FILE_NAME) } else { expanded })
}

fn load_tokens(path: &str) -> Option<Tokens> {
    Tokens::from_json(&std::fs::read_to_string(token_file_path(path).ok()?).ok()?)
}

/// Owner-only, via a fresh unpredictable sibling renamed into place, so a
/// reader never sees half a file and no planted inode receives the token.
pub fn save_tokens(path: &str, tokens: &Tokens) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let target = token_file_path(path).map_err(|e| std::io::Error::other(e.to_string()))?;
    let dir = target.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut suffix = [0u8; 8];
    rand::fill(&mut suffix);
    let hex: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
    let tmp = dir.join(format!(".{name}.{hex}.tmp"));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
        file.write_all(tokens.to_json().as_bytes())?;
        drop(file);
        std::fs::rename(&tmp, &target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// The claims of a JWT without checking its signature: the client has no
/// key. `alg: none` is refused so an unsigned token cannot pose as one.
fn jwt_claims(token: &str) -> Option<serde_json::Map<String, Value>> {
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::URL_SAFE,
        base64::engine::GeneralPurposeConfig::new().with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let mut parts = token.split('.');
    let decode = |part: &str| serde_json::from_slice::<Value>(&engine.decode(part).ok()?).ok();
    let header = decode(parts.next()?)?;
    let claims = decode(parts.next()?)?;
    if header.as_object()?.get("alg").and_then(Value::as_str) == Some("none") {
        return None;
    }
    claims.as_object().cloned()
}

/// `exp` comes from an unverified token, so anything but a finite number
/// (or a numeric string) reads as "not expiring" rather than failing.
fn expires_soon(token: &str, now: f64) -> bool {
    let exp = match jwt_claims(token).and_then(|c| c.get("exp").cloned()) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    exp.is_some_and(|exp| exp.is_finite() && now > exp - REFRESH_MARGIN_SECONDS)
}

fn client_id_of(token: &str) -> Option<String> {
    let value = jwt_claims(token)?.get("client_id").cloned()?;
    match value {
        Value::String(s) if !s.is_empty() => Some(s),
        Value::String(_) | Value::Null | Value::Bool(false) => None,
        other => Some(other.to_string()),
    }
}

fn native_headers(extra: &[(&str, String)]) -> Vec<(String, String)> {
    let mut headers: Vec<(String, String)> = [
        ("User-Agent", NATIVE_API_USER_AGENT),
        ("X-Garmin-User-Agent", NATIVE_X_GARMIN_USER_AGENT),
        ("X-Garmin-Paired-App-Version", "10861"),
        ("X-Garmin-Client-Platform", "Android"),
        ("X-App-Ver", "10861"),
        ("X-Lang", "en"),
        ("X-GCExperience", "GC5"),
        ("Accept-Language", "en-US,en;q=0.9"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    for (name, value) in extra {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        headers.push((name.to_string(), value.clone()));
    }
    headers
}

fn basic_auth(client_id: &str) -> String {
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{client_id}:")))
}

/// Paths are built here, but a `..` segment, query or fragment smuggled in
/// through an id or display name must never reach the request.
fn check_path(path: &str) -> Result<(), GarminError> {
    let decoded = percent_encoding::percent_decode_str(path).decode_utf8_lossy();
    let traversal = decoded.split('/').any(|segment| segment.split(';').next() == Some(".."));
    if traversal || decoded.contains(['?', '#', '\\']) {
        return Err(GarminError::Invalid(format!("Invalid API path: {path:?}")));
    }
    Ok(())
}

/// `quote(value, safe="")`: everything but letters, digits and `_.-~`.
fn quote_segment(value: &str) -> String {
    const RESERVED: &percent_encoding::AsciiSet =
        &percent_encoding::NON_ALPHANUMERIC.remove(b'_').remove(b'.').remove(b'-').remove(b'~');
    percent_encoding::utf8_percent_encode(value, RESERVED).to_string()
}

/// Python's `repr()` of decoded JSON, since the library and the Python app
/// interpolate dicts and lists into their messages that way.
fn py_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap_or_default();
            if f.fract() == 0.0 && f.abs() < 1e16 { format!("{f:.1}") } else { f.to_string() }
        }
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
            let mut out = String::from(quote);
            for c in s.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if c == quote => {
                        out.push('\\');
                        out.push(c);
                    }
                    c => out.push(c),
                }
            }
            out.push(quote);
            out
        }
        Value::Array(items) => format!("[{}]", items.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Value::Object(map) => {
            format!("{{{}}}", map.iter().map(|(k, v)| format!("{}: {}", py_repr(&json!(k)), py_repr(v))).collect::<Vec<_>>().join(", "))
        }
    }
}

fn py_str(value: &Value) -> String {
    value.as_str().map_or_else(|| py_repr(value), str::to_string)
}

fn truthy(value: &Value) -> bool {
    crate::db::truthy(Some(value))
}

/// The library's error text: the status, then whatever message the body
/// offers. The upload and subcategory checks read this text.
fn api_error(response: &HttpResponse) -> GarminError {
    let status = response.status().as_u16();
    let mut message = format!("API Error {status}");
    let detail = serde_json::from_slice::<Value>(response.body()).ok().filter(Value::is_object).and_then(|data| {
        let first = |v: &Value| v.as_array().and_then(|a| a.first()).cloned();
        let nested = || {
            let failures = data.get("detailedImportResult").cloned().unwrap_or(json!({})).get("failures").cloned().unwrap_or(json!([{}]));
            let failure = first(&failures)?;
            first(&failure.get("messages").cloned().unwrap_or(json!([""])))
        };
        [data.get("message").cloned(), data.get("content").cloned()]
            .into_iter()
            .flatten()
            .find(truthy)
            .or_else(nested)
            .filter(truthy)
    });
    if let Some(detail) = detail {
        message.push_str(" - ");
        message.push_str(&py_str(&detail));
    }
    if status == 404 { GarminError::NotFound(message) } else { GarminError::Api(message) }
}

/// Garmin sometimes returns `internalId` as a quoted string; stored as is,
/// every later call with it 404s.
fn sanitize_activity_id(raw: Option<&Value>) -> Option<i64> {
    match raw? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().trim_matches(['\'', '"']).trim().parse().ok(),
        Value::Null => None,
        other => py_str(other).trim().trim_matches(['\'', '"']).trim().parse().ok(),
    }
}

fn is_subcategory_rejection(err: &GarminError) -> bool {
    let message = err.to_string().to_lowercase();
    message.contains("sub-category") || message.contains("subcategory") || message.contains("invalid sub")
}

fn positive_id(activity_id: i64) -> Result<i64, GarminError> {
    if activity_id > 0 { Ok(activity_id) } else { Err(GarminError::Invalid(format!("activity_id must be a positive integer, got {activity_id}"))) }
}

fn date_param(value: &str) -> Result<String, GarminError> {
    let value = value.trim();
    let shaped = value.len() == 10 && value.as_bytes()[4] == b'-' && value.as_bytes()[7] == b'-';
    if !shaped || NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err() {
        return Err(GarminError::Invalid(format!("date must be in format 'YYYY-MM-DD', got: {value}")));
    }
    Ok(value.to_string())
}

/// Kept as Garmin sent it, int or float, since the stored `garmin_data`
/// (and its content hash) must read exactly as the Python app wrote it.
fn number(act: &Value, key: &str) -> Option<Number> {
    act.get(key).and_then(Value::as_number).cloned()
}

fn record_of(act: &Value) -> Result<ActivityRecord, GarminError> {
    let id = act.get("activityId").and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())));
    let text = |key: &str| act.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    Ok(ActivityRecord {
        garmin_activity_id: id.ok_or_else(|| GarminError::Malformed("activity without activityId".into()))?,
        activity_type: act["activityType"]["typeKey"].as_str().unwrap_or("unknown").to_string(),
        title: text("activityName"),
        description: text("description"),
        start_time: text("startTimeGMT"),
        distance: number(act, "distance"),
        duration: number(act, "duration"),
        moving_duration: number(act, "movingDuration"),
        elapsed_duration: number(act, "elapsedDuration"),
        elevation_gain: number(act, "elevationGain"),
        elevation_loss: number(act, "elevationLoss"),
        calories: number(act, "calories"),
        avg_hr: number(act, "averageHR"),
        max_hr: number(act, "maxHR"),
        avg_power: number(act, "avgPower"),
        max_power: number(act, "maxPower"),
        norm_power: number(act, "normPower"),
        aerobic_te: number(act, "aerobicTrainingEffect"),
        anaerobic_te: number(act, "anaerobicTrainingEffect"),
        training_load: number(act, "activityTrainingLoad"),
        avg_cadence: number(act, "averageRunningCadenceInStepsPerMinute"),
        max_cadence: number(act, "maxRunningCadenceInStepsPerMinute"),
        total_sets: number(act, "totalSets"),
        total_reps: number(act, "totalReps"),
        total_volume: number(act, "totalVolume"),
    })
}

/// The first `.fit` member of Garmin's "original" download, in archive order.
pub fn fit_from_zip(zip_bytes: &[u8], activity_id: i64) -> Result<Vec<u8>, GarminError> {
    let malformed = |e: zip::result::ZipError| GarminError::Malformed(e.to_string());
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(malformed)?;
    for index in 0..archive.len() {
        let mut member = archive.by_index(index).map_err(malformed)?;
        if member.name().map_err(malformed)?.to_lowercase().ends_with(".fit") {
            let mut fit = Vec::new();
            member.read_to_end(&mut fit).map_err(|e| GarminError::Malformed(e.to_string()))?;
            return Ok(fit);
        }
    }
    Err(GarminError::Malformed(format!("no .fit file in original download for activity {activity_id}")))
}

pub fn activity_types_of(raw: &Value) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    let mut types: Vec<(String, String)> = Vec::new();
    for entry in raw.as_array().into_iter().flatten() {
        let Some(key) = entry.get("typeKey").and_then(Value::as_str).filter(|k| !k.is_empty()) else {
            continue;
        };
        if key == "all" || !seen.insert(key.to_string()) {
            continue;
        }
        types.push((key.to_string(), crate::mock::python_title(&key.replace('_', " "))));
    }
    types.sort_by(|a, b| a.1.cmp(&b.1));
    types.into_iter().map(|(key, label)| json!({"type_key": key, "label": label})).collect()
}

struct Request<'a> {
    method: http::Method,
    path: &'a str,
    query: &'a [(&'a str, String)],
    headers: Vec<(&'a str, String)>,
    body: Vec<u8>,
}

impl<'a> Request<'a> {
    fn get(path: &'a str) -> Self {
        Self { method: http::Method::GET, path, query: &[], headers: Vec::new(), body: Vec::new() }
    }

    fn with_query(self, query: &'a [(&'a str, String)]) -> Self {
        Self { query, ..self }
    }

    fn json(method: http::Method, path: &'a str, payload: &Value) -> Self {
        let headers = vec![("Content-Type", "application/json".to_string())];
        Self { method, path, query: &[], headers, body: net::json_body(payload) }
    }
}

#[derive(Clone)]
pub struct GarminClient {
    http: Http,
    clock: Clock,
    tokens: Arc<Mutex<Tokens>>,
    refreshing: Arc<tokio::sync::Mutex<()>>,
    token_path: Option<String>,
    display_name: Option<String>,
    pause: Duration,
}

impl GarminClient {
    /// A client over tokens already in hand; [`GarminClient::open`] is the
    /// normal way in.
    pub fn new(http: Http, clock: Clock, tokens: Tokens, token_path: Option<&str>) -> Self {
        Self {
            http,
            clock,
            tokens: Arc::new(Mutex::new(tokens)),
            refreshing: Arc::default(),
            token_path: token_path.map(str::to_string),
            display_name: None,
            pause: Duration::from_secs(1),
        }
    }

    /// No pause after calls or between profile retries, for tests.
    pub fn without_waits(self) -> Self {
        Self { pause: Duration::ZERO, ..self }
    }

    /// Opens the saved session without ever starting a password login, so
    /// the unattended poller cannot trigger an MFA challenge it cannot
    /// answer. Garmin must accept the token for the profile reads.
    pub async fn open(http: Http, clock: Clock, token_dir: &str) -> Result<Self, GarminError> {
        Self::open_with(http, clock, token_dir, Duration::from_secs(1)).await
    }

    pub async fn open_with(http: Http, clock: Clock, token_dir: &str, pause: Duration) -> Result<Self, GarminError> {
        let tokens = load_tokens(token_dir).ok_or(GarminError::SessionExpired)?;
        let mut client = Self { pause, ..Self::new(http, clock, tokens, Some(token_dir)) };
        let proactive = {
            let tokens = client.tokens();
            tokens.di_refresh_token.is_some() && tokens.di_token.as_deref().is_some_and(|t| expires_soon(t, (client.clock)()))
        };
        if proactive {
            client.refresh_session().await;
        }
        client.load_profile_and_settings().await?;
        Ok(client)
    }

    pub fn tokens(&self) -> Tokens {
        self.tokens.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_micros(((self.clock)() * 1_000_000.0) as i64).unwrap_or_default()
    }

    async fn rest(&self) {
        if !self.pause.is_zero() {
            tokio::time::sleep(self.pause).await;
        }
    }

    /// The social profile names the account in URLs; a profile without a
    /// display name is retried, then accepted without one.
    async fn load_profile_and_settings(&mut self) -> Result<(), GarminError> {
        let mut profile = None;
        let mut name = None;
        for attempt in 0..3 {
            match self.get_json(SOCIAL_PROFILE_PATH, &[]).await {
                Ok(data) => {
                    name = data.get("displayName").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).map(str::to_string);
                    profile = Some(data);
                    if name.is_some() {
                        break;
                    }
                }
                Err(_) if attempt == 2 => return Err(GarminError::SessionExpired),
                Err(_) => {}
            }
            if attempt < 2 {
                self.rest().await;
            }
        }
        if !profile.is_some_and(|p| p.is_object()) {
            return Err(GarminError::SessionExpired);
        }
        self.display_name = name;
        for attempt in 0..3 {
            match self.get_json(USER_SETTINGS_PATH, &[]).await {
                Ok(settings) if settings.get("userData").is_some() => return Ok(()),
                Ok(_) => {}
                Err(_) if attempt == 2 => return Err(GarminError::SessionExpired),
                Err(_) => self.rest().await,
            }
        }
        Err(GarminError::SessionExpired)
    }

    async fn refresh_di_token(&self) -> Result<(), GarminError> {
        let tokens = self.tokens();
        let (Some(refresh), Some(client_id)) = (tokens.di_refresh_token.clone(), tokens.di_client_id.clone()) else {
            return Err(GarminError::SessionExpired);
        };
        let headers = native_headers(&[
            ("Authorization", basic_auth(&client_id)),
            ("Accept", "application/json".into()),
            ("Content-Type", "application/x-www-form-urlencoded".into()),
            ("Cache-Control", "no-cache".into()),
        ]);
        let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let body = net::form(&[("grant_type", "refresh_token"), ("client_id", &client_id), ("refresh_token", &refresh)]);
        let request = net::request(http::Method::POST, DI_TOKEN_URL, &headers, body).map_err(|e| GarminError::Network(e.0))?;
        let response = (self.http)(request).await.map_err(|e| GarminError::Network(e.0))?;
        if !(200..300).contains(&response.status().as_u16()) {
            return Err(GarminError::Api(format!("DI token refresh failed: {}", response.status().as_u16())));
        }
        let data: Value = serde_json::from_slice(response.body()).map_err(|e| GarminError::Malformed(e.to_string()))?;
        let access = data.get("access_token").and_then(Value::as_str).ok_or_else(|| GarminError::Malformed("no access_token".into()))?;
        let refreshed = Tokens {
            di_token: Some(access.to_string()),
            di_refresh_token: data.get("refresh_token").and_then(Value::as_str).map(str::to_string).or(tokens.di_refresh_token),
            di_client_id: client_id_of(access).or(Some(client_id)),
        };
        *self.tokens.lock().unwrap_or_else(|e| e.into_inner()) = refreshed;
        Ok(())
    }

    /// A failed refresh is not an error here: the call goes ahead with the
    /// old token and Garmin's answer decides.
    async fn refresh_session(&self) {
        let _guard = self.refreshing.lock().await;
        if self.tokens().di_token.is_none() {
            return;
        }
        match self.refresh_di_token().await {
            Ok(()) => {
                if let Some(path) = &self.token_path {
                    let _ = save_tokens(path, &self.tokens());
                }
            }
            Err(err) => eprintln!("garmin token refresh failed: {err}"),
        }
    }

    async fn send(&self, request: &Request<'_>) -> Result<HttpResponse, GarminError> {
        let token = self.tokens().di_token.ok_or(GarminError::SessionExpired)?;
        let mut extra = vec![("Authorization", format!("Bearer {token}")), ("Accept", "application/json".to_string())];
        extra.extend(request.headers.iter().cloned());
        let headers = native_headers(&extra);
        let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let query: Vec<(&str, &str)> = request.query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let url = net::with_query(&format!("{CONNECTAPI_URL}/{}", request.path.trim_start_matches('/')), &query);
        let outgoing = net::request(request.method.clone(), &url, &headers, request.body.clone()).map_err(|e| GarminError::Network(e.0))?;
        (self.http)(outgoing).await.map_err(|e| GarminError::Network(e.0))
    }

    async fn run(&self, request: Request<'_>) -> Result<Vec<u8>, GarminError> {
        let soon = self.tokens().di_token.as_deref().is_some_and(|t| expires_soon(t, (self.clock)()));
        if soon {
            self.refresh_session().await;
        }
        check_path(request.path)?;
        let mut response = self.send(&request).await?;
        if response.status().as_u16() == 401 {
            self.refresh_session().await;
            response = self.send(&request).await?;
        }
        match response.status().as_u16() {
            204 => Ok(b"{}".to_vec()),
            status if status >= 400 => Err(api_error(&response)),
            _ => Ok(response.into_body()),
        }
    }

    async fn run_json(&self, request: Request<'_>) -> Result<Value, GarminError> {
        let body = self.run(request).await?;
        serde_json::from_slice(&body).map_err(|e| GarminError::Malformed(e.to_string()))
    }

    async fn get_json(&self, path: &str, query: &[(&str, String)]) -> Result<Value, GarminError> {
        self.run_json(Request::get(path).with_query(query)).await
    }

    /// Every call the app makes is followed by a pause, as the Python app's
    /// rate limiter does, since Garmin throttles hard.
    async fn paced<T>(&self, result: Result<T, GarminError>) -> Result<T, GarminError> {
        if result.is_ok() {
            self.rest().await;
        }
        result
    }

    /// A definitive 404 on an activity becomes `ActivityGone`; an outage
    /// never does, whatever its text says.
    fn gone(activity_id: i64) -> impl Fn(GarminError) -> GarminError {
        move |err| if matches!(err, GarminError::NotFound(_)) { GarminError::ActivityGone(activity_id) } else { err }
    }

    /// Newest first, 20 a page, until one older than the lookback.
    pub async fn fetch_recent_activities(&self, lookback_days: i64) -> Result<Vec<ActivityRecord>, GarminError> {
        let cutoff = self.now() - chrono::Duration::days(lookback_days);
        let mut records = Vec::new();
        let mut start = 0;
        loop {
            let query = [("start", start.to_string()), ("limit", PAGE_SIZE.to_string())];
            let batch = self.paced(self.get_json(ACTIVITIES_PATH, &query).await).await?;
            let Some(batch) = batch.as_array().filter(|b| !b.is_empty()) else {
                break;
            };
            let mut reached_cutoff = false;
            for act in batch {
                let started = act.get("startTimeGMT").and_then(Value::as_str).and_then(crate::timefmt::parse_timestamp);
                if started.is_some_and(|s| s < cutoff) {
                    reached_cutoff = true;
                    break;
                }
                records.push(record_of(act)?);
            }
            if reached_cutoff || batch.len() < PAGE_SIZE {
                break;
            }
            start += PAGE_SIZE;
        }
        Ok(records)
    }

    pub async fn download_fit(&self, garmin_activity_id: i64) -> Result<Vec<u8>, GarminError> {
        let path = format!("{FIT_DOWNLOAD_PATH}/{}", positive_id(garmin_activity_id)?);
        let request = Request { headers: vec![("Accept", "*/*".into())], ..Request::get(&path) };
        let zip_bytes = self.paced(self.run(request).await).await?;
        fit_from_zip(&zip_bytes, garmin_activity_id)
    }

    async fn put_activity(&self, activity_id: i64, payload: Value) -> Result<(), GarminError> {
        let path = format!("{ACTIVITY_PATH}/{activity_id}");
        self.paced(self.run_json(Request::json(http::Method::PUT, &path, &payload)).await.map(drop)).await
    }

    pub async fn update_activity_metadata(&self, garmin_activity_id: i64, title: &str, description: &str) -> Result<(), GarminError> {
        let id = positive_id(garmin_activity_id)?;
        self.put_activity(id, json!({"activityId": id.to_string(), "activityName": title})).await?;
        self.put_activity(id, json!({"activityId": id.to_string(), "description": description})).await
    }

    pub async fn set_title(&self, activity_id: i64, title: &str) -> Result<(), GarminError> {
        let id = positive_id(activity_id)?;
        self.put_activity(id, json!({"activityId": id.to_string(), "activityName": title})).await.map_err(Self::gone(id))
    }

    pub async fn set_description(&self, activity_id: i64, description: &str) -> Result<(), GarminError> {
        let id = positive_id(activity_id)?;
        self.put_activity(id, json!({"activityId": id.to_string(), "description": description})).await.map_err(Self::gone(id))
    }

    /// Uploads a FIT (or GPX/TCX) file. A 409 or "duplicate activity", or
    /// failures with no success, is a definite non-import.
    pub async fn upload_fit(&self, fit_path: &Path) -> Result<UploadResult, GarminError> {
        let data = std::fs::read(fit_path).map_err(|_| GarminError::Invalid(format!("FIT file not found: {}", fit_path.display())))?;
        let filename = fit_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let extension = filename.rsplit_once('.').map(|(_, ext)| ext.to_uppercase());
        if !extension.as_deref().is_some_and(|ext| UPLOAD_FORMATS.contains(&ext)) {
            return Err(GarminError::Invalid(format!("Invalid file format for upload: {filename}")));
        }
        let (content_type, body) = net::multipart(&[Part { name: "file", filename: Some(&filename), content_type: None, data: &data }]);
        let request = Request { method: http::Method::POST, headers: vec![("Content-Type", content_type)], body, ..Request::get(UPLOAD_PATH) };
        let response = match self.paced(self.run_json(request).await).await {
            Ok(response) => response,
            Err(err) => {
                let duplicate = err.to_string().to_lowercase().contains("duplicate activity");
                if duplicate || err.to_string().starts_with("API Error 409") {
                    return Err(GarminError::UploadRejected(format!("Garmin rejected upload: {err}")));
                }
                return Err(err);
            }
        };
        let detail = response.get("detailedImportResult").cloned().unwrap_or(json!({}));
        let upload_id = detail.get("uploadId").cloned().filter(|v| !v.is_null());
        let successes = detail.get("successes").cloned().unwrap_or(json!([]));
        let activity_id = successes.as_array().and_then(|s| s.first()).and_then(|first| sanitize_activity_id(first.get("internalId")));
        let failures = detail.get("failures").cloned().unwrap_or(json!([]));
        if truthy(&failures) && activity_id.is_none() && !truthy(&successes) {
            return Err(GarminError::UploadRejected(format!("Garmin rejected upload: {}", py_repr(&failures))));
        }
        eprintln!("fit upload: upload_id={upload_id:?} activity_id={activity_id:?}");
        Ok(UploadResult { upload_id, activity_id })
    }

    pub async fn get_exercise_sets(&self, activity_id: i64) -> Result<Value, GarminError> {
        let path = format!("{ACTIVITY_PATH}/{}/exerciseSets", positive_id(activity_id)?);
        self.paced(self.get_json(&path, &[]).await).await.map_err(Self::gone(activity_id))
    }

    /// Replaces every exercise set on the activity at once.
    pub async fn put_exercise_sets(&self, activity_id: i64, payload: &Value) -> Result<(), GarminError> {
        let path = format!("{ACTIVITY_PATH}/{}/exerciseSets", positive_id(activity_id)?);
        let result = self.run_json(Request::json(http::Method::PUT, &path, payload)).await.map(drop);
        self.paced(result).await.map_err(|err| {
            if is_subcategory_rejection(&err) {
                GarminError::SubcategoryRejected(err.to_string())
            } else {
                Self::gone(activity_id)(err)
            }
        })
    }

    pub async fn delete_activity(&self, activity_id: i64) -> Result<(), GarminError> {
        let path = format!("{ACTIVITY_PATH}/{}", positive_id(activity_id)?);
        let request = Request { method: http::Method::DELETE, ..Request::get(&path) };
        self.paced(self.run(request).await.map(drop)).await?;
        eprintln!("deleted garmin activity {activity_id}");
        Ok(())
    }

    pub async fn get_daily_heart_rates(&self, date: &str) -> Result<Value, GarminError> {
        let date = date_param(date)?;
        let name = self.display_name.as_deref().ok_or_else(|| {
            GarminError::Api("Display name is not set. Your Garmin profile did not include a display name, so this request cannot be made.".into())
        })?;
        let path = format!("{DAILY_HEART_RATE_PATH}/{}", quote_segment(name));
        let data = self.paced(self.get_json(&path, &[("date", date)]).await).await?;
        if data.is_null() {
            return Err(GarminError::Api("No heart rate data received".into()));
        }
        Ok(data)
    }

    /// Physiology for calorie estimates. A missing piece comes back as
    /// `None` for the profile defaults to fill; a failed read is not fatal.
    pub async fn fetch_user_profile(&self) -> UserProfile {
        let user_data = match self.paced(self.get_json(USER_SETTINGS_PATH, &[]).await).await {
            Ok(settings) => settings.get("userData").filter(|d| truthy(d)).cloned().unwrap_or(json!({})),
            Err(err) => {
                eprintln!("user profile fetch failed: {err}");
                json!({})
            }
        };
        let weight = user_data.get("weight").filter(|w| truthy(w)).and_then(|w| w.as_f64().or_else(|| w.as_str()?.trim().parse().ok()));
        let weight_kg = weight.and_then(|w| format!("{:.1}", w / 1000.0).parse().ok());
        let birth = user_data.get("birthDate").filter(|b| truthy(b)).map(py_str);
        let birth_year = birth.and_then(|b| b.chars().take(4).collect::<String>().trim().parse().ok());
        let sex = user_data.get("gender").filter(|g| truthy(g)).map(|g| py_str(g).to_lowercase());

        let today = self.now().format("%Y-%m-%d").to_string();
        let path = format!("{MAX_METRICS_PATH}/{today}/{today}");
        let mut vo2max = None;
        if let Ok(metrics) = self.paced(self.get_json(&path, &[]).await).await {
            let entries = if metrics.is_array() { metrics } else { json!([metrics]) };
            for entry in entries.as_array().into_iter().flatten() {
                let generic = entry.get("generic").filter(|g| truthy(g)).cloned().unwrap_or(json!({}));
                let value = [generic.get("vo2MaxPreciseValue"), generic.get("vo2MaxValue")].into_iter().flatten().find(|v| truthy(v));
                if let Some(value) = value {
                    vo2max = value.as_f64().or_else(|| value.as_str()?.trim().parse().ok());
                    break;
                }
            }
        }
        UserProfile { weight_kg, birth_year, sex, vo2max }
    }

    /// Every activity of any type from the day before `start_time`'s date to
    /// the day after. A failure is an error, never an empty list: the
    /// operation journal diffs these snapshots.
    pub async fn list_activities_near(&self, start_time: &str) -> Result<Vec<Value>, GarminError> {
        let target = crate::timefmt::parse_timestamp(start_time)
            .ok_or_else(|| GarminError::Invalid(format!("unparseable start_time: {start_time:?}")))?;
        let date_from = (target - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
        let date_to = (target + chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
        let mut activities = Vec::new();
        let mut start = 0;
        for _ in 0..MAX_PAGINATED_REQUESTS {
            let query = [
                ("startDate", date_from.clone()),
                ("start", start.to_string()),
                ("limit", PAGE_SIZE.to_string()),
                ("endDate", date_to.clone()),
            ];
            let page = self.get_json(ACTIVITIES_PATH, &query).await?;
            if !truthy(&page) {
                return self.paced(Ok(activities)).await;
            }
            let page = page.as_array().ok_or_else(|| GarminError::Malformed("activities is not a list".into()))?;
            activities.extend(page.iter().cloned());
            start += PAGE_SIZE;
        }
        Err(GarminError::Api(format!("Pagination exceeded {MAX_PAGINATED_REQUESTS} requests; aborting")))
    }

    pub async fn list_activity_ids_near(&self, start_time: &str) -> Result<Vec<i64>, GarminError> {
        let activities = self.list_activities_near(start_time).await?;
        activities
            .iter()
            .filter_map(|act| act.get("activityId").filter(|id| !id.is_null()))
            .map(|id| {
                id.as_i64()
                    .or_else(|| id.as_str().and_then(|s| s.trim().parse().ok()))
                    .ok_or_else(|| GarminError::Malformed(format!("activity id {id} is not a number")))
            })
            .collect()
    }

    /// Garmin's type taxonomy for the autosync checklist, without Garmin's
    /// own "all" pseudo-type.
    pub async fn fetch_activity_types(&self) -> Result<Vec<Value>, GarminError> {
        Ok(activity_types_of(&self.get_json(ACTIVITY_TYPES_PATH, &[]).await?))
    }
}
