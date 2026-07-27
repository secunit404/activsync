"""FastAPI app factory for the React frontend and typed JSON APIs."""

from __future__ import annotations

import asyncio
import logging
import os
import secrets
import sqlite3
from collections.abc import Callable
from pathlib import Path
from urllib.parse import urlencode

import requests
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import FileResponse, RedirectResponse, StreamingResponse
from fastapi.staticfiles import StaticFiles

from activsync import (
    api_routes,
    db,
    dev_mock,
    events,
    hevy_queue_api_routes,
    hevy_tools_api_routes,
    onboarding,
    settings_api_routes,
)
from activsync.garmin_client import (
    GarminClient,
    begin_login as garmin_begin_login,
    complete_login as garmin_complete_login,
    get_client as get_garmin_raw_client,
)
from activsync.strava_client import StravaAuthError, StravaClient

logger = logging.getLogger("activsync.server")

STATIC_DIR = Path(__file__).parent / "static"
WEB_DIR = Path(__file__).parent / "web"
SPA_RESERVED_PREFIXES = ("api", "assets", "static", "strava")


def _garmin_token_dir() -> str:
    return os.environ.get(
        "ACTIVSYNC_GARMIN_TOKEN_DIR",
        os.environ.get("G2S_GARMIN_TOKEN_DIR", "/config/.garminconnect"),
    )


def _mock_mode() -> bool:
    value = os.environ.get(
        "ACTIVSYNC_DEV_MOCK_DATA",
        os.environ.get("G2S_DEV_MOCK_DATA", ""),
    )
    return value.lower() in ("1", "true", "yes")


def _build_garmin_client(conn: sqlite3.Connection):
    if _mock_mode():
        return dev_mock.FakeGarminClient(conn)
    credentials = db.get_config_value(conn, "garmin_credentials")
    if not credentials:
        raise RuntimeError("Garmin credentials are not configured")
    raw = get_garmin_raw_client(
        credentials["email"],
        credentials["password"],
        _garmin_token_dir(),
    )
    return GarminClient(raw)


def _begin_garmin_login(email: str, password: str):
    if _mock_mode():
        return dev_mock.begin_login(email, password)
    return garmin_begin_login(email, password, _garmin_token_dir())


def _complete_garmin_login(pending_auth, mfa_code: str):
    if _mock_mode():
        return dev_mock.complete_login(pending_auth, mfa_code)
    return garmin_complete_login(pending_auth, mfa_code)


def _build_strava_client(conn: sqlite3.Connection):
    if _mock_mode():
        return dev_mock.FakeStravaClient(conn)
    credentials = db.get_config_value(conn, "strava_credentials") or {}
    return StravaClient(
        conn,
        credentials.get("client_id", ""),
        credentials.get("client_secret", ""),
    )


