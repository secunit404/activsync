"""SQLite data model for the Hevy sync leg.

Five tables: hevy_workouts (one row per Hevy workout, with status/claims/
lease), hevy_operations (durable journal for destructive or ambiguous
operations), exercise_templates (cache of Hevy's template catalog),
exercise_mappings (user overrides, winning over built-in tables), and
merge_backups (immutable pre-destruction snapshots). All accessors take the
connection first, return plain dicts, and store times as ISO-8601 UTC strings.
"""

from __future__ import annotations

import json
import sqlite3
from datetime import datetime, timedelta, timezone

STATUSES = (
    "needs_mapping",
    "waiting_watch",
    "syncing",
    "merged",
    "described",
    "replaced",
    "uploaded_passive",
    "linked_existing",
    "failed",
    "needs_review",
    "skipped",
)

SCHEMA = """
CREATE TABLE IF NOT EXISTS hevy_workouts (
    hevy_id TEXT PRIMARY KEY,
    title TEXT NOT NULL DEFAULT '',
    start_time TEXT NOT NULL,
    end_time TEXT NOT NULL,
    source_updated_at TEXT,
    garmin_applied_updated_at TEXT,
    strava_applied_updated_at TEXT,
    payload TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'waiting_watch',
    applied_strategy TEXT,
    provenance TEXT NOT NULL DEFAULT 'activsync',
    source_garmin_activity_id INTEGER,
    garmin_activity_id INTEGER,
    locked_until TEXT,
    lease_owner TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_hw_source ON hevy_workouts(source_garmin_activity_id)
    WHERE source_garmin_activity_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_hw_target ON hevy_workouts(garmin_activity_id)
    WHERE garmin_activity_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS hevy_operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    hevy_id TEXT NOT NULL,
    kind TEXT NOT NULL,                -- replace | upload_passive
    phase TEXT NOT NULL,               -- preparing|uploading|submission_unknown|finalizing|done|failed|needs_review
    next_step TEXT,                    -- upload|resolve|finalize|delete
    source_activity_id INTEGER,
    target_activity_id INTEGER,
    upload_id TEXT,
    pre_upload_ids TEXT NOT NULL DEFAULT '[]',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    delete_attempt_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    locked_until TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_op_open_hevy ON hevy_operations(hevy_id)
    WHERE phase NOT IN ('done','failed');
CREATE UNIQUE INDEX IF NOT EXISTS idx_op_open_source ON hevy_operations(source_activity_id)
    WHERE phase NOT IN ('done','failed') AND source_activity_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS exercise_templates (
    exercise_template_id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    primary_muscle_group TEXT,
    secondary_muscle_groups TEXT NOT NULL DEFAULT '[]',
    equipment_category TEXT,
    is_custom INTEGER NOT NULL DEFAULT 0,
    fetched_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS exercise_mappings (
    exercise_template_id TEXT PRIMARY KEY,
    category INTEGER NOT NULL,
    subcategory INTEGER NOT NULL,
    garmin_rejected INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS merge_backups (
    garmin_activity_id INTEGER PRIMARY KEY,
    hevy_id TEXT,
    replacement_activity_id INTEGER,
    original_sets TEXT,
    original_fit BLOB,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS hevy_events_seen (
    event_type TEXT NOT NULL,
    workout_id TEXT NOT NULL,
    event_timestamp TEXT NOT NULL,
    PRIMARY KEY (event_type, workout_id, event_timestamp)
);
"""


def _now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def init_schema(conn: sqlite3.Connection) -> None:
    """Create the Hevy tables and indexes. Idempotent; called from db.connect."""
    conn.executescript(SCHEMA)
    # migrate pre-release dev DBs created before the lease-owner column
    cols = {row["name"] for row in conn.execute("PRAGMA table_info(hevy_workouts)")}
    if "lease_owner" not in cols:
        conn.execute("ALTER TABLE hevy_workouts ADD COLUMN lease_owner TEXT")
    conn.commit()


# -- hevy_workouts ----------------------------------------------------------


