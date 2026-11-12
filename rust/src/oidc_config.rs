use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{self, DbResult};
use crate::oidc_logo;

pub const OIDC_KEY: &str = "auth_oidc";
pub const DEFAULT_SCOPES: &str = "openid profile email";
pub const OIDC_METHOD: &str = "oidc";
const LOOPBACK_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "::1"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Linked {
    pub iss: String,
    pub sub: String,
    pub label: String,
}

/// Stored in the same shape as the Python app's `auth_oidc` config value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OidcConfig {
    pub display_name: String,
    pub issuer: String,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default = "default_scopes")]
    pub scopes: String,
    #[serde(default)]
    pub auto_redirect: bool,
    #[serde(default)]
    pub linked: Option<Linked>,
}

fn default_scopes() -> String {
    DEFAULT_SCOPES.into()
}

pub fn load(conn: &Connection) -> DbResult<Option<OidcConfig>> {
    Ok(db::get_config_value(conn, OIDC_KEY)?
        .filter(|value| db::truthy(Some(value)))
        .and_then(|value| serde_json::from_value(value).ok()))
}

fn store(conn: &Connection, config: &OidcConfig) -> DbResult<()> {
    db::set_config_value(
        conn,
        OIDC_KEY,
        &serde_json::to_value(config).unwrap_or(Value::Null),
    )
}

pub fn revoke_method(
    conn: &Connection,
    method: &str,
    keep_token_hash: Option<&str>,
) -> DbResult<usize> {
    conn.execute(
        "DELETE FROM auth_sessions WHERE method = ?1 AND token_hash != ?2",
        [method, keep_token_hash.unwrap_or_default()],
    )
}

/// `urlsplit(...).hostname`: lower-cased, without port, user info or IPv6
/// brackets; `None` when empty or malformed.
fn hostname(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = netloc.rsplit('@').next().unwrap_or_default();
    let name = if let Some(inner) = host.strip_prefix('[') {
        inner.split_once(']')?.0
    } else {
        if host.contains(']') {
            return None;
        }
        host.split(':').next().unwrap_or_default()
    };
    (!name.is_empty()).then(|| name.to_lowercase())
}

pub fn normalize_issuer(issuer: &str) -> Result<String, String> {
    let value = issuer.trim().trim_end_matches('/').to_string();
    let host = hostname(&value);
    let secure = value.starts_with("https://") && host.is_some();
    let local = value.starts_with("http://")
        && host.as_deref().is_some_and(|h| LOOPBACK_HOSTS.contains(&h));
    if !(secure || local) {
        return Err(
            "The issuer URL must start with https:// (http:// is accepted only for localhost)"
                .into(),
        );
    }
    Ok(value)
}

pub struct NewConfig<'a> {
    pub display_name: &'a str,
    pub issuer: &'a str,
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub scopes: &'a str,
    pub auto_redirect: bool,
}

/// A new issuer or client ID is a different identity provider, so the link
/// is dropped and a missing secret means none; for the same provider the
/// stored secret and link are kept.
pub fn save(conn: &Connection, new: &NewConfig<'_>) -> Result<OidcConfig, String> {
    let issuer = normalize_issuer(new.issuer)?;
    let existing = load(conn).map_err(|e| e.to_string())?;
    let same = existing
        .as_ref()
        .is_some_and(|e| e.issuer == issuer && e.client_id == new.client_id);
    let scopes = new.scopes.split_whitespace().collect::<Vec<_>>().join(" ");
    let config = OidcConfig {
        display_name: new.display_name.into(),
        issuer,
        client_id: new.client_id.into(),
        client_secret: match new.client_secret {
            Some(secret) => secret.into(),
            None if same => existing
                .as_ref()
                .map(|e| e.client_secret.clone())
                .unwrap_or_default(),
            None => String::new(),
        },
        scopes: if scopes.is_empty() {
            DEFAULT_SCOPES.into()
        } else {
            scopes
        },
        auto_redirect: new.auto_redirect,
        linked: if same {
            existing.as_ref().and_then(|e| e.linked.clone())
        } else {
            None
        },
    };
    store(conn, &config).map_err(|e| e.to_string())?;
    if existing.is_some_and(|e| e.linked.is_some()) && config.linked.is_none() {
        revoke_method(conn, OIDC_METHOD, None).map_err(|e| e.to_string())?;
    }
    Ok(config)
}

/// Linking a different identity signs out every sign-in made through the old
/// one, except `keep_token_hash`, the session doing the linking.
pub fn set_linked(
    conn: &Connection,
    linked: Linked,
    keep_token_hash: Option<&str>,
) -> DbResult<()> {
    let Some(config) = load(conn)? else {
        return Ok(());
    };
    let replaced = config
        .linked
        .as_ref()
        .is_some_and(|old| (&old.iss, &old.sub) != (&linked.iss, &linked.sub));
    store(
        conn,
        &OidcConfig {
            linked: Some(linked),
            ..config
        },
    )?;
    if replaced {
        revoke_method(conn, OIDC_METHOD, keep_token_hash)?;
    }
    Ok(())
}

pub fn delete(conn: &Connection) -> DbResult<()> {
    oidc_logo::delete(conn)?;
    db::set_config_value(conn, OIDC_KEY, &Value::Null)?;
    revoke_method(conn, OIDC_METHOD, None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issuer_rules_match_python() {
        assert_eq!(
            normalize_issuer(" https://id.example.com/ "),
            Ok("https://id.example.com".into())
        );
        assert_eq!(
            normalize_issuer("http://localhost:9091"),
            Ok("http://localhost:9091".into())
        );
        assert_eq!(
            normalize_issuer("http://[::1]:9091/"),
            Ok("http://[::1]:9091".into())
        );
        for bad in [
            "http://id.example.com",
            "https://",
            "ftp://id.example.com",
            "https://[::1",
        ] {
            assert!(normalize_issuer(bad).is_err(), "{bad}");
        }
    }
}
