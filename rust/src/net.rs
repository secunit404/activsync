//! Outbound HTTP for the service clients. Every call goes through `Http`, so
//! tests stand in for Hevy, Strava or an identity provider without a network.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use openidconnect::reqwest;
use openidconnect::{AsyncHttpClient, HttpRequest, HttpResponse, http};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq)]
pub struct HttpError(pub String);

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HttpError {}

pub type HttpFuture = Pin<Box<dyn Future<Output = Result<HttpResponse, HttpError>> + Send>>;
pub type Http = Arc<dyn Fn(HttpRequest) -> HttpFuture + Send + Sync>;

fn wrap(client: &'static LazyLock<reqwest::Client>) -> Http {
    Arc::new(move |request| {
        let client = (*client).clone();
        Box::pin(async move { client.call(request).await.map_err(|e| HttpError(e.to_string())) })
    })
}

/// For Hevy and Strava: a minute per request, the longest the Python app
/// allows (a Strava upload).
pub fn service_http() -> Http {
    static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
        reqwest::Client::builder().timeout(Duration::from_secs(60)).build().expect("HTTP client builds")
    });
    wrap(&CLIENT)
}

/// For an identity provider: redirects are not followed, since a provider
/// answering with one could otherwise point the server at any address.
pub fn provider_http() -> Http {
    static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client builds")
    });
    wrap(&CLIENT)
}

/// `application/x-www-form-urlencoded`, as `requests` encodes a `data=` dict.
pub fn form(pairs: &[(&str, &str)]) -> Vec<u8> {
    openidconnect::url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish().into_bytes()
}

pub fn with_query(url: &str, pairs: &[(&str, &str)]) -> String {
    if pairs.is_empty() {
        return url.to_string();
    }
    let query = openidconnect::url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish();
    format!("{url}?{query}")
}

pub fn request(method: http::Method, url: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Result<HttpRequest, HttpError> {
    let mut builder = http::Request::builder().method(method).uri(url);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    builder.body(body).map_err(|e| HttpError(e.to_string()))
}

pub fn json_body(value: &impl Serialize) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

pub struct Part<'a> {
    pub name: &'a str,
    pub filename: Option<&'a str>,
    pub content_type: Option<&'a str>,
    pub data: &'a [u8],
}

/// A `multipart/form-data` body and its Content-Type header value.
pub fn multipart(parts: &[Part<'_>]) -> (String, Vec<u8>) {
    let boundary = format!("activsync-{}", crate::auth_core::random_token(12));
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        let mut disposition = format!("Content-Disposition: form-data; name=\"{}\"", part.name);
        if let Some(filename) = part.filename {
            disposition.push_str(&format!("; filename=\"{filename}\""));
        }
        body.extend_from_slice(disposition.as_bytes());
        body.extend_from_slice(b"\r\n");
        if let Some(content_type) = part.content_type {
            body.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
        }
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(part.data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

pub fn header<'a>(response: &'a HttpResponse, name: &str) -> Option<&'a str> {
    response.headers().get(name).and_then(|v| v.to_str().ok())
}

pub fn body_text(response: &HttpResponse) -> String {
    String::from_utf8_lossy(response.body()).into_owned()
}
