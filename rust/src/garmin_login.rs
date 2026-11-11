//! Signing in to Garmin with the account's email and password, the one path
//! allowed to use them. It follows the Python `garminconnect` library's
//! chain: the mobile app's sign-in first, then the SSO widget, then the web
//! portal. Each either gets a service ticket, which is exchanged for the DI
//! tokens the rest of the client uses, or a request for a one-time code.
//! Wrong credentials or a code request end the chain; anything else moves on
//! to the next route. The library also retries each route with
//! browser-impersonating TLS, which has no counterpart here.

use std::sync::LazyLock;
use std::time::Duration;

use openidconnect::{HttpResponse, http};
use regex::Regex;
use serde_json::{Value, json};

use crate::garmin_client::{self, CONNECTAPI_URL, DI_TOKEN_URL, GarminClient, GarminError, Tokens, basic_auth, client_id_of, native_headers};
use crate::net::{self, Http};
use crate::schedule::Clock;

const SSO_URL: &str = "https://sso.garmin.com";
const IOS_SSO_CLIENT_ID: &str = "GCM_IOS_DARK";
const IOS_SERVICE_URL: &str = "https://mobile.integration.garmin.com/gcm/ios";
const IOS_LOGIN_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";
const PORTAL_SSO_CLIENT_ID: &str = "GarminConnect";
const PORTAL_SERVICE_URL: &str = "https://connect.garmin.com/app";
const DESKTOP_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const DI_GRANT_TYPE: &str = "https://connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket";
const DI_CLIENT_IDS: [&str; 4] = [
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI",
    "GARMIN_CONNECT_MOBILE_IOS_DI",
];
const MAX_REDIRECTS: usize = 10;

static CSRF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"name="_csrf"\s+value="(.+?)""#).unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<title>(.+?)</title>").unwrap());
static TICKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\?ticket=(ST-[^"&\s]+)"#).unwrap());
static MFA_VARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+(customerGuid|mfaMethod|locale|clientId|codeSentTo)\s*=\s*"([^"]*)"\s*;"#).unwrap());

/// Which sign-in asked for the code, and what finishing it needs.
#[derive(Clone, Debug, Default)]
enum Flow {
    #[default]
    Mobile,
    Portal,
    Widget {
        csrf: String,
        referer: String,
    },
}

/// A sign-in paused on Garmin's one-time code. A wrong code leaves it
/// usable, so the owner can try again against the same challenge.
#[derive(Clone, Debug, Default)]
pub struct PendingLogin {
    flow: Flow,
    mfa_method: String,
    cookies: String,
}

pub enum Login {
    Connected,
    MfaRequired(PendingLogin),
}

enum Attempt {
    Session(Tokens),
    Challenged(PendingLogin),
}

/// Why one sign-in attempt stopped, in the library's terms.
enum Failure {
    /// Wrong credentials, or a code Garmin refused: trying again won't help.
    Auth(String),
    RateLimited(String),
    Connection(String),
}

impl Failure {
    fn message(self) -> String {
        match self {
            Self::Auth(message) | Self::RateLimited(message) | Self::Connection(message) => message,
        }
    }
}

/// Where sign-in reaches Garmin and keeps the session it gets.
#[derive(Clone)]
pub struct SignIn {
    /// Must not follow redirects: each hop's cookies are collected here.
    pub http: Http,
    pub clock: Clock,
    pub token_dir: String,
    /// The pause between profile retries; zero in tests, which also drops
    /// the waits the web routes keep before posting the password.
    pub pause: Duration,
}

/// The cookies Garmin set during one sign-in, sent back on each call.
#[derive(Default)]
struct Jar(Vec<(String, String)>);

impl Jar {
    fn absorb(&mut self, response: &HttpResponse) {
        let set = response.headers().get_all(http::header::SET_COOKIE).iter().filter_map(|v| v.to_str().ok());
        for pair in set.filter_map(|value| value.split(';').next()) {
            let Some((name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            self.0.retain(|(n, _)| n != name);
            self.0.push((name.to_string(), value.to_string()));
        }
    }

    fn header(&self) -> String {
        self.0.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join("; ")
    }
}

fn status_type(res: &Value) -> Option<&str> {
    res.pointer("/responseStatus/type").and_then(Value::as_str)
}

fn rate_limited_in_body(res: &Value) -> bool {
    res.pointer("/error/status-code").and_then(Value::as_str) == Some("429")
}

fn title_of(html: &str) -> String {
    TITLE.captures(html).map(|c| c[1].to_string()).unwrap_or_default()
}

fn mfa_vars(html: &str) -> Vec<(String, String)> {
    MFA_VARS.captures_iter(html).map(|c| (c[1].to_string(), c[2].to_string())).collect()
}

fn var<'a>(vars: &'a [(String, String)], name: &str) -> &'a str {
    vars.iter().rev().find(|(n, _)| n == name).map_or("", |(_, v)| v.as_str())
}

