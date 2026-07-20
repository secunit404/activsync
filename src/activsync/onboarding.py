"""Shared connection and onboarding state transitions.

Routes own HTTP validation and responses. This module owns the durable state
changes that must stay identical across setup, reconnect, and OAuth callbacks.
"""

from __future__ import annotations

import logging
import sqlite3
from datetime import datetime, timezone
from typing import Literal

from activsync import config, db, events, sync, view

SetupStep = Literal["garmin", "strava", "hevy", "syncing"]

logger = logging.getLogger("activsync.onboarding")


def setup_step(
    conn: sqlite3.Connection,
    connections: dict | None = None,
) -> SetupStep | None:
    """Return the active first-run step, or ``None`` after onboarding."""
    if db.get_config_value(conn, "initial_sync_done", default=False):
        return None
    connections = connections or view.connection_status(conn)
    if not connections["garmin"]["connected"]:
        return "garmin"
    if not connections["strava"]["connected"]:
        return "strava"
    if not db.get_config_value(conn, "setup_hevy_done", default=False):
        return "hevy"
    return "syncing"


def store_garmin_categories(
    conn: sqlite3.Connection,
    garmin_client,
    *,
    hold_all: bool,
) -> None:
    """Fetch Garmin activity types and optionally hold every type for review."""
    activity_types = garmin_client.fetch_activity_types()
    db.set_config_value(conn, "garmin_activity_types", activity_types)
    db.set_config_value(
        conn,
        "garmin_activity_types_fetched_at",
        datetime.now(timezone.utc).isoformat(),
    )
    if hold_all:
        cfg = config.load_config(conn)
        cfg["held_activity_types"] = sorted(
            item["type_key"] for item in activity_types
        )
        config.save_config(conn, cfg)


def persist_strava_credentials(
    conn: sqlite3.Connection,
    client_id: str,
    client_secret: str,
) -> None:
    """Save Strava app credentials and invalidate tokens when they change."""
    existing = db.get_config_value(conn, "strava_credentials") or {}
    changed = (
        existing.get("client_id") != client_id
        or existing.get("client_secret") != client_secret
    )
    db.set_config_value(
        conn,
        "strava_credentials",
        {"client_id": client_id, "client_secret": client_secret},
    )
    if changed:
        db.set_config_value(conn, "strava_tokens", None)


def run_catch_up(
    conn: sqlite3.Connection,
    garmin_client,
    strava_client,
    *,
    last_sync_ok_at: str | None = sync.LAST_SYNC_UNSET,
) -> None:
    """Best-effort reconnect catch-up with a durable dashboard summary."""
    try:
        stats = sync.catch_up_sync(
            conn,
            garmin_client,
            strava_client,
            config.load_config(conn),
            datetime.now(timezone.utc),
            last_sync_ok_at=last_sync_ok_at,
        )
    except Exception:
        logger.exception("catch-up sync after reconnect failed")
        events.bus.publish("refresh")
        return

    found_anything = stats.garmin.new or stats.status.linked_existing
    db.set_config_value(
        conn,
        "catch_up_report",
        {
            "new": stats.garmin.new,
            "held": stats.garmin.held_backlog,
            "linked": stats.status.linked_existing,
            "days": stats.lookback_days,
        }
        if found_anything
        else None,
    )
    events.bus.publish("refresh")
