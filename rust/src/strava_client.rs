//! Strava v3 client, as the Python app's `strava_client.py`: OAuth tokens
//! kept in `strava_tokens`, activity upload with the upload-status poll,
//! metadata edits, and window reads for duplicate detection.

use std::time::Duration;

use chrono::{DateTime, Timelike, Utc};
use openidconnect::{HttpResponse, http};
use serde_json::{Value, json};

use crate::db::{self, Db};
use crate::net::{self, Http, Part};
use crate::schedule::Clock;

pub const AUTHORIZE_URL: &str = "https://www.strava.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://www.strava.com/oauth/token";
pub const UPLOAD_URL: &str = "https://www.strava.com/api/v3/uploads";
pub const ACTIVITIES_URL: &str = "https://www.strava.com/api/v3/athlete/activities";
pub const ACTIVITY_URL: &str = "https://www.strava.com/api/v3/activities";
pub const REVOKE_URL: &str = "https://www.strava.com/oauth/revoke";

/// `read` is listed because Strava skips the consent screen on a repeat
/// authorization only when every granted scope is named; `read_all` so
/// duplicate detection also sees private activities.
pub const SCOPE: &str = "read,activity:write,activity:read_all";

/// Strava's short-term quota resets on 15-minute wall-clock boundaries.
const RATE_LIMIT_WINDOW_MINUTES: u32 = 15;
const ACTIVITIES_PER_PAGE: usize = 200;
const MAX_ACTIVITY_PAGES: usize = 10;
pub const DEFAULT_MATCH_TOLERANCE_MINUTES: i64 = 5;
const POLL_ATTEMPTS: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub enum StravaError {
    /// Strava rejected the credentials or refresh token.
    Auth(String),
    /// A 429, with the instant it is worth asking again. An instant rather
    /// than a wait, so a caller acting on it later does not retry early.
    RateLimited(DateTime<Utc>),
    /// Strava refused the uploaded file itself, as opposed to a timeout
    /// after which the upload may still land.
    Rejected(String),
    Upload(String),
    /// A window read that could not reach its end; absence from it must not
    /// be read as "deleted on Strava".
    WindowIncomplete(String),
    Status(u16),
    Network(String),
    Malformed(String),
}

impl std::fmt::Display for StravaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auth(msg) | Self::Rejected(msg) | Self::Upload(msg) | Self::WindowIncomplete(msg) => f.write_str(msg),
            Self::RateLimited(at) => write!(f, "Strava rate limit reached; retry at {}", at.format("%Y-%m-%dT%H:%M:%S+00:00")),
            Self::Status(code) => write!(f, "Strava answered {code}"),
            Self::Network(err) => write!(f, "could not reach Strava: {err}"),
            Self::Malformed(err) => write!(f, "unreadable Strava response: {err}"),
        }
    }
}

impl std::error::Error for StravaError {}

fn seconds_until_quota_reset(now: DateTime<Utc>) -> f64 {
    let into_window = now.minute() % RATE_LIMIT_WINDOW_MINUTES;
    f64::from((RATE_LIMIT_WINDOW_MINUTES - into_window) * 60) - f64::from(now.second())
}

/// Strava often leaves Retry-After off a 429, so the published reset
/// schedule stands in rather than retrying straight into another 429.
fn check_rate_limit(response: &HttpResponse, now: DateTime<Utc>) -> Result<(), StravaError> {
    if response.status().as_u16() != 429 {
        return Ok(());
    }
    let told = net::header(response, "retry-after").and_then(|v| v.trim().parse::<f64>().ok()).map(|s| s.max(0.0));
    let wait = told.unwrap_or_else(|| seconds_until_quota_reset(now));
    Err(StravaError::RateLimited(now + chrono::Duration::milliseconds((wait * 1000.0) as i64)))
}

fn check_status(response: &HttpResponse) -> Result<(), StravaError> {
    let status = response.status().as_u16();
    if (200..300).contains(&status) { Ok(()) } else { Err(StravaError::Status(status)) }
}

fn json_of(response: &HttpResponse) -> Result<Value, StravaError> {
    serde_json::from_slice(response.body()).map_err(|e| StravaError::Malformed(e.to_string()))
}

#[derive(Clone, Debug, PartialEq)]
pub struct StravaActivity {
    pub id: i64,
    pub start_date: DateTime<Utc>,
}