fn mobile_params() -> [(&'static str, &'static str); 3] {
    [("clientId", IOS_SSO_CLIENT_ID), ("locale", "en-US"), ("service", IOS_SERVICE_URL)]
}

fn portal_params() -> [(&'static str, &'static str); 3] {
    [("clientId", PORTAL_SSO_CLIENT_ID), ("locale", "en-US"), ("service", PORTAL_SERVICE_URL)]
}

fn widget_params() -> Vec<(&'static str, String)> {
    let base = format!("{SSO_URL}/sso");
    let embed = format!("{base}/embed");
    vec![
        ("id", "gauth-widget".into()),
        ("embedWidget", "true".into()),
        ("gauthHost", embed.clone()),
        ("service", embed.clone()),
        ("source", embed.clone()),
        ("redirectAfterAccountLoginUrl", embed.clone()),
        ("redirectAfterAccountCreationUrl", embed),
    ]
}

fn mobile_headers() -> Vec<(&'static str, String)> {
    vec![
        ("User-Agent", IOS_LOGIN_UA.into()),
        ("Accept", "application/json, text/plain, */*".into()),
        ("Content-Type", "application/json".into()),
        ("Origin", SSO_URL.into()),
    ]
}

fn portal_headers() -> Vec<(&'static str, String)> {
    vec![
        ("User-Agent", DESKTOP_USER_AGENT.into()),
        ("Accept", "application/json, text/plain, */*".into()),
        ("Accept-Language", "en-US,en;q=0.9".into()),
        ("Content-Type", "application/json".into()),
        ("Origin", SSO_URL.into()),
        ("Referer", format!("{SSO_URL}/portal/sso/en-US/sign-in?clientId={PORTAL_SSO_CLIENT_ID}&service={PORTAL_SERVICE_URL}")),
    ]
}

fn query<'a>(pairs: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    pairs.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

impl SignIn {
    async fn send(&self, method: http::Method, url: &str, headers: &[(&str, String)], jar: &Jar, body: Vec<u8>) -> Result<HttpResponse, String> {
        let cookie = jar.header();
        let mut all: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        if !cookie.is_empty() {
            all.push(("Cookie", &cookie));
        }
        let request = net::request(method, url, &all, body).map_err(|e| e.0)?;
        (self.http)(request).await.map_err(|e| e.0)
    }

    /// A GET that follows redirects itself, keeping every hop's cookies.
    /// Returns the final answer and the address it came from.
    async fn get(&self, url: &str, headers: &[(&str, String)], jar: &mut Jar) -> Result<(HttpResponse, String), String> {
        let mut url = url.to_string();
        for _ in 0..=MAX_REDIRECTS {
            let response = self.send(http::Method::GET, &url, headers, jar, Vec::new()).await?;
            jar.absorb(&response);
            let location = net::header(&response, "location").map(str::to_string);
            match location {
                Some(next) if response.status().is_redirection() => {
                    url = openidconnect::url::Url::parse(&url).and_then(|base| base.join(&next)).map_err(|e| e.to_string())?.to_string();
                }
                _ => return Ok((response, url)),
            }
        }
        Err(format!("too many redirects from {url}"))
    }

    async fn wait_like_a_browser(&self, seconds: std::ops::Range<f64>) {
        if !self.pause.is_zero() {
            tokio::time::sleep(Duration::from_secs_f64(rand::random_range(seconds))).await;
        }
    }

