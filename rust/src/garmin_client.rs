mod activity;
mod tokens;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, NaiveDate, Utc};
use openidconnect::http;
use serde_json::{Value, json};

pub use self::activity::{ActivityRecord, UploadResult, UserProfile};
use self::activity::{
    activity_id_of, activity_types_of, fit_from_zip, record_of, sanitize_activity_id,
};
use self::tokens::load_tokens;
pub use self::tokens::{Tokens, save_tokens, token_file_path};
pub(crate) use self::tokens::{client_id_of, expires_soon};
pub use crate::garmin_errors::GarminError;
use crate::garmin_errors::{py_repr, py_str, truthy};
use crate::garmin_retry::{Request, Waits};
use crate::net::{self, Http, Part};
use crate::schedule::Clock;

pub(crate) const CONNECTAPI_URL: &str = "https://connectapi.garmin.com";
pub(crate) const DI_TOKEN_URL: &str = "https://diauth.garmin.com/di-oauth2-service/oauth/token";
pub(crate) const SOCIAL_PROFILE_PATH: &str = "/userprofile-service/socialProfile";

const NATIVE_API_USER_AGENT: &str = "GCM-Android-5.23";
const NATIVE_X_GARMIN_USER_AGENT: &str = "com.garmin.android.apps.connectmobile/5.23; ; Google/sdk_gphone64_arm64/google; Android/33; Dalvik/2.1.0";
const PAGE_SIZE: usize = 20;
const MAX_PAGINATED_REQUESTS: usize = 2000;
const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);
const CALL_PAUSE: Duration = Duration::from_secs(1);

const ACTIVITIES_PATH: &str = "/activitylist-service/activities/search/activities";
const ACTIVITY_PATH: &str = "/activity-service/activity";
const FIT_DOWNLOAD_PATH: &str = "/download-service/files/activity";
const UPLOAD_PATH: &str = "/upload-service/upload";
const USER_SETTINGS_PATH: &str = "/userprofile-service/userprofile/user-settings";
const MAX_METRICS_PATH: &str = "/metrics-service/metrics/maxmet/daily";
const DAILY_HEART_RATE_PATH: &str = "/wellness-service/wellness/dailyHeartRate";
const ACTIVITY_TYPES_PATH: &str = "/activity-service/activity/activityTypes";
const DOWNLOAD_LABEL: &str = "Download";
const UPLOAD_FORMATS: [&str; 3] = ["FIT", "GPX", "TCX"];

pub(crate) fn native_headers<'a>(
    extra: impl IntoIterator<Item = (&'a str, String)>,
) -> Vec<(&'a str, String)> {
    let mut headers: Vec<(&str, String)> = [
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
    .map(|(k, v)| (k, v.to_string()))
    .collect();
    for (name, value) in extra {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        headers.push((name, value));
    }
    headers
}

pub(crate) fn basic_auth(client_id: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{client_id}:"))
    )
}

/// Paths are built here, but a `..` segment, query or fragment smuggled in
/// through an id or display name must never reach the request.
pub(crate) fn check_path(path: &str) -> Result<(), GarminError> {
    let decoded = percent_encoding::percent_decode_str(path).decode_utf8_lossy();
    let traversal = decoded
        .split('/')
        .any(|segment| segment.split(';').next() == Some(".."));
    if traversal || decoded.contains(['?', '#', '\\']) {
        return Err(GarminError::Invalid(format!("Invalid API path: {path:?}")));
    }
    Ok(())
}

fn is_subcategory_rejection(err: &GarminError) -> bool {
    let message = err.to_string().to_lowercase();
    message.contains("sub-category")
        || message.contains("subcategory")
        || message.contains("invalid sub")
}

fn positive_id(activity_id: i64) -> Result<i64, GarminError> {
    if activity_id > 0 {
        Ok(activity_id)
    } else {
        Err(GarminError::Invalid(format!(
            "activity_id must be a positive integer, got {activity_id}"
        )))
    }
}

fn date_param(value: &str) -> Result<String, GarminError> {
    let value = value.trim();
    let shaped = value.len() == 10 && value.as_bytes()[4] == b'-' && value.as_bytes()[7] == b'-';
    if !shaped || NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err() {
        return Err(GarminError::Invalid(format!(
            "date must be in format 'YYYY-MM-DD', got: {value}"
        )));
    }
    Ok(value.to_string())
}

