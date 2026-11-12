use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};

use activsync::auth_guard::AuthConfig;
use activsync::db::Db;
use activsync::net::HttpError;
use activsync::oidc_flow::OidcHttp;
use activsync::{AppState, build_app, schedule, seed};
use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use base64::Engine;
use chrono::Utc;
use http_body_util::BodyExt;
use openidconnect::core::{
    CoreIdToken, CoreIdTokenClaims, CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey,
};
use openidconnect::url::Url;
use openidconnect::{
    Audience, ClientId, EmptyAdditionalClaims, IssuerUrl, JsonWebKeyId, Nonce, PrivateSigningKey,
    StandardClaims, SubjectIdentifier,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple";
const ISSUER: &str = "https://id.example.test";

fn signing_pem() -> &'static str {
    static PEM: OnceLock<String> = OnceLock::new();
    PEM.get_or_init(|| {
        use rsa::pkcs1::EncodeRsaPrivateKey;
        let key = rsa::RsaPrivateKey::new(&mut rand_core::OsRng, 2048).unwrap();
        key.to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .unwrap()
            .to_string()
    })
}

/// A stand-in identity provider: discovery, keys, token and userinfo.
struct Idp {
    sub: String,
    audience: String,
    nonce_override: Option<String>,
    discovery_down: bool,
    token_error: Option<&'static str>,
    nonce: Option<String>,
    challenge: Option<String>,
    extra_audiences: Vec<String>,
    authorized_party: Option<String>,
    jwks_kid: &'static str,
    token_kid: &'static str,
    calls: Vec<String>,
}

impl Default for Idp {
    fn default() -> Self {
        Self {
            sub: "owner-sub".into(),
            audience: "activsync".into(),
            nonce_override: None,
            discovery_down: false,
            token_error: None,
            nonce: None,
            challenge: None,
            extra_audiences: Vec::new(),
            authorized_party: None,
            jwks_kid: "k1",
            token_kid: "k1",
            calls: Vec::new(),
        }
    }
}

fn key(kid: &str) -> CoreRsaPrivateSigningKey {
    CoreRsaPrivateSigningKey::from_pem(signing_pem(), Some(JsonWebKeyId::new(kid.into()))).unwrap()
}

fn reply(status: u16, body: Value) -> openidconnect::HttpResponse {
    openidconnect::http::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(body.to_string().into_bytes())
        .unwrap()
}

fn answer(idp: &Mutex<Idp>, request: openidconnect::HttpRequest) -> openidconnect::HttpResponse {
    let idp = &mut *idp.lock().unwrap();
    let url = request.uri().to_string();
    idp.calls
        .push(url.strip_prefix(ISSUER).unwrap_or_default().to_string());
    match url.strip_prefix(ISSUER).unwrap_or_default() {
        "/.well-known/openid-configuration" if idp.discovery_down => reply(503, json!({})),
        "/.well-known/openid-configuration" => reply(
            200,
            json!({
                "issuer": format!("{ISSUER}/"),
                "authorization_endpoint": format!("{ISSUER}/authorize"),
                "token_endpoint": format!("{ISSUER}/token"),
                "userinfo_endpoint": format!("{ISSUER}/userinfo"),
                "jwks_uri": format!("{ISSUER}/jwks"),
                "response_types_supported": ["code"],
                "subject_types_supported": ["public"],
                "id_token_signing_alg_values_supported": ["RS256"],
            }),
        ),
        "/jwks" => reply(
            200,
            json!({"keys": [serde_json::to_value(key(idp.jwks_kid).as_verification_key()).unwrap()]}),
        ),
        "/token" => {
            if let Some(error) = idp.token_error {
                return reply(400, json!({"error": error}));
            }
            let form: Vec<(String, String)> =
                openidconnect::url::form_urlencoded::parse(request.body())
                    .into_owned()
                    .collect();
            let field = |name: &str| form.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
            let verifier = field("code_verifier").unwrap_or_default();
            let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(verifier.as_bytes()));
            assert_eq!(
                Some(challenge),
                idp.challenge,
                "PKCE verifier matches the challenge"
            );
            assert_eq!(field("code").as_deref(), Some("good-code"));
            let auth = request.headers()["authorization"].to_str().unwrap();
            let expected = base64::engine::general_purpose::STANDARD.encode("activsync:s3cret");
            assert_eq!(auth, format!("Basic {expected}"));
            let nonce = idp.nonce_override.clone().or(idp.nonce.clone()).unwrap();
            let audiences = std::iter::once(idp.audience.clone())
                .chain(idp.extra_audiences.clone())
                .map(Audience::new)
                .collect();
            let claims = CoreIdTokenClaims::new(
                IssuerUrl::new(format!("{ISSUER}/")).unwrap(),
                audiences,
                Utc::now() + chrono::Duration::minutes(5),
                Utc::now(),
                StandardClaims::new(SubjectIdentifier::new(idp.sub.clone())),
                EmptyAdditionalClaims {},
            )
            .set_nonce(Some(Nonce::new(nonce)))
            .set_authorized_party(idp.authorized_party.clone().map(ClientId::new));
            let token = CoreIdToken::new(
                claims,
                &key(idp.token_kid),
                CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
                None,
                None,
            )
            .unwrap();
            reply(
                200,
                json!({"access_token": "at", "token_type": "Bearer", "id_token": token.to_string()}),
            )
        }
        "/userinfo" => reply(
            200,
            json!({"sub": idp.sub, "preferred_username": "owner@example.test"}),
        ),
        other => panic!("unexpected provider call {other}"),
    }
}

