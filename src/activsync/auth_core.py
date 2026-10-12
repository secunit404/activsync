"""The owner's credential, login throttling, the setup code, and sessions.

Sessions are server-side: the browser holds an opaque token and the database
holds only its SHA-256, so a copied database cannot be replayed as a cookie.
"""

from __future__ import annotations

import logging
import math
import os
import secrets
import sqlite3
import threading
from collections import deque

from argon2 import PasswordHasher, profiles
from argon2.exceptions import InvalidHashError, VerificationError

from activsync import db

logger = logging.getLogger("activsync.auth")

OWNER_KEY = "auth_owner"
THROTTLE_KEY = "auth_login_throttle"

PASSWORD_MIN_LENGTH = 15
PASSWORD_MAX_LENGTH = 256

FREE_FAILURES = 5
MAX_DELAY_SECONDS = 900

IP_ATTEMPT_LIMIT = 10
IP_ATTEMPT_WINDOW_SECONDS = 60

SETUP_CODE_ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789"
SETUP_CODE_MAX_FAILURES = 5


def _env_enabled(name: str) -> bool:
    return os.environ.get(name, "").lower() in ("1", "true", "yes")


def auth_disabled() -> bool:
    return _env_enabled("ACTIVSYNC_AUTH_DISABLED")


def public_url() -> str | None:
    value = os.environ.get("ACTIVSYNC_PUBLIC_URL", "").strip().rstrip("/")
    return value or None


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


def throttle_wait(conn: sqlite3.Connection, now: float) -> int:
    state = db.get_config_value(conn, THROTTLE_KEY) or {}
    next_allowed_at = state.get("next_allowed_at")
    if not next_allowed_at or next_allowed_at <= now:
        return 0
    return math.ceil(next_allowed_at - now)


def record_failure(conn: sqlite3.Connection, now: float) -> None:
    with db.transaction(conn):
        state = db.get_config_value(conn, THROTTLE_KEY) or {}
        failures = int(state.get("failures", 0)) + 1
        delay = (
            min(2 ** (failures - FREE_FAILURES), MAX_DELAY_SECONDS)
            if failures >= FREE_FAILURES
            else 0
        )
        db.set_config_value(
            conn,
            THROTTLE_KEY,
            {"failures": failures, "next_allowed_at": now + delay if delay else None},
        )


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
    raw = "".join(secrets.choice(SETUP_CODE_ALPHABET) for _ in range(8))
    return f"{raw[:4]}-{raw[4:]}"


def _normalize_code(value: str) -> bytes:
    return "".join(ch for ch in value.upper() if ch.isalnum()).encode()


class SetupCode:
    """The one-time code that proves the claimant can read the server log.

    Held in memory only: a restart issues a new one, and so do five wrong
    guesses, so the code is never worth more than five attempts.
    """

    def __init__(self, conn: sqlite3.Connection) -> None:
        self._conn = conn
        self._code: str | None = None
        self._failures = 0
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
            if secrets.compare_digest(_normalize_code(submitted), _normalize_code(self._code)):
                return True
            self._failures += 1
            if self._failures >= SETUP_CODE_MAX_FAILURES:
                self._issue()
            return False

    def discard(self) -> None:
        with self._lock:
            self._code = None

    def _issue(self) -> None:
        self._code = _new_setup_code()
        self._failures = 0
        logger.warning(
            "ActivSync is unclaimed. Open the app and enter setup code %s", self._code
        )
