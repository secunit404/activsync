"""Strava v3 API client: OAuth and (Task 6) activity upload."""

from __future__ import annotations

import logging
import sqlite3
import time
from datetime import datetime, timedelta, timezone

import requests

from activsync import db

logger = logging.getLogger("activsync.strava_client")

STRAVA_AUTHORIZE_URL = "https://www.strava.com/oauth/authorize"
STRAVA_TOKEN_URL = "https://www.strava.com/oauth/token"
STRAVA_UPLOAD_URL = "https://www.strava.com/api/v3/uploads"
STRAVA_ACTIVITIES_URL = "https://www.strava.com/api/v3/athlete/activities"
STRAVA_ACTIVITY_URL = "https://www.strava.com/api/v3/activities"
STRAVA_REVOKE_URL = "https://www.strava.com/oauth/revoke"

# Strava enforces a short-term quota (100 requests by default) that resets on
# 15-minute wall-clock boundaries, not 15 minutes after the first request.
RATE_LIMIT_WINDOW_MINUTES = 15

# Strava's maximum accepted per_page. A 7-day lookback fits in one page for any
# realistic athlete; the page cap only bounds a pathological window.
ACTIVITIES_PER_PAGE = 200
MAX_ACTIVITY_PAGES = 10

DEFAULT_MATCH_TOLERANCE_MINUTES = 5


class StravaAuthError(Exception):
    """Raised when Strava rejects our credentials or refresh token."""


class StravaUploadError(Exception):
    """Raised when a Strava upload fails or Strava rejects the activity."""


class StravaWindowIncompleteError(Exception):
    """Raised when a window fetch could not be read to the end.

    Callers read absence from a window as "deleted on Strava", so a truncated
    window must never be handed back as if it were the whole picture.
    """


class StravaRateLimitError(Exception):
    """Raised when Strava returns 429.

    Carries the instant the caller may retry, so the poller can sit the window
    out rather than spending every tick re-hitting an exhausted quota.

    Deliberately an absolute instant rather than a duration: a duration is only
    meaningful against the clock that produced it. This is raised when the 429
    lands, but the poller applies it against its older tick clock, so passing
    seconds across that gap ends the pause early — right before the quota
    resets, which just earns another 429.
    """

    def __init__(self, message: str, retry_at: datetime):
        super().__init__(message)
        self.retry_at = retry_at


def _seconds_until_quota_reset(now: datetime) -> float:
    """Time until Strava's next 15-minute boundary."""
    minutes_into_window = now.minute % RATE_LIMIT_WINDOW_MINUTES
    return (RATE_LIMIT_WINDOW_MINUTES - minutes_into_window) * 60 - now.second


def _raise_if_rate_limited(response, now: datetime | None = None) -> None:
    """Turn a 429 into a StravaRateLimitError carrying a usable wait.

    Strava often omits Retry-After on 429, so fall back to its published
    reset schedule instead of guessing zero and retrying straight into
    another 429.
    """
    if response.status_code != 429:
        return

    now = now or datetime.now(timezone.utc)
    header = response.headers.get("Retry-After")
    retry_after: float | None = None
    if header:
        try:
            retry_after = max(0.0, float(header))
        except (TypeError, ValueError):
            retry_after = None
    if retry_after is None:
        retry_after = _seconds_until_quota_reset(now)

    retry_at = now + timedelta(seconds=retry_after)
    raise StravaRateLimitError(
        f"Strava rate limit reached; retry at {retry_at.isoformat(timespec='seconds')}",
        retry_at,
    )


def _parse_start_date(raw: str) -> datetime:
    return datetime.fromisoformat(raw.replace("Z", "+00:00"))


def match_closest_activity(
    activities: list[dict],
    start_time: datetime,
    tolerance_minutes: int = DEFAULT_MATCH_TOLERANCE_MINUTES,
) -> int | None:
    """Pick the activity starting closest to start_time, within tolerance.

    Shared by the per-activity lookup and the batch window scan so both agree
    on what counts as "the same activity".
    """
    tolerance_seconds = timedelta(minutes=tolerance_minutes).total_seconds()
    best_id: int | None = None
    best_delta: float | None = None
    for activity in activities:
        delta = abs((activity["start_date"] - start_time).total_seconds())
        if delta > tolerance_seconds:
            continue
        if best_delta is None or delta < best_delta:
            best_id, best_delta = int(activity["id"]), delta
    return best_id


