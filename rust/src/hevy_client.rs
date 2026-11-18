//! Responses are checked defensively, since Hevy warns the event shapes may
//! change. A failure reads as Python's `requests` words it, since that text
//! is stored on the workout.

use std::time::Duration;

use chrono::{DateTime, Utc};
use openidconnect::{HttpResponse, http};
use serde_json::Value;

use crate::net::{self, Http};

const DEFAULT_BASE_URL: &str = "https://api.hevyapp.com/v1";
const RETRY_STATUSES: [u16; 5] = [429, 500, 502, 503, 504];
const MAX_RETRIES: u32 = 5;
const MAX_BACKOFF_SECONDS: f64 = 120.0;
const MAX_RETRY_AFTER_SECONDS: f64 = 21600.0;
const RETRY_AFTER_STATUSES: [u16; 3] = [413, 429, 503];
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq)]
pub enum HevyError {
    /// Hevy rejected the API key (401/403).
    Auth,
    /// Hevy answered 400 or above; `message` is `raise_for_status`'s text.
    Status {
        code: u16,
        message: String,
    },
    Network(String),
    Malformed(String),
}

impl std::fmt::Display for HevyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auth => f.write_str(
                "Hevy API key is invalid or expired. Check your Hevy Pro subscription and regenerate the key at hevy.com/settings.",
            ),
            Self::Status { message, .. } | Self::Network(message) | Self::Malformed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for HevyError {}

/// How long urllib3 sleeps before retry number `retry`: a Retry-After that
/// asks for more than nothing is obeyed, up to six hours, whatever the
/// status; otherwise no wait before the first retry, then the factor doubled
/// each time.
pub fn retry_delay(
    retry: u32,
    backoff_factor: f64,
    retry_after: Option<&str>,
    now: DateTime<Utc>,
) -> Duration {
    let told = retry_after
        .and_then(|value| retry_after_seconds(value, now))
        .filter(|seconds| *seconds > 0.0);
    let seconds = told.unwrap_or_else(|| {
        let backoff = if retry <= 1 {
            0.0
        } else {
            backoff_factor * 2f64.powi(retry as i32 - 1)
        };
        backoff.clamp(0.0, MAX_BACKOFF_SECONDS)
    });
    Duration::from_secs_f64(seconds)
}

fn retry_after_seconds(value: &str, now: DateTime<Utc>) -> Option<f64> {
    let trimmed = value.trim();
    let seconds = if !trimmed.is_empty() && trimmed.bytes().all(|b| b.is_ascii_digit()) {
        trimmed.parse::<f64>().ok()?
    } else {
        let date = DateTime::parse_from_rfc2822(trimmed).ok()?;
        (date.with_timezone(&Utc) - now).num_milliseconds() as f64 / 1000.0
    };
    Some(seconds.clamp(0.0, MAX_RETRY_AFTER_SECONDS))
}

/// An event this app can act on: an update carries the workout's id and
/// `updated_at`, a deletion its id and `deleted_at`.
fn valid_event(event: &Value) -> bool {
    let present = |v: &Value, key: &str| crate::db::truthy(v.get(key));
    match event.get("type").and_then(Value::as_str) {
        Some("updated") => event
            .get("workout")
            .is_some_and(|w| w.is_object() && present(w, "id") && present(w, "updated_at")),
        Some("deleted") => present(event, "id") && present(event, "deleted_at"),
        _ => false,
    }
}

/// A missing or non-integer `page_count` stops the walk: trusting it would
/// page forever against a malformed response.
fn more_pages(data: &Value, page: i64) -> bool {
    data.get("page_count")
        .and_then(Value::as_i64)
        .is_some_and(|count| page < count)
}

/// `Response.raise_for_status()`'s text.
fn status_message(response: &HttpResponse, url: &str) -> String {
    let status = response.status();
    let kind = if status.is_client_error() {
        "Client"
    } else {
        "Server"
    };
    format!(
        "{} {kind} Error: {} for url: {url}",
        status.as_u16(),
        status.canonical_reason().unwrap_or_default()
    )
}

/// The `ConnectionError` text `requests` gives once urllib3 has run out of
/// retries.
fn connection_error(url: &str, cause: &str) -> String {
    let uri = url.parse::<http::Uri>().ok();
    let secure = uri.as_ref().and_then(|u| u.scheme_str()) != Some("http");
    let host = uri.as_ref().and_then(|u| u.host()).unwrap_or_default();
    let port = uri
        .as_ref()
        .and_then(|u| u.port_u16())
        .unwrap_or(if secure { 443 } else { 80 });
    let pool = format!(
        "HTTP{}ConnectionPool(host='{host}', port={port})",
        if secure { "S" } else { "" }
    );
    let target = uri
        .as_ref()
        .and_then(|u| u.path_and_query())
        .map(|p| p.as_str())
        .unwrap_or_default();
    let cause = if cause.contains("timed out") {
        format!("ReadTimeoutError(\"{pool}: Read timed out. (read timeout=30)\")")
    } else {
        cause.to_string()
    };
    format!("{pool}: Max retries exceeded with url: {target} (Caused by {cause})")
}

