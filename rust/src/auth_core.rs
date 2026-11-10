//! The owner's credential, login throttling, the setup code and sessions,
//! stored exactly as the Python app's `auth_core.py` and `auth_devices.py`
//! store them, so a database moves between the two servers.
//!
//! Sessions are server-side: the browser holds an opaque token and the
//! database only its SHA-256, so a copied database cannot be replayed.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use rand::{Rng, RngCore};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::db::{self, DbResult};

pub const OWNER_KEY: &str = "auth_owner";
const THROTTLE_KEY: &str = "auth_login_throttle";
const TRUSTED_DEVICES_KEY: &str = "auth_trusted_devices";

pub const PASSWORD_MIN_LENGTH: usize = 15;
pub const PASSWORD_MAX_LENGTH: usize = 256;

pub const SESSION_IDLE_SECONDS: i64 = 7 * 24 * 3600;
pub const SESSION_ABSOLUTE_SECONDS: i64 = 30 * 24 * 3600;
const LAST_SEEN_WRITE_SECONDS: i64 = 5 * 60;

const FREE_FAILURES: i64 = 5;
const MAX_DELAY_SECONDS: f64 = 900.0;

const IP_ATTEMPT_LIMIT: usize = 10;
const IP_ATTEMPT_WINDOW_SECONDS: f64 = 60.0;

const MAX_TRUSTED_DEVICES: usize = 20;
const DEVICE_MAX_FAILURES: i64 = 10;
pub const DEVICE_COOKIE_MAX_AGE: i64 = 365 * 24 * 3600;

const SETUP_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

pub fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
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
        Self { memory_kib: 65536, time: 3, parallelism: 4, hash_len: 32, salt_len: 16 }
    }

    /// argon2-cffi's CHEAPEST profile, for tests only.
    pub const fn cheapest() -> Self {
        Self { memory_kib: 8, time: 1, parallelism: 1, hash_len: 4, salt_len: 8 }
    }

    fn hasher(&self) -> Result<Argon2<'static>, String> {
        let params = Params::new(self.memory_kib, self.time, self.parallelism, Some(self.hash_len))
            .map_err(|e| e.to_string())?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD_NO_PAD
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
        let [empty, algorithm, version, params, salt, hash] = parts.as_slice() else {
            return None;
        };
        if !empty.is_empty() {
            return None;
        }
        let algorithm = match *algorithm {
            "argon2id" => Algorithm::Argon2id,
            "argon2i" => Algorithm::Argon2i,
            "argon2d" => Algorithm::Argon2d,
            _ => return None,
        };
        let version: u32 = version.strip_prefix("v=")?.parse().ok()?;
        let mut cost = HashCost { memory_kib: 0, time: 0, parallelism: 0, hash_len: 0, salt_len: 0 };
        for pair in params.split(',') {
            let (key, value) = pair.split_once('=')?;
            let value: u32 = value.parse().ok()?;
            match key {
                "m" => cost.memory_kib = value,
                "t" => cost.time = value,
                "p" => cost.parallelism = value,
                _ => return None,
            }
        }
        let salt = b64().decode(salt).ok()?;
        let hash = b64().decode(hash).ok()?;
        cost.salt_len = salt.len();
        cost.hash_len = hash.len();
        Some(Self { algorithm, version, cost, salt, hash })
    }

    fn verify(&self, password: &str) -> bool {
        let version = match self.version {
            0x13 => Version::V0x13,
            0x10 => Version::V0x10,
            _ => return false,
        };
        let Ok(params) = Params::new(self.cost.memory_kib, self.cost.time, self.cost.parallelism, Some(self.hash.len()))
        else {
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

pub fn password_problem(password: &str) -> Option<String> {
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
    let mut salt = vec![0u8; cost.salt_len];
    rand::rng().fill_bytes(&mut salt);
    let mut out = vec![0u8; cost.hash_len];
    cost.hasher()?
        .hash_password_into(password.as_bytes(), &salt, &mut out)
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "$argon2id$v=19$m={},t={},p={}${}${}",
        cost.memory_kib,
        cost.time,
        cost.parallelism,
        b64().encode(&salt),
        b64().encode(&out)
    ))
}

