"""FastAPI app factory and routes."""

from __future__ import annotations

import asyncio
import logging
import os
import secrets
import sqlite3
from datetime import datetime, timezone
from pathlib import Path

import requests
from fastapi import FastAPI, Form, Request
from fastapi.responses import (
    FileResponse,
    HTMLResponse,
    PlainTextResponse,
    RedirectResponse,
    Response,
    StreamingResponse,
)
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates

from activsync import __version__, config, db, dev_mock, events, logging_setup, sync, timeutil, update_check, view
from activsync.garmin_client import (
    GarminClient,
    MfaRequired,
    begin_login as garmin_begin_login,
    complete_login as garmin_complete_login,
    get_client as get_garmin_raw_client,
)
from activsync.strava_client import StravaAuthError, StravaClient

logger = logging.getLogger("activsync.server")

TEMPLATES_DIR = Path(__file__).parent / "templates"
STATIC_DIR = Path(__file__).parent / "static"
ACTIVITIES_PAGE_SIZE = 20
ACTIVITIES_PAGE_SIZES = (10, 20, 50, 100)


def _garmin_token_dir() -> str:
    return os.environ.get("ACTIVSYNC_GARMIN_TOKEN_DIR", os.environ.get("G2S_GARMIN_TOKEN_DIR", "/config/.garminconnect"))


def _mock_mode() -> bool:
    """Whether the local dev mock is active. Same semantics as main.MOCK_MODE:
    only explicit truthy values enable it, so ACTIVSYNC_DEV_MOCK_DATA=0 stays
    off (a bare truthiness check would treat the string "0" as on)."""
    value = os.environ.get("ACTIVSYNC_DEV_MOCK_DATA", os.environ.get("G2S_DEV_MOCK_DATA", ""))
    return value.lower() in ("1", "true", "yes")


def _build_garmin_client(conn: sqlite3.Connection):
    if _mock_mode():
        return dev_mock.FakeGarminClient(conn)
    creds = db.get_config_value(conn, "garmin_credentials")
    if not creds:
        raise RuntimeError("Garmin credentials are not configured")
    raw = get_garmin_raw_client(creds["email"], creds["password"], _garmin_token_dir())
    return GarminClient(raw)


def _begin_garmin_login(email: str, password: str):
    """Attempt a Garmin login synchronously; raises MfaRequired if challenged.

    Kept separate from _build_garmin_client (used by the background poller and
    activity actions, which have no way to prompt a human for an MFA code) —
    this is only called from routes that can hold the pending login across a
    request/response round-trip and show the user a code-entry form.
    """
    if _mock_mode():
        return dev_mock.begin_login(email, password)
    return garmin_begin_login(email, password, _garmin_token_dir())


def _complete_garmin_login(pending_auth, mfa_code: str):
    if _mock_mode():
        return dev_mock.complete_login(pending_auth, mfa_code)
    return garmin_complete_login(pending_auth, mfa_code)


def _publish_failure_message(stats: "sync.PublishStats") -> str:
    """Wording for uploads that failed inside a batch that otherwise succeeded."""
    if not stats.failed:
        return ""
    noun = "activity" if stats.failed == 1 else "activities"
    return (
        f"{stats.failed} {noun} could not be published to Strava and stayed pending — "
        "check the logs for the reason, then try again."
    )


def _build_strava_client(conn: sqlite3.Connection):
    if _mock_mode():
        return dev_mock.FakeStravaClient(conn)
    creds = db.get_config_value(conn, "strava_credentials") or {}
    return StravaClient(conn, creds.get("client_id", ""), creds.get("client_secret", ""))


def _asset_version(name: str) -> str:
    """Cache-busting token for a stylesheet.

    Production ships an immutable image per release, so the version is a fine
    token. Dev is the problem: the version never moves while CSS is being
    edited, and uvicorn's reloader only watches Python — so the browser keeps
    serving the stylesheet it cached at the start of the session and quietly
    renders something other than what is on disk. Key off the file's mtime
    there instead.
    """
    if not _mock_mode():
        return __version__
    try:
        return str(int((STATIC_DIR / "css" / f"{name}.css").stat().st_mtime))
    except OSError:
        return __version__


