//! Sessions are server-side: the browser holds an opaque token and the
//! database only its SHA-256, so a copied database cannot be replayed.

use rusqlite::{Connection, OptionalExtension, params};

use super::{hash_token, random_token};
use crate::db::DbResult;

pub(crate) const SESSION_ABSOLUTE_SECONDS: i64 = 30 * 24 * 3600;
const SESSION_IDLE_SECONDS: i64 = 7 * 24 * 3600;
const LAST_SEEN_WRITE_SECONDS: i64 = 5 * 60;

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

pub(crate) fn create_session(
    conn: &Connection,
    method: &str,
    ip: &str,
    user_agent: &str,
    now: f64,
) -> DbResult<String> {
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

pub(crate) fn resolve_session(
    conn: &Connection,
    token: &str,
    now: f64,
) -> DbResult<Option<Session>> {
    let Some(session) = conn
        .query_row(
            "SELECT * FROM auth_sessions WHERE token_hash = ?1",
            [hash_token(token)],
            session_from_row,
        )
        .optional()?
    else {
        return Ok(None);
    };
    if now >= session.expires_at as f64
        || now - session.last_seen_at as f64 >= SESSION_IDLE_SECONDS as f64
    {
        revoke_session(conn, &session.token_hash)?;
        return Ok(None);
    }
    if now - session.last_seen_at as f64 >= LAST_SEEN_WRITE_SECONDS as f64 {
        let seen = now.trunc() as i64;
        conn.execute(
            "UPDATE auth_sessions SET last_seen_at = ?1 WHERE token_hash = ?2",
            params![seen, session.token_hash],
        )?;
        return Ok(Some(Session {
            last_seen_at: seen,
            ..session
        }));
    }
    Ok(Some(session))
}

pub(crate) fn revoke_session(conn: &Connection, token_hash: &str) -> DbResult<()> {
    conn.execute(
        "DELETE FROM auth_sessions WHERE token_hash = ?1",
        [token_hash],
    )?;
    Ok(())
}

pub(crate) fn revoke_public_id(conn: &Connection, public_id: &str) -> DbResult<bool> {
    Ok(conn.execute(
        "DELETE FROM auth_sessions WHERE public_id = ?1",
        [public_id],
    )? > 0)
}

pub(crate) fn revoke_all(conn: &Connection, keep_token_hash: Option<&str>) -> DbResult<usize> {
    conn.execute(
        "DELETE FROM auth_sessions WHERE token_hash != ?1",
        [keep_token_hash.unwrap_or("")],
    )
}

pub(crate) fn list_sessions(conn: &Connection) -> DbResult<Vec<Session>> {
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
    fn idle_sessions_expire() {
        let db = crate::db::Db::open_in_memory().unwrap();
        let conn = db.lock();
        let token = create_session(&conn, "password", "1.2.3.4", "ua", 1000.0).unwrap();
        assert!(
            resolve_session(&conn, &token, 1000.0 + 400.0)
                .unwrap()
                .is_some()
        );
        let later = 1400.0 + SESSION_IDLE_SECONDS as f64;
        assert!(resolve_session(&conn, &token, later).unwrap().is_none());
        assert!(list_sessions(&conn).unwrap().is_empty());
    }
}
