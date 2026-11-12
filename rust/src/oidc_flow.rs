//! The OIDC authorization-code flow with PKCE and nonce, as the Python app
//! runs it through Authlib. Per-browser flow state lives in memory behind
//! an opaque cookie, standing in for the Python app's signed session cookie.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use chrono::{DateTime, Utc};
use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreJsonWebKeySet, CoreJwsSigningAlgorithm,
    CoreProviderMetadata, CoreUserInfoClaims,
};
use openidconnect::{
    AsyncHttpClient, AuthorizationCode, ClaimsVerificationError, ClientId, ClientSecret, CsrfToken, HttpRequest,
    JwsSigningAlgorithm, Nonce, OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl,
    RequestTokenError, Scope, SignatureVerificationError, TokenResponse, http,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::auth_core::random_token;
use crate::net::{self, Http, HttpError, HttpFuture};
use crate::oidc_config::OidcConfig;

const FLOW_SECONDS: f64 = 600.0;
const MAX_BROWSERS: usize = 1024;
const LEEWAY_SECONDS: f64 = 120.0;
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
    fn with<T>(&self, key: Option<&str>, now: f64, change: impl FnOnce(&mut BrowserState) -> T) -> (String, T) {
        let mut map = self.browsers.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|_, s| now - s.touched < FLOW_SECONDS);
        let key = key.filter(|k| map.contains_key(*k)).map(str::to_string).unwrap_or_else(|| random_token(32));
        if !map.contains_key(&key) && map.len() >= MAX_BROWSERS {
            let oldest = map.iter().min_by(|a, b| a.1.touched.total_cmp(&b.1.touched)).map(|(k, _)| k.clone());
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
        self.with(key, now, |s| s.link_grant_until = Some(now + LINK_GRANT_SECONDS)).0
    }

    pub fn consume_link_grant(&self, key: Option<&str>, now: f64) -> bool {
        let Some(key) = key else {
            return false;
        };
        let mut map = self.browsers.lock().unwrap_or_else(|p| p.into_inner());
        map.get_mut(key).and_then(|s| s.link_grant_until.take()).is_some_and(|until| now < until)
    }

    fn start_flow(&self, key: Option<&str>, now: f64, flow: Flow) -> String {
        self.with(key, now, |s| s.flow = Some(flow)).0
    }

    pub fn take_flow(&self, key: Option<&str>) -> Option<Flow> {
        let key = key?;
        self.browsers.lock().unwrap_or_else(|p| p.into_inner()).get_mut(key).and_then(|s| s.flow.take())
    }

    /// Called when the sign-in settings are saved or removed.
    pub fn forget_provider(&self) {
        *self.provider.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    fn cached(&self, key: &ProviderKey) -> Option<Discovered> {
        self.provider.lock().unwrap_or_else(|p| p.into_inner()).clone().filter(|d| d.key == *key)
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
/// A document that omits its signing algorithms means RS256, as in Authlib.
async fn discovered(http: &OidcHttp, sessions: &OidcSessions, config: &OidcConfig) -> Result<CoreProviderMetadata, String> {
    let key = ProviderKey::of(config);
    if let Some(found) = sessions.cached(&key) {
        return Ok(found.metadata);
    }
    let url = discovery_url(&config.issuer);
    let mut document: Value = get_json(http, &url).await?;
    let advertised = document.get("id_token_signing_alg_values_supported").and_then(Value::as_array);
    if advertised.is_none_or(|algs| algs.is_empty()) {
        document["id_token_signing_alg_values_supported"] = json!(["RS256"]);
    }
    let metadata: CoreProviderMetadata = serde_json::from_value(document).map_err(|e| format!("{url}: {e}"))?;
    sessions.store_discovery(Discovered { key, metadata: metadata.clone(), jwks: None });
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
    if !refresh {
        if let Some(jwks) = sessions.cached(&key).and_then(|d| d.jwks) {
            return Ok(jwks);
        }
    }
    let jwks: CoreJsonWebKeySet = get_json(http, metadata.jwks_uri().as_str()).await?;
    sessions.store_jwks(&key, jwks.clone());
    Ok(jwks)
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
    let client = match discovered(http, sessions, config).await.and_then(|m| client_for(m, config, redirect_uri)) {
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

fn decode_segment(jwt: &str, index: usize) -> Option<Value> {
    let segment = jwt.split('.').nth(index)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(segment.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

fn half_hash(text: &str, alg: &str) -> Option<String> {
    let digest = match alg {
        "EdDSA" => Sha512::digest(text.as_bytes()).to_vec(),
        _ => match alg.get(2..)? {
            "256" => Sha256::digest(text.as_bytes()).to_vec(),
            "384" => Sha384::digest(text.as_bytes()).to_vec(),
            "512" => Sha512::digest(text.as_bytes()).to_vec(),
            _ => return None,
        },
    };
    Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&digest[..digest.len() / 2]))
}

struct Expected<'a> {
    issuer: &'a str,
    client_id: &'a str,
    nonce: &'a str,
    access_token: &'a str,
}

/// The ID token claim checks Authlib makes once the signature holds (its
/// `CodeIDToken.validate` with the app's `claims_options`): the client must be
/// among the audiences but others may be too; `azp` is required when the sole
/// audience is not the client, or there are several, and must then be the
/// client; time claims get two minutes of leeway.
fn authlib_problem(header: &Value, claims: &Value, expected: &Expected<'_>, now: f64) -> Result<(), String> {
    let get = |name: &str| claims.get(name).filter(|v| !v.is_null());
    for name in ["iss", "sub", "aud", "exp", "iat"] {
        if get(name).is_none() {
            return Err(format!("missing claim: {name}"));
        }
    }
    let invalid = |name: &str| Err(format!("invalid claim: {name}"));
    match get("iss").and_then(Value::as_str) {
        Some(iss) if !iss.is_empty() && iss == expected.issuer => {}
        _ => return invalid("iss"),
    }
    if get("sub").and_then(Value::as_str).is_none() {
        return invalid("sub");
    }
    let audiences: Vec<&str> = match get("aud") {
        Some(Value::String(aud)) => vec![aud.as_str()],
        Some(Value::Array(list)) => match list.iter().map(Value::as_str).collect::<Option<Vec<_>>>() {
            Some(list) => list,
            None => return invalid("aud"),
        },
        _ => return invalid("aud"),
    };
    if !audiences.contains(&expected.client_id) {
        return invalid("aud");
    }
    let number = |name: &str| get(name).map(|v| v.as_f64().ok_or_else(|| format!("invalid claim: {name}")));
    if number("exp").transpose()?.is_some_and(|exp| exp < now - LEEWAY_SECONDS) {
        return Err("expired token".into());
    }
    if number("nbf").transpose()?.is_some_and(|nbf| nbf > now + LEEWAY_SECONDS) {
        return Err("invalid claim: nbf".into());
    }
    if number("iat").transpose()?.is_some_and(|iat| iat > now + LEEWAY_SECONDS) {
        return Err("invalid claim: iat".into());
    }
    if truthy(claims.get("auth_time")) && !claims["auth_time"].is_number() {
        return invalid("auth_time");
    }
    if !expected.nonce.is_empty() {
        match get("nonce") {
            None => return Err("missing claim: nonce".into()),
            Some(nonce) if nonce.as_str() != Some(expected.nonce) => return invalid("nonce"),
            Some(_) => {}
        }
    }
    if truthy(claims.get("amr")) && !claims["amr"].is_array() {
        return invalid("amr");
    }
    let azp = claims.get("azp").filter(|_| truthy(claims.get("azp")));
    let sole_client = audiences.len() == 1 && audiences[0] == expected.client_id;
    if !sole_client && azp.is_none() {
        return Err("missing claim: azp".into());
    }
    if azp.is_some_and(|azp| azp.as_str() != Some(expected.client_id)) {
        return invalid("azp");
    }
    if let Some(at_hash) = claims.get("at_hash").filter(|_| truthy(claims.get("at_hash"))) {
        if !expected.access_token.is_empty() {
            let alg = header.get("alg").and_then(Value::as_str).unwrap_or_default();
            if half_hash(expected.access_token, alg).as_deref() != at_hash.as_str() {
                return invalid("at_hash");
            }
        }
    }
    Ok(())
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
    let unreachable = |err: String| {
        crate::log_warn!("oidc_flow", "OIDC provider {} is unreachable: {err}", config.issuer);
        fail("oidc_unavailable", &intent)
    };
    let metadata = discovered(http, sessions, config).await.map_err(unreachable)?;
    let client = client_for(metadata.clone(), config, &flow.redirect_uri).map_err(unreachable)?;
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
    let algs = allowed_algs(&metadata);
    let nonce = Nonce::new(flow.nonce.clone());
    let mut refresh = false;
    let (client, claims) = loop {
        let jwks = signing_keys(http, sessions, config, &metadata, refresh).await.map_err(unreachable)?;
        let client = client_for(metadata.clone().set_jwks(jwks), config, &flow.redirect_uri).map_err(unreachable)?;
        let verifier = client
            .id_token_verifier()
            .set_allowed_algs(algs.clone())
            .set_other_audience_verifier_fn(|_| true)
            .set_time_fn(|| DateTime::<Utc>::MIN_UTC);
        let verified = id_token.claims(&verifier, &nonce).map(Clone::clone);
        drop(verifier);
        match verified {
            Ok(claims) => break (client, claims),
            Err(ClaimsVerificationError::SignatureVerification(SignatureVerificationError::NoMatchingKey)) if !refresh => {
                refresh = true;
            }
            Err(err) => {
                crate::log_warn!("oidc_flow", "rejected an OIDC ID token from {}: {err}", config.issuer);
                return Err(fail("oidc_state", &intent));
            }
        }
    };
    let jwt = serde_json::to_value(id_token).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let expected = Expected {
        issuer: metadata.issuer().as_str(),
        client_id: &config.client_id,
        nonce: &flow.nonce,
        access_token: token.access_token().secret(),
    };
    let header = decode_segment(&jwt, 0).unwrap_or(Value::Null);
    let payload = decode_segment(&jwt, 1).unwrap_or(Value::Null);
    if let Err(err) = authlib_problem(&header, &payload, &expected, now.trunc()) {
        crate::log_warn!("oidc_flow", "rejected an OIDC ID token from {}: {err}", config.issuer);
        return Err(fail("oidc_state", &intent));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_000_000.0;
    const ISSUER: &str = "https://id.example.test/";

    fn verdict(patch: Value) -> bool {
        let mut claims = json!({
            "iss": ISSUER, "sub": "s", "aud": "activsync", "exp": NOW + 300.0, "iat": NOW, "nonce": "n1",
        });
        for (key, value) in patch.as_object().unwrap() {
            if value.is_null() {
                claims.as_object_mut().unwrap().remove(key);
            } else {
                claims[key] = value.clone();
            }
        }
        let expected = Expected { issuer: ISSUER, client_id: "activsync", nonce: "n1", access_token: "at" };
        authlib_problem(&json!({"alg": "RS256"}), &claims, &expected, NOW).is_ok()
    }

    #[test]
    fn id_token_claims_follow_authlib() {
        let cases = [
            ("plain", json!({}), true),
            ("aud_list_one", json!({"aud": ["activsync"]}), true),
            ("aud_two_no_azp", json!({"aud": ["activsync", "other"]}), false),
            ("aud_two_azp", json!({"aud": ["activsync", "other"], "azp": "activsync"}), true),
            ("aud_two_wrong_azp", json!({"aud": ["activsync", "other"], "azp": "other"}), false),
            ("aud_other_only", json!({"aud": ["other"]}), false),
            ("aud_other_azp_client", json!({"aud": ["other"], "azp": "activsync"}), false),
            ("azp_other_single_aud", json!({"azp": "other"}), false),
            ("azp_client", json!({"azp": "activsync"}), true),
            ("aud_empty_list", json!({"aud": []}), false),
            ("exp_within_leeway", json!({"exp": NOW - 100.0}), true),
            ("exp_past_leeway", json!({"exp": NOW - 121.0}), false),
            ("exp_edge", json!({"exp": NOW - 120.0}), true),
            ("iat_within_leeway", json!({"iat": NOW + 100.0}), true),
            ("iat_future", json!({"iat": NOW + 121.0}), false),
            ("nbf_future", json!({"nbf": NOW + 121.0}), false),
            ("nbf_ok", json!({"nbf": NOW + 100.0}), true),
            ("amr_string", json!({"amr": "pwd"}), false),
            ("amr_list", json!({"amr": ["pwd"]}), true),
            ("auth_time_string", json!({"auth_time": "x"}), false),
            ("auth_time_number", json!({"auth_time": 5}), true),
            ("at_hash_ok", json!({"at_hash": "sda5G2fCr6XjIpiNlGJjjQ"}), true),
            ("at_hash_bad", json!({"at_hash": "AAAA"}), false),
            ("nonce_wrong", json!({"nonce": "other"}), false),
            ("nonce_missing", json!({"nonce": null}), false),
            ("iss_other", json!({"iss": "https://evil.example/"}), false),
            ("iss_blank", json!({"iss": ""}), false),
        ];
        for (name, patch, expected) in cases {
            assert_eq!(verdict(patch), expected, "{name}");
        }
    }

    #[test]
    fn half_hash_matches_authlib() {
        assert_eq!(half_hash("at", "RS256").as_deref(), Some("sda5G2fCr6XjIpiNlGJjjQ"));
        assert_eq!(half_hash("at", "ES256K"), None);
        assert_eq!(half_hash("at", "HS1"), None);
    }

    #[test]
    fn flow_state_is_bounded_and_drops_the_oldest() {
        let sessions = OidcSessions::default();
        let first = sessions.grant_link(None, 1.0);
        for i in 0..MAX_BROWSERS {
            sessions.grant_link(None, 2.0 + i as f64 / 1000.0);
        }
        let size = sessions.browsers.lock().unwrap().len();
        assert_eq!(size, MAX_BROWSERS);
        assert!(!sessions.consume_link_grant(Some(&first), 3.0), "the oldest entry was evicted");
    }
}
