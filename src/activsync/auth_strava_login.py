"""Sign in with Strava: one more authorization against the owner's own Strava
app, admitted only when it returns the athlete that app is connected to."""

from __future__ import annotations

import logging
import secrets
import sqlite3

import requests
from starlette.requests import Request

from activsync import auth_core, db
from activsync.strava_client import StravaAuthError, scope_is_sufficient

logger = logging.getLogger("activsync.auth.strava")

FLOW_KEY = "strava_login"


class StravaLoginError(Exception):
    def __init__(self, code: str) -> None:
        super().__init__(code)
        self.code = code


def blocker(conn: sqlite3.Connection) -> str | None:
    if not db.get_config_value(conn, "initial_sync_done", default=False):
        return "Finish setup first."
    tokens = db.get_config_value(conn, "strava_tokens") or {}
    if not tokens.get("refresh_token"):
        return "Connect Strava first."
    if not tokens.get("athlete_id"):
        return "Waiting for the next Strava sync."
    return None


def enabled(conn: sqlite3.Connection) -> bool:
    return bool(auth_core.load_settings(conn)["strava_login"]) and blocker(conn) is None


def begin(request: Request, strava, *, redirect_uri: str, next_path: str) -> str:
    state = secrets.token_urlsafe(32)
    request.session[FLOW_KEY] = {"state": state, "next": next_path}
    return strava.authorize_url(redirect_uri, state)


def finish(
    conn: sqlite3.Connection,
    request: Request,
    strava,
    *,
    code: str | None,
    state: str | None,
    error: str | None,
    scope: str | None,
) -> str:
    """Return the path to land on. Raises StravaLoginError without touching
    the stored tokens unless the athlete is the owner."""
    flow = request.session.pop(FLOW_KEY, None)
    if (
        flow is None
        or not state
        or not secrets.compare_digest(state.encode(), flow["state"].encode())
    ):
        raise StravaLoginError("strava_state")
    if error == "access_denied":
        raise StravaLoginError("strava_denied")
    if error or not code or not enabled(conn):
        raise StravaLoginError("strava_unavailable")

    try:
        payload = strava.fetch_tokens(code)
    except (requests.RequestException, StravaAuthError) as exc:
        logger.warning("Strava sign-in code exchange failed: %s", exc)
        raise StravaLoginError("strava_unavailable") from exc

    owner = (db.get_config_value(conn, "strava_tokens") or {}).get("athlete_id")
    athlete = (payload.get("athlete") or {}).get("id")
    if not owner or athlete is None or int(athlete) != int(owner):
        logger.warning("Strava sign-in by athlete %s refused; the owner is %s", athlete, owner)
        raise StravaLoginError("strava_mismatch")

    # Strava invalidates the previous refresh token as soon as it issues this
    # one, so these become the uploader's tokens even if the scope narrowed.
    strava.store_tokens(payload, granted_scope=scope)
    if not scope_is_sufficient(scope):
        logger.warning(
            "Strava sign-in granted only %r; reconnect Strava in Settings to "
            "restore uploads",
            scope,
        )
    return flow["next"]
