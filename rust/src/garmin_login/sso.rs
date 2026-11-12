//! The SSO's JSON sign-in, which the mobile app and the web portal share.

use openidconnect::http;
use serde_json::{Value, json};

use super::{Attempt, Failure, Flow, Jar, PendingLogin, SSO_URL, SignIn, Tokens};
use crate::net;

const IOS_SSO_CLIENT_ID: &str = "GCM_IOS_DARK";
const IOS_SERVICE_URL: &str = "https://mobile.integration.garmin.com/gcm/ios";
const PORTAL_SSO_CLIENT_ID: &str = "GarminConnect";
const PORTAL_SERVICE_URL: &str = "https://connect.garmin.com/app";

const MOBILE_PARAMS: [(&str, &str); 3] = [
    ("clientId", IOS_SSO_CLIENT_ID),
    ("locale", "en-US"),
    ("service", IOS_SERVICE_URL),
];
const PORTAL_PARAMS: [(&str, &str); 3] = [
    ("clientId", PORTAL_SSO_CLIENT_ID),
    ("locale", "en-US"),
    ("service", PORTAL_SERVICE_URL),
];

/// No User-Agent, here or on the portal: the browser the request presents
/// as supplies its own, so the two always agree.
fn mobile_headers() -> Vec<(&'static str, String)> {
    vec![
        ("Accept", "application/json, text/plain, */*".into()),
        ("Content-Type", "application/json".into()),
        ("Origin", SSO_URL.into()),
    ]
}

fn portal_headers() -> Vec<(&'static str, String)> {
    vec![
        ("Accept", "application/json, text/plain, */*".into()),
        ("Accept-Language", "en-US,en;q=0.9".into()),
        ("Content-Type", "application/json".into()),
        ("Origin", SSO_URL.into()),
        (
            "Referer",
            format!(
                "{SSO_URL}/portal/sso/en-US/sign-in?clientId={PORTAL_SSO_CLIENT_ID}&service={PORTAL_SERVICE_URL}"
            ),
        ),
    ]
}

fn status_type(res: &Value) -> Option<&str> {
    res.pointer("/responseStatus/type").and_then(Value::as_str)
}

fn rate_limited_in_body(res: &Value) -> bool {
    res.pointer("/error/status-code").and_then(Value::as_str) == Some("429")
}