/// The `JSONDecodeError` text for a body that is not JSON at all; any other
/// failure keeps serde's wording.
fn json_message(body: &[u8], error: &serde_json::Error) -> String {
    if !error.is_syntax() || !error.to_string().starts_with("expected value") {
        return error.to_string();
    }
    let text = String::from_utf8_lossy(body);
    let before: usize = text
        .split('\n')
        .take(error.line().saturating_sub(1))
        .map(|l| l.chars().count() + 1)
        .sum();
    let offset = before + error.column().saturating_sub(1);
    format!(
        "Expecting value: line {} column {} (char {offset})",
        error.line(),
        error.column()
    )
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
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').into(),
            api_key: api_key.into(),
            backoff_factor: 2.0,
        })
    }

    pub fn without_backoff(self) -> Self {
        Self {
            backoff_factor: 0.0,
            ..self
        }
    }

    fn wait(&self, retry: u32, retry_after: Option<&str>) -> Duration {
        if self.backoff_factor == 0.0 {
            Duration::ZERO
        } else {
            retry_delay(retry, self.backoff_factor, retry_after, Utc::now())
        }
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
            let response = match (self.http)(net::with_timeout(request, REQUEST_TIMEOUT)).await {
                Ok(response) => response,
                Err(_) if retry < MAX_RETRIES => {
                    retry += 1;
                    tokio::time::sleep(self.wait(retry, None)).await;
                    continue;
                }
                Err(err) => return Err(HevyError::Network(connection_error(&url, &err.0))),
            };
            let status = response.status().as_u16();
            let retry_after = net::header(&response, "retry-after").filter(|v| !v.is_empty());
            let retryable = RETRY_STATUSES.contains(&status)
                || (RETRY_AFTER_STATUSES.contains(&status) && retry_after.is_some());
            if retryable && retry < MAX_RETRIES {
                retry += 1;
                tokio::time::sleep(self.wait(retry, retry_after)).await;
                continue;
            }
            break response;
        };
        let status = response.status().as_u16();
        if status == 401 || status == 403 {
            return Err(HevyError::Auth);
        }
        if status >= 400 {
            return Err(HevyError::Status {
                code: status,
                message: status_message(&response, &url),
            });
        }
        if let Some(remaining) =
            net::header(&response, "x-ratelimit-remaining").and_then(|v| v.parse::<i64>().ok())
            && remaining < 10
        {
            crate::log_warn!(
                "hevy_client",
                "Hevy API rate limit low: {remaining} requests remaining"
            );
        }
        serde_json::from_slice(response.body())
            .map_err(|e| HevyError::Malformed(json_message(response.body(), &e)))
    }

    pub async fn get_user_info(&self) -> Result<Value, HevyError> {
        self.get("/user/info", &[]).await
    }

    /// `None` only when Hevy says the workout does not exist or answers with
    /// something unreadable; an outage is an error, never "deleted".
    pub async fn get_workout(&self, workout_id: &str) -> Result<Option<Value>, HevyError> {
        let data = match self.get(&format!("/workouts/{workout_id}"), &[]).await {
            Err(HevyError::Status { code: 404, .. }) => return Ok(None),
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
        let count = data
            .get("workout_count")
            .filter(|c| c.is_i64() || c.is_u64())
            .and_then(Value::as_i64);
        if count.is_none_or(|c| c < 0) {
            crate::log_warn!("hevy_client", "unusable workout_count from Hevy");
            return Ok(None);
        }
        Ok(count)
    }

    pub async fn get_workouts_page(&self, page: i64, page_size: i64) -> Result<Value, HevyError> {
        self.get(
            "/workouts",
            &[
                ("page", &page.to_string()),
                ("pageSize", &page_size.to_string()),
            ],
        )
        .await
    }

    async fn get_events(&self, since: &str, page: i64, page_size: i64) -> Result<Value, HevyError> {
        let (page, size) = (page.to_string(), page_size.to_string());
        self.get(
            "/workouts/events",
            &[("since", since), ("page", &page), ("pageSize", &size)],
        )
        .await
    }

    /// A malformed entry is skipped, so one bad event cannot stall the cursor.
    pub async fn iter_events_since(&self, since: &str) -> Result<Vec<Value>, HevyError> {
        let mut events = Vec::new();
        let mut page = 1;
        loop {
            let data = self.get_events(since, page, 10).await?;
            for event in data
                .get("events")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
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

    pub async fn get_exercise_template(
        &self,
        template_id: &str,
    ) -> Result<Option<Value>, HevyError> {
        match self
            .get(&format!("/exercise_templates/{template_id}"), &[])
            .await
        {
            Err(HevyError::Status { code: 404, .. }) => Ok(None),
            Ok(data) => Ok(data.is_object().then_some(data)),
            Err(err) => Err(err),
        }
    }

    async fn get_exercise_templates_page(
        &self,
        page: i64,
    ) -> Result<(Vec<Value>, bool), HevyError> {
        let data = self
            .get(
                "/exercise_templates",
                &[("page", &page.to_string()), ("pageSize", "100")],
            )
            .await?;
        let templates = data
            .get("exercise_templates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
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
