"""In-memory fakes for the local dev server (``ACTIVSYNC_DEV_MOCK_DATA``).

These let the entire first-run setup wizard — Garmin login, MFA, Strava OAuth,
and the initial sync — be walked end to end without any real account or network
call. They are wired in only when mock mode is on (see ``server._mock_mode`` and
the ``_build_*`` / login seams); production code paths never reach this module.

Dev conventions, chosen so both the happy path and the error paths of the wizard
can be exercised deliberately:

* Garmin login succeeds for any email/password...
* ...unless the password equals :data:`MFA_TRIGGER_PASSWORD` (``"mfa"``), which
  simulates an MFA challenge so the code-entry modal can be tested.
* During that challenge, :data:`MFA_REJECT_CODE` (``"000000"``) is rejected to
  exercise the error screen; any other code is accepted.
* Strava "OAuth" bounces straight back to the local callback and stores a fake
  token, so no external Strava app or browser round-trip is needed.

Activities are inserted by :mod:`activsync.dev_seed` at startup, so the fake
Garmin client reports no new recent activities — the initial sync simply
confirms the connection and the dashboard shows the seeded data.
"""

from __future__ import annotations

import sqlite3
import time
from datetime import datetime, timezone

from activsync import db
from activsync.garmin_client import MfaRequired

MFA_TRIGGER_PASSWORD = "mfa"
MFA_REJECT_CODE = "000000"

_FALLBACK_ACTIVITY_TYPES = [
    {"type_key": "running", "label": "Running"},
    {"type_key": "cycling", "label": "Cycling"},
    {"type_key": "walking", "label": "Walking"},
]


class _FakePendingAuth:
    """Stand-in for ``garmin_auth.GarminAuth`` during a simulated MFA challenge."""

    def resume_login(self, mfa_code: str):
        if mfa_code == MFA_REJECT_CODE:
            raise ValueError("Invalid MFA code (dev mock rejects 000000)")
        return None


def begin_login(email: str, password: str):
    """Fake Garmin login. Raises :class:`MfaRequired` when the password is
    :data:`MFA_TRIGGER_PASSWORD`, otherwise succeeds immediately."""
    if password == MFA_TRIGGER_PASSWORD:
        raise MfaRequired(_FakePendingAuth())
    return None


def complete_login(pending_auth, mfa_code: str):
    """Finish a simulated MFA challenge; delegates to the pending auth."""
    return pending_auth.resume_login(mfa_code)


class FakeGarminClient:
    """No-network Garmin client. Activities come from :mod:`activsync.dev_seed`,
    so the recent-activity fetch returns nothing new; only the methods the
    wizard and sync actually call are stubbed."""

    def __init__(self, conn: sqlite3.Connection):
        self._conn = conn

    def fetch_activity_types(self) -> list[dict]:
        return (
            db.get_config_value(self._conn, "garmin_activity_types", default=[])
            or _FALLBACK_ACTIVITY_TYPES
        )

    def fetch_recent_activities(self, lookback_days: int) -> list:
        return []

    def download_fit(self, garmin_activity_id: int) -> bytes:
        return b""

    def update_activity_metadata(
        self, garmin_activity_id: int, title: str, description: str
    ) -> None:
        return None


class FakeStravaClient:
    """No-network Strava client. The "OAuth" handshake loops straight back to
    the local callback and stores a fake token so the connection reads as live,
    and duplicate/existence checks are answered without any HTTP call."""

    def __init__(self, conn: sqlite3.Connection):
        self._conn = conn

    def is_connected(self) -> bool:
        tokens = db.get_config_value(self._conn, "strava_tokens") or {}
        return bool(tokens.get("refresh_token"))

    def authorize_url(self, redirect_uri: str, state: str) -> str:
        # Skip Strava entirely: bounce straight to our own callback with a
        # placeholder code that exchange_code accepts unconditionally. The
        # state is echoed back exactly as the real Strava would, so the
        # callback's state check exercises the same path in mock mode.
        separator = "&" if "?" in redirect_uri else "?"
        return f"{redirect_uri}{separator}code=dev-mock-code&state={state}"

    def exchange_code(self, code: str) -> None:
        db.set_config_value(self._conn, "strava_tokens", {
            "access_token": "dev-mock-access-token",
            "refresh_token": "dev-mock-refresh-token",
            "expires_at": int(time.time()) + 6 * 3600,
        })

    def disconnect(self) -> None:
        db.set_config_value(self._conn, "strava_tokens", None)

    def list_activities_between(
        self, after: datetime, before: datetime, now: datetime | None = None
    ) -> list[dict]:
        # Report back exactly what the mock itself "published" and nothing more.
        # The caller reads absence from this window as "deleted on Strava", so
        # returning an empty list would flag every published row as missing;
        # returning anything extra would link a pending row to a duplicate that
        # doesn't exist. Neither is true in dev.
        window = []
        for row in db.list_activities(self._conn, status="published"):
            strava_activity_id = row.get("strava_activity_id")
            if strava_activity_id is None:
                continue
            start = datetime.strptime(
                row["start_time"], "%Y-%m-%d %H:%M:%S"
            ).replace(tzinfo=timezone.utc)
            if after <= start <= before:
                window.append({"id": int(strava_activity_id), "start_date": start})
        return window

    def find_existing_activity(
        self, start_time: datetime, tolerance_minutes: int = 5, now: datetime | None = None
    ) -> int | None:
        # No pre-existing Strava duplicates to link against in dev.
        return None

    def publish(
        self,
        garmin_activity_id: int,
        fit_bytes: bytes,
        name: str | None = None,
        description: str | None = None,
    ) -> int:
        return 9_000_000 + int(garmin_activity_id)

    def update_activity_metadata(
        self, strava_activity_id: int, name: str, description: str
    ) -> None:
        return None
