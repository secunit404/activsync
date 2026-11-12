use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::Value;

use super::GarminError;

const TOKEN_FILE_NAME: &str = "garmin_tokens.json";
/// Refresh this long before the token's `exp`, as the library does.
const REFRESH_MARGIN_SECONDS: f64 = 900.0;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tokens {
    pub di_token: Option<String>,
    pub di_refresh_token: Option<String>,
    pub di_client_id: Option<String>,
}

impl Tokens {
    /// Byte-for-byte what the library's `dumps()` writes, so either server
    /// reads the other's token file.
    pub fn to_json(&self) -> String {
        let field = |v: &Option<String>| serde_json::to_string(v).unwrap_or_else(|_| "null".into());
        format!(
            "{{\"di_token\": {}, \"di_refresh_token\": {}, \"di_client_id\": {}}}",
            field(&self.di_token),
            field(&self.di_refresh_token),
            field(&self.di_client_id)
        )
    }

    fn from_json(text: &str) -> Option<Self> {
        let data: Value = serde_json::from_str(text).ok()?;
        let text_of = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_string);
        let tokens = Self {
            di_token: text_of("di_token"),
            di_refresh_token: text_of("di_refresh_token"),
            di_client_id: text_of("di_client_id"),
        };
        tokens
            .di_token
            .as_deref()
            .is_some_and(|t| !t.is_empty())
            .then_some(tokens)
    }
}

/// The token file a directory or `.json` path names. Refuses `~otheruser`
/// and a symlink anywhere in the path, so a planted link cannot redirect the
/// refresh token elsewhere.
pub fn token_file_path(path: &str) -> Result<PathBuf, GarminError> {
    let mut chars = path.chars();
    if chars.next() == Some('~') && chars.next().is_some_and(|c| c != '/' && c != '\\') {
        return Err(GarminError::Invalid(format!(
            "Token path must not reference another user's home directory: {path:?}"
        )));
    }
    let expanded = match (path.strip_prefix('~'), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest.trim_start_matches('/')),
        _ => PathBuf::from(path),
    };
    for check in expanded.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        if std::fs::symlink_metadata(check).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(GarminError::Invalid(format!(
                "Token path must not be a symlink: {path:?}"
            )));
        }
    }
    let is_json = expanded
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    Ok(if expanded.is_dir() || !is_json {
        expanded.join(TOKEN_FILE_NAME)
    } else {
        expanded
    })
}

pub(super) fn load_tokens(path: &str) -> Option<Tokens> {
    Tokens::from_json(&std::fs::read_to_string(token_file_path(path).ok()?).ok()?)
}

/// Owner-only, via a fresh unpredictable sibling renamed into place, so a
/// reader never sees half a file and no planted inode receives the token.
pub fn save_tokens(path: &str, tokens: &Tokens) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let target = token_file_path(path).map_err(|e| std::io::Error::other(e.to_string()))?;
    let dir = target.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut suffix = [0u8; 8];
    rand::fill(&mut suffix);
    let hex: String = crate::auth_core::hex(&suffix);
    let tmp = dir.join(format!(".{name}.{hex}.tmp"));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(tokens.to_json().as_bytes())?;
        drop(file);
        std::fs::rename(&tmp, &target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// The claims of a JWT without checking its signature: the client has no
/// key. `alg: none` is refused so an unsigned token cannot pose as one.
fn jwt_claims(token: &str) -> Option<serde_json::Map<String, Value>> {
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::URL_SAFE,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let mut parts = token.split('.');
    let decode = |part: &str| serde_json::from_slice::<Value>(&engine.decode(part).ok()?).ok();
    let header = decode(parts.next()?)?;
    let claims = decode(parts.next()?)?;
    if header.as_object()?.get("alg").and_then(Value::as_str) == Some("none") {
        return None;
    }
    match claims {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// `exp` comes from an unverified token, so anything but a finite number
/// (or a numeric string) reads as "not expiring" rather than failing.
pub(crate) fn expires_soon(token: &str, now: f64) -> bool {
    let exp = match jwt_claims(token).and_then(|mut c| c.remove("exp")) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    exp.is_some_and(|exp| exp.is_finite() && now > exp - REFRESH_MARGIN_SECONDS)
}

pub(crate) fn client_id_of(token: &str) -> Option<String> {
    match jwt_claims(token)?.remove("client_id")? {
        Value::String(s) if !s.is_empty() => Some(s),
        Value::String(_) | Value::Null | Value::Bool(false) => None,
        other => Some(other.to_string()),
    }
}
