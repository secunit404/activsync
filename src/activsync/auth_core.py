"""The owner's credential, login throttling, the setup code, and sessions.

Sessions are server-side: the browser holds an opaque token and the database
holds only its SHA-256, so a copied database cannot be replayed as a cookie.
"""

from __future__ import annotations

import logging
import os
import sqlite3

from argon2 import PasswordHasher, profiles
from argon2.exceptions import InvalidHashError, VerificationError

from activsync import db

logger = logging.getLogger("activsync.auth")

OWNER_KEY = "auth_owner"

PASSWORD_MIN_LENGTH = 15
PASSWORD_MAX_LENGTH = 256


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
