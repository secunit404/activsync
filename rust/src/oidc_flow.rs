//! The OIDC authorization-code flow with PKCE and nonce, as the Python app
//! runs it through Authlib. Per-browser flow state lives in memory behind
//! an opaque cookie, standing in for the Python app's signed session cookie.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreJsonWebKeySet, CoreJwsSigningAlgorithm,
    CoreProviderMetadata, CoreUserInfoClaims,
};
use openidconnect::{
    AsyncHttpClient, AuthorizationCode, OAuth2TokenResponse, ClientId, ClientSecret, CsrfToken, HttpRequest, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RequestTokenError, Scope, TokenResponse, http,
};
use serde::de::DeserializeOwned;

use crate::auth_core::random_token;
use crate::net::{self, Http, HttpError, HttpFuture};
use crate::oidc_config::OidcConfig;

const FLOW_SECONDS: f64 = 600.0;
pub const LINK_GRANT_SECONDS: f64 = 300.0;

/// Every call to the identity provider goes through this, so tests can stand
/// in for the provider.
pub type OidcHttp = Http;

pub fn real_http() -> OidcHttp {
    net::provider_http()
}

/// A named client rather than a closure: a closure here makes the handler's
/// future fail axum's `Send` check across the provider calls.
struct Caller<'a>(&'a OidcHttp);

impl<'c> AsyncHttpClient<'c> for Caller<'_> {
    type Error = HttpError;
    type Future = HttpFuture;

    fn call(&'c self, request: HttpRequest) -> HttpFuture {
        (self.0)(request)
    }
}

#[derive(Clone, Debug)]
pub struct Flow {
    pub intent: String,
    pub next: String,
    csrf: String,
    nonce: String,
    verifier: String,
    redirect_uri: String,
}

#[derive(Default)]
struct BrowserState {
    link_grant_until: Option<f64>,
    flow: Option<Flow>,
    touched: f64,
}

/// Flow and link-grant state per browser, keyed by the value of its OIDC
/// cookie. A restart forgets it, so a sign-in in progress then starts over.
#[derive(Clone, Default)]
pub struct OidcSessions(Arc<Mutex<HashMap<String, BrowserState>>>);

impl OidcSessions {
    fn with<T>(&self, key: Option<&str>, now: f64, change: impl FnOnce(&mut BrowserState) -> T) -> (String, T) {
        let mut map = self.0.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, s| now - s.touched < FLOW_SECONDS);
        let key = key.filter(|k| map.contains_key(*k)).map(str::to_string).unwrap_or_else(|| random_token(32));
        let entry = map.entry(key.clone()).or_default();
        entry.touched = now;
        (key, change(entry))
    }

    /// Returns the cookie value to send back.
    pub fn grant_link(&self, key: Option<&str>, now: f64) -> String {
        self.with(key, now, |s| s.link_grant_until = Some(now + LINK_GRANT_SECONDS)).0
    }

    pub fn consume_link_grant(&self, key: Option<&str>, now: f64) -> bool {
        let Some(key) = key else {
            return false;
        };
        let mut map = self.0.lock().unwrap_or_else(|p| p.into_inner());
        map.get_mut(key).and_then(|s| s.link_grant_until.take()).is_some_and(|until| now < until)
    }

    fn start_flow(&self, key: Option<&str>, now: f64, flow: Flow) -> String {
        self.with(key, now, |s| s.flow = Some(flow)).0
    }

    pub fn take_flow(&self, key: Option<&str>) -> Option<Flow> {
        let key = key?;
        self.0.lock().unwrap_or_else(|p| p.into_inner()).get_mut(key).and_then(|s| s.flow.take())
    }
}

#[derive(Debug, PartialEq)]
pub struct OidcError {
    pub code: &'static str,
    pub intent: String,
}

fn fail(code: &'static str, intent: &str) -> OidcError {
    OidcError { code, intent: intent.into() }
}

pub struct OidcResult {
    pub intent: String,
    pub next: String,
    pub iss: String,
    pub sub: String,
    pub label: String,
}

