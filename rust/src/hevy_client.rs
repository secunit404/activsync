//! Hevy API v1 client, as the Python app's `hevy_client.py`: retries only
//! when Hevy signals 429 or 5xx, a distinct error for a rejected key, and the
//! events cursor that drives incremental sync. Responses are checked
//! defensively, since Hevy warns the event shapes may change.

use std::time::Duration;

use openidconnect::{HttpResponse, http};
use serde_json::Value;

use crate::net::{self, Http};

pub const DEFAULT_BASE_URL: &str = "https://api.hevyapp.com/v1";
const RETRY_STATUSES: [u16; 5] = [429, 500, 502, 503, 504];
const MAX_RETRIES: u32 = 5;
const MAX_BACKOFF_SECONDS: f64 = 120.0;

#[derive(Debug, Clone, PartialEq)]
pub enum HevyError {
    /// Hevy rejected the API key (401/403).
    Auth,
    Status(u16),
    Network(String),
    Malformed(String),
}

impl std::fmt::Display for HevyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auth => f.write_str(
                "Hevy API key is invalid or expired. Check your Hevy Pro subscription and regenerate the key at hevy.com/settings.",
            ),
            Self::Status(code) => write!(f, "Hevy answered {code}"),
            Self::Network(err) => write!(f, "could not reach Hevy: {err}"),
            Self::Malformed(err) => write!(f, "unreadable Hevy response: {err}"),
        }
    }
}

impl std::error::Error for HevyError {}

/// An event this app can act on: an update carries the workout's id and
/// `updated_at`, a deletion its id and `deleted_at`.
fn valid_event(event: &Value) -> bool {
    let present = |v: &Value, key: &str| crate::db::truthy(v.get(key));
    match event.get("type").and_then(Value::as_str) {
        Some("updated") => {
            event.get("workout").is_some_and(|w| w.is_object() && present(w, "id") && present(w, "updated_at"))
        }
        Some("deleted") => present(event, "id") && present(event, "deleted_at"),
        _ => false,
    }
}

/// A missing or non-integer `page_count` stops the walk: trusting it would
/// page forever against a malformed response.
fn more_pages(data: &Value, page: i64) -> bool {
    data.get("page_count").and_then(Value::as_i64).is_some_and(|count| page < count)
}

#[derive(Clone)]
pub struct HevyClient {
    http: Http,
    base_url: String,
    api_key: String,
    backoff_factor: f64,
}