struct Client {
    app: Router,
    state: AppState,
    idp: Arc<Mutex<Idp>>,
    cookies: Vec<(String, String)>,
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    json: Value,
}

impl Reply {
    fn location(&self) -> &str {
        self.headers["location"].to_str().unwrap()
    }
}

fn client() -> Client {
    let db = Db::open_in_memory().unwrap();
    seed::seed(&mut db.lock(), Utc::now(), true).unwrap();
    let idp = Arc::new(Mutex::new(Idp::default()));
    let provider = idp.clone();
    let http: OidcHttp = Arc::new(move |request| {
        let response = answer(&provider, request);
        Box::pin(async move { Ok::<_, HttpError>(response) })
    });
    let state = AppState::new(
        db,
        true,
        schedule::system_clock(),
        schedule::UNDO_WINDOW_SECONDS,
        AuthConfig::for_tests(),
    )
    .with_oidc_http(http);
    let peer: SocketAddr = "127.0.0.1:50000".parse().unwrap();
    let app = build_app(state.clone(), None).layer(MockConnectInfo(peer));
    Client {
        app,
        state,
        idp,
        cookies: Vec::new(),
    }
}

impl Client {
    async fn raw(&mut self, mut request: axum::http::request::Builder, body: Body) -> Reply {
        let cookies: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        if !cookies.is_empty() {
            request = request.header("cookie", cookies.join("; "));
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        for set in headers.get_all("set-cookie") {
            let text = set.to_str().unwrap();
            let (pair, attributes) = text.split_once(';').unwrap_or((text, ""));
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(k, _)| k != name);
            if !attributes.contains("Max-Age=0") {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Reply {
            status,
            headers,
            json,
        }
    }

    async fn send(&mut self, method: Method, uri: &str, body: Option<Value>) -> Reply {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:8382")
            .header("x-activsync-request", "1")
            .header("sec-fetch-site", "same-origin");
        let body = match body {
            Some(value) => {
                request = request.header("content-type", "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        self.raw(request, body).await
    }

    async fn visit(&mut self, uri: &str) -> Reply {
        self.raw(
            Request::get(uri).header("host", "127.0.0.1:8382"),
            Body::empty(),
        )
        .await
    }

    async fn claim(&mut self) {
        let code = self.state.setup_code.current().unwrap();
        let reply = self
            .send(
                Method::POST,
                "/api/v1/auth/claim",
                Some(json!({"setupCode": code, "password": PASSWORD})),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
    }

    async fn sign_in(&mut self) {
        let reply = self
            .send(
                Method::POST,
                "/api/v1/auth/login",
                Some(json!({"password": PASSWORD})),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
    }

    async fn upload(&mut self, bytes: Vec<u8>) -> Reply {
        let request = Request::put("/api/v1/auth/oidc/logo")
            .header("host", "127.0.0.1:8382")
            .header("x-activsync-request", "1")
            .header("sec-fetch-site", "same-origin")
            .header("content-type", "image/png");
        self.raw(request, Body::from(bytes)).await
    }

    async fn configure(&mut self) -> Reply {
        self.send(Method::PUT, "/api/v1/auth/oidc", Some(json!({
            "displayName": "Pocket ID", "issuer": format!("{ISSUER}/"), "clientId": "activsync",
            "clientSecret": "s3cret", "scopes": "profile  email", "autoRedirect": false, "currentPassword": PASSWORD,
        })))
        .await
    }

    /// Follows the redirect to the provider, as a browser would, and returns
    /// the state the provider would send back.
    async fn to_provider(&mut self, start: &str) -> String {
        let reply = self.visit(start).await;
        assert_eq!(reply.status, StatusCode::FOUND, "{:?}", reply.headers);
        let url = Url::parse(reply.location()).unwrap();
        assert_eq!(
            url.as_str().split('?').next(),
            Some(format!("{ISSUER}/authorize").as_str())
        );
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let get = |name: &str| {
            query
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert_eq!(get("response_type"), "code");
        assert_eq!(get("client_id"), "activsync");
        assert_eq!(get("scope"), "openid profile email");
        assert_eq!(get("code_challenge_method"), "S256");
        assert_eq!(
            get("redirect_uri"),
            "http://127.0.0.1:8382/auth/oidc/callback"
        );
        let mut idp = self.idp.lock().unwrap();
        idp.nonce = Some(get("nonce"));
        idp.challenge = Some(get("code_challenge"));
        get("state")
    }

    async fn link(&mut self) {
        let reply = self
            .send(
                Method::POST,
                "/api/v1/auth/oidc/link",
                Some(json!({"currentPassword": PASSWORD})),
            )
            .await;
        assert_eq!(
            reply.json["url"],
            "/auth/oidc/login?intent=link&next=%2Fsettings"
        );
        let state = self
            .to_provider("/auth/oidc/login?intent=link&next=%2Fsettings")
            .await;
        let back = self
            .visit(&format!("/auth/oidc/callback?state={state}&code=good-code"))
            .await;
        assert_eq!(
            (back.status, back.location()),
            (StatusCode::SEE_OTHER, "/settings?oidc=linked")
        );
    }
}

#[tokio::test]
async fn link_then_sign_in_with_the_provider() {
    let mut c = client();
    c.claim().await;
    assert_eq!(
        c.configure().await.json["message"],
        "Single sign-on settings saved. Link your account to finish."
    );
    let state = c.send(Method::GET, "/api/v1/auth/oidc", None).await.json;
    assert_eq!(
        state,
        json!({
            "configured": true, "displayName": "Pocket ID", "issuer": ISSUER, "clientId": "activsync",
            "hasSecret": true, "scopes": "profile email", "autoRedirect": false,
            "redirectUri": "http://127.0.0.1:8382/auth/oidc/callback", "linkedLabel": null, "logoUrl": null,
        })
    );

    let ungranted = c.visit("/auth/oidc/login?intent=link").await;
    assert_eq!(ungranted.location(), "/settings?oidcError=link_password");
    c.link().await;
    let state = c.send(Method::GET, "/api/v1/auth/oidc", None).await.json;
    assert_eq!(state["linkedLabel"], "owner@example.test");
    let stored = activsync::db::get_config_value(&c.state.db.lock(), "auth_oidc")
        .unwrap()
        .unwrap();
    assert_eq!(
        stored["linked"],
        json!({"iss": format!("{ISSUER}/"), "sub": "owner-sub", "label": "owner@example.test"})
    );
    assert_eq!(stored["issuer"], ISSUER);

    c.send(Method::POST, "/api/v1/auth/logout", None).await;
    let status = c.send(Method::GET, "/api/v1/auth/status", None).await.json;
    assert_eq!(status["authenticated"], false);
    assert_eq!(
        status["login"]["oidc"],
        json!({"label": "Pocket ID", "autoRedirect": false, "logoUrl": null})
    );

    let state = c.to_provider("/auth/oidc/login?next=%2Factivities").await;
    let callback = format!("/auth/oidc/callback?state={state}&code=good-code");
    let back = c.visit(&callback).await;
    assert_eq!(
        (back.status, back.location()),
        (StatusCode::SEE_OTHER, "/activities")
    );
    let status = c.send(Method::GET, "/api/v1/auth/status", None).await.json;
    assert_eq!(
        (status["authenticated"].clone(), status["method"].clone()),
        (json!(true), json!("oidc"))
    );

    let replayed = c.visit(&callback).await;
    assert_eq!(replayed.location(), "/login?error=oidc_state");
}

#[tokio::test]
async fn another_provider_account_is_refused() {
    let mut c = client();
    c.claim().await;
    c.configure().await;
    c.link().await;
    c.idp.lock().unwrap().sub = "someone-else".into();
    let state = c.to_provider("/auth/oidc/login").await;
    let back = c
        .visit(&format!("/auth/oidc/callback?state={state}&code=good-code"))
        .await;
    assert_eq!(back.location(), "/login?error=oidc_mismatch");
}

#[tokio::test]
async fn bad_answers_from_the_provider_are_rejected() {
    let mut c = client();
    c.claim().await;
    c.configure().await;
    c.link().await;

    let state = c.to_provider("/auth/oidc/login").await;
    let wrong_state = c
        .visit("/auth/oidc/callback?state=forged&code=good-code")
        .await;
    assert_eq!(wrong_state.location(), "/login?error=oidc_state");
    assert!(state != "forged");

    c.to_provider("/auth/oidc/login").await;
    let denied = c.visit("/auth/oidc/callback?error=access_denied").await;
    assert_eq!(denied.location(), "/login?error=oidc_denied");

    for tamper in [
        |idp: &mut Idp| idp.audience = "another-client".into(),
        |idp: &mut Idp| idp.nonce_override = Some("replayed".into()),
    ] {
        let state = c.to_provider("/auth/oidc/login").await;
        tamper(&mut c.idp.lock().unwrap());
        let back = c
            .visit(&format!("/auth/oidc/callback?state={state}&code=good-code"))
            .await;
        assert_eq!(back.location(), "/login?error=oidc_state");
        *c.idp.lock().unwrap() = Idp::default();
    }

    let state = c.to_provider("/auth/oidc/login").await;
    c.idp.lock().unwrap().token_error = Some("access_denied");
    let back = c
        .visit(&format!("/auth/oidc/callback?state={state}&code=good-code"))
        .await;
    assert_eq!(back.location(), "/login?error=oidc_denied");

    c.configure().await;
    c.idp.lock().unwrap().discovery_down = true;
    let down = c.visit("/auth/oidc/login").await;
    assert_eq!(down.location(), "/login?error=oidc_unavailable");
    let status = c.send(Method::GET, "/api/v1/auth/status", None).await.json;
    assert_eq!(
        status["method"], "password",
        "the password session survived every refusal"
    );
}

#[tokio::test]
async fn settings_need_the_password_and_a_safe_issuer() {
    let mut c = client();
    c.claim().await;
    let wrong = c
        .send(
            Method::PUT,
            "/api/v1/auth/oidc",
            Some(json!({
                "displayName": "X", "issuer": ISSUER, "clientId": "c", "currentPassword": "nope",
            })),
        )
        .await;
    assert_eq!(
        (wrong.status, wrong.json["detail"].clone()),
        (StatusCode::BAD_REQUEST, json!("Incorrect password."))
    );
    let insecure = c.send(Method::PUT, "/api/v1/auth/oidc", Some(json!({
        "displayName": "X", "issuer": "http://id.example.test", "clientId": "c", "currentPassword": PASSWORD,
    })))
    .await;
    assert_eq!(insecure.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        insecure.json["detail"],
        "The issuer URL must start with https:// (http:// is accepted only for localhost)"
    );
    let link = c
        .send(
            Method::POST,
            "/api/v1/auth/oidc/link",
            Some(json!({"currentPassword": PASSWORD})),
        )
        .await;
    assert_eq!(
        (link.status, link.json["detail"].clone()),
        (StatusCode::CONFLICT, json!("Set up single sign-on first."))
    );

    c.configure().await;
    c.link().await;
    let kept = c.send(Method::PUT, "/api/v1/auth/oidc", Some(json!({
        "displayName": "Renamed", "issuer": ISSUER, "clientId": "activsync", "currentPassword": PASSWORD,
    })))
    .await;
    assert_eq!(kept.json["message"], "Single sign-on settings saved.");
    let state = c.send(Method::GET, "/api/v1/auth/oidc", None).await.json;
    assert_eq!(
        (state["hasSecret"].clone(), state["linkedLabel"].clone()),
        (json!(true), json!("owner@example.test"))
    );

    let moved = c.send(Method::PUT, "/api/v1/auth/oidc", Some(json!({
        "displayName": "Other", "issuer": "https://other.example.test", "clientId": "activsync", "currentPassword": PASSWORD,
    })))
    .await;
    assert_eq!(
        moved.json["message"],
        "Single sign-on settings saved. Link your account to finish."
    );
    let state = c.send(Method::GET, "/api/v1/auth/oidc", None).await.json;
    assert_eq!(
        (state["hasSecret"].clone(), state["linkedLabel"].clone()),
        (json!(false), Value::Null)
    );

    let removed = c
        .send(
            Method::DELETE,
            "/api/v1/auth/oidc",
            Some(json!({"currentPassword": PASSWORD})),
        )
        .await;
    assert_eq!(removed.json["message"], "Single sign-on removed.");
    assert_eq!(
        c.send(Method::GET, "/api/v1/auth/oidc", None).await.json["configured"],
        false
    );
}

fn png_bytes() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(400, 200, image::Rgba([0, 120, 200, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[tokio::test]
async fn sign_in_button_logo() {
    let mut c = client();
    c.claim().await;
    let early = c.upload(png_bytes()).await;
    assert_eq!(early.status, StatusCode::CONFLICT);
    c.configure().await;
    let junk = c.upload(b"<svg onload=alert(1)>".to_vec()).await;
    assert_eq!(
        (junk.status, junk.json["detail"].clone()),
        (
            StatusCode::BAD_REQUEST,
            json!("Use a PNG, JPEG or WebP image.")
        )
    );
    assert_eq!(c.upload(png_bytes()).await.json["message"], "Logo saved.");

    let url = c.send(Method::GET, "/api/v1/auth/oidc", None).await.json["logoUrl"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(url.starts_with("/api/v1/auth/oidc/logo?v="));
    c.send(Method::POST, "/api/v1/auth/logout", None).await;
    let logo = c.visit(&url).await;
    assert_eq!(
        logo.status,
        StatusCode::OK,
        "the logo is public for the login page"
    );
    assert_eq!(logo.headers["content-type"], "image/png");
    assert_eq!(
        logo.headers["content-security-policy"],
        "default-src 'none'; sandbox"
    );

    c.sign_in().await;
    let removed = c.send(Method::DELETE, "/api/v1/auth/oidc/logo", None).await;
    assert_eq!(
        removed.json["message"],
        "Logo removed. The sign-in button shows a key again."
    );
    assert_eq!(c.visit(&url).await.status, StatusCode::NOT_FOUND);
}

fn fetched(c: &Client, path: &str) -> usize {
    c.idp
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|call| call.as_str() == path)
        .count()
}

async fn sign_in_through_provider(c: &mut Client) -> Reply {
    let state = c.to_provider("/auth/oidc/login").await;
    c.visit(&format!("/auth/oidc/callback?state={state}&code=good-code"))
        .await
}

#[tokio::test]
async fn discovery_and_keys_are_fetched_once_per_provider() {
    let mut c = client();
    c.claim().await;
    c.configure().await;
    c.link().await;
    for _ in 0..2 {
        let back = sign_in_through_provider(&mut c).await;
        assert_eq!(back.location(), "/");
    }
    assert_eq!(fetched(&c, "/.well-known/openid-configuration"), 1);
    assert_eq!(fetched(&c, "/jwks"), 1);

    c.configure().await;
    let back = sign_in_through_provider(&mut c).await;
    assert_eq!(back.location(), "/");
    assert_eq!(
        fetched(&c, "/.well-known/openid-configuration"),
        2,
        "saving the settings drops the cache"
    );
    assert_eq!(fetched(&c, "/jwks"), 2);
}

#[tokio::test]
async fn an_unknown_key_id_refetches_the_keys_once() {
    let mut c = client();
    c.claim().await;
    c.configure().await;
    c.link().await;
    assert_eq!(fetched(&c, "/jwks"), 1);

    {
        let mut idp = c.idp.lock().unwrap();
        idp.jwks_kid = "k2";
        idp.token_kid = "k2";
    }
    let back = sign_in_through_provider(&mut c).await;
    assert_eq!(back.location(), "/", "the rotated key is picked up");
    assert_eq!(fetched(&c, "/jwks"), 2);

    c.idp.lock().unwrap().token_kid = "missing";
    let back = sign_in_through_provider(&mut c).await;
    assert_eq!(back.location(), "/login?error=oidc_state");
    assert_eq!(
        fetched(&c, "/jwks"),
        3,
        "one refetch per failed callback, not a loop"
    );
}

#[tokio::test]
async fn other_audiences_are_allowed_when_azp_names_the_client() {
    let mut c = client();
    c.claim().await;
    c.configure().await;
    c.link().await;

    c.idp.lock().unwrap().extra_audiences = vec!["another-client".into()];
    let back = sign_in_through_provider(&mut c).await;
    assert_eq!(
        back.location(),
        "/login?error=oidc_state",
        "several audiences need azp"
    );

    c.idp.lock().unwrap().authorized_party = Some("activsync".into());
    let back = sign_in_through_provider(&mut c).await;
    assert_eq!(back.location(), "/");

    c.idp.lock().unwrap().authorized_party = Some("another-client".into());
    let back = sign_in_through_provider(&mut c).await;
    assert_eq!(
        back.location(),
        "/login?error=oidc_state",
        "azp must be this client"
    );
}
