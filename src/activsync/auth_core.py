"""The owner's credential, login throttling, the setup code, and sessions.

Sessions are server-side: the browser holds an opaque token and the database
holds only its SHA-256, so a copied database cannot be replayed as a cookie.
"""

from __future__ import annotations

import dataclasses
import hashlib
import logging
import math
import os
import secrets
import sqlite3
import threading
from collections import deque
from dataclasses import dataclass

from argon2 import PasswordHasher, profiles
from argon2.exceptions import InvalidHashError, VerificationError

from activsync import db

logger = logging.getLogger("activsync.auth")

OWNER_KEY = "auth_owner"
THROTTLE_KEY = "auth_login_throttle"
STATE_SECRET_KEY = "auth_state_secret"

PASSWORD_MIN_LENGTH = 15
PASSWORD_MAX_LENGTH = 256

SESSION_IDLE_SECONDS = 7 * 24 * 3600
SESSION_ABSOLUTE_SECONDS = 30 * 24 * 3600
LAST_SEEN_WRITE_SECONDS = 5 * 60

FREE_FAILURES = 5
MAX_DELAY_SECONDS = 900

IP_ATTEMPT_LIMIT = 10
IP_ATTEMPT_WINDOW_SECONDS = 60

SETUP_CODE_ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789"

def _env_enabled(name: str) -> bool:
    return os.environ.get(name, "").lower() in ("1", "true", "yes")


def auth_disabled() -> bool:
    return _env_enabled("ACTIVSYNC_AUTH_DISABLED")


def public_url() -> str | None:
    value = os.environ.get("ACTIVSYNC_PUBLIC_URL", "").strip().rstrip("/")
    return value or None


def allowed_hosts() -> tuple[str, ...]:
    value = os.environ.get("ACTIVSYNC_ALLOWED_HOSTS", "")
    return tuple(entry.strip() for entry in value.split(",") if entry.strip())


def _hasher() -> PasswordHasher:
    if _env_enabled("ACTIVSYNC_ARGON2_TEST_PARAMS"):
        return PasswordHasher.from_parameters(profiles.CHEAPEST)
    return PasswordHasher()


def password_problem(password: str) -> str | None:
    if len(password) < PASSWORD_MIN_LENGTH:
        return (
            f"Use at least {PASSWORD_MIN_LENGTH} characters. "
            "A few unrelated words make a good passphrase."
        )
    if len(password) > PASSWORD_MAX_LENGTH:
        return f"Use at most {PASSWORD_MAX_LENGTH} characters."
    return None


def owner_claimed(conn: sqlite3.Connection) -> bool:
    owner = db.get_config_value(conn, OWNER_KEY) or {}
    return bool(owner.get("password_hash"))


def set_password(conn: sqlite3.Connection, password: str, now: float) -> None:
    problem = password_problem(password)
    if problem:
        raise ValueError(problem)
    db.set_config_value(
        conn,
        OWNER_KEY,
        {"password_hash": _hasher().hash(password), "password_changed_at": int(now)},
    )


def verify_password(conn: sqlite3.Connection, password: str) -> bool:
    owner = db.get_config_value(conn, OWNER_KEY) or {}
    stored = owner.get("password_hash")
    if not stored:
        return False
    hasher = _hasher()
    try:
        hasher.verify(stored, password)
    except (VerificationError, InvalidHashError):
        return False
    if hasher.check_needs_rehash(stored):
        db.set_config_value(
            conn, OWNER_KEY, {**owner, "password_hash": hasher.hash(password)}
        )
    return True


def bucket_wait(state: dict, now: float) -> int:
    next_allowed_at = state.get("next_allowed_at")
    if not next_allowed_at or next_allowed_at <= now:
        return 0
    return math.ceil(next_allowed_at - now)


def bucket_after_failure(state: dict, now: float) -> dict:
    failures = int(state.get("failures", 0)) + 1
    delay = (
        min(2 ** (failures - FREE_FAILURES), MAX_DELAY_SECONDS)
        if failures >= FREE_FAILURES
        else 0
    )
    return {**state, "failures": failures, "next_allowed_at": now + delay if delay else None}


