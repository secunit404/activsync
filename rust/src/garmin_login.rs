//! Signing in to Garmin with the account's email and password, the one path
//! allowed to use them. Like the Python `garminconnect` library it tries the
//! mobile app's sign-in, then the SSO widget, then the web portal. Each
//! either gets a service ticket, which is exchanged for the DI tokens the
//! rest of the client uses, or a request for a one-time code. Wrong
//! credentials or a code request end the chain; anything else moves on to
//! the next route. As in the library, each route's requests look like a
//! browser down to the TLS handshake, and the mobile and portal routes try
//! several browsers in turn, since Cloudflare rate-limits each separately.
//! The library's further retries over plain TLS have no counterpart here.

mod sso;
mod widget;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use openidconnect::{HttpRequest, HttpResponse, http};
use serde_json::Value;

use crate::garmin_client::{
    self, CONNECTAPI_URL, DI_TOKEN_URL, GarminClient, GarminError, SOCIAL_PROFILE_PATH, Tokens,
    basic_auth, client_id_of, native_headers,
};
use crate::net::{self, Browser, Http};
use crate::schedule::Clock;

const SSO_URL: &str = "https://sso.garmin.com";
const DI_GRANT_TYPE: &str =
    "https://connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket";
const DI_CLIENT_IDS: [&str; 4] = [
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4",
    "GARMIN_CONNECT_MOBILE_ANDROID_DI",
    "GARMIN_CONNECT_MOBILE_IOS_DI",
];
const MAX_REDIRECTS: usize = 10;

#[derive(Clone, Copy)]
enum Route {
    Mobile,
    Widget,
    Portal,
}

impl Route {
    const ALL: [Self; 3] = [Self::Mobile, Self::Widget, Self::Portal];

    /// The browsers the library presents this route as, in its order.
    fn browsers(self) -> &'static [Browser] {
        match self {
            Self::Mobile => &[Browser::SafariIos, Browser::Safari, Browser::Chrome120],
            Self::Widget => &[Browser::Chrome],
            Self::Portal => &[
                Browser::Safari,
                Browser::SafariIos,
                Browser::Chrome120,
                Browser::Edge101,
                Browser::Chrome,
            ],
        }
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Mobile => "mobile",
            Self::Widget => "widget",
            Self::Portal => "portal",
        })
    }
}

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
    /// The browser the sign-in looked like, which the code goes in as too.
    browser: Option<Browser>,
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
    fn message(&self) -> &str {
        match self {
            Self::Auth(message) | Self::RateLimited(message) | Self::Connection(message) => message,
        }
    }
}