fn owner(conn: &Connection) -> DbResult<Value> {
    Ok(db::get_config_value(conn, OWNER_KEY)?.filter(Value::is_object).unwrap_or_else(|| json!({})))
}

pub fn owner_claimed(conn: &Connection) -> DbResult<bool> {
    Ok(db::truthy(owner(conn)?.get("password_hash")))
}

fn owner_record(password_hash: &str, now: f64) -> Value {
    json!({"password_hash": password_hash, "password_changed_at": now.trunc() as i64})
}

pub fn set_password_hash(conn: &Connection, password_hash: &str, now: f64) -> DbResult<()> {
    db::set_config_value(conn, OWNER_KEY, &owner_record(password_hash, now))
}

/// Stores the first owner's password, or returns false if someone claimed
/// first. The caller hashes beforehand: argon2 is slow, and the check and the
/// write must share one lock.
pub fn claim_owner(conn: &Connection, password_hash: &str, now: f64) -> DbResult<bool> {
    if owner_claimed(conn)? {
        return Ok(false);
    }
    set_password_hash(conn, password_hash, now)?;
    Ok(true)
}

/// Verifies against the stored hash, rehashing it when it was made with other
/// cost settings.
pub fn verify_password(conn: &Connection, password: &str, cost: HashCost) -> DbResult<bool> {
    let record = owner(conn)?;
    let Some(stored) = record.get("password_hash").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
        return Ok(false);
    };
    let Some(parsed) = Encoded::parse(stored) else {
        return Ok(false);
    };
    if !parsed.verify(password) {
        return Ok(false);
    }
    if !parsed.uses(&cost) {
        if let Ok(rehashed) = hash_password(password, cost) {
            let mut updated = record.clone();
            updated["password_hash"] = json!(rehashed);
            db::set_config_value(conn, OWNER_KEY, &updated)?;
        }
    }
    Ok(true)
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
        2f64.powi((failures - FREE_FAILURES) as i32).min(MAX_DELAY_SECONDS)
    } else {
        0.0
    };
    let mut next = if state.is_object() { state.clone() } else { json!({}) };
    next["failures"] = json!(failures);
    next["next_allowed_at"] = if delay > 0.0 { json!(now + delay) } else { Value::Null };
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
    db::set_config_value(conn, THROTTLE_KEY, &json!({"failures": 0, "next_allowed_at": null}))
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
pub fn trust_device(conn: &Connection, now: f64, replacing: Option<&str>) -> DbResult<String> {
    let device_id = random_token(32);
    let dropped = replacing.map(hash_token);
    let entry = json!({
        "hash": hash_token(&device_id),
        "created_at": now.trunc() as i64,
        "failures": 0,
        "next_allowed_at": null,
    });
    let mut kept: Vec<Value> = vec![entry];
    kept.extend(devices(conn)?.into_iter().filter(|d| d["hash"].as_str() != dropped.as_deref()));
    kept.truncate(MAX_TRUSTED_DEVICES);
    db::set_config_value(conn, TRUSTED_DEVICES_KEY, &Value::Array(kept))?;
    Ok(device_id)
}

pub fn is_trusted(conn: &Connection, device_id: Option<&str>) -> DbResult<bool> {
    let Some(device_id) = device_id.filter(|d| !d.is_empty()) else {
        return Ok(false);
    };
    let digest = hash_token(device_id);
    Ok(devices(conn)?.iter().any(|d| d["hash"].as_str() == Some(digest.as_str())))
}

/// Forgets every trusted browser except the one asking, which would otherwise
/// fall into the shared bucket anyone can keep exhausted.
pub fn clear_trusted_devices(conn: &Connection, keep_device_id: Option<&str>) -> DbResult<()> {
    let kept = keep_device_id.map(hash_token);
    let remaining: Vec<Value> =
        devices(conn)?.into_iter().filter(|d| kept.is_some() && d["hash"].as_str() == kept.as_deref()).collect();
    db::set_config_value(conn, TRUSTED_DEVICES_KEY, &Value::Array(remaining))
}

