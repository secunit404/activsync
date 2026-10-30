"""Server-side undo window for Hevy match decisions.

Choosing Merge, Replace or Description only on an awaiting workout is written
to the hevy_match_schedule table with a deadline and applied to Garmin by
HevyMatchScheduler once that deadline passes, so the decision goes through
whether or not the page that asked for it is still open. Undo deletes the row
while it is still waiting.

Every state change is one guarded statement run under db's connection lock:
the scheduler claims with `UPDATE ... WHERE state = 'waiting'` and undo
deletes with `DELETE ... WHERE state = 'waiting'`, so exactly one of them
matches the row and the other sees zero rows.
"""

from __future__ import annotations

import logging
import sqlite3
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass
from datetime import datetime, timezone

from activsync import db, events, hevy_db
from activsync.hevy_client import HevyAuthError

logger = logging.getLogger("activsync.hevy_match")

UNDO_WINDOW_SECONDS = 5.0

WAITING = "waiting"
APPLYING = "applying"
DONE = "done"
FAILED = "failed"
SKIPPED = "skipped"

_RESULT_RETENTION_SECONDS = 3600
_MAX_SLEEP_SECONDS = 30.0
_LABELS = {"merged": "Merged", "replaced": "Replaced", "described": "Described"}

AUTH_MESSAGE = "Hevy rejected the stored API key — reconnect Hevy."
MAPPING_MESSAGE = "Map the missing exercises before choosing Merge or Replace."
FAILURE_MESSAGE = "Could not apply that Hevy match. Check the server log for details."


class AlreadyScheduled(Exception):
    """The workout already has a decision waiting or being applied."""


@dataclass(frozen=True)
class Scheduled:
    hevy_id: str
    strategy: str
    deadline: float


def schedule(
    conn: sqlite3.Connection,
    hevy_id: str,
    strategy: str,
    now: float,
    *,
    window_seconds: float = UNDO_WINDOW_SECONDS,
) -> Scheduled:
    scheduled = Scheduled(hevy_id, strategy, now + window_seconds)
    with db.transaction(conn):
        active = conn.execute(
            "SELECT 1 FROM hevy_match_schedule WHERE hevy_id = ? "
            f"AND state IN ('{WAITING}', '{APPLYING}')",
            (hevy_id,),
        ).fetchone()
        if active is not None:
            raise AlreadyScheduled()
        conn.execute(
            """INSERT INTO hevy_match_schedule (hevy_id, strategy, deadline, state)
               VALUES (?, ?, ?, ?)
               ON CONFLICT(hevy_id) DO UPDATE SET
                   strategy = excluded.strategy,
                   deadline = excluded.deadline,
                   state = excluded.state,
                   message = NULL,
                   finished_at = NULL""",
            (hevy_id, strategy, scheduled.deadline, WAITING),
        )
    return scheduled


def cancel(conn: sqlite3.Connection, hevy_id: str) -> int:
    """Drop the decision if it is still waiting. Returns how many rows were
    dropped; one already claimed by the scheduler stays put."""
    with db.transaction(conn):
        cursor = conn.execute(
            f"DELETE FROM hevy_match_schedule WHERE hevy_id = ? AND state = '{WAITING}'",
            (hevy_id,),
        )
        return cursor.rowcount


def claim_due(conn: sqlite3.Connection, now: float) -> list[dict]:
    with db.transaction(conn):
        rows = conn.execute(
            f"UPDATE hevy_match_schedule SET state = '{APPLYING}' "
            f"WHERE state = '{WAITING}' AND deadline <= ? "
            "RETURNING hevy_id, strategy, deadline",
            (now,),
        ).fetchall()
    return sorted((dict(row) for row in rows), key=lambda row: row["deadline"])


def finish(
    conn: sqlite3.Connection,
    hevy_id: str,
    state: str,
    message: str | None,
    now: float,
) -> None:
    conn.execute(
        "UPDATE hevy_match_schedule SET state = ?, message = ?, finished_at = ? "
        f"WHERE hevy_id = ? AND state = '{APPLYING}'",
        (state, message, now, hevy_id),
    )
    conn.commit()


def recover(conn: sqlite3.Connection) -> int:
    """Hand decisions a previous process was applying back to the queue.

    Re-applying an interrupted decision is safe: the apply step re-reads the
    workout first and finishes as skipped unless it is still awaiting_match,
    and hevy_sync.apply_match_choice refuses a workout with an open operation,
    so Replace (the only strategy that journals its work) resumes through the
    operation journal rather than being repeated here. Merge and Describe are
    single idempotent Garmin writes, and the merge backup keeps the first
    snapshot. If the dead process still holds the workout's lease, the apply
    fails with the generic message and the owner chooses again.
    """
    cursor = conn.execute(
        f"UPDATE hevy_match_schedule SET state = '{WAITING}' WHERE state = '{APPLYING}'"
    )
    conn.commit()
    return cursor.rowcount


def prune(conn: sqlite3.Connection, now: float) -> None:
    conn.execute(
        "DELETE FROM hevy_match_schedule WHERE finished_at IS NOT NULL AND finished_at < ?",
        (now - _RESULT_RETENTION_SECONDS,),
    )
    conn.commit()


