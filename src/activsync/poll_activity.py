"""Whether the user has just trained, which makes a fast Garmin poll worth it.

Right after a session the watch syncs, a second activity or a Hevy workout
often follows, and every minute of polling delay is felt. The rest of the
day nothing arrives, so the configured (slower) interval applies.
"""

from __future__ import annotations

import sqlite3
from datetime import datetime, timedelta

from activsync import db, hevy_db
from activsync.timeutil import parse_timestamp

ACTIVE_WINDOW = timedelta(hours=2)
ACTIVE_GARMIN_INTERVAL_SECONDS = 5 * 60

_MAX_SESSION = timedelta(hours=24)

_PENDING_HEVY_STATUSES = ("waiting_watch", "syncing")


def recently_active(conn: sqlite3.Connection, now: datetime) -> bool:
    window_start = now - ACTIVE_WINDOW
    return (
        _garmin_activity_ended_since(conn, window_start, now)
        or _hevy_workout_ended_since(conn, window_start, now)
        or any(
            hevy_db.has_workouts_with_status(conn, status)
            for status in _PENDING_HEVY_STATUSES
        )
    )


def _garmin_activity_ended_since(
    conn: sqlite3.Connection, window_start: datetime, now: datetime
) -> bool:
    scan_from = (now - _MAX_SESSION).strftime("%Y-%m-%d %H:%M:%S")
    for activity in db.list_activities_started_since(conn, scan_from):
        start = parse_timestamp(activity["start_time"])
        if start is None:
            continue
        end = start + timedelta(seconds=db.activity_duration_seconds(activity))
        if end >= window_start:
            return True
    return False


def _hevy_workout_ended_since(
    conn: sqlite3.Connection, window_start: datetime, now: datetime
) -> bool:
    scan_from = (now - _MAX_SESSION).isoformat()[:10]
    for workout in hevy_db.list_workouts_started_since(conn, scan_from):
        end = parse_timestamp(workout["end_time"])
        if end is not None and end >= window_start:
            return True
    return False