impl HevyClient {
    pub fn new(http: Http, api_key: &str) -> Result<Self, &'static str> {
        Self::with_base_url(http, api_key, DEFAULT_BASE_URL)
    }

    pub fn with_base_url(http: Http, api_key: &str, base_url: &str) -> Result<Self, &'static str> {
        if api_key.is_empty() {
            return Err("Hevy API key required");
        }
        Ok(Self { http, base_url: base_url.trim_end_matches('/').into(), api_key: api_key.into(), backoff_factor: 2.0 })
    }

    /// No waiting between retries, for tests.
    pub fn without_backoff(self) -> Self {
        Self { backoff_factor: 0.0, ..self }
    }

    /// urllib3's schedule: no wait before the first retry, then the factor
    /// doubled each time, unless Hevy says how long in Retry-After.
    fn wait(&self, retry: u32, response: &HttpResponse) -> Duration {
        let told = net::header(response, "retry-after").and_then(|v| v.trim().parse::<f64>().ok());
        let backoff = if retry <= 1 { 0.0 } else { self.backoff_factor * 2f64.powi(retry as i32 - 1) };
        let seconds = told.map_or(backoff.min(MAX_BACKOFF_SECONDS), |t| t.max(0.0));
        Duration::from_secs_f64(if self.backoff_factor == 0.0 { 0.0 } else { seconds })
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, HevyError> {
        let url = net::with_query(&format!("{}{path}", self.base_url), query);
        let mut retry = 0;
        let response = loop {
            let request = net::request(
                http::Method::GET,
                &url,
                &[("api-key", &self.api_key), ("accept", "application/json")],
                Vec::new(),
            )
            .map_err(|e| HevyError::Network(e.0))?;
            let response = match (self.http)(request).await {
                Ok(response) => response,
                Err(_) if retry < MAX_RETRIES => {
                    retry += 1;
                    let backoff = if retry <= 1 { 0.0 } else { self.backoff_factor * 2f64.powi(retry as i32 - 1) };
                    tokio::time::sleep(Duration::from_secs_f64(backoff.min(MAX_BACKOFF_SECONDS))).await;
                    continue;
                }
                Err(err) => return Err(HevyError::Network(err.0)),
            };
            let status = response.status().as_u16();
            if RETRY_STATUSES.contains(&status) && retry < MAX_RETRIES {
                retry += 1;
                tokio::time::sleep(self.wait(retry, &response)).await;
                continue;
            }
            break response;
        };
        let status = response.status().as_u16();
        if status == 401 || status == 403 {
            return Err(HevyError::Auth);
        }
        if !(200..300).contains(&status) {
            return Err(HevyError::Status(status));
        }
        if let Some(remaining) = net::header(&response, "x-ratelimit-remaining").and_then(|v| v.parse::<i64>().ok()) {
            if remaining < 10 {
                crate::log_warn!("hevy_client", "Hevy API rate limit low: {remaining} requests remaining");
            }
        }
        serde_json::from_slice(response.body()).map_err(|e| HevyError::Malformed(e.to_string()))
    }

    /// Used to check an API key when Hevy is connected.
    pub async fn get_user_info(&self) -> Result<Value, HevyError> {
        self.get("/user/info", &[]).await
    }

    /// `None` only when Hevy says the workout does not exist or answers with
    /// something unreadable; an outage is an error, never "deleted".
    pub async fn get_workout(&self, workout_id: &str) -> Result<Option<Value>, HevyError> {
        let data = match self.get(&format!("/workouts/{workout_id}"), &[]).await {
            Err(HevyError::Status(404)) => return Ok(None),
            other => other?,
        };
        if data.get("id").is_some() {
            return Ok(Some(data));
        }
        if let Some(workout) = data.get("workout").filter(|w| w.is_object()) {
            return Ok(Some(workout.clone()));
        }
        crate::log_warn!("hevy_client", "malformed workout response for {workout_id}");
        Ok(None)
    }

    /// `None` means unknown, never zero: it only bounds a scan.
    pub async fn get_workout_count(&self) -> Result<Option<i64>, HevyError> {
        let data = self.get("/workouts/count", &[]).await?;
        let count = data.get("workout_count").filter(|c| c.is_i64() || c.is_u64()).and_then(Value::as_i64);
        if count.is_none_or(|c| c < 0) {
            crate::log_warn!("hevy_client", "unusable workout_count from Hevy");
            return Ok(None);
        }
        Ok(count)
    }

    pub async fn get_workouts_page(&self, page: i64, page_size: i64) -> Result<Value, HevyError> {
        self.get("/workouts", &[("page", &page.to_string()), ("pageSize", &page_size.to_string())]).await
    }

    pub async fn get_events(&self, since: &str, page: i64, page_size: i64) -> Result<Value, HevyError> {
        let (page, size) = (page.to_string(), page_size.to_string());
        self.get("/workouts/events", &[("since", since), ("page", &page), ("pageSize", &size)]).await
    }

    /// Every event newer than `since` across all pages. A malformed entry is
    /// skipped, so one bad event cannot stall the cursor.
    pub async fn iter_events_since(&self, since: &str) -> Result<Vec<Value>, HevyError> {
        let mut events = Vec::new();
        let mut page = 1;
        loop {
            let data = self.get_events(since, page, 10).await?;
            for event in data.get("events").and_then(Value::as_array).into_iter().flatten() {
                if valid_event(event) {
                    events.push(event.clone());
                } else {
                    crate::log_warn!("hevy_client", "skipping malformed hevy event");
                }
            }
            if !more_pages(&data, page) {
                return Ok(events);
            }
            page += 1;
        }
    }

    pub async fn get_exercise_template(&self, template_id: &str) -> Result<Option<Value>, HevyError> {
        match self.get(&format!("/exercise_templates/{template_id}"), &[]).await {
            Err(HevyError::Status(404)) => Ok(None),
            Ok(data) => Ok(data.is_object().then_some(data)),
            Err(err) => Err(err),
        }
    }

    /// One page of exercise templates, and whether another follows.
    pub async fn get_exercise_templates_page(&self, page: i64) -> Result<(Vec<Value>, bool), HevyError> {
        let data = self.get("/exercise_templates", &[("page", &page.to_string()), ("pageSize", "100")]).await?;
        let templates = data.get("exercise_templates").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok((templates, more_pages(&data, page)))
    }

    pub async fn iter_all_exercise_templates(&self) -> Result<Vec<Value>, HevyError> {
        let mut templates = Vec::new();
        let mut page = 1;
        loop {
            let (batch, more) = self.get_exercise_templates_page(page).await?;
            templates.extend(batch);
            if !more {
                return Ok(templates);
            }
            page += 1;
        }
    }
}