def next_deadline(conn: sqlite3.Connection) -> float | None:
    row = conn.execute(
        f"SELECT MIN(deadline) AS deadline FROM hevy_match_schedule WHERE state = '{WAITING}'"
    ).fetchone()
    return row["deadline"] if row else None


def active_by_workout(conn: sqlite3.Connection) -> dict[str, dict]:
    rows = conn.execute(
        "SELECT hevy_id, deadline, state FROM hevy_match_schedule "
        f"WHERE state IN ('{WAITING}', '{APPLYING}')"
    ).fetchall()
    return {row["hevy_id"]: dict(row) for row in rows}


def is_active(conn: sqlite3.Connection, hevy_id: str) -> bool:
    row = conn.execute(
        "SELECT 1 FROM hevy_match_schedule WHERE hevy_id = ? "
        f"AND state IN ('{WAITING}', '{APPLYING}')",
        (hevy_id,),
    ).fetchone()
    return row is not None


def get(conn: sqlite3.Connection, hevy_id: str) -> dict | None:
    row = conn.execute(
        "SELECT * FROM hevy_match_schedule WHERE hevy_id = ?", (hevy_id,)
    ).fetchone()
    return dict(row) if row else None


def deadline_iso(deadline: float) -> str:
    return datetime.fromtimestamp(deadline, timezone.utc).isoformat()


class HevyMatchScheduler:
    """Applies each scheduled Hevy match decision once its deadline passes.

    A thread rather than an asyncio task for the same reason as the publish
    scheduler: applying talks to Garmin synchronously. Without an
    `apply_hevy_match` it claims nothing, so a server that cannot apply
    leaves decisions waiting instead of failing them.
    """

    def __init__(
        self,
        conn: sqlite3.Connection,
        *,
        apply_hevy_match: Callable[[str, str], str] | None,
        clock: Callable[[], float] = time.time,
        window_seconds: float = UNDO_WINDOW_SECONDS,
    ):
        self._conn = conn
        self._apply = apply_hevy_match
        self._clock = clock
        self.window_seconds = window_seconds
        self._wake = threading.Event()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    @property
    def can_apply(self) -> bool:
        return self._apply is not None

    def now(self) -> float:
        return self._clock()

    def schedule(self, hevy_id: str, strategy: str) -> Scheduled:
        scheduled = schedule(
            self._conn, hevy_id, strategy, self._clock(),
            window_seconds=self.window_seconds,
        )
        self._wake.set()
        events.bus.publish("refresh")
        return scheduled

    def cancel(self, hevy_id: str) -> int:
        cancelled = cancel(self._conn, hevy_id)
        if cancelled:
            events.bus.publish("refresh")
        return cancelled

    def run_due(self) -> int:
        """Apply every decision whose deadline has passed. Returns how many."""
        if self._apply is None:
            return 0
        claimed = claim_due(self._conn, self._clock())
        if not claimed:
            return 0
        events.bus.publish("refresh")
        for row in claimed:
            state, message = self._run(row["hevy_id"], row["strategy"])
            finish(self._conn, row["hevy_id"], state, message, self._clock())
            events.bus.publish("refresh")
        prune(self._conn, self._clock())
        return len(claimed)

    def _run(self, hevy_id: str, strategy: str) -> tuple[str, str | None]:
        workout = hevy_db.get_workout(self._conn, hevy_id)
        if workout is None or workout["status"] != "awaiting_match":
            # Skipped or handled elsewhere during the window: the owner's
            # later decision wins.
            return SKIPPED, None
        try:
            status = self._apply(hevy_id, strategy)
        except HevyAuthError:
            db.set_config_value(self._conn, "hevy_auth_ok", False)
            return FAILED, AUTH_MESSAGE
        except Exception:
            logger.exception("Hevy match decision failed for %s", hevy_id)
            return FAILED, FAILURE_MESSAGE
        if status == "needs_mapping":
            return FAILED, MAPPING_MESSAGE
        label = _LABELS.get(status, "Updated")
        return DONE, f"{label} {workout['title'] or hevy_id}."

    def _sleep_seconds(self) -> float:
        deadline = next_deadline(self._conn)
        if deadline is None:
            return _MAX_SLEEP_SECONDS
        return min(_MAX_SLEEP_SECONDS, max(0.0, deadline - self._clock()))

    def _loop(self) -> None:
        recovered = recover(self._conn)
        if recovered:
            logger.warning(
                "resuming %d Hevy match decision(s) interrupted by a restart", recovered
            )
        while not self._stop.is_set():
            self._wake.clear()
            try:
                self.run_due()
                timeout = self._sleep_seconds()
            except Exception:
                logger.exception("Hevy match scheduler pass failed")
                timeout = 1.0
            self._wake.wait(timeout)

    def start(self) -> None:
        self._stop.clear()
        self._thread = threading.Thread(
            target=self._loop, name="hevy-match-scheduler", daemon=True
        )
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        self._wake.set()
        if self._thread is not None:
            self._thread.join(timeout=2)