async fn get_json<T: DeserializeOwned>(http: &OidcHttp, url: &str) -> Result<T, String> {
    let request = http::Request::builder()
        .method(http::Method::GET)
        .uri(url)
        .header(http::header::ACCEPT, "application/json")
        .body(Vec::new())
        .map_err(|e| e.to_string())?;
    let response = http(request).await.map_err(|e| e.0)?;
    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }
    serde_json::from_slice(response.body()).map_err(|e| format!("{url}: {e}"))
}

pub fn discovery_url(issuer: &str) -> String {
    format!("{issuer}/.well-known/openid-configuration")
}

/// Discovery is read from the configured issuer as given; the ID token's
/// `iss` must then equal the issuer the document names, as Authlib checks.
async fn metadata(http: &OidcHttp, config: &OidcConfig) -> Result<CoreProviderMetadata, String> {
    let metadata: CoreProviderMetadata = get_json(http, &discovery_url(&config.issuer)).await?;
    let jwks: CoreJsonWebKeySet = get_json(http, metadata.jwks_uri().as_str()).await?;
    Ok(metadata.set_jwks(jwks))
}

fn client_for(
    metadata: CoreProviderMetadata,
    config: &OidcConfig,
    redirect_uri: &str,
) -> Result<
    CoreClient<
        openidconnect::EndpointSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointMaybeSet,
        openidconnect::EndpointMaybeSet,
    >,
    String,
> {
    let secret = Some(config.client_secret.clone()).filter(|s| !s.is_empty()).map(ClientSecret::new);
    Ok(CoreClient::from_provider_metadata(metadata, ClientId::new(config.client_id.clone()), secret)
        .set_redirect_uri(RedirectUrl::new(redirect_uri.into()).map_err(|e| e.to_string())?))
}

/// The provider's sign-in page to send the browser to, and the cookie value
/// that finds this flow again on the way back.
pub async fn begin(
    http: &OidcHttp,
    sessions: &OidcSessions,
    config: &OidcConfig,
    cookie: Option<&str>,
    now: f64,
    redirect_uri: &str,
    intent: &str,
    next: &str,
) -> Result<(String, String), OidcError> {
    let client = match metadata(http, config).await.and_then(|m| client_for(m, config, redirect_uri)) {
        Ok(client) => client,
        Err(err) => {
            crate::log_warn!("oidc_flow", "could not start OIDC sign-in with {}: {err}", config.issuer);
            return Err(fail("oidc_unavailable", intent));
        }
    };
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request =
        client.authorize_url(CoreAuthenticationFlow::AuthorizationCode, CsrfToken::new_random, Nonce::new_random);
    for scope in config.scopes.split_whitespace().filter(|s| *s != "openid") {
        request = request.add_scope(Scope::new(scope.into()));
    }
    let (url, csrf, nonce) = request.set_pkce_challenge(challenge).url();
    let flow = Flow {
        intent: intent.into(),
        next: next.into(),
        csrf: csrf.secret().clone(),
        nonce: nonce.secret().clone(),
        verifier: verifier.secret().clone(),
        redirect_uri: redirect_uri.into(),
    };
    Ok((url.to_string(), sessions.start_flow(cookie, now, flow)))
}

const ALLOWED_ALGS: [CoreJwsSigningAlgorithm; 9] = [
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha384,
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha512,
    CoreJwsSigningAlgorithm::RsaSsaPssSha256,
    CoreJwsSigningAlgorithm::RsaSsaPssSha384,
    CoreJwsSigningAlgorithm::RsaSsaPssSha512,
    CoreJwsSigningAlgorithm::EcdsaP256Sha256,
    CoreJwsSigningAlgorithm::EcdsaP384Sha384,
    CoreJwsSigningAlgorithm::EdDsa,
];

pub struct Callback<'a> {
    pub state: Option<&'a str>,
    pub code: Option<&'a str>,
    pub error: Option<&'a str>,
}

