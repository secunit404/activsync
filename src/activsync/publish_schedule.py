"""Server-side undo window for publishing to Strava.

A publish the owner confirms is written to the publish_schedule table with a
deadline and sent by PublishScheduler once that deadline passes, so it goes
through whether or not the page that asked for it is still open. Undo deletes
the row while it is still waiting.

Every state change is one guarded statement run under db's connection lock:
the scheduler claims with `UPDATE ... WHERE state = 'waiting'` and undo
deletes with `DELETE ... WHERE state = 'waiting'`, so exactly one of them
matches the row and the other sees zero rows.
"""

from __future__ import annotations

import logging
import secrets
import sqlite3
import threading
import time
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from datetime import datetime, timezone

from activsync import db, events, sync
from activsync.strava_client import StravaAuthError

logger = logging.getLogger("activsync.publish")

UNDO_WINDOW_SECONDS = 5.0
PUBLISHABLE_STATUSES = ("held", "pending", "missing")

WAITING = "waiting"
SENDING = "sending"
PUBLISHED = "published"
FAILED = "failed"
BLOCKED = "blocked"
SKIPPED = "skipped"

_RESULT_RETENTION_SECONDS = 3600
# Upper bound on one sleep, so a deadline written by another process (or a
# missed wake-up) is never more than this late.
_MAX_SLEEP_SECONDS = 30.0


class AlreadyScheduled(Exception):
    """One of the activities is already waiting to publish or being sent."""


@dataclass(frozen=True)
class Scheduled:
    batch_id: str
    deadline: float


def _placeholders(values: list) -> str:
    return ", ".join("?" for _ in values)


def schedule(
    conn: sqlite3.Connection,
    activities: Iterable[dict],
    now: float,
    *,
    kind: str,
    window_seconds: float = UNDO_WINDOW_SECONDS,
) -> Scheduled:
    rows = list(activities)
    ids = [row["garmin_activity_id"] for row in rows]
    batch = Scheduled(batch_id=secrets.token_urlsafe(9), deadline=now + window_seconds)
    with db.transaction(conn):
        active = conn.execute(
            f"SELECT 1 FROM publish_schedule WHERE garmin_activity_id IN ({_placeholders(ids)}) "
            f"AND state IN ('{WAITING}', '{SENDING}')",
            ids,
        ).fetchone()
        if active is not None:
            raise AlreadyScheduled()
        # A finished outcome from an earlier publish of the same activity
        # (republishing a missing row) is replaced, not kept beside it.
        conn.executemany(
            """INSERT INTO publish_schedule
               (garmin_activity_id, batch_id, batch_kind, deadline, state, prior_status)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT(garmin_activity_id) DO UPDATE SET
                   batch_id = excluded.batch_id,
                   batch_kind = excluded.batch_kind,
                   deadline = excluded.deadline,
                   state = excluded.state,
                   prior_status = excluded.prior_status,
                   message = NULL,
                   finished_at = NULL""",
            [
                (row["garmin_activity_id"], batch.batch_id, kind, batch.deadline,
                 WAITING, row["publish_status"])
                for row in rows
            ],
        )
    return batch


def cancel(conn: sqlite3.Connection, garmin_activity_ids: Iterable[int]) -> int:
    """Drop the schedule of every listed activity still waiting. Returns how
    many were dropped; one already claimed by the scheduler stays put."""
    ids = list(garmin_activity_ids)
    with db.transaction(conn):
        cursor = conn.execute(
            f"DELETE FROM publish_schedule WHERE garmin_activity_id IN ({_placeholders(ids)}) "
            f"AND state = '{WAITING}'",
            ids,
        )
        return cursor.rowcount


def claim_due(conn: sqlite3.Connection, now: float) -> list[dict]:
    with db.transaction(conn):
        rows = conn.execute(
            f"UPDATE publish_schedule SET state = '{SENDING}' "
            f"WHERE state = '{WAITING}' AND deadline <= ? "
            "RETURNING garmin_activity_id, batch_id, deadline",
            (now,),
        ).fetchall()
    return sorted((dict(row) for row in rows), key=lambda row: row["deadline"])


def finish(
    conn: sqlite3.Connection,
    garmin_activity_id: int,
    state: str,
    message: str | None,
    now: float,
) -> None:
    conn.execute(
        f"UPDATE publish_schedule SET state = ?, message = ?, finished_at = ? "
        f"WHERE garmin_activity_id = ? AND state = '{SENDING}'",
        (state, message, now, garmin_activity_id),
    )
    conn.commit()


def recover(conn: sqlite3.Connection) -> int:
    """Hand rows a previous process was sending back to the queue.

    Their deadline has passed, so they go out on the next pass. Whether the
    interrupted upload reached Strava is unknown; the publish path asks
    Strava for an activity at the same start time before uploading, which
    links a finished upload instead of duplicating it.
    """
    cursor = conn.execute(
        f"UPDATE publish_schedule SET state = '{WAITING}' WHERE state = '{SENDING}'"
    )
    conn.commit()
    return cursor.rowcount


def prune(conn: sqlite3.Connection, now: float) -> None:
    conn.execute(
        "DELETE FROM publish_schedule WHERE finished_at IS NOT NULL AND finished_at < ?",
        (now - _RESULT_RETENTION_SECONDS,),
    )
    conn.commit()


