//! The owner's credential, login throttling, the setup code and sessions,
//! stored exactly as the Python app's `auth_core.py` and `auth_devices.py`
//! store them, so a database moves between the two servers.

mod sessions;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use rand::{Rng, RngCore};
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::db::{self, DbResult};

pub(crate) use sessions::{
    SESSION_ABSOLUTE_SECONDS, create_session, list_sessions, resolve_session, revoke_all,
    revoke_public_id, revoke_session,
};
pub use sessions::{Session, prune_expired_sessions};

pub(crate) const OWNER_KEY: &str = "auth_owner";
const THROTTLE_KEY: &str = "auth_login_throttle";
const TRUSTED_DEVICES_KEY: &str = "auth_trusted_devices";

const PASSWORD_MIN_LENGTH: usize = 15;
const PASSWORD_MAX_LENGTH: usize = 256;

const FREE_FAILURES: i64 = 5;
const MAX_DELAY_SECONDS: f64 = 900.0;

const IP_ATTEMPT_LIMIT: usize = 10;
const IP_ATTEMPT_WINDOW_SECONDS: f64 = 60.0;

const MAX_TRUSTED_DEVICES: usize = 20;
const DEVICE_MAX_FAILURES: i64 = 10;
pub(crate) const DEVICE_COOKIE_MAX_AGE: i64 = 365 * 24 * 3600;

const SETUP_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn hash_token(token: &str) -> String {
    sha256_hex(token.as_bytes())
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// argon2id cost settings. `standard()` matches argon2-cffi's defaults, which
/// is what the Python app hashes with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HashCost {
    pub memory_kib: u32,
    pub time: u32,
    pub parallelism: u32,
    pub hash_len: usize,
    pub salt_len: usize,
}

impl HashCost {
    pub const fn standard() -> Self {
        Self {
            memory_kib: 65536,
            time: 3,
            parallelism: 4,
            hash_len: 32,
            salt_len: 16,
        }
    }

    /// argon2-cffi's CHEAPEST profile, for tests only.
    pub const fn cheapest() -> Self {
        Self {
            memory_kib: 8,
            time: 1,
            parallelism: 1,
            hash_len: 4,
            salt_len: 8,
        }
    }
}

/// An argon2 hash in the PHC string form argon2-cffi writes,
/// `$argon2id$v=19$m=65536,t=3,p=4$<salt>$<hash>`. Parsed by hand because the
/// generic PHC parser rejects the 4-byte hashes of argon2-cffi's test profile.
struct Encoded {
    algorithm: Algorithm,
    version: u32,
    cost: HashCost,
    salt: Vec<u8>,
    hash: Vec<u8>,
}

impl Encoded {
    fn parse(text: &str) -> Option<Self> {
        let parts: Vec<&str> = text.split('$').collect();
        let ["", algorithm, version, params, salt, hash] = parts.as_slice() else {
            return None;
        };
        let algorithm = match *algorithm {
            "argon2id" => Algorithm::Argon2id,
            "argon2i" => Algorithm::Argon2i,
            "argon2d" => Algorithm::Argon2d,
            _ => return None,
        };
        let version: u32 = version.strip_prefix("v=")?.parse().ok()?;
        let (mut memory_kib, mut time, mut parallelism) = (0, 0, 0);
        for pair in params.split(',') {
            let (key, value) = pair.split_once('=')?;
            let value: u32 = value.parse().ok()?;
            match key {
                "m" => memory_kib = value,
                "t" => time = value,
                "p" => parallelism = value,
                _ => return None,
            }
        }
        let salt = STANDARD_NO_PAD.decode(salt).ok()?;
        let hash = STANDARD_NO_PAD.decode(hash).ok()?;
        Some(Self {
            algorithm,
            version,
            cost: HashCost {
                memory_kib,
                time,
                parallelism,
                hash_len: hash.len(),
                salt_len: salt.len(),
            },
            salt,
            hash,
        })
    }

    fn verify(&self, password: &str) -> bool {
        let version = match self.version {
            0x13 => Version::V0x13,
            0x10 => Version::V0x10,
            _ => return false,
        };
        let Ok(params) = Params::new(
            self.cost.memory_kib,
            self.cost.time,
            self.cost.parallelism,
            Some(self.hash.len()),
        ) else {
            return false;
        };
        let mut out = vec![0u8; self.hash.len()];
        Argon2::new(self.algorithm, version, params)
            .hash_password_into(password.as_bytes(), &self.salt, &mut out)
            .is_ok()
            && constant_time_eq(&out, &self.hash)
    }