class StravaClient:
    def __init__(self, conn: sqlite3.Connection, client_id: str, client_secret: str):
        self._conn = conn
        self._client_id = client_id
        self._client_secret = client_secret

    def is_connected(self) -> bool:
        tokens = db.get_config_value(self._conn, "strava_tokens")
        return bool(tokens and tokens.get("refresh_token"))

    def authorize_url(self, redirect_uri: str, state: str) -> str:
        # activity:read_all (not just activity:read) so duplicate-detection
        # also sees activities the athlete has marked private — otherwise a
        # private duplicate wouldn't be found and we'd re-upload it anyway.
        # `state` is echoed back to the callback unchanged; the caller keeps
        # the issued value and rejects any callback that doesn't match.
        return (
            f"{STRAVA_AUTHORIZE_URL}?client_id={self._client_id}"
            f"&redirect_uri={redirect_uri}&response_type=code"
            f"&approval_prompt=auto&scope=activity:write,activity:read_all"
            f"&state={state}"
        )

    def exchange_code(self, code: str) -> None:
        response = requests.post(STRAVA_TOKEN_URL, data={
            "client_id": self._client_id,
            "client_secret": self._client_secret,
            "code": code,
            "grant_type": "authorization_code",
        }, timeout=30)
        response.raise_for_status()
        payload = response.json()
        db.set_config_value(self._conn, "strava_tokens", {
            "access_token": payload["access_token"],
            "refresh_token": payload["refresh_token"],
            "expires_at": payload["expires_at"],
        })
        logger.info("strava connected")

    def _get_access_token(self) -> str:
        tokens = db.get_config_value(self._conn, "strava_tokens")
        if not tokens:
            raise StravaAuthError("Strava is not connected")

        if tokens["expires_at"] > time.time() + 60:
            return tokens["access_token"]

        response = requests.post(STRAVA_TOKEN_URL, data={
            "client_id": self._client_id,
            "client_secret": self._client_secret,
            "grant_type": "refresh_token",
            "refresh_token": tokens["refresh_token"],
        }, timeout=30)
        if response.status_code == 401:
            raise StravaAuthError("Strava refresh token was revoked")
        response.raise_for_status()

        payload = response.json()
        refreshed = {
            "access_token": payload["access_token"],
            "refresh_token": payload["refresh_token"],
            "expires_at": payload["expires_at"],
        }
        db.set_config_value(self._conn, "strava_tokens", refreshed)
        return refreshed["access_token"]

    def disconnect(self) -> None:
        """Forget the stored Strava connection, both locally and on Strava's side.

        Uses /oauth/revoke (Strava's replacement for the older
        /oauth/deauthorize, which stops working 2027-06-01) so the app also
        disappears from the athlete's "My Apps" list on Strava, not just
        locally. The revoke call is best-effort — even if it fails (network
        error, already-revoked token), we still forget the local tokens so
        the app doesn't keep trying to use a connection the user asked to end.
        """
        tokens = db.get_config_value(self._conn, "strava_tokens")
        if tokens and tokens.get("access_token"):
            try:
                requests.post(
                    STRAVA_REVOKE_URL,
                    auth=(self._client_id, self._client_secret),
                    data={"token": tokens["access_token"], "token_type_hint": "access_token"},
                    timeout=10,
                )
            except requests.RequestException:
                pass
        db.set_config_value(self._conn, "strava_tokens", None)
        logger.info("strava disconnected")

    def activity_exists(self, strava_activity_id: int, now: datetime | None = None) -> bool:
        """Check whether a previously-published activity is still on Strava.

        Used to detect activities deleted on Strava's side so they can be
        flagged for a manual republish instead of silently pointing at a
        dead link forever.
        """
        access_token = self._get_access_token()
        headers = {"Authorization": f"Bearer {access_token}"}
        response = requests.get(
            f"{STRAVA_ACTIVITY_URL}/{strava_activity_id}", headers=headers, timeout=30
        )
        if response.status_code == 404:
            return False
        if response.status_code == 401:
            raise StravaAuthError("Strava access token rejected")
        _raise_if_rate_limited(response, now)
        response.raise_for_status()
        return True

    def update_activity_metadata(self, strava_activity_id: int, name: str, description: str) -> None:
        access_token = self._get_access_token()
        headers = {"Authorization": f"Bearer {access_token}"}
        response = requests.put(
            f"{STRAVA_ACTIVITY_URL}/{strava_activity_id}",
            headers=headers,
            json={"name": name, "description": description},
            timeout=30,
        )
        if response.status_code == 401:
            raise StravaAuthError("Strava access token rejected")
        _raise_if_rate_limited(response)
        response.raise_for_status()

    def _fetch_activities_page(
        self,
        headers: dict,
        after: datetime,
        before: datetime,
        per_page: int,
        page: int,
        now: datetime | None = None,
    ) -> list[dict]:
        response = requests.get(
            STRAVA_ACTIVITIES_URL,
            headers=headers,
            params={
                "after": int(after.timestamp()),
                "before": int(before.timestamp()),
                "per_page": per_page,
                "page": page,
            },
            timeout=30,
        )
        if response.status_code == 401:
            raise StravaAuthError(
                "Strava rejected the read request — the connected app may be "
                "missing the activity:read_all scope. Reconnect Strava in Settings."
            )
        _raise_if_rate_limited(response, now)
        response.raise_for_status()
        return [
            {"id": int(a["id"]), "start_date": _parse_start_date(a["start_date"])}
            for a in response.json()
        ]

    def list_activities_between(
        self, after: datetime, before: datetime, now: datetime | None = None
    ) -> list[dict]:
        """Fetch every athlete activity in a time window, paginating as needed.

        One window fetch replaces a per-activity lookup: reconciling N
        activities costs one or two requests instead of N, which is what keeps
        the poller inside Strava's short-term quota. Returns dicts of
        {"id": int, "start_date": datetime}.
        """
        access_token = self._get_access_token()
        headers = {"Authorization": f"Bearer {access_token}"}

        activities: list[dict] = []
        for page in range(1, MAX_ACTIVITY_PAGES + 1):
            batch = self._fetch_activities_page(
                headers, after, before, ACTIVITIES_PER_PAGE, page, now
            )
            activities.extend(batch)
            if len(batch) < ACTIVITIES_PER_PAGE:
                return activities

        raise StravaWindowIncompleteError(
            f"more than {MAX_ACTIVITY_PAGES * ACTIVITIES_PER_PAGE} Strava activities "
            f"in the window {after.isoformat()}..{before.isoformat()}"
        )

    def find_existing_activity(
        self,
        start_time: datetime,
        tolerance_minutes: int = DEFAULT_MATCH_TOLERANCE_MINUTES,
        now: datetime | None = None,
    ) -> int | None:
        """Look for a Strava activity already near this start time.

        Catches activities that reached Strava through a path other than this
        app (Garmin's own native Strava connection, a manual upload, etc.) —
        those carry none of our external_id markers, so Strava's own
        upload-level dedup can't catch them and we'd otherwise create a
        duplicate. Matches purely on start_date proximity since that's the
        only field guaranteed to line up regardless of upload source.

        This is the single-activity path used right before an upload. Bulk
        reconciliation uses list_activities_between instead — one request for
        the whole window rather than one per activity.
        """
        access_token = self._get_access_token()
        headers = {"Authorization": f"Bearer {access_token}"}
        window = timedelta(minutes=tolerance_minutes)

        activities = self._fetch_activities_page(
            headers,
            after=start_time - window,
            before=start_time + window,
            per_page=30,
            page=1,
            now=now,
        )
        return match_closest_activity(activities, start_time, tolerance_minutes)

    def publish(self, garmin_activity_id: int, fit_bytes: bytes, name: str | None = None, description: str | None = None) -> int:
        access_token = self._get_access_token()
        headers = {"Authorization": f"Bearer {access_token}"}
        files = {"file": (f"{garmin_activity_id}.fit", fit_bytes)}
        data = {"data_type": "fit", "external_id": f"garmin-{garmin_activity_id}"}
        # Without this, Strava ignores the FIT file's own metadata and
        # auto-generates a name from start date/location — the Garmin title
        # (set in Garmin Connect, not embedded in the FIT file) never lands.
        if name:
            data["name"] = name
        if description:
            data["description"] = description

        response = requests.post(
            STRAVA_UPLOAD_URL, headers=headers, files=files, data=data, timeout=60
        )
        if response.status_code == 401:
            raise StravaAuthError("Strava access token rejected")
        _raise_if_rate_limited(response)
        response.raise_for_status()

        upload_id = response.json()["id"]
        return self._poll_upload(upload_id, access_token)

    def _poll_upload(
        self, upload_id: int, access_token: str, attempts: int = 10, delay_s: float = 2.0
    ) -> int:
        headers = {"Authorization": f"Bearer {access_token}"}
        for _ in range(attempts):
            response = requests.get(
                f"{STRAVA_UPLOAD_URL}/{upload_id}", headers=headers, timeout=30
            )
            _raise_if_rate_limited(response)
            response.raise_for_status()
            payload = response.json()

            if payload.get("error"):
                raise StravaUploadError(payload["error"])
            if payload.get("activity_id"):
                return int(payload["activity_id"])

            time.sleep(delay_s)

        raise StravaUploadError(f"upload {upload_id} did not resolve in time")
