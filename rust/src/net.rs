//! Outbound HTTP for the service clients. Every call goes through `Http`, so
//! tests stand in for Hevy, Strava or an identity provider without a network.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use openidconnect::reqwest;
use openidconnect::{HttpRequest, HttpResponse, http};
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
        Box::pin(async move { execute(&client, request).await.map_err(HttpError) })
    })
}

async fn execute(client: &reqwest::Client, request: HttpRequest) -> Result<HttpResponse, String> {
    let timeout = request.extensions().get::<Timeout>().map(|t| t.0);
    let mut outgoing = reqwest::Request::try_from(request).map_err(|e| describe(&e))?;
    if let Some(timeout) = timeout {
        *outgoing.timeout_mut() = Some(timeout);
    }
    let response = client.execute(outgoing).await.map_err(|e| describe(&e))?;
    let mut builder = http::Response::builder()
        .status(response.status())
        .version(response.version());
    for (name, value) in response.headers() {
        builder = builder.header(name, value);
    }
    let body = response.bytes().await.map_err(|e| describe(&e))?;
    builder.body(body.to_vec()).map_err(|e| e.to_string())
}

fn describe(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Bundle variables in the order `requests` reads them.
const REQUESTS_CA_VARS: [&str; 2] = ["REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"];
/// The same for `httpx`, which the identity-provider client uses.
const SSL_CERT_VARS: [&str; 2] = ["SSL_CERT_FILE", "SSL_CERT_DIR"];

/// Certificates from the first of `vars` that is set: a PEM bundle, or a
/// directory of them. Added to the built-in roots rather than replacing them.
pub fn extra_roots(
    vars: &[&str],
    lookup: impl Fn(&str) -> Option<String>,
) -> Vec<reqwest::Certificate> {
    let Some(path) = vars
        .iter()
        .find_map(|var| lookup(var).filter(|value| !value.is_empty()))
    else {
        return Vec::new();
    };
    let path = Path::new(&path);
    let files: Vec<_> = if path.is_dir() {
        std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect()
    } else {
        vec![path.to_path_buf()]
    };
    let roots: Vec<_> = files
        .iter()
        .filter_map(|file| std::fs::read(file).ok())
        .filter_map(|pem| reqwest::Certificate::from_pem_bundle(&pem).ok())
        .flatten()
        .collect();
    if roots.is_empty() {
        crate::log_warn!(
            "net",
            "no CA certificates could be read from {}",
            path.display()
        );
    }
    roots
}

fn builder_with_roots(vars: &[&str]) -> reqwest::ClientBuilder {
    extra_roots(vars, |name| std::env::var(name).ok())
        .into_iter()
        .fold(reqwest::Client::builder(), |builder, root| {
            builder.add_root_certificate(root)
        })
}

/// For Hevy and Strava: a minute per request unless the request names its
/// own, the longest the Python app allows (a Strava upload).
pub fn service_http() -> Http {
    static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
        builder_with_roots(&REQUESTS_CA_VARS)
            .timeout(Duration::from_secs(60))
            .build()
            .expect("HTTP client builds")
    });
    wrap(&CLIENT)
}

/// For Garmin: the library waits 15 s for an API call and 30 s for a token
/// refresh; the requests say which, and this is the longer for any that do not.
pub fn garmin_http() -> Http {
    static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
        builder_with_roots(&REQUESTS_CA_VARS)
            .timeout(Duration::from_secs(30))
            .build()
            .expect("HTTP client builds")
    });
    wrap(&CLIENT)
}

/// The browser a Garmin sign-in request presents itself as, in TLS and
/// HTTP/2 and in the headers the request leaves unset. Named after the
/// `curl_cffi` profiles the Python library impersonates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Browser {
    SafariIos,
    Safari,
    Chrome120,
    Edge101,
    Chrome,
}

impl Browser {
    fn profile(self) -> wreq_util::Profile {
        match self {
            Self::SafariIos => wreq_util::Profile::SafariIos26,
            Self::Safari => wreq_util::Profile::Safari26,
            Self::Chrome120 => wreq_util::Profile::Chrome120,
            Self::Edge101 => wreq_util::Profile::Edge101,
            Self::Chrome => wreq_util::Profile::Chrome131,
        }
    }
}

/// For Garmin's sign-in, which follows redirects itself to keep each hop's
/// cookies. A request carrying a `Browser` extension goes out looking like
/// that browser; any other goes out as the other clients' requests do.
pub fn garmin_sign_in_http() -> Http {
    static PLAIN: LazyLock<reqwest::Client> = LazyLock::new(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .expect("HTTP client builds")
    });
    static BROWSER: LazyLock<wreq::Client> = LazyLock::new(|| {
        wreq::Client::builder()
            .redirect(wreq::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .expect("HTTP client builds")
    });
    let plain = wrap(&PLAIN);
    Arc::new(
        move |request| match request.extensions().get::<Browser>().copied() {
            Some(browser) => Box::pin(as_browser(&BROWSER, browser, request)),
            None => plain(request),
        },
    )
}

/// The request's own headers replace the profile's of the same name, so a
/// route's User-Agent or Accept is sent once, as the route set it.
async fn as_browser(
    client: &wreq::Client,
    browser: Browser,
    request: HttpRequest,
) -> Result<HttpResponse, HttpError> {
    let (parts, body) = request.into_parts();
    let response = client
        .request(parts.method, parts.uri.to_string())
        .emulation(browser.profile())
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .map_err(|e| HttpError(e.to_string()))?;
    let mut builder = http::Response::builder()
        .status(response.status())
        .version(response.version());
    for (name, value) in response.headers() {
        builder = builder.header(name, value);
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| HttpError(e.to_string()))?;
    builder
        .body(bytes.to_vec())
        .map_err(|e| HttpError(e.to_string()))
}

/// For an identity provider: redirects are not followed, since a provider
/// answering with one could otherwise point the server at any address.
pub fn provider_http() -> Http {
    static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
        builder_with_roots(&SSL_CERT_VARS)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client builds")
    });
    wrap(&CLIENT)
}

/// `application/x-www-form-urlencoded`, as `requests` encodes a `data=` dict.
pub fn form(pairs: &[(&str, &str)]) -> Vec<u8> {
    openidconnect::url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
        .into_bytes()
}

pub fn with_query(url: &str, pairs: &[(&str, &str)]) -> String {
    if pairs.is_empty() {
        return url.to_string();
    }
    let query = openidconnect::url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    format!("{url}?{query}")
}

pub fn request(
    method: http::Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> Result<HttpRequest, HttpError> {
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

/// How long one request may take, in place of the client's own limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timeout(pub Duration);

pub fn with_timeout(mut request: HttpRequest, timeout: Duration) -> HttpRequest {
    request.extensions_mut().insert(Timeout(timeout));
    request
}
