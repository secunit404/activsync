"""App configuration: defaults plus DB-backed overrides."""

from __future__ import annotations

import os
import sqlite3

from activsync import db
from activsync.hevy_description import (
    DEFAULT_TEMPLATE,
    DEFAULT_TITLE_TEMPLATE,
    LEGACY_DEFAULT_TEMPLATE,
)

DEFAULT_CONFIG = {
    "garmin_poll_interval_minutes": 20,
    "strava_poll_interval_minutes": 5,
    "lookback_days": 7,
    "display_timezone": "Europe/Stockholm",
    "hevy_enabled": False,
    "hevy_match_mode": "automatic",
    "hevy_title_template": DEFAULT_TITLE_TEMPLATE,
    "hevy_description_template": DEFAULT_TEMPLATE,
    "hevy_summary_on_structured": True,
    # Task-1 spike gate: S1+S2+S3 passed, so replace is the default and all
    # three strategies are selectable.
    "hevy_watch_strategy": "replace",
    "hevy_poll_interval_minutes": 10,
    "hevy_grace_minutes": 120,
    # Auto-detected from the user's own watch FIT (replace path persists it);
    # manual override in settings; None falls back to a per-install identity.
    "hevy_device_identity": None,
    "held_activity_types": [],
}


def load_config(conn: sqlite3.Connection) -> dict:
    cfg = dict(DEFAULT_CONFIG)
    stored = db.get_config_value(conn, "settings", default={})
    if "hevy_match_mode" not in stored and _development_mode():
        cfg["hevy_match_mode"] = "review"
    cfg.update(stored)
    if cfg["hevy_description_template"] == LEGACY_DEFAULT_TEMPLATE:
        cfg["hevy_description_template"] = DEFAULT_TEMPLATE
    return cfg


def _development_mode() -> bool:
    for name in ("ACTIVSYNC_DEV_MOCK_DATA", "ACTIVSYNC_MANUAL_ONLY"):
        if os.environ.get(name, "").lower() in ("1", "true", "yes"):
            return True
    return False


def save_config(conn: sqlite3.Connection, cfg: dict) -> None:
    db.set_config_value(conn, "settings", cfg)
