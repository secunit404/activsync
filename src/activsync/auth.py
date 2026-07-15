"""Single-user password auth.

Passwords are stretched with PBKDF2-HMAC-SHA256 (algorithm + parameters stored
alongside the hash so they can be raised later). Sessions are opaque per-session
tokens: the client holds a random token, the DB stores only its SHA-256 hash with
an expiry, so sessions can be expired and revoked without exposing a shared secret.
"""

from __future__ import annotations

import hashlib
import hmac
import os
import secrets
import sqlite3
from datetime import datetime, timedelta, timezone

from fastapi import Request

from activsync import db

SESSION_COOKIE = "activsync_session"
SESSION_TTL = timedelta(days=30)
_DEFAULT_ITERATIONS = 600_000


def _iterations() -> int:
    """PBKDF2 iteration count; overridable via env so tests can run fast."""
    return int(os.environ.get("ACTIVSYNC_PBKDF2_ITERATIONS", os.environ.get("G2S_PBKDF2_ITERATIONS", str(_DEFAULT_ITERATIONS))))


def _hash_password(password: str, salt: str, iterations: int) -> str:
    derived = hashlib.pbkdf2_hmac(
        "sha256", password.encode("utf-8"), bytes.fromhex(salt), iterations
    )
    return derived.hex()


def _token_hash(token: str) -> str:
    return hashlib.sha256(token.encode("utf-8")).hexdigest()


def is_configured(conn: sqlite3.Connection) -> bool:
    return db.get_config_value(conn, "auth") is not None


def set_password(conn: sqlite3.Connection, password: str) -> None:
    salt = secrets.token_hex(16)
    iterations = _iterations()
    db.set_config_value(conn, "auth", {
        "algo": "pbkdf2_sha256",
        "iterations": iterations,
        "salt": salt,
        "password_hash": _hash_password(password, salt, iterations),
    })
    # A password change revokes every existing session.
    db.delete_all_sessions(conn)


def verify_password(conn: sqlite3.Connection, password: str) -> bool:
    record = db.get_config_value(conn, "auth")
    if not record:
        return False
    candidate = _hash_password(password, record["salt"], record["iterations"])
    return hmac.compare_digest(candidate, record["password_hash"])


def create_session(conn: sqlite3.Connection, now: datetime | None = None) -> str:
    """Create a session and return the raw token to store in the client's cookie."""
    now = now or datetime.now(timezone.utc)
    token = secrets.token_urlsafe(32)
    db.insert_session(
        conn, _token_hash(token), now.isoformat(), (now + SESSION_TTL).isoformat()
    )
    return token


def is_logged_in(
    conn: sqlite3.Connection, request: Request, now: datetime | None = None
) -> bool:
    if (os.environ.get("ACTIVSYNC_DEV_MOCK_DATA", os.environ.get("G2S_DEV_MOCK_DATA", ""))).lower() in ("1", "true", "yes"):
        return True
    cookie = request.cookies.get(SESSION_COOKIE)
    if not cookie:
        return False
    row = db.get_session(conn, _token_hash(cookie))
    if row is None:
        return False
    now = now or datetime.now(timezone.utc)
    if datetime.fromisoformat(row["expires_at"]) < now:
        db.delete_session(conn, _token_hash(cookie))
        return False
    return True


def destroy_session(conn: sqlite3.Connection, request: Request) -> None:
    cookie = request.cookies.get(SESSION_COOKIE)
    if cookie:
        db.delete_session(conn, _token_hash(cookie))