/// Checks the provider's answer against the flow it started: state, PKCE,
/// the ID token's signature, issuer, audience and nonce.
pub async fn finish(
    http: &OidcHttp,
    config: Option<&OidcConfig>,
    flow: Option<Flow>,
    callback: Callback<'_>,
) -> Result<OidcResult, OidcError> {
    let intent = flow.as_ref().map_or("login".to_string(), |f| f.intent.clone());
    let Some(config) = config else {
        return Err(fail("oidc_unavailable", &intent));
    };
    let Some(flow) = flow else {
        return Err(fail("oidc_state", &intent));
    };
    if callback.error == Some("access_denied") {
        return Err(fail("oidc_denied", &intent));
    }
    if callback.state != Some(flow.csrf.as_str()) {
        return Err(fail("oidc_state", &intent));
    }
    if let Some(error) = callback.error {
        crate::log_warn!("oidc_flow", "OIDC sign-in with {} failed: {error}", config.issuer);
        return Err(fail("oidc_unavailable", &intent));
    }
    let Some(code) = callback.code else {
        crate::log_warn!("oidc_flow", "OIDC provider {} returned no code", config.issuer);
        return Err(fail("oidc_unavailable", &intent));
    };
    let client = match metadata(http, config).await.and_then(|m| client_for(m, config, &flow.redirect_uri)) {
        Ok(client) => client,
        Err(err) => {
            crate::log_warn!("oidc_flow", "OIDC provider {} is unreachable: {err}", config.issuer);
            return Err(fail("oidc_unavailable", &intent));
        }
    };
    let caller = Caller(http);
    let exchange = client.exchange_code(AuthorizationCode::new(code.into())).map_err(|err| {
        crate::log_warn!("oidc_flow", "OIDC provider {} has no token endpoint: {err}", config.issuer);
        fail("oidc_unavailable", &intent)
    })?;
    let token = match exchange.set_pkce_verifier(PkceCodeVerifier::new(flow.verifier.clone())).request_async(&caller).await {
        Ok(token) => token,
        Err(RequestTokenError::ServerResponse(response)) => {
            crate::log_warn!("oidc_flow", "OIDC sign-in with {} failed: {response}", config.issuer);
            let denied = response.error().to_string() == "access_denied";
            return Err(fail(if denied { "oidc_denied" } else { "oidc_unavailable" }, &intent));
        }
        Err(err) => {
            crate::log_warn!("oidc_flow", "OIDC provider {} is unreachable: {err}", config.issuer);
            return Err(fail("oidc_unavailable", &intent));
        }
    };
    let Some(id_token) = token.id_token() else {
        crate::log_warn!("oidc_flow", "OIDC provider {} returned no ID token", config.issuer);
        return Err(fail("oidc_unavailable", &intent));
    };
    let verifier = client.id_token_verifier().set_allowed_algs(ALLOWED_ALGS);
    let claims = match id_token.claims(&verifier, &Nonce::new(flow.nonce.clone())) {
        Ok(claims) => claims.clone(),
        Err(err) => {
            crate::log_warn!("oidc_flow", "rejected an OIDC ID token from {}: {err}", config.issuer);
            return Err(fail("oidc_state", &intent));
        }
    };
    let iss = claims.issuer().as_str().to_string();
    let sub = claims.subject().as_str().to_string();
    if iss.is_empty() || sub.is_empty() {
        crate::log_warn!("oidc_flow", "OIDC provider {} returned no ID token", config.issuer);
        return Err(fail("oidc_unavailable", &intent));
    }
    let info: Option<CoreUserInfoClaims> = match client.user_info(token.access_token().clone(), Some(claims.subject().clone())) {
        Ok(request) => request.request_async(&caller).await.map_err(|e| crate::log_warn!("oidc_flow", "OIDC userinfo lookup failed: {e}")).ok(),
        Err(_) => None,
    };
    let from_info = info.as_ref().and_then(|i| {
        i.preferred_username()
            .map(|v| v.as_str().to_string())
            .or_else(|| i.email().map(|v| v.as_str().to_string()))
            .or_else(|| i.name().and_then(|n| n.get(None)).map(|v| v.as_str().to_string()))
    });
    let from_token = claims
        .preferred_username()
        .map(|v| v.as_str().to_string())
        .or_else(|| claims.email().map(|v| v.as_str().to_string()))
        .or_else(|| claims.name().and_then(|n| n.get(None)).map(|v| v.as_str().to_string()));
    let label = from_info.or(from_token).filter(|l| !l.is_empty()).unwrap_or_else(|| sub.clone());
    Ok(OidcResult { intent, next: flow.next, iss, sub, label })
}