/// The activity starting closest to `start`, within the tolerance; shared by
/// the single lookup and the window scan so both agree on "the same one".
pub fn match_closest_activity(activities: &[StravaActivity], start: DateTime<Utc>, tolerance_minutes: i64) -> Option<i64> {
    let tolerance = (tolerance_minutes * 60) as f64;
    let mut best: Option<(i64, f64)> = None;
    for activity in activities {
        let delta = ((activity.start_date - start).num_milliseconds() as f64 / 1000.0).abs();
        if delta <= tolerance && best.is_none_or(|(_, d)| delta < d) {
            best = Some((activity.id, delta));
        }
    }
    best.map(|(id, _)| id)
}

#[derive(Clone)]
pub struct StravaClient {
    http: Http,
    db: Db,
    clock: Clock,
    client_id: String,
    client_secret: String,
    poll_delay: Duration,
}

impl StravaClient {
    pub fn new(http: Http, db: Db, clock: Clock, client_id: &str, client_secret: &str) -> Self {
        Self {
            http,
            db,
            clock,
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            poll_delay: Duration::from_secs(2),
        }
    }

    /// No pause between upload-status checks, for tests.
    pub fn without_waits(self) -> Self {
        Self { poll_delay: Duration::ZERO, ..self }
    }

    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_micros(((self.clock)() * 1_000_000.0) as i64).unwrap_or_default()
    }

    async fn send(&self, method: http::Method, url: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Result<HttpResponse, StravaError> {
        let request = net::request(method, url, headers, body).map_err(|e| StravaError::Network(e.0))?;
        (self.http)(request).await.map_err(|e| StravaError::Network(e.0))
    }

    async fn post_form(&self, url: &str, pairs: &[(&str, &str)]) -> Result<HttpResponse, StravaError> {
        let headers = [("content-type", "application/x-www-form-urlencoded")];
        self.send(http::Method::POST, url, &headers, net::form(pairs)).await
    }

    pub fn is_connected(&self) -> bool {
        let tokens = db::get_config_value(&self.db.lock(), "strava_tokens").ok().flatten();
        tokens.is_some_and(|t| db::truthy(t.get("refresh_token")))
    }

    /// `state` comes back to the callback unchanged; the caller keeps the
    /// value it issued and rejects any callback that does not match.
    pub fn authorize_url(&self, redirect_uri: &str, state: &str) -> String {
        format!(
            "{AUTHORIZE_URL}?client_id={}&redirect_uri={redirect_uri}&response_type=code&approval_prompt=auto&scope={SCOPE}&state={state}",
            self.client_id
        )
    }

    pub async fn exchange_code(&self, code: &str, granted_scope: Option<&str>) -> Result<(), StravaError> {
        let response = self
            .post_form(TOKEN_URL, &[
                ("client_id", &self.client_id),
                ("client_secret", &self.client_secret),
                ("code", code),
                ("grant_type", "authorization_code"),
            ])
            .await?;
        check_status(&response)?;
        let payload = json_of(&response)?;
        let mut tokens = json!({
            "access_token": payload["access_token"],
            "refresh_token": payload["refresh_token"],
            "expires_at": payload["expires_at"],
        });
        if let Some(scope) = granted_scope {
            tokens["scope"] = json!(scope);
        }
        db::set_config_value(&self.db.lock(), "strava_tokens", &tokens).map_err(|e| StravaError::Malformed(e.to_string()))?;
        crate::log_info!("strava_client", "strava connected");
        Ok(())
    }

    async fn access_token(&self) -> Result<String, StravaError> {
        let tokens = db::get_config_value(&self.db.lock(), "strava_tokens").ok().flatten().filter(|t| db::truthy(Some(t)));
        let Some(tokens) = tokens else {
            return Err(StravaError::Auth("Strava is not connected".into()));
        };
        let expires_at = tokens["expires_at"].as_f64().unwrap_or(0.0);
        if expires_at > (self.clock)() + 60.0 {
            return Ok(tokens["access_token"].as_str().unwrap_or_default().to_string());
        }
        let refresh = tokens["refresh_token"].as_str().unwrap_or_default().to_string();
        let response = self
            .post_form(TOKEN_URL, &[
                ("client_id", &self.client_id),
                ("client_secret", &self.client_secret),
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh),
            ])
            .await?;
        if response.status().as_u16() == 401 {
            return Err(StravaError::Auth("Strava refresh token was revoked".into()));
        }
        check_status(&response)?;
        let payload = json_of(&response)?;
        let mut refreshed = tokens.clone();
        for key in ["access_token", "refresh_token", "expires_at"] {
            refreshed[key] = payload[key].clone();
        }
        db::set_config_value(&self.db.lock(), "strava_tokens", &refreshed).map_err(|e| StravaError::Malformed(e.to_string()))?;
        Ok(refreshed["access_token"].as_str().unwrap_or_default().to_string())
    }

    /// Forgets the connection here and on Strava. The revoke is best-effort:
    /// the local tokens go even if Strava cannot be reached.
    pub async fn disconnect(&self) {
        let tokens = db::get_config_value(&self.db.lock(), "strava_tokens").ok().flatten();
        if let Some(access) = tokens.as_ref().and_then(|t| t["access_token"].as_str()).filter(|a| !a.is_empty()) {
            use base64::Engine;
            let basic = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", self.client_id, self.client_secret));
            let auth = format!("Basic {basic}");
            let body = net::form(&[("token", access), ("token_type_hint", "access_token")]);
            let headers = [("authorization", auth.as_str()), ("content-type", "application/x-www-form-urlencoded")];
            let _ = self.send(http::Method::POST, REVOKE_URL, &headers, body).await;
        }
        let _ = db::set_config_value(&self.db.lock(), "strava_tokens", &Value::Null);
        crate::log_info!("strava_client", "strava disconnected");
    }

    pub async fn update_activity_metadata(&self, strava_activity_id: i64, name: &str, description: &str) -> Result<(), StravaError> {
        let bearer = format!("Bearer {}", self.access_token().await?);
        let body = net::json_body(&json!({"name": name, "description": description}));
        let url = format!("{ACTIVITY_URL}/{strava_activity_id}");
        let response = self
            .send(http::Method::PUT, &url, &[("authorization", &bearer), ("content-type", "application/json")], body)
            .await?;
        if response.status().as_u16() == 401 {
            return Err(StravaError::Auth("Strava access token rejected".into()));
        }
        check_rate_limit(&response, self.now())?;
        check_status(&response)
    }

    async fn activities_page(
        &self,
        bearer: &str,
        after: DateTime<Utc>,
        before: DateTime<Utc>,
        per_page: usize,
        page: usize,
    ) -> Result<Vec<StravaActivity>, StravaError> {
        let url = net::with_query(ACTIVITIES_URL, &[
            ("after", &after.timestamp().to_string()),
            ("before", &before.timestamp().to_string()),
            ("per_page", &per_page.to_string()),
            ("page", &page.to_string()),
        ]);
        let response = self.send(http::Method::GET, &url, &[("authorization", bearer)], Vec::new()).await?;
        if response.status().as_u16() == 401 {
            return Err(StravaError::Auth(
                "Strava rejected the read request — the connected app may be missing the activity:read_all scope. \
                 Reconnect Strava in Settings."
                    .into(),
            ));
        }
        check_rate_limit(&response, self.now())?;
        check_status(&response)?;
        let items = json_of(&response)?;
        items
            .as_array()
            .ok_or_else(|| StravaError::Malformed("activities is not a list".into()))?
            .iter()
            .map(|a| {
                let id = a["id"].as_i64().ok_or_else(|| StravaError::Malformed("activity without id".into()))?;
                let start = a["start_date"].as_str().and_then(crate::timefmt::parse_timestamp);
                let start_date = start.ok_or_else(|| StravaError::Malformed("activity without start_date".into()))?;
                Ok(StravaActivity { id, start_date })
            })
            .collect()
    }

    /// Every activity in a window, paged; one read reconciles many
    /// activities, which keeps the poller inside Strava's quota.
    pub async fn list_activities_between(&self, after: DateTime<Utc>, before: DateTime<Utc>) -> Result<Vec<StravaActivity>, StravaError> {
        let bearer = format!("Bearer {}", self.access_token().await?);
        let mut activities = Vec::new();
        for page in 1..=MAX_ACTIVITY_PAGES {
            let batch = self.activities_page(&bearer, after, before, ACTIVITIES_PER_PAGE, page).await?;
            let last = batch.len() < ACTIVITIES_PER_PAGE;
            activities.extend(batch);
            if last {
                return Ok(activities);
            }
        }
        Err(StravaError::WindowIncomplete(format!(
            "more than {} Strava activities in the window {}..{}",
            MAX_ACTIVITY_PAGES * ACTIVITIES_PER_PAGE,
            db::iso(after),
            db::iso(before)
        )))
    }

    /// An activity already on Strava near this start, however it got there
    /// (Garmin's own Strava link, a manual upload), so publishing does not
    /// make a duplicate.
    pub async fn find_existing_activity(&self, start: DateTime<Utc>, tolerance_minutes: i64) -> Result<Option<i64>, StravaError> {
        let bearer = format!("Bearer {}", self.access_token().await?);
        let window = chrono::Duration::minutes(tolerance_minutes);
        let activities = self.activities_page(&bearer, start - window, start + window, 30, 1).await?;
        Ok(match_closest_activity(&activities, start, tolerance_minutes))
    }

    pub async fn publish(&self, garmin_activity_id: i64, fit: &[u8], name: Option<&str>, description: Option<&str>) -> Result<i64, StravaError> {
        let filename = format!("{garmin_activity_id}.fit");
        let file = Part { name: "file", filename: Some(&filename), content_type: Some("application/octet-stream"), data: fit };
        self.upload(file, "fit", garmin_activity_id, name, description).await
    }

    /// A strength activity in Strava's JSON upload format.
    pub async fn publish_strength(
        &self,
        garmin_activity_id: i64,
        document: &Value,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<i64, StravaError> {
        let filename = format!("{garmin_activity_id}.json");
        let data = net::json_body(document);
        let file = Part { name: "file", filename: Some(&filename), content_type: Some("application/json"), data: &data };
        self.upload(file, "json", garmin_activity_id, name, description).await
    }

    /// The name is sent because Strava otherwise ignores the file's own and
    /// names the activity from its date and place.
    async fn upload(
        &self,
        file: Part<'_>,
        data_type: &str,
        garmin_activity_id: i64,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<i64, StravaError> {
        let token = self.access_token().await?;
        let bearer = format!("Bearer {token}");
        let external_id = format!("garmin-{garmin_activity_id}");
        let mut parts = vec![
            Part { name: "data_type", filename: None, content_type: None, data: data_type.as_bytes() },
            Part { name: "external_id", filename: None, content_type: None, data: external_id.as_bytes() },
        ];
        if let Some(name) = name.filter(|n| !n.is_empty()) {
            parts.push(Part { name: "name", filename: None, content_type: None, data: name.as_bytes() });
        }
        if let Some(description) = description.filter(|d| !d.is_empty()) {
            parts.push(Part { name: "description", filename: None, content_type: None, data: description.as_bytes() });
        }
        parts.push(file);
        let (content_type, body) = net::multipart(&parts);
        let response = self
            .send(http::Method::POST, UPLOAD_URL, &[("authorization", &bearer), ("content-type", &content_type)], body)
            .await?;
        let status = response.status().as_u16();
        if status == 401 {
            return Err(StravaError::Auth("Strava access token rejected".into()));
        }
        check_rate_limit(&response, self.now())?;
        if (400..500).contains(&status) {
            let text: String = net::body_text(&response).chars().take(200).collect();
            return Err(StravaError::Rejected(format!("upload rejected ({status}): {text}")));
        }
        check_status(&response)?;
        let upload_id = json_of(&response)?["id"]
            .as_i64()
            .ok_or_else(|| StravaError::Malformed("upload without id".into()))?;
        self.poll_upload(upload_id, &bearer).await
    }

    async fn poll_upload(&self, upload_id: i64, bearer: &str) -> Result<i64, StravaError> {
        let url = format!("{UPLOAD_URL}/{upload_id}");
        for _ in 0..POLL_ATTEMPTS {
            let response = self.send(http::Method::GET, &url, &[("authorization", bearer)], Vec::new()).await?;
            check_rate_limit(&response, self.now())?;
            check_status(&response)?;
            let payload = json_of(&response)?;
            if let Some(error) = payload.get("error").filter(|e| db::truthy(Some(e))) {
                return Err(StravaError::Rejected(error.as_str().map_or_else(|| error.to_string(), str::to_string)));
            }
            if let Some(id) = payload.get("activity_id").filter(|a| db::truthy(Some(a))).and_then(Value::as_i64) {
                return Ok(id);
            }
            tokio::time::sleep(self.poll_delay).await;
        }
        Err(StravaError::Upload(format!("upload {upload_id} did not resolve in time")))
    }
}
