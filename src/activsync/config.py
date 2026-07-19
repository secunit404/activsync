"""App configuration: defaults plus DB-backed overrides."""

from __future__ import annotations

import sqlite3

from activsync import db

DEFAULT_CONFIG = {
    "garmin_poll_interval_minutes": 20,
    "strava_poll_interval_minutes": 5,
    "lookback_days": 7,
    "display_timezone": "Europe/Stockholm",
    "hevy2garmin_marker": "— synced by hevy2garmin",
    "hevy2garmin_marker_enabled": False,
    "hevy_enabled": False,
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
    cfg.update(stored)
    return cfg


def save_config(conn: sqlite3.Connection, cfg: dict) -> None:
    db.set_config_value(conn, "settings", cfg)
