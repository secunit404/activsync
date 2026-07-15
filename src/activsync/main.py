"""Process entrypoint: builds the app, starts the poller, exposes `app` for uvicorn."""

from __future__ import annotations

import os
from contextlib import asynccontextmanager

from activsync import config, db, logging_setup, timeutil
from activsync.dev_seed import seed as seed_dev_data
from activsync.garmin_client import GarminClient, get_client as get_garmin_raw_client
from activsync.poller import Poller
from activsync.server import create_app
from activsync.strava_client import StravaClient
from activsync.update_check import UpdateChecker


def _env_value(name: str, legacy_name: str) -> str | None:
    return os.environ.get(name) or os.environ.get(legacy_name)


MOCK_MODE = (_env_value("ACTIVSYNC_DEV_MOCK_DATA", "G2S_DEV_MOCK_DATA") or "").lower() in ("1", "true", "yes")


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
    seed_dev_data(_conn)


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


_poller = Poller(_conn, garmin_factory=_garmin_factory, strava_factory=_strava_factory)
_update_checker = UpdateChecker()


@asynccontextmanager
async def _lifespan(app):
    if not MOCK_MODE:
        _poller.start()
        _update_checker.start()
    try:
        yield
    finally:
        if not MOCK_MODE:
            _poller.stop()
            _update_checker.stop()


app = create_app(_conn, lifespan=_lifespan)
