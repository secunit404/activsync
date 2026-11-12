//! The OIDC authorization-code flow with PKCE and nonce, as the Python app
//! runs it through Authlib. Per-browser flow state lives in memory behind
//! an opaque cookie, standing in for the Python app's signed session cookie.

mod claims;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreJsonWebKeySet, CoreJwsSigningAlgorithm,
    CoreProviderMetadata, CoreUserInfoClaims,
};
use openidconnect::{
    AsyncHttpClient, AuthorizationCode, ClaimsVerificationError, ClientId, ClientSecret, CsrfToken,
    EndUserEmail, EndUserName, EndUserUsername, EndpointMaybeSet, EndpointNotSet, EndpointSet,
    HttpRequest, JwsSigningAlgorithm, LocalizedClaim, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RequestTokenError, Scope,
    SignatureVerificationError, TokenResponse, http,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::auth_core::random_token;
use crate::net::{self, Http, HttpError, HttpFuture};
use crate::oidc_config::OidcConfig;

const FLOW_SECONDS: f64 = 600.0;
const MAX_BROWSERS: usize = 1024;
const LINK_GRANT_SECONDS: f64 = 300.0;

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
    intent: String,
    next: String,
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

/// What the cached discovery document was fetched for: a changed issuer,
/// client or scope list is a different provider.
#[derive(Clone, PartialEq)]
struct ProviderKey {
    issuer: String,
    client_id: String,
    client_secret: String,
    scopes: String,
}

impl ProviderKey {
    fn of(config: &OidcConfig) -> Self {
        Self {
            issuer: config.issuer.clone(),
            client_id: config.client_id.clone(),
            client_secret: config.client_secret.clone(),
            scopes: config.scopes.clone(),
        }
    }
}

#[derive(Clone)]
struct Discovered {
    key: ProviderKey,
    metadata: CoreProviderMetadata,
    jwks: Option<CoreJsonWebKeySet>,
}

/// Flow and link-grant state per browser, keyed by the value of its OIDC
/// cookie, plus the provider's discovery document and signing keys, fetched
/// once per configured provider as Authlib does. A restart forgets both, so a
/// sign-in in progress then starts over.
#[derive(Clone, Default)]
pub struct OidcSessions {
    browsers: Arc<Mutex<HashMap<String, BrowserState>>>,
    provider: Arc<Mutex<Option<Discovered>>>,
}

impl OidcSessions {
    fn with<T>(
        &self,
        key: Option<&str>,
        now: f64,
        change: impl FnOnce(&mut BrowserState) -> T,
    ) -> (String, T) {
        let mut map = self.browsers.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, s| now - s.touched < FLOW_SECONDS);
        let key = key
            .filter(|k| map.contains_key(*k))
            .map(str::to_string)
            .unwrap_or_else(|| random_token(32));
        if !map.contains_key(&key) && map.len() >= MAX_BROWSERS {
            let oldest = map
                .iter()
                .min_by(|a, b| a.1.touched.total_cmp(&b.1.touched))
                .map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                map.remove(&oldest);
            }
        }
        let entry = map.entry(key.clone()).or_default();
        entry.touched = now;
        (key, change(entry))
    }

    /// Returns the cookie value to send back.
    pub fn grant_link(&self, key: Option<&str>, now: f64) -> String {
        self.with(key, now, |s| {
            s.link_grant_until = Some(now + LINK_GRANT_SECONDS)
        })
        .0
    }

    pub fn consume_link_grant(&self, key: Option<&str>, now: f64) -> bool {
        let Some(key) = key else {
            return false;
        };
        let mut map = self.browsers.lock().unwrap_or_else(|p| p.into_inner());
        map.get_mut(key)
            .and_then(|s| s.link_grant_until.take())
            .is_some_and(|until| now < until)
    }

    fn start_flow(&self, key: Option<&str>, now: f64, flow: Flow) -> String {
        self.with(key, now, |s| s.flow = Some(flow)).0
    }

    pub fn take_flow(&self, key: Option<&str>) -> Option<Flow> {
        let key = key?;
        self.browsers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(key)
            .and_then(|s| s.flow.take())
    }

    pub fn forget_provider(&self) {
        *self.provider.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    fn cached(&self, key: &ProviderKey) -> Option<Discovered> {
        self.provider
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .filter(|d| d.key == *key)
    }

    fn store_discovery(&self, discovered: Discovered) {
        *self.provider.lock().unwrap_or_else(|p| p.into_inner()) = Some(discovered);
    }

    fn store_jwks(&self, key: &ProviderKey, jwks: CoreJsonWebKeySet) {
        let mut cached = self.provider.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(discovered) = cached.as_mut().filter(|d| d.key == *key) {
            discovered.jwks = Some(jwks);
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct OidcError {
    pub code: &'static str,
    pub intent: String,
}

fn fail(code: &'static str, intent: &str) -> OidcError {
    OidcError {
        code,
        intent: intent.into(),
    }
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

/// Discovery is read from the configured issuer as given; the ID token's
/// `iss` must then equal the issuer the document names, as Authlib checks.
/// A document that omits its signing algorithms means RS256, as in Authlib.
async fn discovered(
    http: &OidcHttp,
    sessions: &OidcSessions,
    config: &OidcConfig,
) -> Result<CoreProviderMetadata, String> {
    let key = ProviderKey::of(config);
    if let Some(found) = sessions.cached(&key) {
        return Ok(found.metadata);
    }
    let url = format!("{}/.well-known/openid-configuration", config.issuer);
    let mut document: Value = get_json(http, &url).await?;
    let advertised = document
        .get("id_token_signing_alg_values_supported")
        .and_then(Value::as_array);
    if advertised.is_none_or(|algs| algs.is_empty()) {
        document["id_token_signing_alg_values_supported"] = json!(["RS256"]);
    }
    let metadata: CoreProviderMetadata =
        serde_json::from_value(document).map_err(|e| format!("{url}: {e}"))?;
    sessions.store_discovery(Discovered {
        key,
        metadata: metadata.clone(),
        jwks: None,
    });
    Ok(metadata)
}

/// The cached signing keys, fetched on first use or when `refresh` asks for
/// them again after a token named a key the cache lacks.
async fn signing_keys(
    http: &OidcHttp,
    sessions: &OidcSessions,
    config: &OidcConfig,
    metadata: &CoreProviderMetadata,
    refresh: bool,
) -> Result<CoreJsonWebKeySet, String> {
    let key = ProviderKey::of(config);
    if !refresh && let Some(jwks) = sessions.cached(&key).and_then(|d| d.jwks) {
        return Ok(jwks);
    }
    let jwks: CoreJsonWebKeySet = get_json(http, metadata.jwks_uri().as_str()).await?;
    sessions.store_jwks(&key, jwks.clone());
    Ok(jwks)
}

type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

fn client_for(
    metadata: CoreProviderMetadata,
    config: &OidcConfig,
    redirect_uri: &str,
) -> Result<Client, String> {
    let secret =
        (!config.client_secret.is_empty()).then(|| ClientSecret::new(config.client_secret.clone()));
    let redirect = RedirectUrl::new(redirect_uri.into()).map_err(|e| e.to_string())?;
    Ok(CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(config.client_id.clone()),
        secret,
    )
    .set_redirect_uri(redirect))
}

/// Where a sign-in returns to, and what it was started for.
pub struct Return<'a> {
    pub redirect_uri: &'a str,
    pub intent: &'a str,
    pub next: &'a str,
}

/// The provider's sign-in page to send the browser to, and the cookie value
/// that finds this flow again on the way back.
pub async fn begin(
    http: &OidcHttp,
    sessions: &OidcSessions,
    config: &OidcConfig,
    cookie: Option<&str>,
    now: f64,
    to: Return<'_>,
) -> Result<(String, String), OidcError> {
    let Return {
        redirect_uri,
        intent,
        next,
    } = to;
    let client = match discovered(http, sessions, config)
        .await
        .and_then(|m| client_for(m, config, redirect_uri))
    {
        Ok(client) => client,
        Err(err) => {
            crate::log_warn!(
                "oidc_flow",
                "could not start OIDC sign-in with {}: {err}",
                config.issuer
            );
            return Err(fail("oidc_unavailable", intent));
        }
    };
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = client.authorize_url(
        CoreAuthenticationFlow::AuthorizationCode,
        CsrfToken::new_random,
        Nonce::new_random,
    );
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

/// Signing algorithms the provider advertises, less those Authlib cannot
/// verify with the provider's published keys: it never keys an HMAC with the
/// client secret.
fn allowed_algs(metadata: &CoreProviderMetadata) -> Vec<CoreJwsSigningAlgorithm> {
    metadata
        .id_token_signing_alg_values_supported()
        .iter()
        .filter(|alg| !alg.uses_shared_secret() && **alg != CoreJwsSigningAlgorithm::None)
        .cloned()
        .collect()
}

fn display_label(
    username: Option<&EndUserUsername>,
    email: Option<&EndUserEmail>,
    name: Option<&LocalizedClaim<EndUserName>>,
) -> Option<String> {
    username
        .map(|v| v.as_str())
        .or_else(|| email.map(|v| v.as_str()))
        .or_else(|| name.and_then(|n| n.get(None)).map(|v| v.as_str()))
        .map(str::to_string)
}

pub struct Callback<'a> {
    pub state: Option<&'a str>,
    pub code: Option<&'a str>,
    pub error: Option<&'a str>,
}

/// Checks the provider's answer against the flow it started: state, PKCE,
/// the ID token's signature, issuer, audience and nonce.
pub async fn finish(
    http: &OidcHttp,
    sessions: &OidcSessions,
    config: Option<&OidcConfig>,
    flow: Option<Flow>,
    callback: Callback<'_>,
    now: f64,
) -> Result<OidcResult, OidcError> {
    let intent = flow
        .as_ref()
        .map_or("login".to_string(), |f| f.intent.clone());
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
        crate::log_warn!(
            "oidc_flow",
            "OIDC sign-in with {} failed: {error}",
            config.issuer
        );
        return Err(fail("oidc_unavailable", &intent));
    }
    let Some(code) = callback.code else {
        crate::log_warn!(
            "oidc_flow",
            "OIDC provider {} returned no code",
            config.issuer
        );
        return Err(fail("oidc_unavailable", &intent));
    };
    let unreachable = |err: String| {
        crate::log_warn!(
            "oidc_flow",
            "OIDC provider {} is unreachable: {err}",
            config.issuer
        );
        fail("oidc_unavailable", &intent)
    };
    let metadata = discovered(http, sessions, config)
        .await
        .map_err(unreachable)?;
    let client = client_for(metadata.clone(), config, &flow.redirect_uri).map_err(unreachable)?;
    let caller = Caller(http);
    let exchange = client
        .exchange_code(AuthorizationCode::new(code.into()))
        .map_err(|err| {
            crate::log_warn!(
                "oidc_flow",
                "OIDC provider {} has no token endpoint: {err}",
                config.issuer
            );
            fail("oidc_unavailable", &intent)
        })?;
    let token = match exchange
        .set_pkce_verifier(PkceCodeVerifier::new(flow.verifier.clone()))
        .request_async(&caller)
        .await
    {
        Ok(token) => token,
        Err(RequestTokenError::ServerResponse(response)) => {
            crate::log_warn!(
                "oidc_flow",
                "OIDC sign-in with {} failed: {response}",
                config.issuer
            );
            let denied = response.error().to_string() == "access_denied";
            return Err(fail(
                if denied {
                    "oidc_denied"
                } else {
                    "oidc_unavailable"
                },
                &intent,
            ));
        }
        Err(err) => {
            crate::log_warn!(
                "oidc_flow",
                "OIDC provider {} is unreachable: {err}",
                config.issuer
            );
            return Err(fail("oidc_unavailable", &intent));
        }
    };
    let Some(id_token) = token.id_token() else {
        crate::log_warn!(
            "oidc_flow",
            "OIDC provider {} returned no ID token",
            config.issuer
        );
        return Err(fail("oidc_unavailable", &intent));
    };
    let algs = allowed_algs(&metadata);
    let nonce = Nonce::new(flow.nonce.clone());
    let mut refresh = false;
    let (client, claims) = loop {
        let jwks = signing_keys(http, sessions, config, &metadata, refresh)
            .await
            .map_err(unreachable)?;
        let client = client_for(metadata.clone().set_jwks(jwks), config, &flow.redirect_uri)
            .map_err(unreachable)?;
        let verifier = client
            .id_token_verifier()
            .set_allowed_algs(algs.clone())
            .set_other_audience_verifier_fn(|_| true)
            .set_time_fn(|| DateTime::<Utc>::MIN_UTC);
        let verified = id_token.claims(&verifier, &nonce).cloned();
        drop(verifier);
        match verified {
            Ok(claims) => break (client, claims),
            Err(ClaimsVerificationError::SignatureVerification(
                SignatureVerificationError::NoMatchingKey,
            )) if !refresh => {
                refresh = true;
            }
            Err(err) => {
                crate::log_warn!(
                    "oidc_flow",
                    "rejected an OIDC ID token from {}: {err}",
                    config.issuer
                );
                return Err(fail("oidc_state", &intent));
            }
        }
    };
    let jwt = serde_json::to_value(id_token)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let expected = claims::Expected {
        issuer: metadata.issuer().as_str(),
        client_id: &config.client_id,
        nonce: &flow.nonce,
        access_token: token.access_token().secret(),
    };
    if let Err(err) = claims::check(&jwt, &expected, now.trunc()) {
        crate::log_warn!(
            "oidc_flow",
            "rejected an OIDC ID token from {}: {err}",
            config.issuer
        );
        return Err(fail("oidc_state", &intent));
    }
    let iss = claims.issuer().as_str().to_string();
    let sub = claims.subject().as_str().to_string();
    if iss.is_empty() || sub.is_empty() {
        crate::log_warn!(
            "oidc_flow",
            "OIDC provider {} returned no ID token",
            config.issuer
        );
        return Err(fail("oidc_unavailable", &intent));
    }
    let info: Option<CoreUserInfoClaims> =
        match client.user_info(token.access_token().clone(), Some(claims.subject().clone())) {
            Ok(request) => request
                .request_async(&caller)
                .await
                .map_err(|e| crate::log_warn!("oidc_flow", "OIDC userinfo lookup failed: {e}"))
                .ok(),
            Err(_) => None,
        };
    let from_info = info
        .as_ref()
        .and_then(|i| display_label(i.preferred_username(), i.email(), i.name()));
    let from_token = display_label(claims.preferred_username(), claims.email(), claims.name());
    let label = from_info
        .or(from_token)
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| sub.clone());
    Ok(OidcResult {
        intent,
        next: flow.next,
        iss,
        sub,
        label,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_state_is_bounded_and_drops_the_oldest() {
        let sessions = OidcSessions::default();
        let first = sessions.grant_link(None, 1.0);
        for i in 0..MAX_BROWSERS {
            sessions.grant_link(None, 2.0 + i as f64 / 1000.0);
        }
        let size = sessions.browsers.lock().unwrap().len();
        assert_eq!(size, MAX_BROWSERS);
        assert!(
            !sessions.consume_link_grant(Some(&first), 3.0),
            "the oldest entry was evicted"
        );
    }
}