    /// Starts a sign-in. A session Garmin still accepts is kept without
    /// checking the password, as the library does with a token store.
    pub async fn begin(&self, email: &str, password: &str) -> Result<Login, GarminError> {
        if GarminClient::open_with(self.http.clone(), self.clock.clone(), &self.token_dir, self.pause).await.is_ok() {
            return Ok(Login::Connected);
        }
        let mut last = None;
        let mut rate_limited = 0;
        let routes = ["mobile", "widget", "portal"];
        for route in routes {
            let attempt = match route {
                "mobile" => self.mobile_login(email, password).await,
                "widget" => self.widget_login(email, password).await,
                _ => self.portal_login(email, password).await,
            };
            match attempt {
                Ok(Attempt::Challenged(pending)) => return Ok(Login::MfaRequired(pending)),
                Ok(Attempt::Session(tokens)) if self.token_accepted(&tokens).await => {
                    self.save(&tokens)?;
                    return Ok(Login::Connected);
                }
                Ok(Attempt::Session(_)) => {
                    crate::log_warn!("garmin_login", "Garmin {route} sign-in got a token the API rejected; trying the next route");
                    last = Some(format!("{route}: token rejected by API tier"));
                }
                Err(Failure::Auth(message)) => return Err(GarminError::Invalid(message)),
                Err(Failure::RateLimited(message)) => {
                    crate::log_warn!("garmin_login", "Garmin {route} sign-in returned 429: {message}");
                    rate_limited += 1;
                    last = Some(message);
                }
                Err(Failure::Connection(message)) => {
                    crate::log_warn!("garmin_login", "Garmin {route} sign-in failed: {message}");
                    last = Some(message);
                }
            }
        }
        Err(GarminError::Invalid(if rate_limited == routes.len() {
            "All login strategies rate limited (429). Try again later or check your IP/network.".into()
        } else {
            format!("Login failed: All login strategies exhausted: {}", last.unwrap_or_default())
        }))
    }

    /// Finishes a paused sign-in with the owner's code and keeps the session.
    pub async fn complete(&self, pending: &PendingLogin, code: &str) -> Result<(), GarminError> {
        let verified = match &pending.flow {
            Flow::Widget { csrf, referer } => self.verify_widget_code(pending, csrf, referer, code).await,
            Flow::Mobile | Flow::Portal => self.verify_code(pending, code).await,
        };
        let tokens = verified.map_err(|failure| GarminError::Invalid(failure.message()))?;
        if !self.token_accepted(&tokens).await {
            return Err(GarminError::Invalid("token rejected by API tier after MFA".into()));
        }
        GarminClient::signed_in(self.http.clone(), self.clock.clone(), tokens.clone(), self.pause).await?;
        self.save(&tokens)
    }

    fn save(&self, tokens: &Tokens) -> Result<(), GarminError> {
        garmin_client::save_tokens(&self.token_dir, tokens).map_err(|e| GarminError::Invalid(format!("Could not save the Garmin session: {e}")))
    }

    async fn mobile_login(&self, email: &str, password: &str) -> Result<Attempt, Failure> {
        let url = net::with_query(&format!("{SSO_URL}/mobile/api/login"), &mobile_params());
        self.json_login(&url, &mobile_headers(), Jar::default(), email, password, Flow::Mobile).await
    }

    /// The desktop portal: its sign-in page first for cookies, then a pause
    /// as a person would take to type, which Cloudflare looks for.
    async fn portal_login(&self, email: &str, password: &str) -> Result<Attempt, Failure> {
        let page = net::with_query(
            &format!("{SSO_URL}/portal/sso/en-US/sign-in"),
            &[("clientId", PORTAL_SSO_CLIENT_ID), ("service", PORTAL_SERVICE_URL)],
        );
        let page_headers = [
            ("User-Agent", DESKTOP_USER_AGENT.to_string()),
            ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".into()),
            ("Accept-Language", "en-US,en;q=0.9".into()),
        ];
        let mut jar = Jar::default();
        let (response, _) = self.get(&page, &page_headers, &mut jar).await.map_err(Failure::Connection)?;
        if response.status().as_u16() == 429 {
            return Err(Failure::RateLimited("Portal login GET returned 429 — Cloudflare blocking this request.".into()));
        }
        self.wait_like_a_browser(10.0..20.0).await;
        let url = net::with_query(&format!("{SSO_URL}/portal/api/login"), &portal_params());
        self.json_login(&url, &portal_headers(), jar, email, password, Flow::Portal).await
    }

