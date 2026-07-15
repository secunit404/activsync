"""SQLite data layer: activities table + app_config key/value store."""

from __future__ import annotations

import json
import sqlite3
from datetime import datetime

SCHEMA = """
CREATE TABLE IF NOT EXISTS activities (
    garmin_activity_id INTEGER PRIMARY KEY,
    activity_type TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    start_time TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    publish_status TEXT NOT NULL,
    strava_activity_id INTEGER,
    first_seen_at TEXT NOT NULL,
    held_since TEXT,
    published_at TEXT,
    garmin_data TEXT NOT NULL DEFAULT '{}',
    hold_reason TEXT
);

CREATE TABLE IF NOT EXISTS app_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);
"""


def connect(path: str) -> sqlite3.Connection:
    conn = sqlite3.connect(path, check_same_thread=False)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA journal_mode=WAL")
    conn.executescript(SCHEMA)
    # migrate pre-existing DBs that lack later columns
    cols = {row["name"] for row in conn.execute("PRAGMA table_info(activities)")}
    if "garmin_data" not in cols:
        conn.execute("ALTER TABLE activities ADD COLUMN garmin_data TEXT NOT NULL DEFAULT '{}'")
    if "hold_reason" not in cols:
        # Why a row is 'held': 'category' (its type is not an autosync
        # category) or 'backlog' (it predates the normal window and was only
        # seen because a reconnect catch-up widened it). Pre-existing held rows
        # migrate to NULL, which reads as 'category' — the only kind of hold
        # that existed before catch-up.
        conn.execute("ALTER TABLE activities ADD COLUMN hold_reason TEXT")
    return conn


