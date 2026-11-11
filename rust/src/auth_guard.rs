//! The request pipeline in front of every route, as in the Python app's
//! `auth_middleware.py`: trusted proxy headers, the Host check, sign-in,
//! the body cap for anonymous writes, CSRF, and security headers.

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use base64::Engine;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::auth_core::{self, HashCost};

pub const SESSION_COOKIE: &str = "activsync_session";
pub const SECURE_SESSION_COOKIE: &str = "__Host-activsync_session";
pub const DEVICE_COOKIE: &str = "activsync_device";
pub const SECURE_DEVICE_COOKIE: &str = "__Host-activsync_device";

const PUBLIC_BODY_LIMIT: usize = 16 * 1024;
const TOO_LARGE: &str = "That request is too large.";
const UNKNOWN_HOST_MESSAGE: &str = "Unknown host — set ACTIVSYNC_PUBLIC_URL or add it to ACTIVSYNC_ALLOWED_HOSTS";
const PUBLIC_HOST_MESSAGE: &str = "Unknown host — ActivSync answers only on the host in ACTIVSYNC_PUBLIC_URL";
const TRUST_GATEWAY_VARIABLE: &str = "ACTIVSYNC_AUTH_PROXY_TRUST_GATEWAY";

const PUBLIC_ROUTES: &[(&str, &str)] = &[
    ("GET", "/health"),
    ("GET", "/favicon.ico"),
    ("GET", "/apple-touch-icon.png"),
    ("GET", "/api/v1/auth/status"),
    ("POST", "/api/v1/auth/login"),
    ("POST", "/api/v1/auth/claim"),
    ("GET", "/auth/oidc/login"),
    ("GET", "/auth/oidc/callback"),
    ("GET", "/api/v1/auth/oidc/logo"),
];
const SPA_RESERVED_PREFIXES: &[&str] = &["api", "assets", "strava", "auth"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Network {
    address: IpAddr,
    prefix: u8,
}

impl Network {
    /// Parses "10.0.0.0/8" or a bare address. Host bits are ignored, as with
    /// Python's `ip_network(strict=False)`.
    pub fn parse(text: &str) -> Option<Self> {
        let (addr, prefix) = match text.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (text, None),
        };
        let address: IpAddr = addr.trim().parse().ok()?;
        let max = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p.trim().parse::<u8>().ok().filter(|p| *p <= max)?,
            None => max,
        };
        Some(Self { address, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.address, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = if self.prefix == 0 { 0 } else { u32::MAX << (32 - u32::from(self.prefix)) };
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = if self.prefix == 0 { 0 } else { u128::MAX << (128 - u32::from(self.prefix)) };
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let masked = match self.address {
            IpAddr::V4(a) => {
                let mask = if self.prefix == 0 { 0 } else { u32::MAX << (32 - u32::from(self.prefix)) };
                IpAddr::V4((u32::from(a) & mask).into())
            }
            IpAddr::V6(a) => {
                let mask = if self.prefix == 0 { 0 } else { u128::MAX << (128 - u32::from(self.prefix)) };
                IpAddr::V6((u128::from(a) & mask).into())
            }
        };
        write!(f, "{masked}/{}", self.prefix)
    }
}

/// Sign-in through an authenticating reverse proxy.
#[derive(Clone, Debug)]
pub struct ProxyAuth {
    pub header: String,
    pub trusted: Vec<Network>,
    pub user: String,
    pub trusted_gateways: Vec<IpAddr>,
}

impl ProxyAuth {
    /// Refuses a trusted range that holds the container's default gateway:
    /// Docker relays outside connections from that address, so anyone who can
    /// reach the port could send the header and be signed in as the owner.
    pub fn from_env(get: impl Fn(&str) -> Option<String>, gateways: &[IpAddr]) -> Result<Option<Self>, String> {
        let var = |name: &str| get(name).map(|v| v.trim().to_string()).unwrap_or_default();
        let header = var("ACTIVSYNC_AUTH_PROXY_HEADER");
        if header.is_empty() {
            return Ok(None);
        }
        let trusted_raw = var("ACTIVSYNC_AUTH_PROXY_TRUSTED");
        let user = var("ACTIVSYNC_AUTH_PROXY_USER");
        if trusted_raw.is_empty() {
            return Err("ACTIVSYNC_AUTH_PROXY_HEADER is set, so ACTIVSYNC_AUTH_PROXY_TRUSTED must list the proxy's address range (for example 172.18.0.0/16).".into());
        }
        if user.is_empty() {
            return Err("ACTIVSYNC_AUTH_PROXY_HEADER is set, so ACTIVSYNC_AUTH_PROXY_USER must name the user the proxy signs in as.".into());
        }
        let mut trusted = Vec::new();
        for item in trusted_raw.split(',').map(str::trim).filter(|i| !i.is_empty()) {
            let network = Network::parse(item)
                .ok_or_else(|| format!("ACTIVSYNC_AUTH_PROXY_TRUSTED has an invalid address range: {item}"))?;
            if network.prefix == 0 {
                return Err(format!("ACTIVSYNC_AUTH_PROXY_TRUSTED must not trust every address ({item})."));
            }
            trusted.push(network);
        }
        let in_range: Vec<IpAddr> = gateways.iter().copied().filter(|g| trusted.iter().any(|n| n.contains(*g))).collect();
        let trust_gateway = matches!(var(TRUST_GATEWAY_VARIABLE).to_lowercase().as_str(), "1" | "true" | "yes");
        if !in_range.is_empty() && !trust_gateway {
            let listed: Vec<String> = in_range.iter().map(IpAddr::to_string).collect();
            return Err(format!(
                "ACTIVSYNC_AUTH_PROXY_TRUSTED must not include this container's default gateway ({}). \
                 Docker's gateway relays connections that come from outside the Docker network, so anyone \
                 who can reach the published port could send the proxy header and be signed in as you. \
                 Trust only the reverse proxy's own address: give it a fixed IP and trust its /32. If the \
                 proxy runs on the Docker host itself, set {TRUST_GATEWAY_VARIABLE}=1 and keep the port \
                 reachable from that host alone.",
                listed.join(", ")
            ));
        }
        Ok(Some(Self { header, trusted, user, trusted_gateways: if trust_gateway { in_range } else { Vec::new() } }))
    }

    pub fn trusts(&self, peer: Option<IpAddr>) -> bool {
        peer.is_some_and(|ip| self.trusted.iter().any(|n| n.contains(ip)))
    }
}

/// Gateways of default routes from the Linux routing tables; empty where
/// they do not exist, such as on a Mac.
pub fn read_default_gateways() -> Vec<IpAddr> {
    let mut found = Vec::new();
    if let Ok(text) = std::fs::read_to_string("/proc/net/route") {
        for fields in text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>()) {
            if fields.len() < 3 || fields[1] != "00000000" {
                continue;
            }
            if let Ok(word) = u32::from_str_radix(fields[2], 16) {
                let ip = std::net::Ipv4Addr::from(word.to_le_bytes());
                if !ip.is_unspecified() {
                    found.push(IpAddr::V4(ip));
                }
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string("/proc/net/ipv6_route") {
        for fields in text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>()) {
            if fields.len() < 5 || fields[0] != "0".repeat(32) || fields[1] != "00" {
                continue;
            }
            if let Ok(value) = u128::from_str_radix(fields[4], 16) {
                let ip = std::net::Ipv6Addr::from(value);
                if !ip.is_unspecified() {
                    found.push(IpAddr::V6(ip));
                }
            }
        }
    }
    found
}

/// Who may set X-Forwarded-For/-Proto, from `FORWARDED_ALLOW_IPS`.
#[derive(Clone, Debug)]
pub struct ForwardedTrust {
    always: bool,
    networks: Vec<Network>,
    literals: Vec<String>,
}

impl ForwardedTrust {
    pub fn parse(value: &str) -> Self {
        let mut trust = Self { always: false, networks: Vec::new(), literals: Vec::new() };
        for item in value.split(',').map(str::trim).filter(|i| !i.is_empty()) {
            if item == "*" {
                trust.always = true;
            } else if let Some(network) = Network::parse(item) {
                trust.networks.push(network);
            } else {
                trust.literals.push(item.to_string());
            }
        }
        trust
    }

    fn trusts(&self, host: &str) -> bool {
        self.always
            || host.parse::<IpAddr>().is_ok_and(|ip| self.networks.iter().any(|n| n.contains(ip)))
            || self.literals.iter().any(|l| l == host)
    }

    /// uvicorn's rule: the rightmost forwarded address that is not itself a
    /// trusted proxy is the client.
    fn client_from(&self, forwarded_for: &str) -> String {
        let hosts: Vec<&str> = forwarded_for.split(',').map(str::trim).collect();
        if self.always {
            return hosts[0].to_string();
        }
        hosts.iter().rev().find(|h| !self.trusts(h)).unwrap_or(&hosts[0]).to_string()
    }
}

#[derive(Clone)]
pub struct AuthConfig {
    pub disabled: bool,
    pub public_url: Option<String>,
    pub allowed_hosts: Vec<String>,
    pub proxy: Option<ProxyAuth>,
    pub forwarded: ForwardedTrust,
    pub cost: HashCost,
    pub csp: Arc<String>,
}

impl AuthConfig {
    /// Reads the same environment variables as the Python app.
    pub fn from_env() -> Result<Self, String> {
        let get = |name: &str| std::env::var(name).ok();
        let enabled = |name: &str| matches!(get(name).unwrap_or_default().to_lowercase().as_str(), "1" | "true" | "yes");
        let public_url = get("ACTIVSYNC_PUBLIC_URL").map(|v| v.trim().trim_end_matches('/').to_string()).filter(|v| !v.is_empty());
        Ok(Self {
            disabled: enabled("ACTIVSYNC_AUTH_DISABLED"),
            public_url,
            allowed_hosts: get("ACTIVSYNC_ALLOWED_HOSTS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            proxy: ProxyAuth::from_env(get, &read_default_gateways())?,
            forwarded: ForwardedTrust::parse(&get("FORWARDED_ALLOW_IPS").unwrap_or_else(|| "127.0.0.1".into())),
            cost: if enabled("ACTIVSYNC_ARGON2_TEST_PARAMS") { HashCost::cheapest() } else { HashCost::standard() },
            csp: Arc::new(content_security_policy(None)),
        })
    }

    /// Sign-in on, nothing else configured, cheap hashing: for tests.
    pub fn for_tests() -> Self {
        Self {
            disabled: false,
            public_url: None,
            allowed_hosts: Vec::new(),
            proxy: None,
            forwarded: ForwardedTrust::parse("127.0.0.1"),
            cost: HashCost::cheapest(),
            csp: Arc::new(content_security_policy(None)),
        }
    }

    pub fn sign_in_off() -> Self {
        Self { disabled: true, ..Self::for_tests() }
    }

    fn public_origin(&self) -> Option<(String, String)> {
        let url = self.public_url.as_deref()?;
        let (scheme, rest) = url.split_once("://")?;
        let netloc = rest.split('/').next().unwrap_or_default();
        Some((scheme.to_lowercase(), normalize_host(netloc, Some(&scheme.to_lowercase()))))
    }

    fn force_https(&self) -> bool {
        self.public_origin().is_some_and(|(scheme, _)| scheme == "https")
    }
}

pub fn normalize_host(host: &str, scheme: Option<&str>) -> String {
    let host = host.trim().to_lowercase();
    let (name, port) = if let Some(rest) = host.strip_prefix('[') {
        match rest.split_once(']') {
            Some((inner, after)) => {
                let port = match after.strip_prefix(':') {
                    Some(p) => p,
                    None if after.is_empty() => "",
                    None => return host,
                };
                (format!("[{inner}]"), port.to_string())
            }
            None => return host,
        }
    } else {
        let mut parts = host.splitn(2, ':');
        let name = parts.next().unwrap_or_default().to_string();
        let port = parts.next().unwrap_or_default().to_string();
        if port.contains(':') || name.contains(['[', ']']) || port.contains(['[', ']']) {
            return host;
        }
        (name, port)
    };
    let name = name.strip_suffix('.').map(str::to_string).unwrap_or(name);
    let is_default = match scheme {
        Some("http") => port == "80",
        Some("https") => port == "443",
        _ => port == "80" || port == "443",
    };
    if port.is_empty() || is_default { name } else { format!("{name}:{port}") }
}

/// Without a public URL, a DNS-rebinding attacker's domain makes Origin equal
/// Host, so the CSRF check alone would pass; only known names get in.
pub fn host_is_allowed(host: Option<&str>, allowed: &[String]) -> bool {
    let Some(host) = host.filter(|h| !h.is_empty()) else {
        return false;
    };
    let host = normalize_host(host, None);
    if let Some(rest) = host.strip_prefix('[') {
        let Some((literal, after)) = rest.split_once(']') else {
            return false;
        };
        return (after.is_empty() || after.starts_with(':')) && literal.parse::<std::net::Ipv6Addr>().is_ok();
    }
    let name = host.split(':').next().unwrap_or_default();
    if name.parse::<std::net::Ipv4Addr>().is_ok() || name == "localhost" || name.ends_with(".localhost") {
        return true;
    }
    allowed.iter().map(|e| normalize_host(e, None)).filter(|e| !e.is_empty()).any(|entry| match entry.strip_prefix('.') {
        Some(base) => name == base || name.ends_with(&entry),
        None => name == entry,
    })
}

/// React Router's SPA build inlines its bootstrap scripts, so they are
/// allowed by hash rather than with 'unsafe-inline'.
pub fn content_security_policy(index_html: Option<&Path>) -> String {
    let html = index_html.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let pattern = regex::Regex::new(r"(?is)<script\b([^>]*)>(.*?)</script>").expect("valid pattern");
    let mut script_src = vec!["'self'".to_string()];
    for caps in pattern.captures_iter(&html) {
        let (attributes, body) = (&caps[1], &caps[2]);
        if attributes.contains("src=") || body.trim().is_empty() {
            continue;
        }
        let digest = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()));
        script_src.push(format!("'sha256-{digest}'"));
    }
    [
        "default-src 'self'".to_string(),
        format!("script-src {}", script_src.join(" ")),
        "style-src 'self' 'unsafe-inline'".into(),
        "img-src 'self' data:".into(),
        "font-src 'self' data:".into(),
        "connect-src 'self'".into(),
        "object-src 'none'".into(),
        "base-uri 'self'".into(),
        "form-action 'self'".into(),
        "frame-ancestors 'none'".into(),
    ]
    .join("; ")
}

pub fn is_public(method: &Method, path: &str) -> bool {
    let method = if method == Method::HEAD { "GET" } else { method.as_str() };
    if PUBLIC_ROUTES.contains(&(method, path)) || (method == "GET" && path.starts_with("/assets/")) {
        return true;
    }
    let first = path.trim_start_matches('/').split('/').next().unwrap_or_default();
    method == "GET" && !SPA_RESERVED_PREFIXES.contains(&first)
}

fn is_unsafe(method: &Method) -> bool {
    matches!(*method, Method::POST | Method::PUT | Method::PATCH | Method::DELETE)
}

pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.trim_matches('"'))
}