    fn uses(&self, cost: &HashCost) -> bool {
        self.algorithm == Algorithm::Argon2id && self.version == 0x13 && self.cost == *cost
    }
}

pub(crate) fn password_problem(password: &str) -> Option<String> {
    let length = password.chars().count();
    if length < PASSWORD_MIN_LENGTH {
        return Some(format!(
            "Use at least {PASSWORD_MIN_LENGTH} characters. A few unrelated words make a good passphrase."
        ));
    }
    if length > PASSWORD_MAX_LENGTH {
        return Some(format!("Use at most {PASSWORD_MAX_LENGTH} characters."));
    }
    None
}

pub fn hash_password(password: &str, cost: HashCost) -> Result<String, String> {
    if let Some(problem) = password_problem(password) {
        return Err(problem);
    }
    let params = Params::new(
        cost.memory_kib,
        cost.time,
        cost.parallelism,
        Some(cost.hash_len),
    )
    .map_err(|e| e.to_string())?;
    let mut salt = vec![0u8; cost.salt_len];
    rand::rng().fill_bytes(&mut salt);
    let mut out = vec![0u8; cost.hash_len];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), &salt, &mut out)
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "$argon2id$v=19$m={},t={},p={}${}${}",
        cost.memory_kib,
        cost.time,
        cost.parallelism,
        STANDARD_NO_PAD.encode(&salt),
        STANDARD_NO_PAD.encode(&out)
    ))
}

pub(crate) async fn hash_password_blocking(
    password: String,
    cost: HashCost,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || hash_password(&password, cost))
        .await
        .map_err(|err| err.to_string())?
}

fn owner(conn: &Connection) -> DbResult<Value> {
    Ok(db::get_config_value(conn, OWNER_KEY)?
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({})))
}

pub fn owner_claimed(conn: &Connection) -> DbResult<bool> {
    Ok(db::truthy(owner(conn)?.get("password_hash")))
}

pub fn set_password_hash(conn: &Connection, password_hash: &str, now: f64) -> DbResult<()> {
    db::set_config_value(
        conn,
        OWNER_KEY,
        &json!({"password_hash": password_hash, "password_changed_at": now.trunc() as i64}),
    )
}

/// Stores the first owner's password, or returns false if someone claimed
/// first. The caller hashes beforehand: argon2 is slow, and the check and the
/// write must share one lock.
pub(crate) fn claim_owner(conn: &Connection, password_hash: &str, now: f64) -> DbResult<bool> {
    if owner_claimed(conn)? {
        return Ok(false);
    }
    set_password_hash(conn, password_hash, now)?;
    Ok(true)
}

fn stored_hash(conn: &Connection) -> DbResult<Option<String>> {
    Ok(owner(conn)?
        .get("password_hash")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string))
}

/// The outcome of checking a password against a stored hash, with the
/// replacement hash when the stored one was made with other cost settings.
#[derive(Default)]
pub struct Verdict {
    pub verified: bool,
    stored: Option<String>,
    rehash: Option<String>,
}

/// The slow, database-free half of a password check. Run it off the database
/// lock: argon2 takes long enough to stall every other request.
pub fn verify_stored(stored: Option<&str>, password: &str, cost: HashCost) -> Verdict {
    let Some((stored, parsed)) = stored
        .and_then(|stored| Some((stored, Encoded::parse(stored)?)))
        .filter(|(_, parsed)| parsed.verify(password))
    else {
        return Verdict::default();
    };
    Verdict {
        verified: true,
        stored: Some(stored.to_string()),
        rehash: if parsed.uses(&cost) {
            None
        } else {
            hash_password(password, cost).ok()
        },
    }
}

fn store_rehash(conn: &Connection, verdict: &Verdict) -> DbResult<()> {
    let Some(rehashed) = &verdict.rehash else {
        return Ok(());
    };
    let mut record = owner(conn)?;
    if record.get("password_hash").and_then(Value::as_str) != verdict.stored.as_deref() {
        return Ok(());
    }
    record["password_hash"] = json!(rehashed);
    db::set_config_value(conn, OWNER_KEY, &record)
}

fn bucket_wait(state: &Value, now: f64) -> i64 {
    match state.get("next_allowed_at").and_then(Value::as_f64) {
        Some(next) if next > now => (next - now).ceil() as i64,
        _ => 0,
    }
}

fn bucket_after_failure(state: &Value, now: f64) -> Value {
    let failures = state.get("failures").and_then(Value::as_i64).unwrap_or(0) + 1;
    let delay = if failures >= FREE_FAILURES {
        2f64.powi((failures - FREE_FAILURES) as i32)
            .min(MAX_DELAY_SECONDS)
    } else {
        0.0
    };
    let mut next = if state.is_object() {
        state.clone()
    } else {
        json!({})
    };
    next["failures"] = json!(failures);
    next["next_allowed_at"] = if delay > 0.0 {
        json!(now + delay)
    } else {
        Value::Null
    };
    next
}