def throttle_wait(conn: sqlite3.Connection, now: float) -> int:
    return bucket_wait(db.get_config_value(conn, THROTTLE_KEY) or {}, now)


def record_failure(conn: sqlite3.Connection, now: float) -> None:
    with db.transaction(conn):
        state = db.get_config_value(conn, THROTTLE_KEY) or {}
        db.set_config_value(conn, THROTTLE_KEY, bucket_after_failure(state, now))


def reserve_attempt(conn: sqlite3.Connection, now: float) -> int:
    """Return the seconds to wait, or 0 after counting this attempt as a
    failure up front, so parallel guesses cannot all pass one check while
    argon2 runs. Success undoes it with `reset_throttle`."""
    with db.transaction(conn):
        state = db.get_config_value(conn, THROTTLE_KEY) or {}
        wait = bucket_wait(state, now)
        if not wait:
            db.set_config_value(conn, THROTTLE_KEY, bucket_after_failure(state, now))
    return wait


def reset_throttle(conn: sqlite3.Connection) -> None:
    db.set_config_value(conn, THROTTLE_KEY, {"failures": 0, "next_allowed_at": None})


class AttemptLimiter:
    """Per-client sliding window in front of the account throttle, so one
    address cannot keep the owner locked out by burning the shared backoff."""

    def __init__(
        self,
        limit: int = IP_ATTEMPT_LIMIT,
        window_seconds: int = IP_ATTEMPT_WINDOW_SECONDS,
    ) -> None:
        self._limit = limit
        self._window = window_seconds
        self._attempts: dict[str, deque[float]] = {}
        self._lock = threading.Lock()

    def retry_after(self, key: str, now: float) -> int:
        """Record an attempt and return 0, or return the seconds to wait."""
        cutoff = now - self._window
        with self._lock:
            self._attempts = {
                client: stamps
                for client, stamps in self._attempts.items()
                if stamps and stamps[-1] > cutoff
            }
            recent = deque(stamp for stamp in self._attempts.get(key, ()) if stamp > cutoff)
            if len(recent) >= self._limit:
                self._attempts[key] = recent
                return max(1, math.ceil(recent[0] + self._window - now))
            recent.append(now)
            self._attempts[key] = recent
            return 0


def _new_setup_code() -> str:
    raw = "".join(secrets.choice(SETUP_CODE_ALPHABET) for _ in range(16))
    return "-".join(raw[i : i + 4] for i in range(0, 16, 4))


def _normalize_code(value: str) -> bytes:
    return "".join(ch for ch in value.upper() if ch.isalnum()).encode()


class SetupCode:
    """The one-time code that proves the claimant can read the server log.

    Held in memory only; a restart issues a new one. Wrong guesses never
    rotate it: rotation would let any anonymous client invalidate the code in
    the owner's log at will. Sixteen characters (~79 bits) keep guessing out of
    reach even for a distributed attacker, so no global cap is needed.
    """

    def __init__(self, conn: sqlite3.Connection) -> None:
        self._conn = conn
        self._code: str | None = None
        self._lock = threading.Lock()

    @property
    def current(self) -> str | None:
        return self._code

    def ensure(self) -> None:
        with self._lock:
            if auth_disabled() or owner_claimed(self._conn):
                self._code = None
            elif self._code is None:
                self._issue()

    def reissue(self) -> str:
        with self._lock:
            self._issue()
            return self._code

    def check(self, submitted: str) -> bool:
        with self._lock:
            if self._code is None:
                return False
            return secrets.compare_digest(
                _normalize_code(submitted), _normalize_code(self._code)
            )

    def discard(self) -> None:
        with self._lock:
            self._code = None

    def _issue(self) -> None:
        self._code = _new_setup_code()
        logger.warning(
            "ActivSync is unclaimed. Open the app and enter setup code %s", self._code
        )


@dataclass(frozen=True)
class Session:
    token_hash: str
    public_id: str
    method: str
    created_at: int
    last_seen_at: int
    expires_at: int
    ip: str
    user_agent: str


