//! Signing in to Garmin with the account's email and password, the one path
//! allowed to use them. It follows the Python `garminconnect` library's
//! mobile sign-in: Garmin's SSO answers with a service ticket, or asks for a
//! one-time code first, and the ticket is exchanged for the DI tokens the
//! rest of the client uses. The library's browser-impersonating fallbacks
//! have no counterpart here.

use std::time::Duration;

use serde_json::{Value, json};

use crate::garmin_client::{self, CONNECTAPI_URL, DI_TOKEN_URL, GarminClient, GarminError, Tokens, basic_auth, client_id_of, native_headers};
use openidconnect::{HttpResponse, http};

use crate::net::{self, Http};
use crate::schedule::Clock;

const SSO_URL: &str = "https://sso.garmin.com";
const IOS_SSO_CLIENT_ID: &str = "GCM_IOS_DARK";
const IOS_SERVICE_URL: &str = "https://mobile.integration.garmin.com/gcm/ios";
const IOS_LOGIN_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";
const PORTAL_SSO_CLIENT_ID: &str = "GarminConnect";
const PORTAL_SERVICE_URL: &str = "https://connect.garmin.com/app";
const DI_GRANT_TYPE: &str = "https://connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket";
const DI_CLIENT_IDS: [&str; 4] = [
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI",
    "GARMIN_CONNECT_MOBILE_IOS_DI",
];

/// A sign-in paused on Garmin's one-time code. A wrong code leaves it
/// usable, so the owner can try again against the same challenge.
#[derive(Clone, Debug, Default)]
pub struct PendingLogin {
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
    fn into_error(self) -> GarminError {
        GarminError::Invalid(match self {
            Self::Auth(message) => message,
            Self::RateLimited(_) => "All login strategies rate limited (429). Try again later or check your IP/network.".into(),
            Self::Connection(message) => format!("Login failed: All login strategies exhausted: {message}"),
        })
    }
}

/// Where sign-in reaches Garmin and keeps the session it gets.
#[derive(Clone)]
pub struct SignIn {
    pub http: Http,
    pub clock: Clock,
    pub token_dir: String,
    /// The pause between profile retries; zero in tests.
    pub pause: Duration,
}

fn login_params() -> [(&'static str, &'static str); 3] {
    [("clientId", IOS_SSO_CLIENT_ID), ("locale", "en-US"), ("service", IOS_SERVICE_URL)]
}