/// Where sign-in reaches Garmin and keeps the session it gets.
#[derive(Clone)]
pub struct SignIn {
    /// Must not follow redirects: sign-in follows them itself, collecting
    /// each hop's cookies.
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
        let set = response
            .headers()
            .get_all(http::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok());
        for pair in set.filter_map(|value| value.split(';').next()) {
            let Some((name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            self.0.retain(|(n, _)| n != name);
            self.0.push((name.to_string(), value.to_string()));
        }
    }

    fn from_header(header: &str) -> Self {
        let pairs = header
            .split(';')
            .filter_map(|pair| pair.trim().split_once('='));
        Self(pairs.map(|(n, v)| (n.to_string(), v.to_string())).collect())
    }

    fn header(&self) -> String {
        self.0
            .iter()
            .map(|(n, v)| format!("{n}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

impl SignIn {
    async fn send(
        &self,
        method: http::Method,
        url: &str,
        headers: &[(&str, String)],
        jar: &Jar,
        body: Vec<u8>,
    ) -> Result<HttpResponse, String> {
        let cookie = jar.header();
        let mut all: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        if !cookie.is_empty() {
            all.push(("Cookie", &cookie));
        }
        let request = net::request(method, url, &all, body).map_err(|e| e.0)?;
        (self.http)(request).await.map_err(|e| e.0)
    }

    async fn get(
        &self,
        url: &str,
        headers: &[(&str, String)],
        jar: &mut Jar,
    ) -> Result<(HttpResponse, String), String> {
        self.follow(http::Method::GET, url, headers, jar, Vec::new())
            .await
    }

    /// Sends and follows redirects itself, keeping every hop's cookies, the
    /// way Python's `requests` does: after a 301, 302 or 303 a form post
    /// carries on as a plain GET; a 307 or 308 posts it again. Returns the
    /// final answer and the address it came from.
    async fn follow(
        &self,
        mut method: http::Method,
        url: &str,
        headers: &[(&str, String)],
        jar: &mut Jar,
        mut body: Vec<u8>,
    ) -> Result<(HttpResponse, String), String> {
        let mut url = url.to_string();
        let mut headers = headers.to_vec();
        for _ in 0..=MAX_REDIRECTS {
            let response = self
                .send(method.clone(), &url, &headers, jar, body.clone())
                .await?;
            jar.absorb(&response);
            let location = net::header(&response, "location").map(str::to_string);
            match location {
                Some(next) if response.status().is_redirection() => {
                    url = openidconnect::url::Url::parse(&url)
                        .and_then(|base| base.join(&next))
                        .map_err(|e| e.to_string())?
                        .to_string();
                    if method == http::Method::POST
                        && !matches!(response.status().as_u16(), 307 | 308)
                    {
                        method = http::Method::GET;
                        body = Vec::new();
                        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("Content-Type"));
                    }
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
        if GarminClient::open_with(
            self.http.clone(),
            self.clock.clone(),
            &self.token_dir,
            self.pause,
        )
        .await
        .is_ok()
        {
            return Ok(Login::Connected);
        }
        let mut last = None;
        let mut rate_limited = 0;
        for route in Route::ALL {
            match self.rotate(route, email, password).await {
                Ok(Attempt::Challenged(pending)) => return Ok(Login::MfaRequired(pending)),
                Ok(Attempt::Session(tokens)) if self.token_accepted(&tokens).await => {
                    self.save(&tokens)?;
                    return Ok(Login::Connected);
                }
                Ok(Attempt::Session(_)) => {
                    crate::log_warn!(
                        "garmin_login",
                        "Garmin {route} sign-in got a token the API rejected; trying the next route"
                    );
                    last = Some(format!("{route}: token rejected by API tier"));
                }
                Err(Failure::Auth(message)) => return Err(GarminError::Invalid(message)),
                Err(Failure::RateLimited(message)) => {
                    crate::log_warn!(
                        "garmin_login",
                        "Garmin {route} sign-in returned 429: {message}"
                    );
                    rate_limited += 1;
                    last = Some(message);
                }
                Err(Failure::Connection(message)) => {
                    crate::log_warn!("garmin_login", "Garmin {route} sign-in failed: {message}");
                    last = Some(message);
                }
            }
        }
        Err(GarminError::Invalid(if rate_limited == Route::ALL.len() {
            "All login strategies rate limited (429). Try again later or check your IP/network."
                .into()
        } else {
            format!(
                "Login failed: All login strategies exhausted: {}",
                last.unwrap_or_default()
            )
        }))
    }

    /// One route as each of its browsers in turn, until one gets past the
    /// rate limit or the bot check.
    async fn rotate(&self, route: Route, email: &str, password: &str) -> Result<Attempt, Failure> {
        let mut last = Failure::Connection("no browser to sign in as".into());
        for &browser in route.browsers() {
            let sign_in = self.presenting(browser);
            let attempt = match route {
                Route::Mobile => sign_in.mobile_login(email, password).await,
                Route::Widget => sign_in.widget_login(email, password).await,
                Route::Portal => sign_in.portal_login(email, password).await,
            };
            match attempt {
                Ok(Attempt::Challenged(pending)) => {
                    return Ok(Attempt::Challenged(PendingLogin {
                        browser: Some(browser),
                        ..pending
                    }));
                }
                Err(failure @ (Failure::RateLimited(_) | Failure::Connection(_))) => {
                    crate::log_debug!(
                        "garmin_login",
                        "Garmin sign-in as {browser:?} failed: {}",
                        failure.message()
                    );
                    last = failure;
                }
                outcome => return outcome,
            }
        }
        Err(last)
    }

    /// This sign-in with every request looking like `browser`, unless a call
    /// inside it has already picked one: the ticket exchange always looks
    /// like Chrome, whichever route got the ticket.
    fn presenting(&self, browser: Browser) -> SignIn {
        let http = self.http.clone();
        let http: Http = Arc::new(move |mut request: HttpRequest| {
            if request.extensions().get::<Browser>().is_none() {
                request.extensions_mut().insert(browser);
            }
            http(request)
        });
        SignIn {
            http,
            ..self.clone()
        }
    }

    /// Finishes a paused sign-in with the owner's code and keeps the session.
    pub async fn complete(&self, pending: &PendingLogin, code: &str) -> Result<(), GarminError> {
        let sign_in = pending
            .browser
            .map_or_else(|| self.clone(), |browser| self.presenting(browser));
        let verified = match &pending.flow {
            Flow::Widget { csrf, referer } => {
                sign_in
                    .verify_widget_code(pending, csrf, referer, code)
                    .await
            }
            Flow::Mobile | Flow::Portal => sign_in.verify_code(pending, code).await,
        };
        let tokens =
            verified.map_err(|failure| GarminError::Invalid(failure.message().to_string()))?;
        if !self.token_accepted(&tokens).await {
            return Err(GarminError::Invalid(
                "token rejected by API tier after MFA".into(),
            ));
        }
        GarminClient::signed_in(
            self.http.clone(),
            self.clock.clone(),
            tokens.clone(),
            self.pause,
        )
        .await?;
        self.save(&tokens)
    }

    fn save(&self, tokens: &Tokens) -> Result<(), GarminError> {
        garmin_client::save_tokens(&self.token_dir, tokens)
            .map_err(|e| GarminError::Invalid(format!("Could not save the Garmin session: {e}")))
    }

    /// Trades the SSO's service ticket for DI tokens, trying each known
    /// app identity in turn. `service` must be the one the ticket was for.
    async fn exchange_ticket(&self, ticket: &str, service: &str) -> Result<Tokens, Failure> {
        let chrome = self.presenting(Browser::Chrome);
        for client_id in DI_CLIENT_IDS {
            let headers = native_headers([
                ("Authorization", basic_auth(client_id)),
                (
                    "Accept",
                    "application/json,text/html;q=0.9,*/*;q=0.8".into(),
                ),
                ("Content-Type", "application/x-www-form-urlencoded".into()),
                ("Cache-Control", "no-cache".into()),
            ]);
            let body = net::form(&[
                ("client_id", client_id),
                ("service_ticket", ticket),
                ("grant_type", DI_GRANT_TYPE),
                ("service_url", service),
            ]);
            let Ok(response) = chrome
                .send(
                    http::Method::POST,
                    DI_TOKEN_URL,
                    &headers,
                    &Jar::default(),
                    body,
                )
                .await
            else {
                continue;
            };
            let status = response.status().as_u16();
            if status == 429 {
                return Err(Failure::RateLimited(
                    "DI token exchange rate limited".into(),
                ));
            }
            if !response.status().is_success() {
                continue;
            }
            let data: Value = serde_json::from_slice(response.body()).unwrap_or(Value::Null);
            let Some(access) = data.get("access_token").and_then(Value::as_str) else {
                continue;
            };
            return Ok(Tokens {
                di_token: Some(access.to_string()),
                di_refresh_token: data
                    .get("refresh_token")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                di_client_id: client_id_of(access).or_else(|| Some(client_id.to_string())),
            });
        }
        Err(Failure::Auth(
            "DI token exchange failed for all client IDs".into(),
        ))
    }

    /// Garmin's auth host can issue a token its API then refuses; only a
    /// definite 401/403 counts, so an outage never fails a good sign-in.
    async fn token_accepted(&self, tokens: &Tokens) -> bool {
        let bearer = format!("Bearer {}", tokens.di_token.as_deref().unwrap_or_default());
        let headers = native_headers([
            ("Authorization", bearer),
            ("Accept", "application/json".into()),
        ]);
        let url = format!("{CONNECTAPI_URL}{SOCIAL_PROFILE_PATH}");
        match self
            .send(
                http::Method::GET,
                &url,
                &headers,
                &Jar::default(),
                Vec::new(),
            )
            .await
        {
            Ok(response) => !matches!(response.status().as_u16(), 401 | 403),
            Err(_) => true,
        }
    }
}
