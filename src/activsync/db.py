"""SQLite data layer: activities table + app_config key/value store."""

from __future__ import annotations

import json
import sqlite3
import threading
from contextlib import contextmanager
from datetime import datetime

# `check_same_thread=False` (below) only disables Python's ownership check —
# it does NOT make the sqlite3 module safe for concurrent use of one
# Connection from multiple threads. FastAPI runs every sync route handler in
# a worker thread pool, and this app shares a single Connection across all of
# them (see main.py's module-level `_conn`), so two requests landing at the
# same moment reliably corrupted each other's reads: observed
# `sqlite3.InterfaceError: bad parameter or other API misuse` and rows from
# one query silently showing up in another's result under concurrent
# requests (e.g. plain `curl` fired in parallel at /api/v1/app).
#
# The actual corruption source was the stdlib's per-connection LRU cache of
# *compiled statement objects* (keyed by SQL text) that `cached_statements=0`
# below now disables — two concurrent calls executing the same SQL text with
# different bind parameters could be handed the same not-thread-safe
# statement object mid-bind/step. SQLite itself is compiled serialized
# (`sqlite3.threadsafety == 3`), so with that cache gone, a plain per-call
# lock around `execute`/`commit`/etc. is enough to keep single-statement
# calls safe on this shared connection.
#
# It is an `RLock`, not a plain `Lock`, for a different reason than a stale
# version of this comment used to claim: `Connection.execute` does NOT
# internally call `self.cursor()` (verified against CPython 3.14 — it never
# instantiates our old cursor subclass). The real reentrancy need comes from
# `transaction()` below: it holds `_DB_LOCK` across an entire multi-statement
# unit of work, including the `execute()`/`commit()` calls made from inside
# that `with` block — and those calls independently re-acquire `_DB_LOCK` via
# the overrides just below. A plain `Lock` would deadlock on that nesting.
_DB_LOCK = threading.RLock()


class _LockingConnection(sqlite3.Connection):
    """Serializes all statement execution and fetching against `_DB_LOCK` —
    see the module docstring above `_DB_LOCK` for why a shared, multi-thread
    Connection needs this. Every call site in this codebase goes through
    `conn.execute(...)`/`conn.executemany(...)`/`conn.executescript(...)`/
    `conn.commit()`/`conn.rollback()` (no call site holds its own
    `.cursor()`), so overriding those five covers the whole app without
    touching any of those call sites."""

    def execute(self, *args, **kwargs):
        with _DB_LOCK:
            return super().execute(*args, **kwargs)

    def executemany(self, *args, **kwargs):
        with _DB_LOCK:
            return super().executemany(*args, **kwargs)

    def executescript(self, *args, **kwargs):
        with _DB_LOCK:
            return super().executescript(*args, **kwargs)

    def commit(self):
        with _DB_LOCK:
            return super().commit()

    def rollback(self):
        with _DB_LOCK:
            return super().rollback()


@contextmanager
def transaction(conn: sqlite3.Connection):
    """Hold `_DB_LOCK` across an entire multi-statement unit of work, from
    before its first statement through its final `commit()`/`rollback()`.

    With `isolation_level=""` (the sqlite3 default we use), the implicit
    `BEGIN` opened by the first write is connection-global. On a connection
    shared across threads, a second thread's unrelated `commit()` landing
    between two statements of a "single" logical unit commits that unit's
    first statement early — the second thread didn't ask to commit someone
    else's half-finished work, but `sqlite3.Connection.commit()` doesn't know
    the difference. Per-call locking on `execute`/`commit` (above) doesn't
    prevent this: each call takes and releases the lock individually, so
    another thread can still slip a full execute+commit cycle of its own in
    between two calls of the "atomic" unit.

    Usage — replace the unit's own `commit()`/`rollback()` with this:

        with db.transaction(conn):
            conn.execute(...)
            conn.execute(...)
            # no manual commit() — this context manager commits on a clean
            # exit and rolls back (then re-raises) on any exception.
    """
    with _DB_LOCK:
        try:
            yield conn
            conn.commit()
        except Exception:
            conn.rollback()
            raise


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
"""


def connect(path: str) -> sqlite3.Connection:
    # cached_statements=0: the stdlib sqlite3 module keeps a per-connection
    # LRU cache of *compiled statement objects* keyed by SQL text, reused
    # across calls that pass the same SQL string (e.g. every
    # `get_config_value` call executes the identical
    # "SELECT value FROM app_config WHERE key = ?"). Two concurrent
    # get_config_value calls for *different keys* could be handed that same
    # shared, not-thread-safe statement object mid-bind/step, mixing one
    # call's row into the other's — this is what actually produced the
    # `sqlite3.InterfaceError`s and cross-contaminated config reads under
    # concurrent requests (SQLite itself is compiled serialized/thread-safe
    # here — `sqlite3.threadsafety == 3` — so the corruption was in this
    # Python-level cache, not the C library). Disabling the cache forces a
    # fresh compile per call, which is fine at this app's request volume.
    conn = sqlite3.connect(
        path, check_same_thread=False, cached_statements=0, factory=_LockingConnection
    )
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
    # Password auth was removed before the first public release, but databases
    # created under it still carry the session table and the stored password
    # hash. Nothing reads either, and a dead feature's credential is not worth
    # keeping at rest. Both statements are no-ops on a database that never had
    # them, so this stays in connect() rather than needing a version check.
    conn.execute("DROP TABLE IF EXISTS sessions")
    conn.execute("DELETE FROM app_config WHERE key = 'auth'")
    conn.commit()
    # Hevy sync tables live in their own module; local import avoids a cycle.
    from activsync import hevy_db
    hevy_db.init_schema(conn)
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
