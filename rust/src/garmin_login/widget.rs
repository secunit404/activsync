//! The SSO's embeddable HTML sign-in form, which sits outside the per-app
//! rate limits of the JSON sign-in.

use std::sync::LazyLock;

use openidconnect::http;
use regex::Regex;
use serde_json::json;

use super::{Attempt, Failure, Flow, Jar, PendingLogin, SSO_URL, SignIn, Tokens};
use crate::net;

const WIDGET_URL: &str = "https://sso.garmin.com/sso";
const EMBED_URL: &str = "https://sso.garmin.com/sso/embed";
const WIDGET_PARAMS: [(&str, &str); 7] = [
    ("id", "gauth-widget"),
    ("embedWidget", "true"),
    ("gauthHost", EMBED_URL),
    ("service", EMBED_URL),
    ("source", EMBED_URL),
    ("redirectAfterAccountLoginUrl", EMBED_URL),
    ("redirectAfterAccountCreationUrl", EMBED_URL),
];

static CSRF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"name="_csrf"\s+value="(.+?)""#).unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<title>(.+?)</title>").unwrap());
static TICKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\?ticket=(ST-[^"&\s]+)"#).unwrap());
static MFA_VARS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"var\s+(customerGuid|mfaMethod|locale|clientId|codeSentTo)\s*=\s*"([^"]*)"\s*;"#)
        .unwrap()
});

fn capture(pattern: &Regex, text: &str) -> Option<String> {
    pattern.captures(text).map(|c| c[1].to_string())
}

fn mfa_vars(html: &str) -> Vec<(String, String)> {
    MFA_VARS
        .captures_iter(html)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect()
}

fn var<'a>(vars: &'a [(String, String)], name: &str) -> &'a str {
    vars.iter()
        .rev()
        .find(|(n, _)| n == name)
        .map_or("", |(_, v)| v.as_str())
}

impl SignIn {
    pub(super) async fn widget_login(
        &self,
        email: &str,
        password: &str,
    ) -> Result<Attempt, Failure> {
        let mut jar = Jar::default();
        let embed_url = net::with_query(
            EMBED_URL,
            &[
                ("id", "gauth-widget"),
                ("embedWidget", "true"),
                ("gauthHost", WIDGET_URL),
            ],
        );
        let (response, _) = self
            .get(&embed_url, &[], &mut jar)
            .await
            .map_err(Failure::Connection)?;
        match response.status().as_u16() {
            429 => return Err(Failure::RateLimited("Widget embed GET returned 429".into())),
            s if !(200..300).contains(&s) => {
                return Err(Failure::Connection(format!("Widget embed returned {s}")));
            }
            _ => {}
        }
        let signin_url = net::with_query(&format!("{WIDGET_URL}/signin"), &WIDGET_PARAMS);
        let (page, page_url) = self
            .get(&signin_url, &[("Referer", EMBED_URL.to_string())], &mut jar)
            .await
            .map_err(Failure::Connection)?;
        if page.status().as_u16() == 429 {
            return Err(Failure::RateLimited(
                "Widget signin GET returned 429".into(),
            ));
        }
        let csrf = capture(&CSRF, &net::body_text(&page)).ok_or(Failure::Connection(
            "Widget login: missing CSRF token".into(),
        ))?;
        self.wait_like_a_browser(3.0..8.0).await;

        let form = net::form(&[
            ("username", email),
            ("password", password),
            ("embed", "true"),
            ("_csrf", &csrf),
        ]);
        let headers = [
            ("Referer", page_url),
            (
                "Content-Type",
                "application/x-www-form-urlencoded".to_string(),
            ),
        ];
        let (response, landed) = self
            .follow(http::Method::POST, &signin_url, &headers, &mut jar, form)
            .await
            .map_err(Failure::Connection)?;
        let status = response.status().as_u16();
        if status == 429 {
            return Err(Failure::RateLimited(
                "Widget signin POST returned 429".into(),
            ));
        }
        let text = net::body_text(&response);
        let title = capture(&TITLE, &text).unwrap_or_default();
        let lower = title.to_lowercase();
        if [
            "bad gateway",
            "service unavailable",
            "cloudflare",
            "502",
            "503",
        ]
        .iter()
        .any(|h| lower.contains(h))
        {
            return Err(Failure::Connection(format!(
                "Widget login: server error '{title}'"
            )));
        }
        if ["locked", "invalid", "incorrect", "account error"]
            .iter()
            .any(|h| lower.contains(h))
        {
            return Err(Failure::Auth(format!(
                "Widget authentication failed: '{title}'"
            )));
        }
        if lower.contains("unable to sign in") || lower.contains("unable to login") {
            return Err(Failure::Connection(format!(
                "Widget login: account restricted '{title}'"
            )));
        }
        let vars = mfa_vars(&text);
        let method = var(&vars, "mfaMethod");
        if lower.contains("mfa")
            || (lower.contains("authentication application") && !method.is_empty())
        {
            self.request_widget_code(&vars, &landed, &jar).await?;
            let flow = Flow::Widget {
                csrf: capture(&CSRF, &text).unwrap_or_default(),
                referer: landed,
            };
            return Ok(Attempt::Challenged(PendingLogin {
                flow,
                mfa_method: method.to_string(),
                cookies: jar.header(),
                browser: None,
            }));
        }
        if title != "Success" {
            return Err(Failure::Connection(format!(
                "Widget login: unexpected title '{title}' (HTTP {status})"
            )));
        }
        let ticket = capture(&TICKET, &text).ok_or(Failure::Connection(
            "Widget login: missing service ticket".into(),
        ))?;
        Ok(Attempt::Session(
            self.exchange_ticket(&ticket, EMBED_URL).await?,
        ))
    }