/// `urllib.parse.quote(value, safe="")`.
pub fn quote(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// How the current request reached the app and who sent it.
#[derive(Clone, Debug)]
pub struct RequestContext {
    pub peer: Option<IpAddr>,
    pub client_ip: String,
    pub https: bool,
    pub auth: Option<AuthState>,
}

impl RequestContext {
    pub fn session_cookie(&self) -> &'static str {
        if self.https { SECURE_SESSION_COOKIE } else { SESSION_COOKIE }
    }

    pub fn device_cookie(&self) -> &'static str {
        if self.https { SECURE_DEVICE_COOKIE } else { DEVICE_COOKIE }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthState {
    pub method: String,
    pub token_hash: Option<String>,
    pub public_id: Option<String>,
    pub user: Option<String>,
}

impl AuthState {
    pub fn of(method: &str) -> Self {
        Self { method: method.into(), token_hash: None, public_id: None, user: None }
    }
}

enum Authenticated {
    Yes(AuthState),
    No,
    Rejected,
}

fn authenticate(state: &AppState, headers: &HeaderMap, ctx: &RequestContext) -> Result<Authenticated, rusqlite::Error> {
    let auth = &state.auth;
    if auth.disabled {
        return Ok(Authenticated::Yes(AuthState::of("disabled")));
    }
    if let Some(proxy) = &auth.proxy {
        if let Some(value) = headers.get(proxy.header.as_str()).and_then(|v| v.to_str().ok()) {
            if proxy.trusts(ctx.peer) {
                if value == proxy.user {
                    return Ok(Authenticated::Yes(AuthState { user: Some(value.into()), ..AuthState::of("proxy") }));
                }
                crate::log_warn!("auth_guard", "proxy at {:?} asserted {}={value:?}, which is not the configured owner", ctx.peer, proxy.header);
                return Ok(Authenticated::Rejected);
            }
            crate::log_warn!("auth_guard", "ignored {} from untrusted peer {:?}; is something bypassing the proxy?", proxy.header, ctx.peer);
        }
    }
    let Some(token) = cookie(headers, ctx.session_cookie()).filter(|t| !t.is_empty()) else {
        return Ok(Authenticated::No);
    };
    let now = state.scheduler.now();
    Ok(match auth_core::resolve_session(&state.db.lock(), token, now)? {
        Some(session) => Authenticated::Yes(AuthState {
            method: session.method,
            token_hash: Some(session.token_hash),
            public_id: Some(session.public_id),
            user: None,
        }),
        None => Authenticated::No,
    })
}

fn origin_of(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let scheme = scheme.to_lowercase();
            let netloc = rest.split(['/', '?', '#']).next().unwrap_or_default();
            let netloc = netloc.rsplit('@').next().unwrap_or_default();
            format!("{scheme}://{}", normalize_host(netloc, Some(&scheme)))
        }
        None => url.to_string(),
    }
}