def hash_token(token: str) -> str:
    return hashlib.sha256(token.encode()).hexdigest()


def create_session(
    conn: sqlite3.Connection,
    *,
    method: str,
    ip: str,
    user_agent: str,
    now: float,
) -> str:
    token = secrets.token_urlsafe(32)
    issued = int(now)
    with db.transaction(conn):
        conn.execute(
            """INSERT INTO auth_sessions
               (token_hash, public_id, method, created_at, last_seen_at,
                expires_at, ip, user_agent)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
            (
                hash_token(token),
                secrets.token_urlsafe(12),
                method,
                issued,
                issued,
                issued + SESSION_ABSOLUTE_SECONDS,
                ip[:64],
                user_agent[:256],
            ),
        )
    return token


def resolve_session(
    conn: sqlite3.Connection, token: str, now: float
) -> Session | None:
    row = conn.execute(
        "SELECT * FROM auth_sessions WHERE token_hash = ?", (hash_token(token),)
    ).fetchone()
    if row is None:
        return None
    session = Session(**dict(row))
    if now >= session.expires_at or now - session.last_seen_at >= SESSION_IDLE_SECONDS:
        revoke_session(conn, session.token_hash)
        return None
    if now - session.last_seen_at >= LAST_SEEN_WRITE_SECONDS:
        with db.transaction(conn):
            conn.execute(
                "UPDATE auth_sessions SET last_seen_at = ? WHERE token_hash = ?",
                (int(now), session.token_hash),
            )
        return dataclasses.replace(session, last_seen_at=int(now))
    return session


def revoke_session(conn: sqlite3.Connection, token_hash: str) -> None:
    with db.transaction(conn):
        conn.execute("DELETE FROM auth_sessions WHERE token_hash = ?", (token_hash,))


def revoke_public_id(conn: sqlite3.Connection, public_id: str) -> bool:
    with db.transaction(conn):
        cursor = conn.execute(
            "DELETE FROM auth_sessions WHERE public_id = ?", (public_id,)
        )
    return cursor.rowcount > 0


def revoke_all(conn: sqlite3.Connection, *, keep_token_hash: str | None = None) -> int:
    with db.transaction(conn):
        cursor = conn.execute(
            "DELETE FROM auth_sessions WHERE token_hash != ?", (keep_token_hash or "",)
        )
    return cursor.rowcount


def revoke_method(
    conn: sqlite3.Connection, method: str, *, keep_token_hash: str | None = None
) -> int:
    with db.transaction(conn):
        cursor = conn.execute(
            "DELETE FROM auth_sessions WHERE method = ? AND token_hash != ?",
            (method, keep_token_hash or ""),
        )
    return cursor.rowcount


def list_sessions(conn: sqlite3.Connection) -> list[Session]:
    rows = conn.execute(
        "SELECT * FROM auth_sessions ORDER BY last_seen_at DESC"
    ).fetchall()
    return [Session(**dict(row)) for row in rows]


def prune_expired_sessions(conn: sqlite3.Connection, now: float) -> int:
    with db.transaction(conn):
        cursor = conn.execute(
            "DELETE FROM auth_sessions WHERE expires_at <= ? OR last_seen_at <= ?",
            (int(now), int(now) - SESSION_IDLE_SECONDS),
        )
    return cursor.rowcount


def state_secret(conn: sqlite3.Connection) -> str:
    with db.transaction(conn):
        secret = db.get_config_value(conn, STATE_SECRET_KEY)
        if not secret:
            secret = secrets.token_hex(32)
            db.set_config_value(conn, STATE_SECRET_KEY, secret)
    return secret


LOOPBACK_HOSTS = frozenset({"127.0.0.1", "localhost", "::1"})


def warn_if_disabled_on_network(bind_host: str) -> None:
    if auth_disabled() and bind_host not in LOOPBACK_HOSTS:
        logger.error(
            "Authentication is disabled (ACTIVSYNC_AUTH_DISABLED) and ActivSync "
            "listens on %s. Anyone who can reach it has full access.",
            bind_host,
        )