def upsert_workout(
    conn: sqlite3.Connection,
    hevy_id: str,
    title: str,
    start_time: str,
    end_time: str,
    source_updated_at: str,
    payload: dict,
) -> None:
    """Insert a workout (status waiting_watch) or refresh it from a newer Hevy
    revision. Older/equal revisions are ignored; status never regresses here —
    only set_workout_status moves it."""
    now = _now_iso()
    existing = conn.execute(
        "SELECT source_updated_at FROM hevy_workouts WHERE hevy_id = ?", (hevy_id,)
    ).fetchone()
    if existing is None:
        conn.execute(
            """INSERT INTO hevy_workouts
               (hevy_id, title, start_time, end_time, source_updated_at,
                payload, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
            (hevy_id, title, start_time, end_time, source_updated_at,
             json.dumps(payload), now, now),
        )
    else:
        current = existing["source_updated_at"]
        if current is not None and source_updated_at <= current:
            return
        conn.execute(
            """UPDATE hevy_workouts
               SET title = ?, start_time = ?, end_time = ?,
                   source_updated_at = ?, payload = ?, updated_at = ?
               WHERE hevy_id = ?""",
            (title, start_time, end_time, source_updated_at,
             json.dumps(payload), now, hevy_id),
        )
    conn.commit()


def get_workout(conn: sqlite3.Connection, hevy_id: str) -> dict | None:
    row = conn.execute(
        "SELECT * FROM hevy_workouts WHERE hevy_id = ?", (hevy_id,)
    ).fetchone()
    return dict(row) if row else None


def list_workouts(conn: sqlite3.Connection, status: str | None = None) -> list[dict]:
    if status is not None:
        rows = conn.execute(
            "SELECT * FROM hevy_workouts WHERE status = ? ORDER BY start_time DESC",
            (status,),
        ).fetchall()
    else:
        rows = conn.execute(
            "SELECT * FROM hevy_workouts ORDER BY start_time DESC"
        ).fetchall()
    return [dict(row) for row in rows]


_BLOCKING_STATUSES = (
    "needs_mapping",
    "waiting_watch",
    "syncing",
    "failed",
    "needs_review",
)

_PROMOTABLE_STATUSES = (
    "merged",
    "described",
    "replaced",
    "uploaded_passive",
)


def blocking_workout_for_activity(
    conn: sqlite3.Connection, garmin_activity_id: int
) -> dict | None:
    """Return the Hevy workout that currently owns an activity.

    An unresolved workout blocks through either of its durable activity links.
    An open journal operation blocks independently of workout status because a
    resumed operation may still replace or delete the referenced activity.
    """
    status_placeholders = ", ".join("?" for _ in _BLOCKING_STATUSES)
    row = conn.execute(
        f"""SELECT workout.*
            FROM hevy_workouts AS workout
            WHERE (
                workout.status IN ({status_placeholders})
                AND (
                    workout.source_garmin_activity_id = ?
                    OR workout.garmin_activity_id = ?
                )
            ) OR EXISTS (
                SELECT 1
                FROM hevy_operations AS operation
                WHERE operation.hevy_id = workout.hevy_id
                  AND operation.phase NOT IN ('done', 'failed')
                  AND (
                      operation.source_activity_id = ?
                      OR operation.target_activity_id = ?
                  )
            )
            ORDER BY CASE WHEN workout.status IN ({status_placeholders}) THEN 0 ELSE 1 END,
                     workout.created_at
            LIMIT 1""",
        (
            *_BLOCKING_STATUSES,
            garmin_activity_id,
            garmin_activity_id,
            garmin_activity_id,
            garmin_activity_id,
            *_BLOCKING_STATUSES,
        ),
    ).fetchone()
    return dict(row) if row else None


def linked_workout_for_activity(
    conn: sqlite3.Connection, garmin_activity_id: int
) -> dict | None:
    """Return any durable Hevy link for an activity, regardless of status."""
    row = conn.execute(
        """SELECT * FROM hevy_workouts
           WHERE source_garmin_activity_id = ? OR garmin_activity_id = ?
           ORDER BY created_at
           LIMIT 1""",
        (garmin_activity_id, garmin_activity_id),
    ).fetchone()
    return dict(row) if row else None


def promotable_workout_for_activity(
    conn: sqlite3.Connection, garmin_activity_id: int
) -> dict | None:
    """Return an ActivSync-produced terminal link eligible to bypass holds."""
    status_placeholders = ", ".join("?" for _ in _PROMOTABLE_STATUSES)
    row = conn.execute(
        f"""SELECT * FROM hevy_workouts
            WHERE garmin_activity_id = ?
              AND provenance = 'activsync'
              AND status IN ({status_placeholders})
            LIMIT 1""",
        (garmin_activity_id, *_PROMOTABLE_STATUSES),
    ).fetchone()
    return dict(row) if row else None


def set_workout_status(
    conn: sqlite3.Connection, hevy_id: str, status: str, error: str | None = None
) -> None:
    conn.execute(
        "UPDATE hevy_workouts SET status = ?, error = ?, updated_at = ? WHERE hevy_id = ?",
        (status, error, _now_iso(), hevy_id),
    )
    conn.commit()


def delete_workout(conn: sqlite3.Connection, hevy_id: str) -> None:
    conn.execute("DELETE FROM hevy_workouts WHERE hevy_id = ?", (hevy_id,))
    conn.commit()


def claim_source(
    conn: sqlite3.Connection, hevy_id: str, source_garmin_activity_id: int
) -> bool:
    """Claim a watch activity as this workout's source. The partial UNIQUE
    index blocks cross-row double claims; the WHERE guard makes a claim
    immutable (no overwrite) and a missing row a failure, not a silent
    success. Re-claiming the identical source is an idempotent True."""
    try:
        cur = conn.execute(
            """UPDATE hevy_workouts SET source_garmin_activity_id = ?, updated_at = ?
               WHERE hevy_id = ?
                 AND (source_garmin_activity_id IS NULL
                      OR source_garmin_activity_id = ?)""",
            (source_garmin_activity_id, _now_iso(), hevy_id,
             source_garmin_activity_id),
        )
        conn.commit()
        return cur.rowcount == 1
    except sqlite3.IntegrityError:
        conn.rollback()
        return False


def link_target(
    conn: sqlite3.Connection,
    hevy_id: str,
    garmin_activity_id: int,
    applied_strategy: str,
    provenance: str = "activsync",
) -> None:
    """Record the final linked activity. applied_strategy is set once and
    never changed afterwards — future edits follow the original strategy."""
    cur = conn.execute(
        """UPDATE hevy_workouts
           SET garmin_activity_id = ?,
               applied_strategy = COALESCE(applied_strategy, ?),
               provenance = ?, updated_at = ?
           WHERE hevy_id = ?""",
        (garmin_activity_id, applied_strategy, provenance, _now_iso(), hevy_id),
    )
    conn.commit()
    if cur.rowcount != 1:
        raise ValueError(f"link_target: no hevy workout {hevy_id!r}")


def acquire_lease(
    conn: sqlite3.Connection, hevy_id: str, now: datetime, seconds: int = 300
) -> str | None:
    """Take the per-workout execution lease. Returns an owner token, or None
    while another holder's unexpired lease is in place. The token must be
    presented to release_lease — a worker whose lease expired and was taken
    over cannot clear the new holder's lease."""
    import secrets

    token = secrets.token_hex(8)
    now_iso = now.isoformat()
    until = (now + timedelta(seconds=seconds)).isoformat()
    cur = conn.execute(
        """UPDATE hevy_workouts SET locked_until = ?, lease_owner = ?
           WHERE hevy_id = ? AND (locked_until IS NULL OR locked_until <= ?)""",
        (until, token, hevy_id, now_iso),
    )
    conn.commit()
    return token if cur.rowcount == 1 else None


def release_lease(conn: sqlite3.Connection, hevy_id: str, token: str) -> None:
    conn.execute(
        """UPDATE hevy_workouts SET locked_until = NULL, lease_owner = NULL
           WHERE hevy_id = ? AND lease_owner = ?""",
        (hevy_id, token),
    )
    conn.commit()


def set_applied(
    conn: sqlite3.Connection, hevy_id: str, side: str, updated_at: str
) -> None:
    if side not in ("garmin", "strava"):
        raise ValueError(f"unknown applied side: {side!r}")
    column = f"{side}_applied_updated_at"
    conn.execute(
        f"UPDATE hevy_workouts SET {column} = ?, updated_at = ? WHERE hevy_id = ?",
        (updated_at, _now_iso(), hevy_id),
    )
    conn.commit()


# -- hevy_operations --------------------------------------------------------


def open_operation(
    conn: sqlite3.Connection,
    hevy_id: str,
    kind: str,
    source_activity_id: int | None,
    pre_upload_ids: list,
) -> int | None:
    """Open a journal row. Returns None when an open operation already exists
    for this workout or this source activity (partial unique indexes)."""
    now = _now_iso()
    try:
        cur = conn.execute(
            """INSERT INTO hevy_operations
               (hevy_id, kind, phase, source_activity_id, pre_upload_ids,
                created_at, updated_at)
               VALUES (?, ?, 'preparing', ?, ?, ?, ?)""",
            (hevy_id, kind, source_activity_id, json.dumps(pre_upload_ids), now, now),
        )
        conn.commit()
        return cur.lastrowid
    except sqlite3.IntegrityError:
        conn.rollback()
        return None


def _decode_operation(row: sqlite3.Row) -> dict:
    op = dict(row)
    try:
        op["pre_upload_ids"] = json.loads(op["pre_upload_ids"] or "[]")
    except (TypeError, ValueError):
        op["pre_upload_ids"] = []
    return op


def get_open_operation(conn: sqlite3.Connection, hevy_id: str) -> dict | None:
    row = conn.execute(
        """SELECT * FROM hevy_operations
           WHERE hevy_id = ? AND phase NOT IN ('done','failed')""",
        (hevy_id,),
    ).fetchone()
    return _decode_operation(row) if row else None


_OPERATION_FIELDS = {
    "phase", "next_step", "source_activity_id", "target_activity_id",
    "upload_id", "pre_upload_ids", "attempt_count", "delete_attempt_count",
    "last_error", "locked_until",
}


def update_operation(conn: sqlite3.Connection, op_id: int, **fields) -> None:
    unknown = set(fields) - _OPERATION_FIELDS
    if unknown:
        raise ValueError(f"unknown operation fields: {sorted(unknown)}")
    if not fields:
        return
    if "pre_upload_ids" in fields and not isinstance(fields["pre_upload_ids"], str):
        fields = {**fields, "pre_upload_ids": json.dumps(fields["pre_upload_ids"])}
    assignments = ", ".join(f"{name} = ?" for name in fields)
    values = list(fields.values())
    conn.execute(
        f"UPDATE hevy_operations SET {assignments}, updated_at = ? WHERE id = ?",
        (*values, _now_iso(), op_id),
    )
    conn.commit()


def close_operation(conn: sqlite3.Connection, op_id: int, phase: str) -> None:
    conn.execute(
        "UPDATE hevy_operations SET phase = ?, updated_at = ? WHERE id = ?",
        (phase, _now_iso(), op_id),
    )
    conn.commit()


def set_operation_outcome(
    conn: sqlite3.Connection,
    op_id: int,
    hevy_id: str,
    phase: str,
    status: str,
    error: str | None,
) -> None:
    """Atomically park or fail an operation and its workout.

    Closing the operation before updating the workout can leave a closed
    journal behind an actionable ``syncing`` row if the process dies between
    commits. That row would be eligible to open and submit a second operation.
    Both records therefore transition together or neither does.
    """
    if phase not in ("failed", "needs_review"):
        raise ValueError(f"invalid operation outcome phase: {phase!r}")
    if status not in STATUSES:
        raise ValueError(f"invalid workout status: {status!r}")
    now = _now_iso()
    try:
        op_cur = conn.execute(
            """UPDATE hevy_operations SET phase = ?, last_error = ?, updated_at = ?
               WHERE id = ? AND hevy_id = ?""",
            (phase, error, now, op_id, hevy_id),
        )
        workout_cur = conn.execute(
            """UPDATE hevy_workouts SET status = ?, error = ?, updated_at = ?
               WHERE hevy_id = ?""",
            (status, error, now, hevy_id),
        )
        if op_cur.rowcount != 1 or workout_cur.rowcount != 1:
            raise ValueError(
                f"operation outcome target missing: op={op_id}, hevy={hevy_id!r}"
            )
        conn.commit()
    except Exception:
        conn.rollback()
        raise


def complete_operation(
    conn: sqlite3.Connection,
    op_id: int,
    hevy_id: str,
    target_activity_id: int,
    applied_strategy: str,
    status: str,
    applied_updated_at: str | None,
) -> None:
    """The terminal transition, in ONE transaction: close the operation, link
    the target, set the terminal status, and record the applied revision.
    Split across separate commits, a crash in between would leave a closed
    journal with an unlinked replacement — a later tick would upload again."""
    now = _now_iso()
    try:
        conn.execute(
            "UPDATE hevy_operations SET phase = 'done', updated_at = ? WHERE id = ?",
            (now, op_id),
        )
        conn.execute(
            """UPDATE hevy_workouts
               SET garmin_activity_id = ?,
                   applied_strategy = COALESCE(applied_strategy, ?),
                   status = ?, error = NULL,
                   garmin_applied_updated_at = ?, updated_at = ?
               WHERE hevy_id = ?""",
            (target_activity_id, applied_strategy, status, applied_updated_at,
             now, hevy_id),
        )
        conn.commit()
    except Exception:
        conn.rollback()
        raise


def wake_needs_mapping(
    conn: sqlite3.Connection,
    *,
    template_id: str | None = None,
    hevy_id: str | None = None,
) -> int:
    """Wake only mapping rows affected by one concrete change.

    A saved mapping targets workouts containing that template; a newer Hevy
    revision targets its own workout. Waking every parked row would also retry
    unrelated Garmin-rejected pairs whose mappings have not changed.
    """
    if (template_id is None) == (hevy_id is None):
        raise ValueError("provide exactly one of template_id or hevy_id")

    affected: list[str] = []
    for row in list_workouts(conn, status="needs_mapping"):
        if hevy_id is not None:
            if row["hevy_id"] == hevy_id:
                affected.append(row["hevy_id"])
            continue
        try:
            payload = json.loads(row["payload"] or "{}")
        except (TypeError, ValueError):
            continue
        exercises = payload.get("exercises", []) if isinstance(payload, dict) else []
        if any(
            isinstance(exercise, dict)
            and exercise.get("exercise_template_id") == template_id
            for exercise in exercises
        ):
            affected.append(row["hevy_id"])

    if not affected:
        return 0
    now = _now_iso()
    conn.executemany(
        """UPDATE hevy_workouts SET status = 'waiting_watch', error = NULL,
               updated_at = ?
           WHERE hevy_id = ? AND status = 'needs_mapping'""",
        [(now, workout_id) for workout_id in affected],
    )
    conn.commit()
    return len(affected)


# -- merge_backups ----------------------------------------------------------


def save_backup(
    conn: sqlite3.Connection,
    garmin_activity_id: int,
    hevy_id: str | None,
    original_sets: dict,
    original_fit: bytes | None,
) -> None:
    """Persist the pre-destruction snapshot. INSERT OR IGNORE — the first
    backup is the truth; later calls must never overwrite it."""
    conn.execute(
        """INSERT OR IGNORE INTO merge_backups
           (garmin_activity_id, hevy_id, original_sets, original_fit, created_at)
           VALUES (?, ?, ?, ?, ?)""",
        (garmin_activity_id, hevy_id, json.dumps(original_sets), original_fit,
         _now_iso()),
    )
    conn.commit()


def get_backup(conn: sqlite3.Connection, garmin_activity_id: int) -> dict | None:
    row = conn.execute(
        "SELECT * FROM merge_backups WHERE garmin_activity_id = ?",
        (garmin_activity_id,),
    ).fetchone()
    return dict(row) if row else None


# -- exercise_templates -----------------------------------------------------


def upsert_template(conn: sqlite3.Connection, template: dict) -> None:
    conn.execute(
        """INSERT INTO exercise_templates
           (exercise_template_id, title, primary_muscle_group,
            secondary_muscle_groups, equipment_category, is_custom, fetched_at)
           VALUES (?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT(exercise_template_id) DO UPDATE SET
               title = excluded.title,
               primary_muscle_group = excluded.primary_muscle_group,
               secondary_muscle_groups = excluded.secondary_muscle_groups,
               equipment_category = excluded.equipment_category,
               is_custom = excluded.is_custom,
               fetched_at = excluded.fetched_at""",
        (
            template["exercise_template_id"],
            template.get("title", ""),
            template.get("primary_muscle_group"),
            json.dumps(template.get("secondary_muscle_groups", [])),
            template.get("equipment_category"),
            1 if template.get("is_custom") else 0,
            _now_iso(),
        ),
    )
    conn.commit()


def list_templates(conn: sqlite3.Connection, only_custom: bool = False) -> list[dict]:
    query = "SELECT * FROM exercise_templates"
    if only_custom:
        query += " WHERE is_custom = 1"
    query += " ORDER BY title"
    return [dict(row) for row in conn.execute(query).fetchall()]


def get_template(conn: sqlite3.Connection, template_id: str) -> dict | None:
    row = conn.execute(
        "SELECT * FROM exercise_templates WHERE exercise_template_id = ?",
        (template_id,),
    ).fetchone()
    return dict(row) if row else None


# -- exercise_mappings ------------------------------------------------------


def save_mapping(
    conn: sqlite3.Connection, template_id: str, category: int, subcategory: int
) -> None:
    """Create or replace a user mapping. Saving clears garmin_rejected — the
    user has chosen a new pair to try."""
    now = _now_iso()
    conn.execute(
        """INSERT INTO exercise_mappings
           (exercise_template_id, category, subcategory, garmin_rejected,
            created_at, updated_at)
           VALUES (?, ?, ?, 0, ?, ?)
           ON CONFLICT(exercise_template_id) DO UPDATE SET
               category = excluded.category,
               subcategory = excluded.subcategory,
               garmin_rejected = 0,
               updated_at = excluded.updated_at""",
        (template_id, category, subcategory, now, now),
    )
    conn.commit()


def get_mapping(conn: sqlite3.Connection, template_id: str) -> dict | None:
    row = conn.execute(
        "SELECT * FROM exercise_mappings WHERE exercise_template_id = ?",
        (template_id,),
    ).fetchone()
    return dict(row) if row else None


def list_mappings(conn: sqlite3.Connection) -> list[dict]:
    rows = conn.execute(
        "SELECT * FROM exercise_mappings ORDER BY exercise_template_id"
    ).fetchall()
    return [dict(row) for row in rows]


def delete_mapping(conn: sqlite3.Connection, template_id: str) -> None:
    conn.execute(
        "DELETE FROM exercise_mappings WHERE exercise_template_id = ?", (template_id,)
    )
    conn.commit()


def mark_mapping_rejected(conn: sqlite3.Connection, template_id: str) -> None:
    conn.execute(
        "UPDATE exercise_mappings SET garmin_rejected = 1, updated_at = ? "
        "WHERE exercise_template_id = ?",
        (_now_iso(), template_id),
    )
    conn.commit()


# -- hevy_events_seen -------------------------------------------------------


def record_event_seen(
    conn: sqlite3.Connection, event_type: str, workout_id: str, event_timestamp: str
) -> bool:
    """Record an event's dedupe key. Returns False when it was already seen —
    the safety-lag overlap re-delivers events, and this absorbs them."""
    cur = conn.execute(
        """INSERT OR IGNORE INTO hevy_events_seen
           (event_type, workout_id, event_timestamp) VALUES (?, ?, ?)""",
        (event_type, workout_id, event_timestamp),
    )
    conn.commit()
    return cur.rowcount == 1
