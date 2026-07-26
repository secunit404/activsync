"""Process entrypoint: builds the app, starts the poller, exposes `app` for uvicorn."""

from __future__ import annotations

import os
from contextlib import asynccontextmanager

from activsync import config, db, logging_setup, timeutil
from activsync.dev_seed import seed as seed_dev_data
from activsync.garmin_client import GarminClient, get_client as get_garmin_raw_client
from activsync.hevy_client import HevyClient
from activsync.poller import Poller
from activsync.server import create_app
from activsync.strava_client import StravaClient
from activsync.update_check import UpdateChecker


def _env_value(name: str, legacy_name: str = "") -> str | None:
    return os.environ.get(name) or os.environ.get(legacy_name)


def _env_enabled(name: str, legacy_name: str = "") -> bool:
    return (_env_value(name, legacy_name) or "").lower() in ("1", "true", "yes")


MOCK_MODE = _env_enabled("ACTIVSYNC_DEV_MOCK_DATA", "G2S_DEV_MOCK_DATA")
MANUAL_ONLY = _env_enabled("ACTIVSYNC_MANUAL_ONLY")
# Opt-in signal set only by the Playwright E2E server (npm run dev:e2e), never
# by `make dev` / `make dev-fresh`, so a fresh mock DB can boot straight past
# the first-run wizard for E2E specs while local dev still starts at it.
E2E_SEED_ONBOARDED = (_env_value("ACTIVSYNC_DEV_E2E_ONBOARDED") or "").lower() in ("1", "true", "yes")


def _default_db_path() -> str:
    preferred = "/config/activsync-dev.db" if MOCK_MODE else "/config/activsync.db"
    legacy = "/config/dev-mock.db" if MOCK_MODE else "/config/garmin2strava.db"
    if os.path.exists(legacy) and not os.path.exists(preferred):
        return legacy
    return preferred


DB_PATH = _env_value("ACTIVSYNC_DB_PATH", "G2S_DB_PATH") or _default_db_path()
GARMIN_TOKEN_DIR = _env_value("ACTIVSYNC_GARMIN_TOKEN_DIR", "G2S_GARMIN_TOKEN_DIR") or "/config/.garminconnect"

_conn = db.connect(DB_PATH)
if MOCK_MODE:
    # mark_onboarded only ever applies inside this MOCK_MODE branch, so the
    # env var alone can't reach a real database. seed_dev_data's own guard
    # (never touch a DB holding non-dev activity IDs) is what protects a real
    # database pointed at by ACTIVSYNC_DB_PATH if MOCK_MODE is set against it.
    seed_dev_data(_conn, mark_onboarded=E2E_SEED_ONBOARDED)


def _resolve_log_timezone() -> str:
    stored = config.load_config(_conn).get("display_timezone")
    if stored and timeutil.is_valid_timezone(stored):
        return stored
    tz_env = os.environ.get("TZ")
    if tz_env and timeutil.is_valid_timezone(tz_env):
        return tz_env
    return "Europe/Stockholm"


logging_setup.configure_logging(
    level=os.environ.get("ACTIVSYNC_LOG_LEVEL", "INFO"),
    tz_name=_resolve_log_timezone(),
)


def _garmin_factory() -> GarminClient:
    creds = db.get_config_value(_conn, "garmin_credentials")
    if not creds:
        raise RuntimeError("Garmin credentials are not configured")
    raw = get_garmin_raw_client(creds["email"], creds["password"], GARMIN_TOKEN_DIR)
    return GarminClient(raw)


def _strava_factory() -> StravaClient:
    creds = db.get_config_value(_conn, "strava_credentials") or {}
    return StravaClient(_conn, creds.get("client_id", ""), creds.get("client_secret", ""))


def _hevy_factory() -> HevyClient:
    api_key = db.get_config_value(_conn, "hevy_api_key")
    if not api_key:
        raise RuntimeError("Hevy API key is not configured")
    return HevyClient(api_key=api_key)


_poller = Poller(
    _conn,
    garmin_factory=_garmin_factory,
    strava_factory=_strava_factory,
    hevy_factory=_hevy_factory,
    garmin_polling_enabled=not MANUAL_ONLY,
    strava_polling_enabled=not MANUAL_ONLY,
)
_update_checker = UpdateChecker()


@asynccontextmanager
async def _lifespan(app):
    poller_enabled = not MOCK_MODE
    update_checker_enabled = not MOCK_MODE and not MANUAL_ONLY
    if poller_enabled:
        _poller.start()
    if update_checker_enabled:
        _update_checker.start()
    try:
        yield
    finally:
        if poller_enabled:
            _poller.stop()
        if update_checker_enabled:
            _update_checker.stop()


app = create_app(
    _conn,
    lifespan=_lifespan,
    apply_hevy_match=_poller.apply_hevy_match,
    process_hevy_workout=_poller.process_hevy_workout,
)