/// `name=value` of every cookie Garmin set, as one `Cookie` header.
fn cookies_of(response: &HttpResponse) -> String {
    response
        .headers()
        .get_all(http::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .map(str::trim)
        .filter(|pair| pair.contains('='))
        .collect::<Vec<_>>()
        .join("; ")
}

fn status_type(res: &Value) -> Option<&str> {
    res.pointer("/responseStatus/type").and_then(Value::as_str)
}

fn rate_limited_in_body(res: &Value) -> bool {
    res.pointer("/error/status-code").and_then(Value::as_str) == Some("429")
}

impl SignIn {
    async fn send(&self, method: http::Method, url: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Result<HttpResponse, String> {
        let request = net::request(method, url, headers, body).map_err(|e| e.0)?;
        (self.http)(request).await.map_err(|e| e.0)
    }

    /// Starts a sign-in. A session Garmin still accepts is kept without
    /// checking the password, as the library does with a token store.
    pub async fn begin(&self, email: &str, password: &str) -> Result<Login, GarminError> {
        if GarminClient::open_with(self.http.clone(), self.clock.clone(), &self.token_dir, self.pause).await.is_ok() {
            return Ok(Login::Connected);
        }
        match self.mobile_login(email, password).await.map_err(Failure::into_error)? {
            Attempt::Session(tokens) => {
                self.save(&tokens)?;
                Ok(Login::Connected)
            }
            Attempt::Challenged(pending) => Ok(Login::MfaRequired(pending)),
        }
    }

    /// Finishes a paused sign-in with the owner's code and keeps the session.
    pub async fn complete(&self, pending: &PendingLogin, code: &str) -> Result<(), GarminError> {
        let tokens = self.verify_code(pending, code).await.map_err(|failure| {
            GarminError::Invalid(match failure {
                Failure::Auth(message) | Failure::RateLimited(message) | Failure::Connection(message) => message,
            })
        })?;
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
        let url = net::with_query(&format!("{SSO_URL}/mobile/api/login"), &login_params());
        let body = net::json_body(&json!({"username": email, "password": password, "rememberMe": true, "captchaToken": ""}));
        let headers = login_headers();
        let response = self.send(http::Method::POST, &url, &headers, body).await.map_err(Failure::Connection)?;
        let status = response.status().as_u16();
        if status == 429 {
            return Err(Failure::RateLimited("Mobile login returned 429 — IP rate limited by Garmin".into()));
        }
        if status == 403 {
            return Err(Failure::Connection("Mobile login: HTTP 403 (Cloudflare bot challenge)".into()));
        }
        let Ok(res) = serde_json::from_slice::<Value>(response.body()) else {
            return Err(Failure::Connection(format!("Mobile login failed (non-JSON): HTTP {status}")));
        };
        match status_type(&res) {
            Some("MFA_REQUIRED") => {
                let method = res.pointer("/customerMfaInfo/mfaLastMethodUsed").and_then(Value::as_str).unwrap_or("email");
                Ok(Attempt::Challenged(PendingLogin { mfa_method: method.to_string(), cookies: cookies_of(&response) }))
            }
            Some("SUCCESSFUL") => {
                let ticket = res.get("serviceTicketId").and_then(Value::as_str).unwrap_or_default();
                let tokens = self.exchange_ticket(ticket).await?;
                if !self.token_accepted(&tokens).await {
                    return Err(Failure::Connection("mobile+requests: token rejected by API tier".into()));
                }
                Ok(Attempt::Session(tokens))
            }
            Some("INVALID_USERNAME_PASSWORD") => Err(Failure::Auth("401 Unauthorized (Invalid Username or Password)".into())),
            _ if rate_limited_in_body(&res) => Err(Failure::RateLimited("Mobile login: 429 in JSON body".into())),
            Some("CAPTCHA_REQUIRED") => Err(Failure::Connection("Mobile login: CAPTCHA required (bot challenge)".into())),
            other => Err(Failure::Connection(format!(
                "Mobile login failed: HTTP {status}, responseStatus={}",
                other.unwrap_or("unknown")
            ))),
        }
    }

    /// Garmin's two code endpoints share the sign-in's cookies but may sit
    /// in different rate-limit buckets, so the second is tried too.
    async fn verify_code(&self, pending: &PendingLogin, code: &str) -> Result<Tokens, Failure> {
        let body = json!({"mfaMethod": pending.mfa_method, "mfaVerificationCode": code, "rememberMyBrowser": true,
                          "reconsentList": [], "mfaSetup": false});
        let portal_params = [("clientId", PORTAL_SSO_CLIENT_ID), ("locale", "en-US"), ("service", PORTAL_SERVICE_URL)];
        let endpoints = [
            net::with_query(&format!("{SSO_URL}/mobile/api/mfa/verifyCode"), &login_params()),
            net::with_query(&format!("{SSO_URL}/portal/api/mfa/verifyCode"), &portal_params),
        ];
        let mut headers = login_headers();
        if !pending.cookies.is_empty() {
            headers.push(("Cookie", &pending.cookies));
        }
        let mut failures = Vec::new();
        let mut rate_limited = 0;
        for url in &endpoints {
            let path = url.split('?').next().unwrap_or(url);
            let response = match self.send(http::Method::POST, url, &headers, net::json_body(&body)).await {
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
                return self.exchange_ticket(ticket).await;
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
    /// app identity in turn.
    async fn exchange_ticket(&self, ticket: &str) -> Result<Tokens, Failure> {
        for client_id in DI_CLIENT_IDS {
            let headers = native_headers(&[
                ("Authorization", basic_auth(client_id)),
                ("Accept", "application/json,text/html;q=0.9,*/*;q=0.8".into()),
                ("Content-Type", "application/x-www-form-urlencoded".into()),
                ("Cache-Control", "no-cache".into()),
            ]);
            let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let body = net::form(&[
                ("client_id", client_id),
                ("service_ticket", ticket),
                ("grant_type", DI_GRANT_TYPE),
                ("service_url", IOS_SERVICE_URL),
            ]);
            let Ok(response) = self.send(http::Method::POST, DI_TOKEN_URL, &headers, body).await else {
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
        let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let url = format!("{CONNECTAPI_URL}/userprofile-service/socialProfile");
        match self.send(http::Method::GET, &url, &headers, Vec::new()).await {
            Ok(response) => !matches!(response.status().as_u16(), 401 | 403),
            Err(_) => true,
        }
    }
}

fn login_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("User-Agent", IOS_LOGIN_UA),
        ("Accept", "application/json, text/plain, */*"),
        ("Content-Type", "application/json"),
        ("Origin", SSO_URL),
    ]
}
