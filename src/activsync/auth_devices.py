"""Trusted browsers for login throttling (OWASP's device-cookie defence).

Every successful sign-in hands the browser a random device id. An attempt
carrying a trusted id is throttled on that device's own counters instead of
the shared bucket, so anonymous guessing can exhaust only the shared bucket,
never the owner's known browsers.
"""

from __future__ import annotations

import secrets
import sqlite3
from dataclasses import dataclass

from activsync import auth_core, db

TRUSTED_DEVICES_KEY = "auth_trusted_devices"
MAX_TRUSTED_DEVICES = 20
DEVICE_MAX_FAILURES = 10
DEVICE_COOKIE_MAX_AGE = 365 * 24 * 3600


@dataclass(frozen=True)
class PasswordCheck:
    verified: bool
    retry_after: int = 0


def _devices(conn: sqlite3.Connection) -> list[dict]:
    return db.get_config_value(conn, TRUSTED_DEVICES_KEY) or []


def trust_device(
    conn: sqlite3.Connection, now: float, *, replacing: str | None = None
) -> str:
    device_id = secrets.token_urlsafe(32)
    dropped = auth_core.hash_token(replacing) if replacing else None
    entry = {
        "hash": auth_core.hash_token(device_id),
        "created_at": int(now),
        "failures": 0,
        "next_allowed_at": None,
    }
    with db.transaction(conn):
        kept = [device for device in _devices(conn) if device["hash"] != dropped]
        db.set_config_value(
            conn, TRUSTED_DEVICES_KEY, [entry, *kept][:MAX_TRUSTED_DEVICES]
        )
    return device_id


def is_trusted(conn: sqlite3.Connection, device_id: str | None) -> bool:
    if not device_id:
        return False
    digest = auth_core.hash_token(device_id)
    return any(device["hash"] == digest for device in _devices(conn))


def clear_trusted_devices(conn: sqlite3.Connection) -> None:
    db.set_config_value(conn, TRUSTED_DEVICES_KEY, [])


def reserve_device_attempt(
    conn: sqlite3.Connection, device_id: str, now: float
) -> int | None:
    """Like `auth_core.reserve_attempt` on the device's own counters. None
    means the device is not trusted. Reaching the failure cap drops it, since
    that looks like a stolen cookie."""
    digest = auth_core.hash_token(device_id)
    with db.transaction(conn):
        devices = _devices(conn)
        entry = next((device for device in devices if device["hash"] == digest), None)
        if entry is None:
            return None
        wait = auth_core.bucket_wait(entry, now)
        if wait:
            return wait
        updated = auth_core.bucket_after_failure(entry, now)
        remaining = (
            [device for device in devices if device["hash"] != digest]
            if updated["failures"] >= DEVICE_MAX_FAILURES
            else [updated if device["hash"] == digest else device for device in devices]
        )
        db.set_config_value(conn, TRUSTED_DEVICES_KEY, remaining)
    return 0


def _reset_device(conn: sqlite3.Connection, device_id: str) -> None:
    digest = auth_core.hash_token(device_id)
    with db.transaction(conn):
        db.set_config_value(
            conn,
            TRUSTED_DEVICES_KEY,
            [
                {**device, "failures": 0, "next_allowed_at": None}
                if device["hash"] == digest
                else device
                for device in _devices(conn)
            ],
        )


def check_password(
    conn: sqlite3.Connection,
    password: str,
    now: float,
    *,
    device_id: str | None = None,
) -> PasswordCheck:
    """Reserve, then verify: a throttled attempt is never verified, so timing
    reveals nothing."""
    wait = reserve_device_attempt(conn, device_id, now) if device_id else None
    on_device = wait is not None
    if wait is None:
        wait = auth_core.reserve_attempt(conn, now)
    if wait:
        return PasswordCheck(verified=False, retry_after=wait)
    if not auth_core.verify_password(conn, password):
        return PasswordCheck(verified=False)
    if on_device:
        _reset_device(conn, device_id)
    else:
        auth_core.reset_throttle(conn)
    return PasswordCheck(verified=True)