pub fn csrf_problem(headers: &HeaderMap, expected_origin: &str) -> Option<String> {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let expected = origin_of(expected_origin);
    if get("x-activsync-request") != Some("1") {
        return Some("missing X-ActivSync-Request header".into());
    }
    let origin = get("origin");
    if origin == Some("null") {
        return Some("opaque Origin".into());
    }
    if let Some(site) = get("sec-fetch-site") {
        return (site != "same-origin").then(|| format!("Sec-Fetch-Site is {site}"));
    }
    if let Some(origin) = origin.filter(|o| !o.is_empty()) {
        return (origin_of(origin) != expected).then(|| format!("Origin {origin} does not match"));
    }
    if let Some(referer) = get("referer").filter(|r| !r.is_empty()) {
        return (origin_of(referer) != expected).then(|| "Referer does not match".into());
    }
    Some("no Origin or Referer".into())
}

fn detail(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "detail": message }))).into_response()
}

fn add_security_headers(response: &mut Response, csp: &str, https: bool) {
    let headers = response.headers_mut();
    let mut set = |name: &'static str, value: &str| {
        if !headers.contains_key(name) {
            if let Ok(value) = HeaderValue::from_str(value) {
                headers.insert(name, value);
            }
        }
    };
    set("content-security-policy", csp);
    set("referrer-policy", "same-origin");
    set("x-content-type-options", "nosniff");
    if https {
        set("strict-transport-security", "max-age=31536000");
    }
}