def create_app(
    conn: sqlite3.Connection,
    lifespan=None,
    *,
    web_dir: Path | None = None,
    apply_hevy_match: Callable[[str, str], str] | None = None,
    process_hevy_workout: Callable[[str], str] | None = None,
) -> FastAPI:
    app = FastAPI(title="ActivSync", lifespan=lifespan)
    resolved_web_dir = web_dir or WEB_DIR
    react_index = resolved_web_dir / "index.html"

    if (resolved_web_dir / "assets").is_dir():
        app.mount(
            "/assets",
            StaticFiles(directory=str(resolved_web_dir / "assets")),
            name="react-assets",
        )

    pending_garmin_mfa: dict = {}
    app.include_router(
        api_routes.create_router(
            conn,
            mock_mode=_mock_mode,
            build_garmin_client=_build_garmin_client,
            build_strava_client=_build_strava_client,
        )
    )
    app.include_router(
        settings_api_routes.create_router(
            conn,
            mock_mode=_mock_mode,
            build_garmin_client=_build_garmin_client,
            build_strava_client=_build_strava_client,
            begin_garmin_login=_begin_garmin_login,
            complete_garmin_login=_complete_garmin_login,
            pending_garmin_mfa=pending_garmin_mfa,
        )
    )
    app.include_router(
        hevy_tools_api_routes.create_router(
            conn,
            mock_mode=_mock_mode,
            process_hevy_workout=process_hevy_workout,
        )
    )
    app.include_router(
        hevy_queue_api_routes.create_router(
            conn,
            apply_hevy_match=apply_hevy_match,
        )
    )

    @app.get("/health")
    def health() -> dict:
        return {"status": "ok"}

    @app.get("/favicon.ico", include_in_schema=False)
    def favicon() -> FileResponse:
        return FileResponse(STATIC_DIR / "favicon.png", media_type="image/png")

    @app.get("/apple-touch-icon.png", include_in_schema=False)
    def apple_touch_icon() -> FileResponse:
        return FileResponse(
            STATIC_DIR / "apple-touch-icon.png",
            media_type="image/png",
        )

    @app.get("/api/events")
    async def sse_events(request: Request):
        queue = events.bus.subscribe()

        async def stream():
            try:
                while True:
                    if await request.is_disconnected():
                        break
                    try:
                        event_name = await asyncio.wait_for(queue.get(), timeout=15)
                        yield f"event: {event_name}\ndata: \n\n"
                    except asyncio.TimeoutError:
                        yield ": keepalive\n\n"
            finally:
                events.bus.unsubscribe(queue)

        return StreamingResponse(
            stream(),
            media_type="text/event-stream",
            headers={"Cache-Control": "no-cache"},
        )

    def strava_landing_target() -> str:
        return (
            "/settings"
            if db.get_config_value(conn, "initial_sync_done", default=False)
            else "/setup"
        )

    def strava_error(message: str) -> RedirectResponse:
        query = urlencode({"stravaError": message})
        return RedirectResponse(f"{strava_landing_target()}?{query}", status_code=303)

    @app.get("/strava/connect")
    def strava_connect(request: Request):
        credentials = db.get_config_value(conn, "strava_credentials") or {}
        if not credentials.get("client_id") or not credentials.get("client_secret"):
            return strava_error(
                "Save your Strava client ID and client secret before connecting."
            )
        strava = _build_strava_client(conn)
        redirect_uri = str(request.url_for("strava_callback"))
        state = secrets.token_urlsafe(32)
        db.set_config_value(conn, "strava_oauth_state", state)
        return RedirectResponse(strava.authorize_url(redirect_uri, state))

    @app.get("/strava/callback")
    def strava_callback(
        code: str | None = None,
        state: str | None = None,
        error: str | None = None,
    ):
        expected_state = db.get_config_value(conn, "strava_oauth_state")
        db.set_config_value(conn, "strava_oauth_state", None)

        if error == "access_denied":
            return strava_error(
                "The Strava authorization was declined, so nothing was connected. "
                "Press Connect and choose Authorize on Strava's page to continue."
            )
        if error or not code:
            logger.warning("strava authorization did not complete: error=%s", error)
            return strava_error(
                "Strava did not send an authorization back. Try again and check "
                "the Authorization Callback Domain if the problem continues."
            )
        if (
            not expected_state
            or not state
            or not secrets.compare_digest(state, expected_state)
        ):
            return strava_error(
                "That Strava response did not match the connection request, so it "
                "was ignored. Start the connection again."
            )

        strava = _build_strava_client(conn)
        try:
            strava.exchange_code(code)
        except (requests.RequestException, StravaAuthError) as exc:
            logger.warning("strava code exchange failed: %s", exc)
            return strava_error(
                "Could not complete the Strava connection. Check your client ID "
                "and client secret, then try again."
            )

        if onboarding.setup_step(conn) is not None:
            return RedirectResponse("/setup", status_code=303)
        onboarding.run_catch_up(
            conn,
            _build_garmin_client(conn),
            _build_strava_client(conn),
        )
        return RedirectResponse("/settings", status_code=303)

    @app.get("/{path:path}", include_in_schema=False)
    def spa(path: str):
        first_segment = path.split("/", 1)[0]
        if first_segment in SPA_RESERVED_PREFIXES or not react_index.is_file():
            raise HTTPException(status_code=404)
        return FileResponse(
            react_index,
            media_type="text/html",
            headers={"Cache-Control": "no-cache"},
        )

    return app