/// Counts this attempt as a failure up front, so parallel guesses cannot all
/// pass one check while argon2 runs. Returns the seconds to wait instead when
/// the bucket is still cooling down.
fn reserve_attempt(conn: &Connection, now: f64) -> DbResult<i64> {
    let state = db::get_config_value(conn, THROTTLE_KEY)?.unwrap_or_else(|| json!({}));
    let wait = bucket_wait(&state, now);
    if wait == 0 {
        db::set_config_value(conn, THROTTLE_KEY, &bucket_after_failure(&state, now))?;
    }
    Ok(wait)
}

fn reset_throttle(conn: &Connection) -> DbResult<()> {
    db::set_config_value(
        conn,
        THROTTLE_KEY,
        &json!({"failures": 0, "next_allowed_at": null}),
    )
}

fn devices(conn: &Connection) -> DbResult<Vec<Value>> {
    Ok(match db::get_config_value(conn, TRUSTED_DEVICES_KEY)? {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    })
}

/// Every successful sign-in trusts the browser: attempts from a trusted
/// browser are throttled on its own counters, so anonymous guessing can only
/// exhaust the shared bucket, never the owner's known browsers.
pub(crate) fn trust_device(
    conn: &Connection,
    now: f64,
    replacing: Option<&str>,
) -> DbResult<String> {
    let device_id = random_token(32);
    let dropped = replacing.map(hash_token);
    let entry = json!({
        "hash": hash_token(&device_id),
        "created_at": now.trunc() as i64,
        "failures": 0,
        "next_allowed_at": null,
    });
    let kept: Vec<Value> = std::iter::once(entry)
        .chain(
            devices(conn)?
                .into_iter()
                .filter(|d| d["hash"].as_str() != dropped.as_deref()),
        )
        .take(MAX_TRUSTED_DEVICES)
        .collect();
    db::set_config_value(conn, TRUSTED_DEVICES_KEY, &Value::Array(kept))?;
    Ok(device_id)
}

pub(crate) fn is_trusted(conn: &Connection, device_id: &str) -> DbResult<bool> {
    if device_id.is_empty() {
        return Ok(false);
    }
    let digest = hash_token(device_id);
    Ok(devices(conn)?
        .iter()
        .any(|d| d["hash"].as_str() == Some(digest.as_str())))
}

/// Forgets every trusted browser except the one asking, which would otherwise
/// fall into the shared bucket anyone can keep exhausted.
pub(crate) fn clear_trusted_devices(
    conn: &Connection,
    keep_device_id: Option<&str>,
) -> DbResult<()> {
    let kept = keep_device_id.map(hash_token);
    let remaining: Vec<Value> = devices(conn)?
        .into_iter()
        .filter(|d| kept.is_some() && d["hash"].as_str() == kept.as_deref())
        .collect();
    db::set_config_value(conn, TRUSTED_DEVICES_KEY, &Value::Array(remaining))
}

/// `None` means the device is not trusted. Reaching the failure cap drops
/// the device, since that looks like a stolen cookie.
fn reserve_device_attempt(conn: &Connection, device_id: &str, now: f64) -> DbResult<Option<i64>> {
    let digest = hash_token(device_id);
    let is_this = |d: &Value| d["hash"].as_str() == Some(digest.as_str());
    let all = devices(conn)?;
    let Some(entry) = all.iter().find(|d| is_this(d)) else {
        return Ok(None);
    };
    let wait = bucket_wait(entry, now);
    if wait > 0 {
        return Ok(Some(wait));
    }
    let updated = bucket_after_failure(entry, now);
    let dropped = updated["failures"].as_i64().unwrap_or(0) >= DEVICE_MAX_FAILURES;
    let remaining: Vec<Value> = all
        .iter()
        .filter_map(|d| match is_this(d) {
            false => Some(d.clone()),
            true if dropped => None,
            true => Some(updated.clone()),
        })
        .collect();
    db::set_config_value(conn, TRUSTED_DEVICES_KEY, &Value::Array(remaining))?;
    Ok(Some(0))
}

fn reset_device(conn: &Connection, device_id: &str) -> DbResult<()> {
    let digest = hash_token(device_id);
    let all: Vec<Value> = devices(conn)?
        .into_iter()
        .map(|mut d| {
            if d["hash"].as_str() == Some(digest.as_str()) {
                d["failures"] = json!(0);
                d["next_allowed_at"] = Value::Null;
            }
            d
        })
        .collect();
    db::set_config_value(conn, TRUSTED_DEVICES_KEY, &Value::Array(all))
}