/// The whole guard as one middleware, in the order the Python app stacks its
/// layers. The peer is read before X-Forwarded-For is applied, because only
/// the real socket peer may decide who can assert the proxy sign-in header.
pub async fn guard(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let extensions = request.extensions();
    let peer = extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())
        .or_else(|| extensions.get::<MockConnectInfo<SocketAddr>>().map(|c| c.0.ip()));
    let headers = request.headers().clone();
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let mut client_ip = peer.map(|p| p.to_string()).unwrap_or_default();
    let mut https = false;
    if peer.is_some_and(|p| state.auth.forwarded.trusts(&p.to_string())) {
        if let Some(proto) = get("x-forwarded-proto").map(|p| p.trim().to_lowercase()) {
            if matches!(proto.as_str(), "http" | "https" | "ws" | "wss") {
                https = proto == "https" || proto == "wss";
            }
        }
        if let Some(forwarded) = get("x-forwarded-for") {
            client_ip = state.auth.forwarded.client_from(forwarded);
        }
    }
    if state.auth.force_https() {
        https = true;
    }
    let csp = state.auth.csp.clone();
    let mut response = guarded(state, request, next, peer, client_ip, https).await;
    add_security_headers(&mut response, &csp, https);
    response
}

async fn guarded(
    state: AppState,
    mut request: Request,
    next: Next,
    peer: Option<IpAddr>,
    client_ip: String,
    https: bool,
) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().clone();
    let headers = request.headers().clone();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let public_origin = state.auth.public_origin();

    if path != "/health" {
        let accepted = match &public_origin {
            Some((scheme, expected)) => normalize_host(host.unwrap_or_default(), Some(scheme)) == *expected,
            None => host_is_allowed(host, &state.auth.allowed_hosts),
        };
        if !accepted {
            crate::log_warn!("auth_guard", "rejected a request for Host {:?}", host.unwrap_or_default().chars().take(255).collect::<String>());
            let message = if public_origin.is_some() { PUBLIC_HOST_MESSAGE } else { UNKNOWN_HOST_MESSAGE };
            return (StatusCode::BAD_REQUEST, message).into_response();
        }
    }

    let mut ctx = RequestContext { peer, client_ip, https, auth: None };
    let auth = match authenticate(&state, &headers, &ctx) {
        Ok(auth) => auth,
        Err(err) => {
            crate::log_error!("auth_guard", "database error while signing in: {err}");
            return detail(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
        }
    };
    ctx.auth = match auth {
        Authenticated::Rejected => return detail(StatusCode::UNAUTHORIZED, "Not signed in"),
        Authenticated::Yes(auth) => Some(auth),
        Authenticated::No => None,
    };

    if ctx.auth.is_none() && !is_public(&method, &path) {
        if path.starts_with("/api/") {
            return detail(StatusCode::UNAUTHORIZED, "Not signed in");
        }
        let target = match request.uri().query() {
            Some(query) => format!("{path}?{query}"),
            None => path.clone(),
        };
        return Redirect::to(&format!("/login?next={}", quote(&target))).into_response();
    }

    if is_unsafe(&method) {
        let expected = match &public_origin {
            Some((scheme, host)) => format!("{scheme}://{host}"),
            None => format!("{}://{}", if https { "https" } else { "http" }, host.unwrap_or_default()),
        };
        if let Some(problem) = csrf_problem(&headers, &expected) {
            crate::log_warn!("auth_guard", "blocked {method} {path}: {problem}");
            return detail(StatusCode::FORBIDDEN, "This request was blocked as cross-site.");
        }
    }

    if ctx.auth.is_none() && is_unsafe(&method) {
        let declared = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<usize>().ok());
        if declared.is_some_and(|n| n > PUBLIC_BODY_LIMIT) {
            return detail(StatusCode::PAYLOAD_TOO_LARGE, TOO_LARGE);
        }
        let (parts, body) = request.into_parts();
        let Ok(bytes) = axum::body::to_bytes(body, PUBLIC_BODY_LIMIT).await else {
            return detail(StatusCode::PAYLOAD_TOO_LARGE, TOO_LARGE);
        };
        request = Request::from_parts(parts, Body::from(bytes));
    }

    request.extensions_mut().insert(ctx);
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_rules_match_python() {
        let allowed = vec!["activsync.lan".to_string(), ".example.com".to_string()];
        let subdomains = vec![".activsync.lan".to_string()];
        for (host, first, second) in [
            ("LocalHost:8381", true, true),
            ("127.0.0.1:80", true, true),
            ("[::1]:8381", true, true),
            ("evil.com", false, false),
            ("activsync.lan:8381", true, true),
            ("a.activsync.lan", false, true),
            ("foo.localhost.", true, true),
        ] {
            assert_eq!(host_is_allowed(Some(host), &allowed), first, "{host}");
            assert_eq!(host_is_allowed(Some(host), &subdomains), second, "{host}");
        }
        assert_eq!(normalize_host("Example.COM.:443", Some("https")), "example.com");
        assert_eq!(normalize_host("example.com:80", None), "example.com");
        assert_eq!(normalize_host("example.com:8080", Some("http")), "example.com:8080");
    }

    #[test]
    fn csp_hashes_inline_scripts_like_python() {
        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("index.html");
        std::fs::write(&index, "<html><script>window.__x=1;</script><script src=\"/a.js\"></script><SCRIPT type=\"module\">\nimport \"/b.js\";\n</SCRIPT></html>").unwrap();
        let csp = content_security_policy(Some(&index));
        assert!(csp.contains("script-src 'self' 'sha256-GSSatg7gUwSPT29G1ec+dGak8j9ROqNcU/EVwJlQKR8=' 'sha256-0m4Ci6KYl0BDgrQIXNQexRzunmX5ARwrRbPWX0n33JI='"), "{csp}");
        assert!(csp.ends_with("frame-ancestors 'none'"));
    }

    #[test]
    fn forwarded_for_takes_rightmost_untrusted() {
        let trust = ForwardedTrust::parse("127.0.0.1, 10.0.0.0/8");
        assert_eq!(trust.client_from("203.0.113.9, 10.1.2.3"), "203.0.113.9");
        assert_eq!(trust.client_from("1.1.1.1, 203.0.113.9"), "203.0.113.9");
        assert_eq!(trust.client_from("10.0.0.1, 127.0.0.1"), "10.0.0.1");
        assert_eq!(ForwardedTrust::parse("*").client_from("1.1.1.1, 2.2.2.2"), "1.1.1.1");
    }

    #[test]
    fn proxy_config_errors_match_python() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
        };
        assert!(ProxyAuth::from_env(env(&[]), &[]).unwrap().is_none());
        let err = ProxyAuth::from_env(env(&[("ACTIVSYNC_AUTH_PROXY_HEADER", "Remote-User")]), &[]).unwrap_err();
        assert!(err.contains("must list the proxy's address range"));
        let wide = &[("ACTIVSYNC_AUTH_PROXY_HEADER", "Remote-User"), ("ACTIVSYNC_AUTH_PROXY_TRUSTED", "0.0.0.0/0"), ("ACTIVSYNC_AUTH_PROXY_USER", "me")];
        assert_eq!(ProxyAuth::from_env(env(wide), &[]).unwrap_err(), "ACTIVSYNC_AUTH_PROXY_TRUSTED must not trust every address (0.0.0.0/0).");
        let docker = &[("ACTIVSYNC_AUTH_PROXY_HEADER", "Remote-User"), ("ACTIVSYNC_AUTH_PROXY_TRUSTED", "172.18.0.0/16"), ("ACTIVSYNC_AUTH_PROXY_USER", "me")];
        let gateway: IpAddr = "172.18.0.1".parse().unwrap();
        assert!(ProxyAuth::from_env(env(docker), &[gateway]).unwrap_err().contains("default gateway (172.18.0.1)"));
        let proxy = ProxyAuth::from_env(env(docker), &[]).unwrap().unwrap();
        assert!(proxy.trusts(Some("172.18.4.2".parse().unwrap())));
        assert!(!proxy.trusts(Some("172.19.0.2".parse().unwrap())));
        assert_eq!(proxy.trusted[0].to_string(), "172.18.0.0/16");
    }
}
