use std::future::Future;
use std::time::Duration;

use openidconnect::{HttpResponse, http};
use serde_json::Value;

use crate::garmin_client::{API_LABEL, API_TIMEOUT, CONNECTAPI_URL, GarminClient, check_path, expires_soon, native_headers};
use crate::garmin_errors::{GarminError, api_error, status_of};
use crate::net;

const RETRY_ATTEMPTS: u32 = 3;
const RATE_LIMIT_ATTEMPTS: u32 = 3;
const RATE_LIMIT_BASE_WAIT: Duration = Duration::from_secs(30);
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX_FACTOR: u32 = 10;

pub(crate) struct Request<'a> {
    pub(crate) method: http::Method,
    pub(crate) path: &'a str,
    pub(crate) query: &'a [(&'a str, String)],
    pub(crate) headers: Vec<(&'a str, String)>,
    pub(crate) body: Vec<u8>,
}

impl<'a> Request<'a> {
    pub(crate) fn get(path: &'a str) -> Self {
        Self { method: http::Method::GET, path, query: &[], headers: Vec::new(), body: Vec::new() }
    }

    pub(crate) fn with_query(self, query: &'a [(&'a str, String)]) -> Self {
        Self { query, ..self }
    }

    pub(crate) fn json(method: http::Method, path: &'a str, payload: &Value) -> Self {
        let headers = vec![("Content-Type", "application/json".to_string())];
        Self { method, path, query: &[], headers, body: net::json_body(payload) }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Waits {
    pub(crate) pause: Duration,
    pub(crate) rate_limit: Duration,
    pub(crate) backoff: Duration,
}

impl Waits {
    pub(crate) fn from_pause(pause: Duration) -> Self {
        if pause.is_zero() {
            Self { pause, rate_limit: Duration::ZERO, backoff: Duration::ZERO }
        } else {
            Self { pause, rate_limit: RATE_LIMIT_BASE_WAIT, backoff: BACKOFF_MIN }
        }
    }

    /// Exponential with 50-100% jitter, capped at ten times the first wait.
    fn backoff_for(&self, attempt: u32) -> Duration {
        let base = (self.backoff * 2u32.saturating_pow(attempt)).min(self.backoff * BACKOFF_MAX_FACTOR);
        base.mul_f64(0.5 + rand::random::<f64>() * 0.5)
    }
}

impl GarminClient {
    pub(crate) async fn send(&self, request: &Request<'_>) -> Result<HttpResponse, GarminError> {
        let token = self.tokens().di_token.ok_or(GarminError::SessionExpired)?;
        let mut extra = vec![("Authorization", format!("Bearer {token}")), ("Accept", "application/json".to_string())];
        extra.extend(request.headers.iter().cloned());
        let headers = native_headers(&extra);
        let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let query: Vec<(&str, &str)> = request.query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let url = net::with_query(&format!("{CONNECTAPI_URL}/{}", request.path.trim_start_matches('/')), &query);
        let outgoing = net::request(request.method.clone(), &url, &headers, request.body.clone()).map_err(|e| GarminError::Network(e.0))?;
        (self.http)(net::with_timeout(outgoing, API_TIMEOUT)).await.map_err(|e| GarminError::Network(e.0))
    }

    pub(crate) async fn run(&self, request: &Request<'_>) -> Result<Vec<u8>, GarminError> {
        let soon = self.tokens().di_token.as_deref().is_some_and(|t| expires_soon(t, (self.clock)()));
        if soon {
            self.refresh_session().await;
        }
        check_path(request.path)?;
        let mut response = self.send(request).await?;
        if response.status().as_u16() == 401 {
            self.refresh_session().await;
            response = self.send(request).await?;
        }
        match response.status().as_u16() {
            204 => Ok(b"{}".to_vec()),
            status if status >= 400 => Err(api_error(&response)),
            _ => Ok(response.into_body()),
        }
    }

    pub(crate) async fn run_json(&self, request: &Request<'_>) -> Result<Value, GarminError> {
        let body = self.run(request).await?;
        serde_json::from_slice(&body).map_err(|e| GarminError::Malformed(e.to_string()))
    }

    /// The library's `connectapi` and `download`: a 5xx or a network failure
    /// is retried with a jittered backoff, and what finally fails is worded
    /// by status. The profile reads at sign-in and the PUT, POST and DELETE
    /// calls go through [`GarminClient::run`] alone, as the library sends them.
    pub(crate) async fn call(&self, label: &str, request: &Request<'_>) -> Result<Vec<u8>, GarminError> {
        let mut attempt = 0;
        loop {
            let err = match self.run(request).await {
                Ok(body) => return Ok(body),
                Err(err) => err,
            };
            let retryable = match &err {
                GarminError::Network(_) => true,
                GarminError::Api(message) => status_of(message).is_some_and(|status| status >= 500),
                _ => false,
            };
            if !retryable {
                return Err(Self::worded(label, err));
            }
            if attempt >= RETRY_ATTEMPTS {
                crate::log_error!("garmin_client", "{label} failed for path '{}': {err}", request.path);
                return Err(Self::worded(label, err));
            }
            let delay = self.waits.backoff_for(attempt);
            crate::log_warn!(
                "garmin_client",
                "{label} attempt {}/{} failed (path={}) - retrying in {:.1}s: {err}",
                attempt + 1,
                RETRY_ATTEMPTS + 1,
                request.path,
                delay.as_secs_f64()
            );
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            attempt += 1;
        }
    }

    pub(crate) fn worded(label: &str, err: GarminError) -> GarminError {
        match err {
            GarminError::Network(text) => GarminError::Network(format!("Connection error: {text}")),
            GarminError::NotFound(message) => GarminError::NotFound(format!("{label} client error (404): {message}")),
            GarminError::Api(message) => match status_of(&message) {
                Some(401) => GarminError::Api(format!("Authentication failed: {message}")),
                Some(429) => GarminError::Api(format!("Rate limit exceeded: {message}")),
                Some(status) if (400..500).contains(&status) => GarminError::Api(format!("{label} client error ({status}): {message}")),
                Some(_) => GarminError::Api(format!("{label} HTTP error: {message}")),
                None => GarminError::Api(message),
            },
            other => other,
        }
    }

    pub(crate) async fn call_json(&self, request: &Request<'_>) -> Result<Value, GarminError> {
        let body = self.call(API_LABEL, request).await?;
        serde_json::from_slice(&body).map_err(|e| GarminError::Malformed(e.to_string()))
    }

    pub(crate) async fn get_json(&self, path: &str, query: &[(&str, String)]) -> Result<Value, GarminError> {
        self.call_json(&Request::get(path).with_query(query)).await
    }

    pub(crate) async fn raw_json(&self, path: &str, query: &[(&str, String)]) -> Result<Value, GarminError> {
        self.run_json(&Request::get(path).with_query(query)).await
    }

    /// The Python app's rate limiter: a pause after every call that works,
    /// and a rate limit the library reported is waited out (30 s, then 60 s)
    /// before the whole operation is repeated.
    pub(crate) async fn limited<T, Fut>(&self, mut operation: impl FnMut() -> Fut) -> Result<T, GarminError>
    where
        Fut: Future<Output = Result<T, GarminError>>,
    {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match operation().await {
                Ok(value) => {
                    self.rest().await;
                    return Ok(value);
                }
                Err(err) if err.is_rate_limited() && attempt < RATE_LIMIT_ATTEMPTS => {
                    let wait = self.waits.rate_limit * attempt;
                    crate::log_warn!(
                        "rate_limit",
                        "rate limited (429); waiting {}s before retry {}/{}",
                        wait.as_secs(),
                        attempt + 1,
                        RATE_LIMIT_ATTEMPTS
                    );
                    if !wait.is_zero() {
                        tokio::time::sleep(wait).await;
                    }
                }
                Err(err) => {
                    if err.is_rate_limited() {
                        crate::log_error!("rate_limit", "rate limited (429): {RATE_LIMIT_ATTEMPTS} retries exhausted");
                    }
                    return Err(err);
                }
            }
        }
    }

    /// For the calls the library sends without its retry wrapper: they are
    /// still followed by a pause, but a 429 is final.
    pub(crate) async fn paced<T>(&self, result: Result<T, GarminError>) -> Result<T, GarminError> {
        if result.is_ok() {
            self.rest().await;
        }
        result
    }
}