fn service_ticket(res: &Value) -> &str {
    res.get("serviceTicketId")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

impl SignIn {
    pub(super) async fn mobile_login(
        &self,
        email: &str,
        password: &str,
    ) -> Result<Attempt, Failure> {
        let url = net::with_query(&format!("{SSO_URL}/mobile/api/login"), &MOBILE_PARAMS);
        self.json_login(
            &url,
            &mobile_headers(),
            Jar::default(),
            email,
            password,
            Flow::Mobile,
        )
        .await
    }

    /// The desktop portal: its sign-in page first for cookies, then a pause
    /// as a person would take to type, which Cloudflare looks for.
    pub(super) async fn portal_login(
        &self,
        email: &str,
        password: &str,
    ) -> Result<Attempt, Failure> {
        let page = net::with_query(
            &format!("{SSO_URL}/portal/sso/en-US/sign-in"),
            &[
                ("clientId", PORTAL_SSO_CLIENT_ID),
                ("service", PORTAL_SERVICE_URL),
            ],
        );
        let page_headers = [
            (
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".to_string(),
            ),
            ("Accept-Language", "en-US,en;q=0.9".into()),
        ];
        let mut jar = Jar::default();
        let (response, _) = self
            .get(&page, &page_headers, &mut jar)
            .await
            .map_err(Failure::Connection)?;
        if response.status().as_u16() == 429 {
            return Err(Failure::RateLimited(
                "Portal login GET returned 429 — Cloudflare blocking this request.".into(),
            ));
        }
        self.wait_like_a_browser(10.0..20.0).await;
        let url = net::with_query(&format!("{SSO_URL}/portal/api/login"), &PORTAL_PARAMS);
        self.json_login(&url, &portal_headers(), jar, email, password, Flow::Portal)
            .await
    }

    async fn json_login(
        &self,
        url: &str,
        headers: &[(&str, String)],
        mut jar: Jar,
        email: &str,
        password: &str,
        flow: Flow,
    ) -> Result<Attempt, Failure> {
        let (name, service) = match flow {
            Flow::Portal => ("Portal", PORTAL_SERVICE_URL),
            _ => ("Mobile", IOS_SERVICE_URL),
        };
        let body = net::json_body(
            &json!({"username": email, "password": password, "rememberMe": true, "captchaToken": ""}),
        );
        let response = self
            .send(http::Method::POST, url, headers, &jar, body)
            .await
            .map_err(Failure::Connection)?;
        jar.absorb(&response);
        let status = response.status().as_u16();
        if status == 429 {
            return Err(Failure::RateLimited(format!(
                "{name} login returned 429 — IP rate limited by Garmin"
            )));
        }
        if status == 403 {
            return Err(Failure::Connection(format!(
                "{name} login: HTTP 403 (Cloudflare bot challenge)"
            )));
        }
        let Ok(res) = serde_json::from_slice::<Value>(response.body()) else {
            return Err(Failure::Connection(format!(
                "{name} login failed (non-JSON): HTTP {status}"
            )));
        };
        match status_type(&res) {
            Some("MFA_REQUIRED") => {
                let method = res
                    .pointer("/customerMfaInfo/mfaLastMethodUsed")
                    .and_then(Value::as_str)
                    .unwrap_or("email");
                Ok(Attempt::Challenged(PendingLogin {
                    flow,
                    mfa_method: method.to_string(),
                    cookies: jar.header(),
                    browser: None,
                }))
            }
            Some("SUCCESSFUL") => Ok(Attempt::Session(
                self.exchange_ticket(service_ticket(&res), service).await?,
            )),
            Some("INVALID_USERNAME_PASSWORD") => Err(Failure::Auth(
                "401 Unauthorized (Invalid Username or Password)".into(),
            )),
            _ if rate_limited_in_body(&res) => Err(Failure::RateLimited(format!(
                "{name} login: 429 in JSON body"
            ))),
            Some("CAPTCHA_REQUIRED") => Err(Failure::Connection(format!(
                "{name} login: CAPTCHA required (bot challenge)"
            ))),
            other => Err(Failure::Connection(format!(
                "{name} login failed: HTTP {status}, responseStatus={}",
                other.unwrap_or("unknown")
            ))),
        }
    }

    /// Garmin's two JSON code endpoints share the sign-in's cookies but may
    /// sit in different rate-limit buckets, so the other is tried too.
    pub(super) async fn verify_code(
        &self,
        pending: &PendingLogin,
        code: &str,
    ) -> Result<Tokens, Failure> {
        let body = json!({"mfaMethod": pending.mfa_method, "mfaVerificationCode": code, "rememberMyBrowser": true,
                          "reconsentList": [], "mfaSetup": false});
        let mobile = net::with_query(
            &format!("{SSO_URL}/mobile/api/mfa/verifyCode"),
            &MOBILE_PARAMS,
        );
        let portal = net::with_query(
            &format!("{SSO_URL}/portal/api/mfa/verifyCode"),
            &PORTAL_PARAMS,
        );
        let (endpoints, headers, service) = match pending.flow {
            Flow::Portal => ([portal, mobile], portal_headers(), PORTAL_SERVICE_URL),
            _ => ([mobile, portal], mobile_headers(), IOS_SERVICE_URL),
        };
        let jar = Jar::from_header(&pending.cookies);
        let mut failures = Vec::new();
        let mut rate_limited = 0;
        for url in &endpoints {
            let path = url.split('?').next().unwrap_or(url);
            let response = match self
                .send(
                    http::Method::POST,
                    url,
                    &headers,
                    &jar,
                    net::json_body(&body),
                )
                .await
            {
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
                failures.push(format!(
                    "{path}: HTTP {} non-JSON",
                    response.status().as_u16()
                ));
                continue;
            };
            if rate_limited_in_body(&res) {
                failures.push(format!("{path}: 429 in JSON body"));
                rate_limited += 1;
                continue;
            }
            if status_type(&res) == Some("SUCCESSFUL") {
                return self.exchange_ticket(service_ticket(&res), service).await;
            }
            let status = status_type(&res)
                .or_else(|| res.pointer("/error/status-code").and_then(Value::as_str))
                .unwrap_or("unknown");
            failures.push(format!("{path}: {status}"));
        }
        let listed = failures
            .iter()
            .map(|f| format!("'{f}'"))
            .collect::<Vec<_>>()
            .join(", ");
        if rate_limited == endpoints.len() {
            return Err(Failure::RateLimited(format!(
                "MFA verification rate limited on all endpoints: [{listed}]"
            )));
        }
        Err(Failure::Auth(format!(
            "MFA verification failed: [{listed}]"
        )))
    }
}