    /// The SSO's JSON sign-in, shared by the mobile and portal routes.
    async fn json_login(&self, url: &str, headers: &[(&str, String)], mut jar: Jar, email: &str, password: &str, flow: Flow) -> Result<Attempt, Failure> {
        let name = if matches!(flow, Flow::Portal) { "Portal" } else { "Mobile" };
        let body = net::json_body(&json!({"username": email, "password": password, "rememberMe": true, "captchaToken": ""}));
        let response = self.send(http::Method::POST, url, headers, &jar, body).await.map_err(Failure::Connection)?;
        jar.absorb(&response);
        let status = response.status().as_u16();
        if status == 429 {
            return Err(Failure::RateLimited(format!("{name} login returned 429 — IP rate limited by Garmin")));
        }
        if status == 403 {
            return Err(Failure::Connection(format!("{name} login: HTTP 403 (Cloudflare bot challenge)")));
        }
        let Ok(res) = serde_json::from_slice::<Value>(response.body()) else {
            return Err(Failure::Connection(format!("{name} login failed (non-JSON): HTTP {status}")));
        };
        let service = if matches!(flow, Flow::Portal) { PORTAL_SERVICE_URL } else { IOS_SERVICE_URL };
        match status_type(&res) {
            Some("MFA_REQUIRED") => {
                let method = res.pointer("/customerMfaInfo/mfaLastMethodUsed").and_then(Value::as_str).unwrap_or("email");
                Ok(Attempt::Challenged(PendingLogin { flow, mfa_method: method.to_string(), cookies: jar.header() }))
            }
            Some("SUCCESSFUL") => {
                let ticket = res.get("serviceTicketId").and_then(Value::as_str).unwrap_or_default();
                Ok(Attempt::Session(self.exchange_ticket(ticket, service).await?))
            }
            Some("INVALID_USERNAME_PASSWORD") => Err(Failure::Auth("401 Unauthorized (Invalid Username or Password)".into())),
            _ if rate_limited_in_body(&res) => Err(Failure::RateLimited(format!("{name} login: 429 in JSON body"))),
            Some("CAPTCHA_REQUIRED") => Err(Failure::Connection(format!("{name} login: CAPTCHA required (bot challenge)"))),
            other => Err(Failure::Connection(format!("{name} login failed: HTTP {status}, responseStatus={}", other.unwrap_or("unknown")))),
        }
    }

    /// The SSO's embeddable HTML sign-in form, which sits outside the
    /// per-app rate limits of the JSON sign-in.
    async fn widget_login(&self, email: &str, password: &str) -> Result<Attempt, Failure> {
        let base = format!("{SSO_URL}/sso");
        let embed = format!("{base}/embed");
        let mut jar = Jar::default();
        let embed_url = net::with_query(&embed, &[("id", "gauth-widget"), ("embedWidget", "true"), ("gauthHost", &base)]);
        let (response, _) = self.get(&embed_url, &[], &mut jar).await.map_err(Failure::Connection)?;
        match response.status().as_u16() {
            429 => return Err(Failure::RateLimited("Widget embed GET returned 429".into())),
            s if !(200..300).contains(&s) => return Err(Failure::Connection(format!("Widget embed returned {s}"))),
            _ => {}
        }
        let params = widget_params();
        let signin_url = net::with_query(&format!("{base}/signin"), &query(&params));
        let (page, page_url) = self.get(&signin_url, &[("Referer", embed.clone())], &mut jar).await.map_err(Failure::Connection)?;
        if page.status().as_u16() == 429 {
            return Err(Failure::RateLimited("Widget signin GET returned 429".into()));
        }
        let page_text = net::body_text(&page);
        let csrf = CSRF.captures(&page_text).map(|c| c[1].to_string()).ok_or(Failure::Connection("Widget login: missing CSRF token".into()))?;
        self.wait_like_a_browser(3.0..8.0).await;

        let form = net::form(&[("username", email), ("password", password), ("embed", "true"), ("_csrf", &csrf)]);
        let headers = [("Referer", page_url), ("Content-Type", "application/x-www-form-urlencoded".to_string())];
        let response = self.send(http::Method::POST, &signin_url, &headers, &jar, form).await.map_err(Failure::Connection)?;
        jar.absorb(&response);
        if response.status().as_u16() == 429 {
            return Err(Failure::RateLimited("Widget signin POST returned 429".into()));
        }
        let text = net::body_text(&response);
        let title = title_of(&text);
        let lower = title.to_lowercase();
        if ["bad gateway", "service unavailable", "cloudflare", "502", "503"].iter().any(|h| lower.contains(h)) {
            return Err(Failure::Connection(format!("Widget login: server error '{title}'")));
        }
        if ["locked", "invalid", "incorrect", "account error"].iter().any(|h| lower.contains(h)) {
            return Err(Failure::Auth(format!("Widget authentication failed: '{title}'")));
        }
        if lower.contains("unable to sign in") || lower.contains("unable to login") {
            return Err(Failure::Connection(format!("Widget login: account restricted '{title}'")));
        }
        let vars = mfa_vars(&text);
        let method = var(&vars, "mfaMethod");
        if lower.contains("mfa") || (lower.contains("authentication application") && !method.is_empty()) {
            self.request_widget_code(&vars, &signin_url, &jar).await?;
            let csrf = CSRF.captures(&text).map(|c| c[1].to_string()).unwrap_or_default();
            let flow = Flow::Widget { csrf, referer: signin_url };
            return Ok(Attempt::Challenged(PendingLogin { flow, mfa_method: method.to_string(), cookies: jar.header() }));
        }
        if title != "Success" {
            return Err(Failure::Connection(format!("Widget login: unexpected title '{title}'")));
        }
        let ticket = TICKET.captures(&text).map(|c| c[1].to_string()).ok_or(Failure::Connection("Widget login: missing service ticket".into()))?;
        Ok(Attempt::Session(self.exchange_ticket(&ticket, &embed).await?))
    }