/// `None` means the device is not trusted. Reaching the failure cap drops
/// the device, since that looks like a stolen cookie.
fn reserve_device_attempt(conn: &Connection, device_id: &str, now: f64) -> DbResult<Option<i64>> {
    let digest = hash_token(device_id);
    let all = devices(conn)?;
    let Some(entry) = all.iter().find(|d| d["hash"].as_str() == Some(digest.as_str())).cloned() else {
        return Ok(None);
    };
    let wait = bucket_wait(&entry, now);
    if wait > 0 {
        return Ok(Some(wait));
    }
    let updated = bucket_after_failure(&entry, now);
    let failures = updated["failures"].as_i64().unwrap_or(0);
    let remaining: Vec<Value> = all
        .into_iter()
        .filter_map(|d| {
            if d["hash"].as_str() != Some(digest.as_str()) {
                Some(d)
            } else if failures >= DEVICE_MAX_FAILURES {
                None
            } else {
                Some(updated.clone())
            }
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

#[derive(Debug, PartialEq)]
pub struct PasswordCheck {
    pub verified: bool,
    pub retry_after: i64,
}

/// Reserve, then verify: a throttled attempt is never verified, so timing
/// reveals nothing.
pub fn check_password(
    conn: &Connection,
    password: &str,
    now: f64,
    device_id: Option<&str>,
    cost: HashCost,
) -> DbResult<PasswordCheck> {
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
        return Ok(PasswordCheck { verified: false, retry_after: wait });
    }
    if !verify_password(conn, password, cost)? {
        return Ok(PasswordCheck { verified: false, retry_after: 0 });
    }
    match device_id {
        Some(id) if on_device => reset_device(conn, id)?,
        _ => reset_throttle(conn)?,
    }
    Ok(PasswordCheck { verified: true, retry_after: 0 })
}

/// Per-client sliding window in front of the account throttle, so one address
/// cannot keep the owner locked out by burning the shared backoff.
#[derive(Clone, Default)]
pub struct AttemptLimiter(Arc<Mutex<HashMap<String, VecDeque<f64>>>>);

impl AttemptLimiter {
    /// Records an attempt and returns 0, or returns the seconds to wait.
    pub fn retry_after(&self, key: &str, now: f64) -> i64 {
        let cutoff = now - IP_ATTEMPT_WINDOW_SECONDS;
        let mut attempts = self.0.lock().unwrap_or_else(|p| p.into_inner());
        attempts.retain(|_, stamps| stamps.back().is_some_and(|last| *last > cutoff));
        let recent: VecDeque<f64> =
            attempts.get(key).map(|s| s.iter().copied().filter(|t| *t > cutoff).collect()).unwrap_or_default();
        if recent.len() >= IP_ATTEMPT_LIMIT {
            let wait = (recent[0] + IP_ATTEMPT_WINDOW_SECONDS - now).ceil() as i64;
            attempts.insert(key.to_string(), recent);
            return wait.max(1);
        }
        let mut recent = recent;
        recent.push_back(now);
        attempts.insert(key.to_string(), recent);
        0
    }
}

/// The one-time code that proves the claimant can read the server log. Held
/// in memory only; a restart issues a new one. Wrong guesses never rotate it,
/// or any anonymous client could invalidate the code in the owner's log.
#[derive(Clone, Default)]
pub struct SetupCode(Arc<Mutex<Option<String>>>);

fn new_setup_code() -> String {
    let mut rng = rand::rng();
    let raw: Vec<char> = (0..16)
        .map(|_| SETUP_CODE_ALPHABET[rng.random_range(0..SETUP_CODE_ALPHABET.len())] as char)
        .collect();
    raw.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

fn normalize_code(value: &str) -> Vec<u8> {
    value.to_uppercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>().into_bytes()
}

impl SetupCode {
    pub fn current(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn ensure(&self, conn: &Connection, auth_disabled: bool) -> DbResult<()> {
        let mut code = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if auth_disabled || owner_claimed(conn)? {
            *code = None;
        } else if code.is_none() {
            *code = Some(Self::issue());
        }
        Ok(())
    }

    pub fn reissue(&self) -> String {
        let fresh = Self::issue();
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(fresh.clone());
        fresh
    }

    pub fn check(&self, submitted: &str) -> bool {
        match &*self.0.lock().unwrap_or_else(|p| p.into_inner()) {
            Some(code) => constant_time_eq(&normalize_code(submitted), &normalize_code(code)),
            None => false,
        }
    }

    pub fn discard(&self) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    fn issue() -> String {
        let code = new_setup_code();
        eprintln!("ActivSync is unclaimed. Open the app and enter setup code {code}");
        code
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub token_hash: String,
    pub public_id: String,
    pub method: String,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub expires_at: i64,
    pub ip: String,
    pub user_agent: String,
}

fn session_from_row(row: &rusqlite::Row<'_>) -> DbResult<Session> {
    Ok(Session {
        token_hash: row.get("token_hash")?,
        public_id: row.get("public_id")?,
        method: row.get("method")?,
        created_at: row.get("created_at")?,
        last_seen_at: row.get("last_seen_at")?,
        expires_at: row.get("expires_at")?,
        ip: row.get("ip")?,
        user_agent: row.get("user_agent")?,
    })
}

fn truncate(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

pub fn create_session(conn: &Connection, method: &str, ip: &str, user_agent: &str, now: f64) -> DbResult<String> {
    let token = random_token(32);
    let issued = now.trunc() as i64;
    conn.execute(
        "INSERT INTO auth_sessions
           (token_hash, public_id, method, created_at, last_seen_at, expires_at, ip, user_agent)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7)",
        params![
            hash_token(&token),
            random_token(12),
            method,
            issued,
            issued + SESSION_ABSOLUTE_SECONDS,
            truncate(ip, 64),
            truncate(user_agent, 256)
        ],
    )?;
    Ok(token)
}

pub fn resolve_session(conn: &Connection, token: &str, now: f64) -> DbResult<Option<Session>> {
    let Some(session) = conn
        .query_row("SELECT * FROM auth_sessions WHERE token_hash = ?1", [hash_token(token)], session_from_row)
        .optional()?
    else {
        return Ok(None);
    };
    if now >= session.expires_at as f64 || now - session.last_seen_at as f64 >= SESSION_IDLE_SECONDS as f64 {
        revoke_session(conn, &session.token_hash)?;
        return Ok(None);
    }
    if now - session.last_seen_at as f64 >= LAST_SEEN_WRITE_SECONDS as f64 {
        let seen = now.trunc() as i64;
        conn.execute("UPDATE auth_sessions SET last_seen_at = ?1 WHERE token_hash = ?2", params![seen, session.token_hash])?;
        return Ok(Some(Session { last_seen_at: seen, ..session }));
    }
    Ok(Some(session))
}

pub fn revoke_session(conn: &Connection, token_hash: &str) -> DbResult<()> {
    conn.execute("DELETE FROM auth_sessions WHERE token_hash = ?1", [token_hash])?;
    Ok(())
}

pub fn revoke_public_id(conn: &Connection, public_id: &str) -> DbResult<bool> {
    Ok(conn.execute("DELETE FROM auth_sessions WHERE public_id = ?1", [public_id])? > 0)
}

pub fn revoke_all(conn: &Connection, keep_token_hash: Option<&str>) -> DbResult<usize> {
    conn.execute("DELETE FROM auth_sessions WHERE token_hash != ?1", [keep_token_hash.unwrap_or("")])
}

pub fn list_sessions(conn: &Connection) -> DbResult<Vec<Session>> {
    let mut stmt = conn.prepare("SELECT * FROM auth_sessions ORDER BY last_seen_at DESC")?;
    let rows = stmt.query_map([], session_from_row)?;
    rows.collect()
}

pub fn prune_expired_sessions(conn: &Connection, now: f64) -> DbResult<usize> {
    let now = now.trunc() as i64;
    conn.execute(
        "DELETE FROM auth_sessions WHERE expires_at <= ?1 OR last_seen_at <= ?2",
        params![now, now - SESSION_IDLE_SECONDS],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_hashes_made_by_the_python_app() {
        let db = crate::db::Db::open_in_memory().unwrap();
        let conn = db.lock();
        let python_standard = "$argon2id$v=19$m=65536,t=3,p=4$OHAqpN2S8c4i1Af7aUvD/w$CojUzSnycSLE8is18+Fo5CpR9JdSF+zfrpeS/q4zM9g";
        set_password_hash(&conn, python_standard, 0.0).unwrap();
        assert!(verify_password(&conn, "correct horse battery staple", HashCost::standard()).unwrap());
        assert!(!verify_password(&conn, "wrong horse battery staple", HashCost::standard()).unwrap());
        assert_eq!(owner(&conn).unwrap()["password_hash"], python_standard, "same cost: no rehash");

        let python_cheapest = "$argon2id$v=19$m=8,t=1,p=1$RVVu42Um648$/g/+Hw";
        set_password_hash(&conn, python_cheapest, 0.0).unwrap();
        assert!(verify_password(&conn, "correct horse battery staple", HashCost::cheapest()).unwrap());
        assert_eq!(owner(&conn).unwrap()["password_hash"], python_cheapest);
        assert!(verify_password(&conn, "correct horse battery staple", HashCost::standard()).unwrap());
        let rehashed = owner(&conn).unwrap()["password_hash"].as_str().unwrap().to_string();
        assert!(rehashed.starts_with("$argon2id$v=19$m=65536,t=3,p=4$"), "{rehashed}");
    }

    #[test]
    fn throttle_backs_off_after_five_failures() {
        let db = crate::db::Db::open_in_memory().unwrap();
        let conn = db.lock();
        set_password_hash(&conn, &hash_password("correct horse battery staple", HashCost::cheapest()).unwrap(), 0.0).unwrap();
        for _ in 0..4 {
            assert_eq!(check_password(&conn, "nope", 100.0, None, HashCost::cheapest()).unwrap().retry_after, 0);
        }
        assert_eq!(check_password(&conn, "nope", 100.0, None, HashCost::cheapest()).unwrap().retry_after, 0);
        let blocked = check_password(&conn, "correct horse battery staple", 100.5, None, HashCost::cheapest()).unwrap();
        assert_eq!(blocked, PasswordCheck { verified: false, retry_after: 1 });
        let ok = check_password(&conn, "correct horse battery staple", 102.0, None, HashCost::cheapest()).unwrap();
        assert!(ok.verified);
        assert_eq!(db::get_config_value(&conn, THROTTLE_KEY).unwrap().unwrap()["failures"], 0);
    }

    #[test]
    fn trusted_device_is_throttled_on_its_own() {
        let db = crate::db::Db::open_in_memory().unwrap();
        let conn = db.lock();
        set_password_hash(&conn, &hash_password("correct horse battery staple", HashCost::cheapest()).unwrap(), 0.0).unwrap();
        let device = trust_device(&conn, 0.0, None).unwrap();
        for _ in 0..8 {
            check_password(&conn, "nope", 10.0, None, HashCost::cheapest()).unwrap();
        }
        assert!(check_password(&conn, "nope", 10.0, None, HashCost::cheapest()).unwrap().retry_after > 0);
        assert!(check_password(&conn, "correct horse battery staple", 10.0, Some(&device), HashCost::cheapest()).unwrap().verified);
    }

    #[test]
    fn limiter_allows_ten_a_minute_per_client() {
        let limiter = AttemptLimiter::default();
        for _ in 0..10 {
            assert_eq!(limiter.retry_after("1.2.3.4", 0.0), 0);
        }
        assert_eq!(limiter.retry_after("1.2.3.4", 1.0), 59);
        assert_eq!(limiter.retry_after("5.6.7.8", 1.0), 0);
        assert_eq!(limiter.retry_after("1.2.3.4", 61.0), 0);
    }

    #[test]
    fn setup_code_ignores_case_and_dashes() {
        let code = SetupCode::default();
        let issued = code.reissue();
        assert_eq!(issued.len(), 19);
        assert!(code.check(&issued.to_lowercase().replace('-', " ")));
        assert!(!code.check("AAAA-BBBB-CCCC-DDDD"));
        code.discard();
        assert!(!code.check(&issued));
    }

    #[test]
    fn idle_sessions_expire() {
        let db = crate::db::Db::open_in_memory().unwrap();
        let conn = db.lock();
        let token = create_session(&conn, "password", "1.2.3.4", "ua", 1000.0).unwrap();
        assert!(resolve_session(&conn, &token, 1000.0 + 400.0).unwrap().is_some());
        let later = 1400.0 + SESSION_IDLE_SECONDS as f64;
        assert!(resolve_session(&conn, &token, later).unwrap().is_none());
        assert!(list_sessions(&conn).unwrap().is_empty());
    }
}