    /// An email or text code may not go out with the password post, so it is
    /// asked for the way the widget's "Request a new code" link does.
    async fn request_widget_code(
        &self,
        vars: &[(String, String)],
        referer: &str,
        jar: &Jar,
    ) -> Result<(), Failure> {
        let method = var(vars, "mfaMethod");
        if !matches!(method.to_lowercase().as_str(), "email" | "sms")
            || !var(vars, "codeSentTo").is_empty()
        {
            return Ok(());
        }
        let url = net::with_query(
            &format!("{SSO_URL}/sso/verifyMFA/mfaCode"),
            &[("clientId", var(vars, "clientId"))],
        );
        let headers = [
            ("Content-Type", "application/json".to_string()),
            ("Accept", "application/json, text/plain, */*".into()),
            ("Referer", referer.to_string()),
        ];
        let body = net::json_body(
            &json!({"customerGuid": var(vars, "customerGuid"), "mfaMethod": method, "locale": var(vars, "locale")}),
        );
        let response = self
            .send(http::Method::POST, &url, &headers, jar, body)
            .await
            .map_err(Failure::Connection)?;
        match response.status().as_u16() {
            429 => Err(Failure::RateLimited(
                "Widget MFA code request returned 429".into(),
            )),
            s if !(200..300).contains(&s) => Err(Failure::Connection(format!(
                "Widget MFA code request returned {s}"
            ))),
            _ => Ok(()),
        }
    }

    pub(super) async fn verify_widget_code(
        &self,
        pending: &PendingLogin,
        csrf: &str,
        referer: &str,
        code: &str,
    ) -> Result<Tokens, Failure> {
        if csrf.is_empty() {
            return Err(Failure::Auth("Widget MFA: missing CSRF token".into()));
        }
        let url = net::with_query(
            &format!("{SSO_URL}/sso/verifyMFA/loginEnterMfaCode"),
            &WIDGET_PARAMS,
        );
        let headers = [
            ("Referer", referer.to_string()),
            ("Content-Type", "application/x-www-form-urlencoded".into()),
        ];
        let form = net::form(&[
            ("mfa-code", code),
            ("embed", "true"),
            ("_csrf", csrf),
            ("fromPage", "setupEnterMfaCode"),
        ]);
        let mut jar = Jar::from_header(&pending.cookies);
        let (response, _) = self
            .follow(http::Method::POST, &url, &headers, &mut jar, form)
            .await
            .map_err(Failure::Connection)?;
        if response.status().as_u16() == 429 {
            return Err(Failure::RateLimited(
                "Widget MFA verify returned 429".into(),
            ));
        }
        let text = net::body_text(&response);
        let title = capture(&TITLE, &text).unwrap_or_default();
        if title != "Success" {
            return Err(Failure::Auth(format!("Widget MFA failed: {title}")));
        }
        let ticket = capture(&TICKET, &text)
            .ok_or(Failure::Auth("Widget MFA: missing service ticket".into()))?;
        self.exchange_ticket(&ticket, EMBED_URL).await
    }
}