    /// An email or text code may not go out with the password post, so it is
    /// asked for the way the widget's "Request a new code" link does.
    async fn request_widget_code(&self, vars: &[(String, String)], referer: &str, jar: &Jar) -> Result<(), Failure> {
        let method = var(vars, "mfaMethod");
        if !matches!(method.to_lowercase().as_str(), "email" | "sms") || !var(vars, "codeSentTo").is_empty() {
            return Ok(());
        }
        let url = net::with_query(&format!("{SSO_URL}/sso/verifyMFA/mfaCode"), &[("clientId", var(vars, "clientId"))]);
        let headers = [
            ("Content-Type", "application/json".to_string()),
            ("Accept", "application/json, text/plain, */*".into()),
            ("Referer", referer.to_string()),
        ];
        let body = net::json_body(&json!({"customerGuid": var(vars, "customerGuid"), "mfaMethod": method, "locale": var(vars, "locale")}));
        let response = self.send(http::Method::POST, &url, &headers, jar, body).await.map_err(Failure::Connection)?;
        match response.status().as_u16() {
            429 => Err(Failure::RateLimited("Widget MFA code request returned 429".into())),
            s if !(200..300).contains(&s) => Err(Failure::Connection(format!("Widget MFA code request returned {s}"))),
            _ => Ok(()),
        }
    }

    async fn verify_widget_code(&self, pending: &PendingLogin, csrf: &str, referer: &str, code: &str) -> Result<Tokens, Failure> {
        if csrf.is_empty() {
            return Err(Failure::Auth("Widget MFA: missing CSRF token".into()));
        }
        let url = net::with_query(&format!("{SSO_URL}/sso/verifyMFA/loginEnterMfaCode"), &query(&widget_params()));
        let headers = [
            ("Referer", referer.to_string()),
            ("Content-Type", "application/x-www-form-urlencoded".into()),
            ("Cookie", pending.cookies.clone()),
        ];
        let form = net::form(&[("mfa-code", code), ("embed", "true"), ("_csrf", csrf), ("fromPage", "setupEnterMfaCode")]);
        let response = self.send(http::Method::POST, &url, &headers, &Jar::default(), form).await.map_err(Failure::Connection)?;
        if response.status().as_u16() == 429 {
            return Err(Failure::RateLimited("Widget MFA verify returned 429".into()));
        }
        let text = net::body_text(&response);
        let title = title_of(&text);
        if title != "Success" {
            return Err(Failure::Auth(format!("Widget MFA failed: {title}")));
        }
        let ticket = TICKET.captures(&text).map(|c| c[1].to_string()).ok_or(Failure::Auth("Widget MFA: missing service ticket".into()))?;
        self.exchange_ticket(&ticket, &format!("{SSO_URL}/sso/embed")).await
    }