def insert_activity(
    conn: sqlite3.Connection,
    garmin_activity_id: int,
    activity_type: str,
    title: str,
    description: str,
    start_time: str,
    content_hash: str,
    publish_status: str,
    now: datetime,
    garmin_data: str = "{}",
    hold_reason: str | None = None,
) -> None:
    held = publish_status == "held"
    held_since = now.isoformat() if held else None
    conn.execute(
        """INSERT INTO activities
           (garmin_activity_id, activity_type, title, description, start_time,
            content_hash, publish_status, first_seen_at, held_since, garmin_data,
            hold_reason)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
        (garmin_activity_id, activity_type, title, description, start_time,
         content_hash, publish_status, now.isoformat(), held_since, garmin_data,
         hold_reason if held else None),
    )
    conn.commit()


def get_activity(conn: sqlite3.Connection, garmin_activity_id: int) -> dict | None:
    row = conn.execute(
        "SELECT * FROM activities WHERE garmin_activity_id = ?", (garmin_activity_id,)
    ).fetchone()
    return dict(row) if row else None


def update_activity_content(
    conn: sqlite3.Connection,
    garmin_activity_id: int,
    title: str,
    description: str,
    activity_type: str,
    content_hash: str,
    publish_status: str,
    garmin_data: str = "{}",
) -> None:
    conn.execute(
        """UPDATE activities
           SET title = ?, description = ?, activity_type = ?, content_hash = ?,
               publish_status = ?, garmin_data = ?,
               hold_reason = CASE WHEN ? = 'held' THEN hold_reason ELSE NULL END
           WHERE garmin_activity_id = ?""",
        (title, description, activity_type, content_hash, publish_status,
         garmin_data, publish_status, garmin_activity_id),
    )
    conn.commit()


def update_activity_metadata(
    conn: sqlite3.Connection,
    garmin_activity_id: int,
    title: str,
    description: str,
    content_hash: str,
) -> None:
    conn.execute(
        """UPDATE activities
           SET title = ?, description = ?, content_hash = ?
           WHERE garmin_activity_id = ?""",
        (title, description, content_hash, garmin_activity_id),
    )
    conn.commit()


def set_publish_status(
    conn: sqlite3.Connection,
    garmin_activity_id: int,
    status: str,
    hold_reason: str | None = None,
) -> None:
    """Set a row's publish status. hold_reason only survives while the row is
    'held' — a row that leaves 'held' (published, excluded, promoted to pending)
    carries no reason to be held."""
    conn.execute(
        "UPDATE activities SET publish_status = ?, hold_reason = ? WHERE garmin_activity_id = ?",
        (status, hold_reason if status == "held" else None, garmin_activity_id),
    )
    conn.commit()


def mark_removed(conn: sqlite3.Connection, garmin_activity_id: int) -> None:
    """Hard-delete an activity that Garmin no longer returns.

    Garmin is the source of truth: once an activity is gone from Garmin, its
    local row has no reason to linger. We do NOT touch any Strava copy — a
    published activity removed from Garmin keeps its Strava entry; only the
    local row is dropped, along with its strava_activity_id link. If the
    activity ever reappears in a later fetch, publish_pending re-links it to the
    existing Strava activity by start time rather than uploading a duplicate, so
    no tombstone is needed to guard against that.
    """
    conn.execute("DELETE FROM activities WHERE garmin_activity_id = ?", (garmin_activity_id,))
    conn.commit()


def set_published(
    conn: sqlite3.Connection, garmin_activity_id: int, strava_activity_id: int, now: datetime
) -> None:
    conn.execute(
        """UPDATE activities
           SET publish_status = 'published', strava_activity_id = ?, published_at = ?,
               hold_reason = NULL
           WHERE garmin_activity_id = ?""",
        (strava_activity_id, now.isoformat(), garmin_activity_id),
    )
    conn.commit()


def list_activities(
    conn: sqlite3.Connection,
    status: str | None = None,
    sort_order: str = "newest",
) -> list[dict]:
    order = "ASC" if sort_order == "oldest" else "DESC"
    if status is not None:
        rows = conn.execute(
            f"SELECT * FROM activities WHERE publish_status = ? ORDER BY start_time {order}",
            (status,),
        ).fetchall()
    else:
        rows = conn.execute(f"SELECT * FROM activities ORDER BY start_time {order}").fetchall()
    return [dict(row) for row in rows]


def list_active_ids_since(conn: sqlite3.Connection, window_start: str) -> set[int]:
    """Activity ids that started at or after window_start.

    Scoped by window because a sync can only speak for the span it fetched:
    an activity outside that span is unknown, not deleted. Removed activities
    are hard-deleted, so every remaining row is active by definition.
    """
    rows = conn.execute(
        "SELECT garmin_activity_id FROM activities WHERE start_time >= ?",
        (window_start,),
    ).fetchall()
    return {row["garmin_activity_id"] for row in rows}


def oldest_unpublished_start_time(conn: sqlite3.Connection) -> str | None:
    """start_time of the oldest activity still awaiting a decision (pending or
    held), or None when there is no such work.

    This is the true extent of the backlog the Strava side has to reconcile —
    unlike the Garmin outage, it is visible during a Strava-only outage, when
    Garmin sync kept succeeding the whole time.
    """
    row = conn.execute(
        "SELECT MIN(start_time) AS oldest FROM activities "
        "WHERE publish_status IN ('pending', 'held')"
    ).fetchone()
    return row["oldest"] if row and row["oldest"] else None


def get_config_value(conn: sqlite3.Connection, key: str, default=None):
    row = conn.execute("SELECT value FROM app_config WHERE key = ?", (key,)).fetchone()
    if row is None:
        return default
    return json.loads(row["value"])


def set_config_value(conn: sqlite3.Connection, key: str, value) -> None:
    conn.execute(
        """INSERT INTO app_config (key, value) VALUES (?, ?)
           ON CONFLICT(key) DO UPDATE SET value = excluded.value""",
        (key, json.dumps(value)),
    )
    conn.commit()


def insert_session(
    conn: sqlite3.Connection, token_hash: str, created_at: str, expires_at: str
) -> None:
    conn.execute(
        "INSERT INTO sessions (token_hash, created_at, expires_at) VALUES (?, ?, ?)",
        (token_hash, created_at, expires_at),
    )
    conn.commit()


def get_session(conn: sqlite3.Connection, token_hash: str) -> dict | None:
    row = conn.execute(
        "SELECT * FROM sessions WHERE token_hash = ?", (token_hash,)
    ).fetchone()
    return dict(row) if row else None


def delete_session(conn: sqlite3.Connection, token_hash: str) -> None:
    conn.execute("DELETE FROM sessions WHERE token_hash = ?", (token_hash,))
    conn.commit()


def delete_all_sessions(conn: sqlite3.Connection) -> None:
    conn.execute("DELETE FROM sessions")
    conn.commit()