def create_app(conn: sqlite3.Connection, lifespan=None) -> FastAPI:
    app = FastAPI(title="ActivSync", lifespan=lifespan)
    app.mount("/static", StaticFiles(directory=str(STATIC_DIR)), name="static")
    templates = Jinja2Templates(directory=str(TEMPLATES_DIR))
    # Available to every template without threading it through each route's
    # context, so the dev banner and tab-title marker render app-wide.
    templates.env.globals["dev_mode"] = _mock_mode()
    templates.env.globals["app_version"] = __version__
    templates.env.globals["asset_version"] = _asset_version
    templates.env.globals["update_status"] = update_check.get_status
    # Single-user app: at most one Garmin login can be mid-MFA-challenge at a
    # time. Holds the in-flight GarminAuth object between whichever POST
    # triggered the challenge — /setup/garmin/connect (first run) or
    # /settings/garmin/reconnect (repairing a broken connection later) — and
    # the matching completion POST, /setup/garmin-mfa or /settings/garmin/mfa.
    # It can't be persisted to SQLite since it isn't serializable.
    pending_garmin_mfa: dict = {}

    def _fetch_and_store_garmin_categories(conn: sqlite3.Connection, *, hold_all: bool) -> None:
        """Fetch Garmin activity types, cache them, and optionally hold every
        category (manual-by-default). Raises on fetch failure — callers decide
        whether that is fatal."""
        garmin = _build_garmin_client(conn)
        types = garmin.fetch_activity_types()
        db.set_config_value(conn, "garmin_activity_types", types)
        db.set_config_value(conn, "garmin_activity_types_fetched_at",
                            datetime.now(timezone.utc).isoformat())
        if hold_all:
            cfg = config.load_config(conn)
            cfg["held_activity_types"] = sorted(t["type_key"] for t in types)
            config.save_config(conn, cfg)

    def _finalize_garmin_connect(conn: sqlite3.Connection) -> None:
        """After a verified Garmin login: cache activity types, and on first-run
        only, hold every category. Best-effort at connect time — a failure here
        must not fail the connect; the first-run initial-sync step re-attempts
        and treats a failure as fatal so the first sync never runs with an
        unknown category set."""
        first_run = not bool(db.get_config_value(conn, "initial_sync_done", default=False))
        try:
            _fetch_and_store_garmin_categories(conn, hold_all=first_run)
        except Exception:
            return

    def _save_and_verify_garmin(email: str, password: str) -> tuple[str, str]:
        """Prove a Garmin login works, and only then persist it.

        Save and verify are one action by design: a saved-but-unverified
        credential is exactly the state that silently stops sync. But a
        FAILED attempt with newly-typed credentials proves nothing about
        whatever is already stored — a healthy connection must survive a
        typo, so credentials are only written on the "ok" outcome. On
        "error", both the stored garmin_credentials and
        garmin_credentials_verified are left untouched.

        The "mfa" outcome is NOT proof the typed password is correct either:
        Garmin issues the MFA challenge before validating the password, so a
        wrong password still raises MfaRequired. Persisting here would let a
        mistyped-password reconnect silently overwrite a healthy saved
        password while the challenge sits unanswered. Instead the typed
        credentials are held in memory alongside the pending auth and only
        committed once the code is verified — see _commit_pending_garmin_credentials.

        Any fresh login attempt also invalidates a prior MFA challenge: it's
        either about to be replaced by a new one, or superseded by an
        outright success, so a stale pending_auth (and any credentials held
        for it) must not survive it.

        Returns ("ok", ""), ("mfa", "") when a code is needed, or
        ("error", message).
        """
        pending_garmin_mfa.pop("auth", None)
        pending_garmin_mfa.pop("credentials", None)
        try:
            _begin_garmin_login(email, password)
        except MfaRequired as exc:
            pending_garmin_mfa["auth"] = exc.pending_auth
            pending_garmin_mfa["credentials"] = {"email": email, "password": password}
            return "mfa", ""
        except Exception as e:
            return "error", str(e)
        db.set_config_value(conn, "garmin_credentials", {"email": email, "password": password})
        _mark_garmin_verified()
        return "ok", ""

    def _commit_pending_garmin_credentials() -> None:
        """Promote the credentials held during an MFA challenge to permanent
        storage. Must only be called after _complete_garmin_login succeeds —
        that's the first point a typed password is actually proven correct."""
        creds = pending_garmin_mfa.pop("credentials", None)
        if creds is not None:
            db.set_config_value(conn, "garmin_credentials", creds)

    def _mark_garmin_verified() -> None:
        db.set_config_value(conn, "garmin_credentials_verified", True)
        db.set_config_value(conn, "garmin_last_sync_error", None)
        _finalize_garmin_connect(conn)

    def _observe_outage() -> str | None:
        """Snapshot how long Garmin has been down, BEFORE anything flips
        garmin_credentials_verified.

        The poller shares this sqlite connection and deliberately leaves its
        _last_garmin_run stale while Garmin is unverified, so the very next tick
        (<= 60s) is already due. The moment the flag flips, that tick can run a
        normal-window sync_garmin which stamps garmin_last_sync_ok_at = now —
        and _mark_garmin_verified makes a network call before the catch-up gets
        to read it. Re-reading the timestamp afterwards would measure a 0-day
        outage, widen nothing, and silently fetch none of the backlog.
        """
        return db.get_config_value(conn, "garmin_last_sync_ok_at")

    def _run_catch_up(last_sync_ok_at: str | None = sync.LAST_SYNC_UNSET) -> None:
        """Bring the activity list up to date across the outage, and record what
        came back so the dashboard can say so. Best-effort: a reconnect that
        succeeded must not be reported as a failure because the follow-up sync
        hiccupped — the poller retries within the minute."""
        try:
            stats = sync.catch_up_sync(
                conn, _build_garmin_client(conn), _build_strava_client(conn),
                config.load_config(conn), datetime.now(timezone.utc),
                last_sync_ok_at=last_sync_ok_at,
            )
        except Exception:
            logger.exception("catch-up sync after reconnect failed")
            events.bus.publish("refresh")
            return
        found_anything = stats.garmin.new or stats.status.linked_existing
        # Set unconditionally: a later quiet reconnect must CLEAR a previous
        # reconnect's banner, not leave it up claiming a backlog that is gone.
        db.set_config_value(conn, "catch_up_report", {
            "new": stats.garmin.new,
            "held": stats.garmin.held_backlog,
            "linked": stats.status.linked_existing,
            "days": stats.lookback_days,
        } if found_anything else None)
        events.bus.publish("refresh")

    def _settings_context(**overrides) -> dict:
        strava_creds = db.get_config_value(conn, "strava_credentials") or {}
        creds = db.get_config_value(conn, "garmin_credentials") or {}
        connections = view.connection_status(conn)
        context = {
            "connections": connections,
            "garmin_email": connections["garmin"]["email"],
            "garmin_credentials_saved": bool(creds.get("email") and creds.get("password")),
            "garmin_connected": connections["garmin"]["connected"],
            "strava_client_id": strava_creds.get("client_id", ""),
            "strava_credentials_saved": bool(
                strava_creds.get("client_id") and strava_creds.get("client_secret")
            ),
            "strava_connected": connections["strava"]["connected"],
            "initial_sync_done": bool(db.get_config_value(conn, "initial_sync_done", default=False)),
            "cfg": config.load_config(conn),
            "activity_types": db.get_config_value(conn, "garmin_activity_types", default=[]),
            "timezones": timeutil.common_timezones(),
        }
        context.update(overrides)
        return context

    def _is_htmx(request: Request) -> bool:
        return request.headers.get("HX-Request") == "true"

    def _saved(request: Request, section: str) -> Response:
        """Answer a settings save.

        htmx posts these forms in place, and 204 keeps it that way: nothing
        swaps, nothing navigates, and the reader stays exactly where they were.
        The redirect is the no-htmx fallback, anchored so a full page load at
        least lands back on the section instead of the top of the page.
        """
        if _is_htmx(request):
            return Response(status_code=204)
        return RedirectResponse(f"/settings#{section}", status_code=303)

    def _persist_strava_credentials(client_id: str, client_secret: str) -> None:
        existing = db.get_config_value(conn, "strava_credentials") or {}
        credentials_changed = (
            existing.get("client_id") != client_id
            or existing.get("client_secret") != client_secret
        )
        db.set_config_value(conn, "strava_credentials", {
            "client_id": client_id,
            "client_secret": client_secret,
        })
        if credentials_changed:
            # OAuth tokens belong to the client application credentials. A
            # changed client ID/secret must be re-authorized before use.
            db.set_config_value(conn, "strava_tokens", None)

    def _setup_step(ctx: dict) -> str | None:
        """Which first-run step to show, or None when the wizard doesn't apply.

        The wizard is strictly one-time. Once initial_sync_done is set, a broken
        connection is repaired in Settings — never by re-entering onboarding,
        which would strand the user away from activities they already have.
        """
        if ctx["initial_sync_done"]:
            return None
        if not ctx["garmin_connected"]:
            return "garmin"
        if not ctx["strava_connected"]:
            return "strava"
        return "syncing"

    def _activity_context(
        sort_order: str = "newest",
        status_filter: str = "",
        page: int = 1,
        page_size: int = ACTIVITIES_PAGE_SIZE,
        **overrides,
    ) -> dict:
        sort_order = "oldest" if sort_order == "oldest" else "newest"
        allowed_statuses = {"pending", "held", "published", "missing", "excluded"}
        status_filter = status_filter if status_filter in allowed_statuses else ""
        page_size = page_size if page_size in ACTIVITIES_PAGE_SIZES else ACTIVITIES_PAGE_SIZE
        all_activities = view.activities_view(
            conn, sort_order=sort_order, status_filter=status_filter
        )
        total_count = len(all_activities)
        page_count = max(1, (total_count + page_size - 1) // page_size)
        page = min(max(page, 1), page_count)
        first_index = (page - 1) * page_size
        activities = all_activities[first_index:first_index + page_size]
        context = {
            "activities": activities,
            "sort_order": sort_order,
            "status_filter": status_filter,
            "page": page,
            "page_size": page_size,
            "page_sizes": ACTIVITIES_PAGE_SIZES,
            "page_count": page_count,
            "total_count": total_count,
            "first_item": first_index + 1 if total_count else 0,
            "last_item": first_index + len(activities),
            "connections": view.connection_status(conn),
            "sync_error": "",
            "catch_up_report": db.get_config_value(conn, "catch_up_report"),
        }
        context.update(overrides)
        return context

    def _page_from_request(request: Request) -> int:
        try:
            return max(int(request.query_params.get("page", "1")), 1)
        except ValueError:
            return 1

    def _page_size_from_request(request: Request) -> int:
        try:
            return int(request.query_params.get("page_size", str(ACTIVITIES_PAGE_SIZE)))
        except ValueError:
            return ACTIVITIES_PAGE_SIZE

    @app.get("/health")
    def health() -> dict:
        return {"status": "ok"}

    @app.get("/favicon.ico", include_in_schema=False)
    def favicon() -> FileResponse:
        # Browsers request /favicon.ico from the root on every visit no matter
        # what the page links, so without this each one logs a 404. Served
        # rather than redirected: browsers accept a PNG here regardless of the
        # .ico extension, and it saves a round trip.
        return FileResponse(STATIC_DIR / "favicon.png", media_type="image/png")

    @app.get("/login")
    @app.post("/login")
    def legacy_auth_route():
        """Keep old bookmarks working now that local password auth is removed."""
        return RedirectResponse("/", status_code=303)

    @app.post("/logout")
    def logout():
        return RedirectResponse("/", status_code=303)

    @app.get("/", response_class=HTMLResponse)
    def dashboard(request: Request):
        ctx = _settings_context()
        if _setup_step(ctx) is not None:
            return RedirectResponse("/setup", status_code=303)
        return templates.TemplateResponse(
            request,
            "dashboard.html",
            _activity_context(
                request.query_params.get("sort", "newest"),
                request.query_params.get("status_filter", ""),
                _page_from_request(request),
                _page_size_from_request(request),
            ),
        )

    @app.post("/api/catch-up-report/dismiss")
    def dismiss_catch_up_report():
        db.set_config_value(conn, "catch_up_report", None)
        return RedirectResponse("/", status_code=303)

    @app.get("/api/activities/table", response_class=HTMLResponse)
    def activity_table(request: Request):
        return templates.TemplateResponse(
            request,
            "partials/activity_table.html",
            _activity_context(
                request.query_params.get("sort", "newest"),
                request.query_params.get("status_filter", ""),
                _page_from_request(request),
                _page_size_from_request(request),
            ),
        )

    @app.get("/api/events")
    async def sse_events(request: Request):
        q = events.bus.subscribe()

        async def stream():
            try:
                while True:
                    if await request.is_disconnected():
                        break
                    try:
                        event_name = await asyncio.wait_for(q.get(), timeout=15)
                        yield f"event: {event_name}\ndata: \n\n"
                    except asyncio.TimeoutError:
                        yield ": keepalive\n\n"
            finally:
                events.bus.unsubscribe(q)

        return StreamingResponse(stream(), media_type="text/event-stream",
                                 headers={"Cache-Control": "no-cache"})

    @app.post("/api/sync/garmin", response_class=HTMLResponse)
    def sync_garmin_route(
        request: Request,
        sort_order: str = Form("newest"),
        status_filter: str = Form(""),
    ):
        if not _settings_context()["garmin_connected"]:
            return templates.TemplateResponse(
                request, "partials/activity_table.html",
                _activity_context(
                    sort_order, status_filter,
                    sync_error="Garmin is disconnected — reconnect it to sync.",
                ),
                status_code=409,
            )
        garmin = _build_garmin_client(conn)
        now = datetime.now(timezone.utc)
        sync.sync_garmin(conn, garmin, config.load_config(conn), now)
        if _settings_context()["strava_connected"]:
            sync.check_strava_status(conn, _build_strava_client(conn), config.load_config(conn), now)
        events.bus.publish("refresh")
        return templates.TemplateResponse(request, "partials/activity_table.html", _activity_context(sort_order, status_filter))

    @app.post("/api/sync/strava/publish", response_class=HTMLResponse)
    def publish_to_strava_route(
        request: Request,
        activity_ids: list[int] = Form([]),
        sort_order: str = Form("newest"),
        status_filter: str = Form(""),
    ):
        settings_context = _settings_context()
        if not settings_context["garmin_connected"] or not settings_context["strava_connected"]:
            return templates.TemplateResponse(
                request, "partials/activity_table.html",
                _activity_context(
                    sort_order, status_filter,
                    sync_error="Both Garmin and Strava must be connected to publish. Reconnect to continue.",
                ),
                status_code=409,
            )
        stats = sync.PublishStats()
        if activity_ids:
            garmin = _build_garmin_client(conn)
            strava = _build_strava_client(conn)
            try:
                stats = sync.publish_pending(
                    conn, garmin, strava, datetime.now(timezone.utc), garmin_activity_ids=set(activity_ids),
                )
            except StravaAuthError as e:
                return templates.TemplateResponse(
                    request, "partials/activity_table.html",
                    _activity_context(sort_order, status_filter, sync_error=str(e)),
                    status_code=409,
                )
        events.bus.publish("refresh")
        # publish_pending swallows per-activity failures so one bad upload can't
        # abort the batch — which leaves this as the only place that can tell the
        # user any of it failed. The rows stay pending and are safe to retry.
        return templates.TemplateResponse(
            request, "partials/activity_table.html",
            _activity_context(sort_order, status_filter, sync_error=_publish_failure_message(stats)),
        )

    @app.post("/api/sync/strava/status", response_class=HTMLResponse)
    def sync_strava_status_route(
        request: Request,
        sort_order: str = Form("newest"),
        status_filter: str = Form(""),
    ):
        if not _settings_context()["strava_connected"]:
            return templates.TemplateResponse(
                request, "partials/activity_table.html",
                _activity_context(
                    sort_order, status_filter,
                    sync_error="Strava is disconnected — reconnect it to check status.",
                ),
                status_code=409,
            )
        strava = _build_strava_client(conn)
        try:
            sync.check_strava_status(conn, strava, config.load_config(conn), datetime.now(timezone.utc))
        except StravaAuthError as e:
            # A revoked token or an app missing activity:read_all surfaces here
            # rather than as a 500, since this is the button a user reaches for
            # precisely when the connection has gone bad.
            return templates.TemplateResponse(
                request, "partials/activity_table.html",
                _activity_context(sort_order, status_filter, sync_error=str(e)),
                status_code=409,
            )
        events.bus.publish("refresh")
        return templates.TemplateResponse(request, "partials/activity_table.html", _activity_context(sort_order, status_filter))

    @app.post("/api/activities/{garmin_activity_id}/publish", response_class=HTMLResponse)
    def publish_activity(
        request: Request,
        garmin_activity_id: int,
        sort_order: str = Form("newest"),
        status_filter: str = Form(""),
    ):
        garmin = _build_garmin_client(conn)
        strava = _build_strava_client(conn)
        sync.publish_now(conn, garmin, strava, garmin_activity_id, datetime.now(timezone.utc))
        events.bus.publish("refresh")
        return templates.TemplateResponse(request, "partials/activity_table.html", _activity_context(sort_order, status_filter))

    @app.post("/api/activities/{garmin_activity_id}/edit", response_class=HTMLResponse)
    def edit_activity(
        request: Request,
        garmin_activity_id: int,
        title: str = Form(...),
        description: str = Form(""),
        sort_order: str = Form("newest"),
        status_filter: str = Form(""),
    ):
        if not title.strip():
            return templates.TemplateResponse(
                request,
                "partials/activity_table.html",
                _activity_context(sort_order, status_filter, edit_error="Could not save edit: Activity title cannot be blank"),
                status_code=502,
            )
        try:
            garmin = _build_garmin_client(conn)
            strava = _build_strava_client(conn)
            sync.edit_activity_metadata(
                conn, garmin, strava, garmin_activity_id, title, description
            )
        except Exception as e:
            return templates.TemplateResponse(
                request,
                "partials/activity_table.html",
                _activity_context(sort_order, status_filter, edit_error=f"Could not save edit: {e}"),
                status_code=502,
            )
        # NOTE: intentionally do not publish an SSE "refresh" here. The response
        # below already swaps #activity-table for the acting tab; echoing a
        # refresh back would swap it a second time, tearing down and reopening
        # the open details dialog (a visible flicker).
        return templates.TemplateResponse(
            request,
            "partials/activity_table.html",
            _activity_context(sort_order, status_filter),
        )

    @app.post("/api/activities/{garmin_activity_id}/exclude", response_class=HTMLResponse)
    def exclude_activity(
        request: Request,
        garmin_activity_id: int,
        sort_order: str = Form("newest"),
        status_filter: str = Form(""),
    ):
        sync.exclude(conn, garmin_activity_id)
        events.bus.publish("refresh")
        return templates.TemplateResponse(request, "partials/activity_table.html", _activity_context(sort_order, status_filter))

    @app.post("/api/activities/{garmin_activity_id}/unexclude", response_class=HTMLResponse)
    def unexclude_activity(
        request: Request,
        garmin_activity_id: int,
        sort_order: str = Form("newest"),
        status_filter: str = Form(""),
    ):
        sync.unexclude(conn, garmin_activity_id, config.load_config(conn))
        events.bus.publish("refresh")
        return templates.TemplateResponse(request, "partials/activity_table.html", _activity_context(sort_order, status_filter))

    @app.get("/setup", response_class=HTMLResponse)
    def setup_wizard(request: Request):
        ctx = _settings_context()
        step = _setup_step(ctx)
        if step is None:
            return RedirectResponse("/", status_code=303)
        ctx["step"] = step
        ctx["show_mfa"] = step == "garmin" and "auth" in pending_garmin_mfa
        return templates.TemplateResponse(request, "setup.html", ctx)

    @app.get("/settings", response_class=HTMLResponse)
    def settings_form(request: Request):
        ctx = _settings_context()
        if _setup_step(ctx) is not None:
            return RedirectResponse("/setup", status_code=303)
        return templates.TemplateResponse(request, "settings.html", ctx)

    @app.post("/setup/garmin/connect")
    def setup_garmin_connect(
        request: Request,
        garmin_email: str = Form(""),
        garmin_password: str = Form(""),
        lookback_days: int = Form(7),
        detected_timezone: str = Form(""),
    ):
        if _settings_context()["initial_sync_done"]:
            # The wizard is strictly one-time (see _setup_step). A stale open
            # tab or bookmark must not be able to POST here after setup and
            # silently reset lookback_days or render setup.html on error.
            return RedirectResponse("/settings", status_code=303)
        existing = db.get_config_value(conn, "garmin_credentials") or {}
        email = (garmin_email or existing.get("email", "")).strip()
        password = garmin_password or existing.get("password", "")
        if not email or not password:
            return templates.TemplateResponse(
                request, "setup.html",
                _settings_context(step="garmin", garmin_email=email,
                                  garmin_error="Enter your Garmin email and password."),
                status_code=400,
            )
        cfg = config.load_config(conn)
        cfg["lookback_days"] = lookback_days
        if timeutil.is_valid_timezone(detected_timezone):
            cfg["display_timezone"] = detected_timezone
        config.save_config(conn, cfg)
        logging_setup.set_log_timezone(cfg["display_timezone"])

        outcome, message = _save_and_verify_garmin(email, password)
        if outcome == "error":
            return templates.TemplateResponse(
                request, "setup.html",
                _settings_context(step="garmin",
                                  garmin_error=f"Could not connect to Garmin: {message}"),
                status_code=502,
            )
        return RedirectResponse("/setup", status_code=303)

    @app.post("/setup/garmin-mfa", response_class=HTMLResponse)
    def setup_garmin_mfa(request: Request, mfa_code: str = Form(...)):
        pending = pending_garmin_mfa.get("auth")
        if pending is None:
            return RedirectResponse("/setup", status_code=303)
        try:
            _complete_garmin_login(pending, mfa_code)
        except Exception as e:
            return templates.TemplateResponse(
                request, "setup.html",
                _settings_context(step="garmin", show_mfa=True, mfa_error=str(e)),
                status_code=401,
            )
        del pending_garmin_mfa["auth"]
        _commit_pending_garmin_credentials()
        _mark_garmin_verified()
        return RedirectResponse("/setup", status_code=303)

    @app.post("/setup/garmin/mfa/cancel")
    def setup_garmin_mfa_cancel():
        """Escape hatch for a stranded MFA challenge (e.g. Garmin expired the
        pending auth while the tab sat open). Without this, setup_wizard's
        show_mfa=True keeps auto-reopening the modal on every /setup load and
        every code 401s, with no way out short of an app restart. The
        credentials typed for the abandoned attempt are discarded along with
        the pending auth — whatever was previously saved survives untouched."""
        pending_garmin_mfa.pop("auth", None)
        pending_garmin_mfa.pop("credentials", None)
        return RedirectResponse("/setup", status_code=303)

    @app.post("/settings/garmin/reconnect")
    def settings_garmin_reconnect(
        request: Request,
        garmin_email: str = Form(""),
        garmin_password: str = Form(""),
    ):
        existing = db.get_config_value(conn, "garmin_credentials") or {}
        email = (garmin_email or existing.get("email", "")).strip()
        password = garmin_password or existing.get("password", "")
        if not email or not password:
            return templates.TemplateResponse(
                request, "settings.html",
                _settings_context(
                    garmin_dialog_open=True, garmin_email=email,
                    garmin_error="Enter your Garmin email and password.",
                ),
                status_code=400,
            )

        # Measure the outage BEFORE the verified flag flips — see _observe_outage.
        last_sync_ok_at = _observe_outage()
        outcome, message = _save_and_verify_garmin(email, password)
        if outcome == "mfa":
            return templates.TemplateResponse(
                request, "settings.html",
                _settings_context(garmin_dialog_open=True, show_mfa=True),
                status_code=200,
            )
        if outcome == "error":
            return templates.TemplateResponse(
                request, "settings.html",
                _settings_context(
                    garmin_dialog_open=True, garmin_email=email,
                    garmin_error=f"Could not connect to Garmin: {message}",
                ),
                status_code=502,
            )
        _run_catch_up(last_sync_ok_at=last_sync_ok_at)
        return RedirectResponse("/settings", status_code=303)

    @app.post("/settings/garmin/mfa")
    def settings_garmin_mfa(request: Request, mfa_code: str = Form(...)):
        pending = pending_garmin_mfa.get("auth")
        if pending is None:
            return RedirectResponse("/settings", status_code=303)
        # Measure the outage BEFORE the verified flag flips — see _observe_outage.
        last_sync_ok_at = _observe_outage()
        try:
            _complete_garmin_login(pending, mfa_code)
        except Exception as e:
            return templates.TemplateResponse(
                request, "settings.html",
                _settings_context(garmin_dialog_open=True, show_mfa=True, mfa_error=str(e)),
                status_code=401,
            )
        del pending_garmin_mfa["auth"]
        _commit_pending_garmin_credentials()
        _mark_garmin_verified()
        _run_catch_up(last_sync_ok_at=last_sync_ok_at)
        return RedirectResponse("/settings", status_code=303)

    @app.post("/setup/initial-sync", response_class=HTMLResponse)
    def setup_initial_sync(request: Request):
        ctx = _settings_context()
        if ctx["initial_sync_done"]:
            # The wizard is strictly one-time (see _setup_step). A stale open tab
            # must not be able to re-post this after setup and run an unwanted
            # sync; send it to the dashboard it should have been on.
            response = Response(status_code=204)
            response.headers["HX-Redirect"] = "/"
            return response
        if not ctx["garmin_connected"] or not ctx["strava_connected"]:
            return RedirectResponse("/setup", status_code=303)
        try:
            now = datetime.now(timezone.utc)
            garmin = _build_garmin_client(conn)
            strava = _build_strava_client(conn)
            cfg = config.load_config(conn)
            if not db.get_config_value(conn, "garmin_activity_types", default=[]):
                # First-run safety (the guard above proves this IS the first run):
                # never run the first sync without categories loaded and held, or
                # the poller would auto-publish everything. Re-attempts here if the
                # connect-time prefetch failed; a failure is fatal (surfaces the
                # error+Retry screen) by design.
                _fetch_and_store_garmin_categories(conn, hold_all=True)
                cfg = config.load_config(conn)  # reload: held_activity_types changed
            sync.sync_garmin(conn, garmin, cfg, now)
            sync.check_strava_status(conn, strava, cfg, now)
        except Exception as e:
            return templates.TemplateResponse(
                request, "partials/setup_sync_error.html", {"error": str(e)}, status_code=200,
            )
        db.set_config_value(conn, "initial_sync_done", True)
        events.bus.publish("refresh")
        # Show an explicit completion screen instead of auto-redirecting, so the
        # user presses a button to enter the app. initial_sync_done is now set,
        # so the "Go to ActivSync" link to / lands on the dashboard and _setup_step
        # closes the wizard for good.
        return templates.TemplateResponse(
            request, "partials/setup_sync_done.html", ctx, status_code=200,
        )

    @app.post("/settings/preferences")
    def settings_preferences_submit(
        request: Request,
        display_timezone: str = Form(...),
        garmin_poll_interval_minutes: int = Form(...),
        strava_poll_interval_minutes: int = Form(...),
        lookback_days: int = Form(...),
        hevy2garmin_marker: str = Form(...),
        hevy2garmin_marker_enabled: bool = Form(False),
    ):
        if not timeutil.is_valid_timezone(display_timezone):
            message = f"Unknown timezone: {display_timezone}"
            if _is_htmx(request):
                return PlainTextResponse(message, status_code=400)
            return templates.TemplateResponse(
                request, "settings.html",
                _settings_context(preferences_error=message),
                status_code=400,
            )
        cfg = config.load_config(conn)
        cfg.update({
            "display_timezone": display_timezone,
            "garmin_poll_interval_minutes": garmin_poll_interval_minutes,
            "strava_poll_interval_minutes": strava_poll_interval_minutes,
            "lookback_days": lookback_days,
            "hevy2garmin_marker": hevy2garmin_marker,
            "hevy2garmin_marker_enabled": hevy2garmin_marker_enabled,
        })
        config.save_config(conn, cfg)
        logging_setup.set_log_timezone(display_timezone)
        return _saved(request, "preferences")

    @app.post("/settings/strava-credentials")
    def settings_strava_credentials_submit(
        request: Request,
        strava_client_id: str = Form(""),
        strava_client_secret: str = Form(""),
    ):
        existing = db.get_config_value(conn, "strava_credentials") or {}
        strava_client_id = strava_client_id.strip()
        strava_client_secret = strava_client_secret or existing.get("client_secret", "")
        if not strava_client_id or not strava_client_secret:
            return templates.TemplateResponse(
                request, "settings.html",
                _settings_context(
                    strava_dialog_open=True,
                    strava_client_id=strava_client_id,
                    strava_error="Enter both Strava client credentials before saving.",
                ),
                status_code=400,
            )
        _persist_strava_credentials(strava_client_id, strava_client_secret)
        # Changed credentials invalidate the tokens (they belong to the client
        # app), so there is nothing to do but re-authorize. Save and verify are
        # one action here too.
        return RedirectResponse("/strava/connect", status_code=303)

    @app.post("/setup/strava/connect")
    def setup_strava_connect(
        request: Request,
        strava_client_id: str = Form(""),
        strava_client_secret: str = Form(""),
    ):
        existing = db.get_config_value(conn, "strava_credentials") or {}
        strava_client_id = strava_client_id.strip()
        strava_client_secret = strava_client_secret or existing.get("client_secret", "")
        if not strava_client_id or not strava_client_secret:
            return templates.TemplateResponse(
                request,
                "setup.html",
                _settings_context(
                    step="strava",
                    strava_client_id=strava_client_id,
                    strava_error="Enter both Strava client credentials before connecting.",
                ),
                status_code=400,
            )
        _persist_strava_credentials(strava_client_id, strava_client_secret)
        return RedirectResponse("/strava/connect", status_code=303)

    @app.post("/settings/autosync")
    def settings_autosync_submit(request: Request, autosync_types: list[str] = Form([])):
        if not _settings_context()["garmin_connected"]:
            message = "Connect to Garmin before changing activity categories."
            if _is_htmx(request):
                return PlainTextResponse(message, status_code=400)
            return templates.TemplateResponse(
                request, "settings.html",
                _settings_context(garmin_sync_error=message),
                status_code=400,
            )

        known_type_keys = {t["type_key"] for t in db.get_config_value(conn, "garmin_activity_types", default=[])}
        held_activity_types = sorted(known_type_keys - set(autosync_types))
        cfg = config.load_config(conn)
        cfg["held_activity_types"] = held_activity_types
        config.save_config(conn, cfg)
        sync.reconcile_held_activities(conn, held_activity_types)
        return _saved(request, "autosync")

    @app.post("/settings/garmin-activity-types/refresh")
    def refresh_garmin_activity_types(request: Request):
        if not _settings_context()["garmin_connected"]:
            message = "Connect to Garmin before refreshing activity categories."
            if _is_htmx(request):
                return PlainTextResponse(message, status_code=400)
            return templates.TemplateResponse(
                request, "settings.html",
                _settings_context(garmin_sync_error=message),
                status_code=400,
            )
        garmin = _build_garmin_client(conn)
        types = garmin.fetch_activity_types()
        db.set_config_value(conn, "garmin_activity_types", types)
        db.set_config_value(conn, "garmin_activity_types_fetched_at", datetime.now(timezone.utc).isoformat())
        if _is_htmx(request):
            # Just the picker: htmx swaps it into the open page, so refreshing
            # redraws the list without reloading or moving the reader.
            return templates.TemplateResponse(
                request, "partials/_autosync_categories.html", _settings_context(),
            )
        return RedirectResponse("/settings#autosync", status_code=303)

    @app.get("/strava/connect")
    def strava_connect(request: Request):
        creds = db.get_config_value(conn, "strava_credentials") or {}
        if not creds.get("client_id") or not creds.get("client_secret"):
            return templates.TemplateResponse(
                request, "settings.html",
                _settings_context(
                    strava_dialog_open=True,
                    strava_error="Save your Strava client ID and client secret before connecting.",
                ),
                status_code=400,
            )
        strava = _build_strava_client(conn)
        redirect_uri = str(request.url_for("strava_callback"))
        state = secrets.token_urlsafe(32)
        db.set_config_value(conn, "strava_oauth_state", state)
        return RedirectResponse(strava.authorize_url(redirect_uri, state))

    def _strava_callback_error(request: Request, message: str):
        """Render an OAuth failure on whichever page the user came from.

        The callback is a landing page the user arrives at from Strava, so a
        raised error would surface as a bare framework response. Mid-setup
        there is no /settings to fall back to, hence the two templates.
        """
        if _settings_context()["initial_sync_done"]:
            return templates.TemplateResponse(
                request, "settings.html",
                _settings_context(strava_dialog_open=True, strava_error=message),
                status_code=400,
            )
        return templates.TemplateResponse(
            request, "setup.html",
            _settings_context(step="strava", strava_error=message),
            status_code=400,
        )

    @app.get("/strava/callback")
    def strava_callback(
        request: Request,
        code: str | None = None,
        state: str | None = None,
        error: str | None = None,
    ):
        # Every parameter is optional: Strava omits `code` entirely when the
        # authorization doesn't complete (denied, or a redirect_uri that
        # doesn't match the one registered on the API application). Declaring
        # `code` required made FastAPI reject those callbacks before this
        # handler ran, so the user got a raw JSON 422 instead of an
        # explanation.
        expected_state = db.get_config_value(conn, "strava_oauth_state")
        # One state per handshake, consumed on arrival: a replayed callback
        # finds nothing to match against.
        db.set_config_value(conn, "strava_oauth_state", None)

        if error == "access_denied":
            # Strava documents access_denied as the athlete rejecting the
            # authorization. A mismatched callback domain never reaches us at
            # all — Strava won't redirect to a URI it hasn't been told to
            # trust — so don't blame the configuration for this one.
            return _strava_callback_error(
                request,
                "The Strava authorization was declined, so nothing was connected. "
                "Press Connect and choose Authorize on Strava's page to continue.",
            )

        if error or not code:
            # `error` is attacker-controllable, so it goes to the log rather than
            # into the page — Strava's codes mean nothing to the user anyway.
            logger.warning("strava authorization did not complete: error=%s", error)
            return _strava_callback_error(
                request,
                "Strava did not send an authorization back, so nothing was "
                "connected. Press Connect to try again — if it keeps happening, "
                "check that your Strava API application's Authorization Callback "
                "Domain matches the address you use to reach ActivSync.",
            )

        if not expected_state or not state or not secrets.compare_digest(state, expected_state):
            return _strava_callback_error(
                request,
                "That Strava response didn't match the connection request that "
                "started it, so it was ignored. Start the connection again from "
                "this page.",
            )

        strava = _build_strava_client(conn)
        try:
            strava.exchange_code(code)
        except (requests.RequestException, StravaAuthError) as e:
            logger.warning("strava code exchange failed: %s", e)
            return _strava_callback_error(
                request,
                "Could not complete the Strava connection — Strava rejected the "
                "authorization. Check your client ID and client secret, then "
                "try again.",
            )

        if not _settings_context()["initial_sync_done"]:
            return RedirectResponse("/setup", status_code=303)
        _run_catch_up()
        return RedirectResponse("/settings", status_code=303)

    @app.post("/strava/disconnect")
    def strava_disconnect(request: Request):
        strava = _build_strava_client(conn)
        strava.disconnect()
        target = "/settings" if _settings_context()["initial_sync_done"] else "/setup"
        return RedirectResponse(target, status_code=303)

    return app