    /// Garmin's two JSON code endpoints share the sign-in's cookies but may
    /// sit in different rate-limit buckets, so the other is tried too.
    async fn verify_code(&self, pending: &PendingLogin, code: &str) -> Result<Tokens, Failure> {
        let body = json!({"mfaMethod": pending.mfa_method, "mfaVerificationCode": code, "rememberMyBrowser": true,
                          "reconsentList": [], "mfaSetup": false});
        let mobile = net::with_query(&format!("{SSO_URL}/mobile/api/mfa/verifyCode"), &mobile_params());
        let portal = net::with_query(&format!("{SSO_URL}/portal/api/mfa/verifyCode"), &portal_params());
        let (endpoints, mut headers, service) = match pending.flow {
            Flow::Portal => ([portal, mobile], portal_headers(), PORTAL_SERVICE_URL),
            _ => ([mobile, portal], mobile_headers(), IOS_SERVICE_URL),
        };
        if !pending.cookies.is_empty() {
            headers.push(("Cookie", pending.cookies.clone()));
        }
        let mut failures = Vec::new();
        let mut rate_limited = 0;
        for url in &endpoints {
            let path = url.split('?').next().unwrap_or(url);
            let response = match self.send(http::Method::POST, url, &headers, &Jar::default(), net::json_body(&body)).await {
                Ok(response) => response,
                Err(err) => {
                    failures.push(format!("{path}: connection error {err}"));
                    continue;
                }
            };
            if response.status().as_u16() == 429 {
                failures.push(format!("{path}: HTTP 429"));
                rate_limited += 1;
                continue;
            }
            let Ok(res) = serde_json::from_slice::<Value>(response.body()) else {
                failures.push(format!("{path}: HTTP {} non-JSON", response.status().as_u16()));
                continue;
            };
            if rate_limited_in_body(&res) {
                failures.push(format!("{path}: 429 in JSON body"));
                rate_limited += 1;
                continue;
            }
            if status_type(&res) == Some("SUCCESSFUL") {
                let ticket = res.get("serviceTicketId").and_then(Value::as_str).unwrap_or_default();
                return self.exchange_ticket(ticket, service).await;
            }
            let status = status_type(&res).or_else(|| res.pointer("/error/status-code").and_then(Value::as_str)).unwrap_or("unknown");
            failures.push(format!("{path}: {status}"));
        }
        let listed = failures.iter().map(|f| format!("'{f}'")).collect::<Vec<_>>().join(", ");
        if rate_limited == endpoints.len() {
            return Err(Failure::RateLimited(format!("MFA verification rate limited on all endpoints: [{listed}]")));
        }
        Err(Failure::Auth(format!("MFA verification failed: [{listed}]")))
    }

    /// Trades the SSO's service ticket for DI tokens, trying each known
    /// app identity in turn. `service` must be the one the ticket was for.
    async fn exchange_ticket(&self, ticket: &str, service: &str) -> Result<Tokens, Failure> {
        for client_id in DI_CLIENT_IDS {
            let headers = native_headers(&[
                ("Authorization", basic_auth(client_id)),
                ("Accept", "application/json,text/html;q=0.9,*/*;q=0.8".into()),
                ("Content-Type", "application/x-www-form-urlencoded".into()),
                ("Cache-Control", "no-cache".into()),
            ]);
            let headers: Vec<(&str, String)> = headers.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            let body = net::form(&[
                ("client_id", client_id),
                ("service_ticket", ticket),
                ("grant_type", DI_GRANT_TYPE),
                ("service_url", service),
            ]);
            let Ok(response) = self.send(http::Method::POST, DI_TOKEN_URL, &headers, &Jar::default(), body).await else {
                continue;
            };
            let status = response.status().as_u16();
            if status == 429 {
                return Err(Failure::RateLimited("DI token exchange rate limited".into()));
            }
            if !(200..300).contains(&status) {
                continue;
            }
            let data: Value = serde_json::from_slice(response.body()).unwrap_or(Value::Null);
            let Some(access) = data.get("access_token").and_then(Value::as_str) else {
                continue;
            };
            return Ok(Tokens {
                di_token: Some(access.to_string()),
                di_refresh_token: data.get("refresh_token").and_then(Value::as_str).map(str::to_string),
                di_client_id: client_id_of(access).or_else(|| Some(client_id.to_string())),
            });
        }
        Err(Failure::Auth("DI token exchange failed for all client IDs".into()))
    }

    /// Garmin's auth host can issue a token its API then refuses; only a
    /// definite 401/403 counts, so an outage never fails a good sign-in.
    async fn token_accepted(&self, tokens: &Tokens) -> bool {
        let bearer = format!("Bearer {}", tokens.di_token.as_deref().unwrap_or_default());
        let headers = native_headers(&[("Authorization", bearer), ("Accept", "application/json".into())]);
        let headers: Vec<(&str, String)> = headers.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let url = format!("{CONNECTAPI_URL}/userprofile-service/socialProfile");
        match self.send(http::Method::GET, &url, &headers, &Jar::default(), Vec::new()).await {
            Ok(response) => !matches!(response.status().as_u16(), 401 | 403),
            Err(_) => true,
        }
    }
}