/// A number, or a number in a string, as Garmin sends profile values.
fn float_of(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

#[derive(Clone)]
pub struct GarminClient {
    pub(crate) http: Http,
    pub(crate) clock: Clock,
    tokens: Arc<Mutex<Tokens>>,
    refreshing: Arc<tokio::sync::Mutex<()>>,
    token_path: Option<String>,
    display_name: Option<String>,
    pub(crate) waits: Waits,
}

impl GarminClient {
    pub fn new(http: Http, clock: Clock, tokens: Tokens, token_path: Option<&str>) -> Self {
        Self {
            http,
            clock,
            tokens: Arc::new(Mutex::new(tokens)),
            refreshing: Arc::default(),
            token_path: token_path.map(str::to_string),
            display_name: None,
            waits: Waits::from_pause(CALL_PAUSE),
        }
    }

    pub fn with_display_name(self, name: &str) -> Self {
        Self {
            display_name: Some(name.to_string()),
            ..self
        }
    }

    pub fn without_waits(self) -> Self {
        Self {
            waits: Waits::from_pause(Duration::ZERO),
            ..self
        }
    }

    /// `rate_limit` is the unit of the escalating rate-limit wait, not the whole wait.
    pub fn with_waits(self, pause: Duration, rate_limit: Duration, backoff: Duration) -> Self {
        Self {
            waits: Waits {
                pause,
                rate_limit,
                backoff,
            },
            ..self
        }
    }

    /// Never starts a password login, so the unattended poller cannot trigger
    /// an MFA challenge it cannot answer.
    pub async fn open(http: Http, clock: Clock, token_dir: &str) -> Result<Self, GarminError> {
        Self::open_with(http, clock, token_dir, CALL_PAUSE).await
    }

    /// A zero `pause` also turns off the rate-limit and retry waits.
    pub async fn open_with(
        http: Http,
        clock: Clock,
        token_dir: &str,
        pause: Duration,
    ) -> Result<Self, GarminError> {
        let tokens = load_tokens(token_dir).ok_or(GarminError::SessionExpired)?;
        let proactive = tokens.di_refresh_token.is_some()
            && tokens
                .di_token
                .as_deref()
                .is_some_and(|t| expires_soon(t, clock()));
        let mut client = Self {
            waits: Waits::from_pause(pause),
            ..Self::new(http, clock, tokens, Some(token_dir))
        };
        if proactive {
            client.refresh_session().await;
        }
        client.load_profile_and_settings().await?;
        Ok(client)
    }

    pub(crate) async fn signed_in(
        http: Http,
        clock: Clock,
        tokens: Tokens,
        pause: Duration,
    ) -> Result<Self, GarminError> {
        let mut client = Self {
            waits: Waits::from_pause(pause),
            ..Self::new(http, clock, tokens, None)
        };
        client.load_profile_and_settings().await?;
        Ok(client)
    }

    pub fn tokens(&self) -> Tokens {
        self.tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    fn now(&self) -> DateTime<Utc> {
        crate::timefmt::from_epoch((self.clock)())
    }

    pub(crate) async fn rest(&self) {
        if !self.waits.pause.is_zero() {
            tokio::time::sleep(self.waits.pause).await;
        }
    }

    async fn load_profile_and_settings(&mut self) -> Result<(), GarminError> {
        let mut profile = None;
        let mut name = None;
        for attempt in 0..3 {
            match self.raw_json(SOCIAL_PROFILE_PATH, &[]).await {
                Ok(data) => {
                    name = data
                        .get("displayName")
                        .and_then(Value::as_str)
                        .filter(|n| !n.trim().is_empty())
                        .map(str::to_string);
                    profile = Some(data);
                    if name.is_some() {
                        break;
                    }
                }
                Err(err) if attempt == 2 => return Err(Self::open_failure(&err)),
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
            match self.raw_json(USER_SETTINGS_PATH, &[]).await {
                Ok(settings) if settings.get("userData").is_some() => return Ok(()),
                Ok(_) => {}
                Err(err) if attempt == 2 => return Err(Self::open_failure(&err)),
                Err(_) => self.rest().await,
            }
        }
        Err(GarminError::SessionExpired)
    }

    /// Unlike the Python library, which calls every failed profile read at
    /// sign-in an auth error, an outage stays an outage.
    fn open_failure(err: &GarminError) -> GarminError {
        if err.is_transient() {
            GarminError::Unavailable
        } else {
            GarminError::SessionExpired
        }
    }

    async fn refresh_di_token(&self) -> Result<(), GarminError> {
        let Tokens {
            di_refresh_token: Some(refresh),
            di_client_id: Some(client_id),
            ..
        } = self.tokens()
        else {
            return Err(GarminError::SessionExpired);
        };
        let headers = native_headers([
            ("Authorization", basic_auth(&client_id)),
            ("Accept", "application/json".into()),
            ("Content-Type", "application/x-www-form-urlencoded".into()),
            ("Cache-Control", "no-cache".into()),
        ]);
        let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let body = net::form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &client_id),
            ("refresh_token", &refresh),
        ]);
        let request = net::request(http::Method::POST, DI_TOKEN_URL, &headers, body)?;
        let response = (self.http)(net::with_timeout(request, TOKEN_TIMEOUT)).await?;
        if !response.status().is_success() {
            return Err(GarminError::Api(format!(
                "DI token refresh failed: {}",
                response.status().as_u16()
            )));
        }
        let data: Value = serde_json::from_slice(response.body())?;
        let access = data
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| GarminError::Malformed("no access_token".into()))?;
        let refreshed = Tokens {
            di_token: Some(access.to_string()),
            di_refresh_token: data
                .get("refresh_token")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(Some(refresh)),
            di_client_id: client_id_of(access).or(Some(client_id)),
        };
        *self.tokens.lock().unwrap_or_else(|e| e.into_inner()) = refreshed;
        Ok(())
    }

    /// A failed refresh is not an error here: the call goes ahead with the
    /// old token and Garmin's answer decides.
    pub(crate) async fn refresh_session(&self) {
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
            Err(err) => crate::log_warn!("garmin_client", "garmin token refresh failed: {err}"),
        }
    }

    /// A definitive 404 on an activity becomes `ActivityGone`; an outage
    /// never does, whatever its text says.
    fn gone(activity_id: i64) -> impl Fn(GarminError) -> GarminError {
        move |err| {
            if matches!(err, GarminError::NotFound(_)) {
                GarminError::ActivityGone(activity_id)
            } else {
                err
            }
        }
    }

    pub async fn fetch_recent_activities(
        &self,
        lookback_days: i64,
    ) -> Result<Vec<ActivityRecord>, GarminError> {
        let cutoff = self.now() - chrono::Duration::days(lookback_days);
        let mut records = Vec::new();
        let mut start = 0;
        loop {
            let query = [
                ("start", start.to_string()),
                ("limit", PAGE_SIZE.to_string()),
            ];
            let batch = self
                .limited(|| self.get_json(ACTIVITIES_PATH, &query))
                .await?;
            let Some(batch) = batch.as_array().filter(|b| !b.is_empty()) else {
                break;
            };
            let mut reached_cutoff = false;
            for act in batch {
                let started = act
                    .get("startTimeGMT")
                    .and_then(Value::as_str)
                    .and_then(crate::timefmt::parse_timestamp);
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
        let request = Request {
            headers: vec![("Accept", "*/*".into())],
            ..Request::get(&path)
        };
        let zip_bytes = self.limited(|| self.call(DOWNLOAD_LABEL, &request)).await?;
        fit_from_zip(&zip_bytes, garmin_activity_id)
    }

    async fn put_activity(&self, activity_id: i64, payload: Value) -> Result<(), GarminError> {
        let path = format!("{ACTIVITY_PATH}/{activity_id}");
        self.paced(
            self.run_json(&Request::json(http::Method::PUT, &path, &payload))
                .await
                .map(drop),
        )
        .await
    }

    async fn put_title(&self, id: i64, title: &str) -> Result<(), GarminError> {
        self.put_activity(
            id,
            json!({"activityId": id.to_string(), "activityName": title}),
        )
        .await
    }

    async fn put_description(&self, id: i64, description: &str) -> Result<(), GarminError> {
        self.put_activity(
            id,
            json!({"activityId": id.to_string(), "description": description}),
        )
        .await
    }

    pub async fn update_activity_metadata(
        &self,
        garmin_activity_id: i64,
        title: &str,
        description: &str,
    ) -> Result<(), GarminError> {
        let id = positive_id(garmin_activity_id)?;
        self.put_title(id, title).await?;
        self.put_description(id, description).await
    }

    pub async fn set_title(&self, activity_id: i64, title: &str) -> Result<(), GarminError> {
        let id = positive_id(activity_id)?;
        self.put_title(id, title).await.map_err(Self::gone(id))
    }

    pub async fn set_description(
        &self,
        activity_id: i64,
        description: &str,
    ) -> Result<(), GarminError> {
        let id = positive_id(activity_id)?;
        self.put_description(id, description)
            .await
            .map_err(Self::gone(id))
    }

    /// A 409, "duplicate activity", or failures with no success is a definite
    /// non-import.
    pub async fn upload_fit(&self, fit_path: &Path) -> Result<UploadResult, GarminError> {
        let data = std::fs::read(fit_path).map_err(|_| {
            GarminError::Invalid(format!("FIT file not found: {}", fit_path.display()))
        })?;
        let filename = fit_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let extension = filename.rsplit_once('.').map(|(_, ext)| ext.to_uppercase());
        if !extension
            .as_deref()
            .is_some_and(|ext| UPLOAD_FORMATS.contains(&ext))
        {
            return Err(GarminError::Invalid(format!(
                "Invalid file format for upload: {filename}"
            )));
        }
        let (content_type, body) = net::multipart(&[Part {
            name: "file",
            filename: Some(&filename),
            content_type: None,
            data: &data,
        }]);
        let request = Request {
            method: http::Method::POST,
            headers: vec![("Content-Type", content_type)],
            body,
            ..Request::get(UPLOAD_PATH)
        };
        let response = self
            .paced(self.run_json(&request).await)
            .await
            .map_err(|err| {
                let text = err.to_string();
                if text.starts_with("API Error 409")
                    || text.to_lowercase().contains("duplicate activity")
                {
                    GarminError::UploadRejected(format!("Garmin rejected upload: {text}"))
                } else {
                    err
                }
            })?;
        let detail = &response["detailedImportResult"];
        let upload_id = detail.get("uploadId").filter(|v| !v.is_null()).cloned();
        let successes = &detail["successes"];
        let activity_id = successes
            .as_array()
            .and_then(|s| s.first())
            .and_then(|first| sanitize_activity_id(first.get("internalId")));
        let failures = &detail["failures"];
        if truthy(failures) && activity_id.is_none() && !truthy(successes) {
            return Err(GarminError::UploadRejected(format!(
                "Garmin rejected upload: {}",
                py_repr(failures)
            )));
        }
        crate::log_info!(
            "garmin_client",
            "fit upload: upload_id={upload_id:?} activity_id={activity_id:?}"
        );
        Ok(UploadResult {
            upload_id,
            activity_id,
        })
    }

    pub async fn get_exercise_sets(&self, activity_id: i64) -> Result<Value, GarminError> {
        let path = format!("{ACTIVITY_PATH}/{}/exerciseSets", positive_id(activity_id)?);
        self.limited(|| self.get_json(&path, &[]))
            .await
            .map_err(Self::gone(activity_id))
    }

    /// Replaces every exercise set on the activity at once.
    pub async fn put_exercise_sets(
        &self,
        activity_id: i64,
        payload: &Value,
    ) -> Result<(), GarminError> {
        let path = format!("{ACTIVITY_PATH}/{}/exerciseSets", positive_id(activity_id)?);
        let result = self
            .run_json(&Request::json(http::Method::PUT, &path, payload))
            .await
            .map(drop);
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
        let request = Request {
            method: http::Method::DELETE,
            ..Request::get(&path)
        };
        self.paced(self.run(&request).await.map(drop)).await?;
        crate::log_info!("garmin_client", "deleted garmin activity {activity_id}");
        Ok(())
    }

    pub async fn get_daily_heart_rates(&self, date: &str) -> Result<Value, GarminError> {
        let date = date_param(date)?;
        let name = self.display_name.as_deref().ok_or_else(|| {
            GarminError::Api("Display name is not set. Your Garmin profile did not include a display name, so this request cannot be made.".into())
        })?;
        let path = format!("{DAILY_HEART_RATE_PATH}/{}", crate::auth_guard::quote(name));
        let query = [("date", date)];
        let data = self.limited(|| self.get_json(&path, &query)).await?;
        if data.is_null() {
            return Err(GarminError::Api("No heart rate data received".into()));
        }
        Ok(data)
    }

    pub async fn fetch_user_profile(&self) -> UserProfile {
        let settings = self
            .limited(|| self.get_json(USER_SETTINGS_PATH, &[]))
            .await
            .unwrap_or_else(|err| {
                crate::log_warn!("garmin_client", "user profile fetch failed: {err}");
                Value::Null
            });
        let user_data = &settings["userData"];
        let weight = user_data
            .get("weight")
            .filter(|w| truthy(w))
            .and_then(float_of);
        let weight_kg = weight.and_then(|w| format!("{:.1}", w / 1000.0).parse().ok());
        let birth = user_data.get("birthDate").filter(|b| truthy(b)).map(py_str);
        let birth_year =
            birth.and_then(|b| b.chars().take(4).collect::<String>().trim().parse().ok());
        let sex = user_data
            .get("gender")
            .filter(|g| truthy(g))
            .map(|g| py_str(g).to_lowercase());

        let today = self.now().format("%Y-%m-%d").to_string();
        let path = format!("{MAX_METRICS_PATH}/{today}/{today}");
        let metrics = self.limited(|| self.get_json(&path, &[])).await;
        let entries = match &metrics {
            Ok(Value::Array(entries)) => entries.as_slice(),
            Ok(entry) => std::slice::from_ref(entry),
            Err(_) => &[],
        };
        let vo2max = entries
            .iter()
            .find_map(|entry| {
                let generic = entry.get("generic")?;
                [
                    generic.get("vo2MaxPreciseValue"),
                    generic.get("vo2MaxValue"),
                ]
                .into_iter()
                .flatten()
                .find(|v| truthy(v))
            })
            .and_then(float_of);
        UserProfile {
            weight_kg,
            birth_year,
            sex,
            vo2max,
        }
    }

    /// A failure is an error, never an empty list: the operation journal diffs
    /// these snapshots.
    pub async fn list_activities_near(&self, start_time: &str) -> Result<Vec<Value>, GarminError> {
        let target = crate::timefmt::parse_timestamp(start_time).ok_or_else(|| {
            GarminError::Invalid(format!("unparseable start_time: {start_time:?}"))
        })?;
        let date_from = (target - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let date_to = (target + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        self.limited(|| self.activities_between(&date_from, &date_to))
            .await
    }

    async fn activities_between(
        &self,
        date_from: &str,
        date_to: &str,
    ) -> Result<Vec<Value>, GarminError> {
        let mut activities = Vec::new();
        let mut start = 0;
        for _ in 0..MAX_PAGINATED_REQUESTS {
            let query = [
                ("startDate", date_from.to_string()),
                ("start", start.to_string()),
                ("limit", PAGE_SIZE.to_string()),
                ("endDate", date_to.to_string()),
            ];
            let page = self.get_json(ACTIVITIES_PATH, &query).await?;
            if !truthy(&page) {
                return Ok(activities);
            }
            let page = page
                .as_array()
                .ok_or_else(|| GarminError::Malformed("activities is not a list".into()))?;
            activities.extend(page.iter().cloned());
            start += PAGE_SIZE;
        }
        Err(GarminError::Api(format!(
            "Pagination exceeded {MAX_PAGINATED_REQUESTS} requests; aborting"
        )))
    }

    pub async fn list_activity_ids_near(&self, start_time: &str) -> Result<Vec<i64>, GarminError> {
        let activities = self.list_activities_near(start_time).await?;
        activities
            .iter()
            .filter_map(|act| act.get("activityId").filter(|id| !id.is_null()))
            .map(|id| {
                activity_id_of(id).ok_or_else(|| {
                    GarminError::Malformed(format!("activity id {id} is not a number"))
                })
            })
            .collect()
    }

    pub async fn fetch_activity_types(&self) -> Result<Vec<Value>, GarminError> {
        Ok(activity_types_of(
            &self.get_json(ACTIVITY_TYPES_PATH, &[]).await?,
        ))
    }
}