/// A password check that has counted its attempt and is ready to verify.
pub struct Reserved {
    on_device: bool,
    pub stored: Option<String>,
}

/// First half of a check, under the database lock: counts this attempt, or
/// returns the seconds to wait. A throttled attempt is never verified, so
/// timing reveals nothing.
pub fn reserve_check(
    conn: &Connection,
    now: f64,
    device_id: Option<&str>,
) -> DbResult<Result<Reserved, i64>> {
    let device_wait = match device_id {
        Some(id) => reserve_device_attempt(conn, id, now)?,
        None => None,
    };
    let on_device = device_wait.is_some();
    let wait = match device_wait {
        Some(wait) => wait,
        None => reserve_attempt(conn, now)?,
    };
    if wait > 0 {
        return Ok(Err(wait));
    }
    Ok(Ok(Reserved {
        on_device,
        stored: stored_hash(conn)?,
    }))
}

/// Last half of a check, under the database lock: a success clears the
/// failures this attempt was counted as and stores a refreshed hash. Returns
/// whether the attempt stands: a password changed while it was being checked
/// voids a match against the old one.
pub fn settle_check(
    conn: &Connection,
    device_id: Option<&str>,
    reserved: &Reserved,
    verdict: &Verdict,
) -> DbResult<bool> {
    if !verdict.verified || stored_hash(conn)? != verdict.stored {
        return Ok(false);
    }
    store_rehash(conn, verdict)?;
    match device_id {
        Some(id) if reserved.on_device => reset_device(conn, id)?,
        _ => reset_throttle(conn)?,
    }
    Ok(true)
}

/// Per-client sliding window in front of the account throttle, so one address
/// cannot keep the owner locked out by burning the shared backoff.
#[derive(Clone, Default)]
pub struct AttemptLimiter(Arc<Mutex<HashMap<String, VecDeque<f64>>>>);

impl AttemptLimiter {
    /// Records an attempt and returns 0, or returns the seconds to wait.
    pub(crate) fn retry_after(&self, key: &str, now: f64) -> i64 {
        let cutoff = now - IP_ATTEMPT_WINDOW_SECONDS;
        let mut attempts = self.0.lock().unwrap_or_else(|p| p.into_inner());
        attempts.retain(|_, stamps| stamps.back().is_some_and(|last| *last > cutoff));
        let mut recent: VecDeque<f64> = attempts
            .get(key)
            .map(|s| s.iter().copied().filter(|t| *t > cutoff).collect())
            .unwrap_or_default();
        let wait = if recent.len() >= IP_ATTEMPT_LIMIT {
            ((recent[0] + IP_ATTEMPT_WINDOW_SECONDS - now).ceil() as i64).max(1)
        } else {
            recent.push_back(now);
            0
        };
        attempts.insert(key.to_string(), recent);
        wait
    }
}

/// The one-time code that proves the claimant can read the server log. Held
/// in memory only; a restart issues a new one. Wrong guesses never rotate it,
/// or any anonymous client could invalidate the code in the owner's log.
#[derive(Clone, Default)]
pub struct SetupCode(Arc<Mutex<Option<String>>>);

fn normalize_code(value: &str) -> Vec<u8> {
    value
        .to_uppercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .into_bytes()
}

impl SetupCode {
    pub fn current(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub(crate) fn ensure(&self, conn: &Connection, auth_disabled: bool) -> DbResult<()> {
        let mut code = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if auth_disabled || owner_claimed(conn)? {
            *code = None;
        } else if code.is_none() {
            *code = Some(Self::issue());
        }
        Ok(())
    }

    pub(crate) fn reissue(&self) -> String {
        let fresh = Self::issue();
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(fresh.clone());
        fresh
    }

    pub(crate) fn check(&self, submitted: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_deref()
            .is_some_and(|code| constant_time_eq(&normalize_code(submitted), &normalize_code(code)))
    }

    pub(crate) fn discard(&self) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    fn issue() -> String {
        let mut rng = rand::rng();
        let code = (0..4)
            .map(|_| {
                (0..4)
                    .map(|_| {
                        char::from(
                            SETUP_CODE_ALPHABET[rng.random_range(0..SETUP_CODE_ALPHABET.len())],
                        )
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("-");
        crate::log_warn!(
            "auth_core",
            "ActivSync is unclaimed. Open the app and enter setup code {code}"
        );
        code
    }
}