def next_deadline(conn: sqlite3.Connection) -> float | None:
    row = conn.execute(
        f"SELECT MIN(deadline) AS deadline FROM publish_schedule WHERE state = '{WAITING}'"
    ).fetchone()
    return row["deadline"] if row else None


def active_by_activity(conn: sqlite3.Connection) -> dict[int, dict]:
    rows = conn.execute(
        "SELECT garmin_activity_id, deadline, state FROM publish_schedule "
        f"WHERE state IN ('{WAITING}', '{SENDING}')"
    ).fetchall()
    return {row["garmin_activity_id"]: dict(row) for row in rows}


def get(conn: sqlite3.Connection, garmin_activity_id: int) -> dict | None:
    row = conn.execute(
        "SELECT * FROM publish_schedule WHERE garmin_activity_id = ?", (garmin_activity_id,)
    ).fetchone()
    return dict(row) if row else None


def batch_rows(conn: sqlite3.Connection, batch_id: str) -> list[dict]:
    rows = conn.execute(
        "SELECT * FROM publish_schedule WHERE batch_id = ?", (batch_id,)
    ).fetchall()
    return [dict(row) for row in rows]


def deadline_iso(deadline: float) -> str:
    return datetime.fromtimestamp(deadline, timezone.utc).isoformat()


class PublishScheduler:
    """Sends each scheduled publish once its deadline passes.

    A thread rather than an asyncio task: the publish path is blocking
    (Garmin FIT download, Strava upload polling) and the poller already runs
    the same code on a thread. The thread sleeps until the earliest deadline
    and is woken early when a new publish is scheduled.
    """

    def __init__(
        self,
        conn: sqlite3.Connection,
        *,
        build_garmin_client: Callable[[sqlite3.Connection], object],
        build_strava_client: Callable[[sqlite3.Connection], object],
        clock: Callable[[], float] = time.time,
        window_seconds: float = UNDO_WINDOW_SECONDS,
    ):
        self._conn = conn
        self._build_garmin_client = build_garmin_client
        self._build_strava_client = build_strava_client
        self._clock = clock
        self.window_seconds = window_seconds
        self._wake = threading.Event()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def now(self) -> float:
        return self._clock()

    def schedule(self, activities: Iterable[dict], *, kind: str) -> Scheduled:
        batch = schedule(
            self._conn, activities, self._clock(), kind=kind,
            window_seconds=self.window_seconds,
        )
        self._wake.set()
        events.bus.publish("refresh")
        return batch

    def cancel(self, garmin_activity_ids: Iterable[int]) -> int:
        cancelled = cancel(self._conn, garmin_activity_ids)
        if cancelled:
            events.bus.publish("refresh")
        return cancelled

    def run_due(self) -> int:
        """Send every publish whose deadline has passed. Returns how many."""
        claimed = claim_due(self._conn, self._clock())
        if not claimed:
            return 0
        events.bus.publish("refresh")
        try:
            garmin = self._build_garmin_client(self._conn)
            strava = self._build_strava_client(self._conn)
        except Exception as exc:
            logger.exception("scheduled publish could not reach Garmin or Strava")
            for row in claimed:
                finish(self._conn, row["garmin_activity_id"], FAILED,
                       f"Could not publish activity: {exc}", self._clock())
            events.bus.publish("refresh")
            return len(claimed)
        for row in claimed:
            state, message = self._send(row["garmin_activity_id"], garmin, strava)
            finish(self._conn, row["garmin_activity_id"], state, message, self._clock())
            events.bus.publish("refresh")
        prune(self._conn, self._clock())
        return len(claimed)

    def _send(self, garmin_activity_id: int, garmin, strava) -> tuple[str, str | None]:
        activity = db.get_activity(self._conn, garmin_activity_id)
        if db.is_published(activity):
            return PUBLISHED, None
        if activity is None or activity["publish_status"] not in PUBLISHABLE_STATUSES:
            # Excluded from another device, or removed on Garmin, during the
            # window: the owner's later decision wins.
            return SKIPPED, None
        try:
            sync.publish_now(
                self._conn, garmin, strava, garmin_activity_id, datetime.now(timezone.utc)
            )
        except sync.PublishBlocked as exc:
            return BLOCKED, f"Activity was blocked by Hevy sync: {exc.reason}."
        except StravaAuthError as exc:
            return FAILED, str(exc)
        except Exception as exc:
            logger.exception("scheduled publish failed for activity %s", garmin_activity_id)
            return FAILED, f"Could not publish activity: {exc}"
        logger.info("published activity %s to Strava", garmin_activity_id)
        return PUBLISHED, None

    def _sleep_seconds(self) -> float:
        deadline = next_deadline(self._conn)
        if deadline is None:
            return _MAX_SLEEP_SECONDS
        return min(_MAX_SLEEP_SECONDS, max(0.0, deadline - self._clock()))

    def _loop(self) -> None:
        recovered = recover(self._conn)
        if recovered:
            logger.warning(
                "resuming %d publish(es) interrupted by a restart", recovered
            )
        while not self._stop.is_set():
            self._wake.clear()
            try:
                self.run_due()
                timeout = self._sleep_seconds()
            except Exception:
                logger.exception("publish scheduler pass failed")
                timeout = 1.0
            self._wake.wait(timeout)

    def start(self) -> None:
        self._stop.clear()
        self._thread = threading.Thread(
            target=self._loop, name="publish-scheduler", daemon=True
        )
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        self._wake.set()
        if self._thread is not None:
            self._thread.join(timeout=2)
