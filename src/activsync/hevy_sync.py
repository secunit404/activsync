"""Hevy sync engine, part 1: event ingestion, mapping gate, watch matching.

The events cursor algorithm (spec §Events cursor): poll with a safety lag,
process events oldest-first (deletion winning equal timestamps), apply each
effect BEFORE recording its dedupe key, and advance the cursor only past
successfully processed events. Effects are idempotent, so a crash anywhere is
re-absorbed on the next poll; a failed effect leaves the cursor behind it.
"""

from __future__ import annotations

import json
import logging
import sqlite3
from datetime import datetime, timedelta, timezone

from activsync import db, hevy_db
from activsync.fit_builder import ResolvedExercise
from activsync.hevy_client import HevyClient
from activsync.hevy_mapper import MappingMiss, lookup_exercise

logger = logging.getLogger("activsync.hevy_sync")

CURSOR_KEY = "hevy_events_cursor"
CURSOR_LAG = timedelta(minutes=5)

OVERLAP_MIN = 0.70
DRIFT_MAX_MIN = 20
GRACE_DEFAULT_MIN = 120

# Statuses meaning "this workout has landed on Garmin (or is mid-flight)":
# a Hevy deletion for one of these is surfaced for review, never auto-applied.
_SYNCED_STATUSES = {"merged", "described", "replaced", "uploaded_passive",
                    "linked_existing", "syncing", "needs_review"}


def _parse_ts(raw: str | None) -> datetime | None:
    """ISO-8601 (with T/Z) or Garmin space-separated timestamp → UTC."""
    if not raw or not isinstance(raw, str):
        return None
    cleaned = raw.strip()
    try:
        if "T" in cleaned:
            parsed = datetime.fromisoformat(cleaned.replace("Z", "+00:00"))
            if parsed.tzinfo is None:
                parsed = parsed.replace(tzinfo=timezone.utc)
            return parsed.astimezone(timezone.utc)
        return datetime.strptime(cleaned, "%Y-%m-%d %H:%M:%S").replace(
            tzinfo=timezone.utc)
    except (ValueError, TypeError):
        return None


# -- event ingestion --------------------------------------------------------


def _event_key(event: dict) -> tuple[str, str, str]:
    """(event_type, workout_id, timestamp) — the dedupe key."""
    if event["type"] == "deleted":
        return ("deleted", event["id"], event["deleted_at"])
    workout = event["workout"]
    return ("updated", workout["id"], workout["updated_at"])


def _apply_updated(conn: sqlite3.Connection, event: dict) -> None:
    workout = event["workout"]
    hevy_db.upsert_workout(
        conn,
        workout["id"],
        workout.get("title", ""),
        workout.get("start_time", ""),
        workout.get("end_time", ""),
        workout["updated_at"],
        workout,
    )


def _apply_deleted(conn: sqlite3.Connection, event: dict) -> None:
    row = hevy_db.get_workout(conn, event["id"])
    if row is None:
        return
    if row["status"] in _SYNCED_STATUSES:
        # Never auto-delete on Garmin; mark and surface (spec §lifecycle).
        hevy_db.set_workout_status(conn, row["hevy_id"], "needs_review",
                                   error="deleted in Hevy")
    else:
        hevy_db.delete_workout(conn, row["hevy_id"])


def ingest_events(conn: sqlite3.Connection, hevy: HevyClient, now: datetime) -> int:
    """Poll Hevy's events feed and apply new events. Returns how many were new.

    A missing cursor initializes to `now` (forward-only — history is never
    re-processed; backfill is an explicit, separate action)."""
    cursor = db.get_config_value(conn, CURSOR_KEY)
    if not cursor:
        db.set_config_value(conn, CURSOR_KEY, now.isoformat())
        return 0

    cursor_dt = _parse_ts(cursor)
    since = (cursor_dt - CURSOR_LAG).isoformat() if cursor_dt else cursor
    events = hevy.iter_events_since(since)

    # Oldest-first so state converges to the newest event; at equal timestamps
    # the deletion applies last (an old update must never resurrect a newer
    # deletion, whatever order Hevy delivered the page in).
    def sort_key(event: dict):
        event_type, _workout_id, ts = _event_key(event)
        return (ts, 1 if event_type == "deleted" else 0)

    new_count = 0
    max_processed = cursor
    try:
        for event in sorted(events, key=sort_key):
            event_type, workout_id, ts = _event_key(event)
            already = conn.execute(
                "SELECT 1 FROM hevy_events_seen WHERE event_type = ? "
                "AND workout_id = ? AND event_timestamp = ?",
                (event_type, workout_id, ts),
            ).fetchone()
            if already:
                continue
            # Effect first, dedupe record second: a crash in between re-runs
            # the idempotent effect next poll. The reverse order would burn
            # the event while its effect never happened.
            if event_type == "updated":
                _apply_updated(conn, event)
            else:
                _apply_deleted(conn, event)
            hevy_db.record_event_seen(conn, event_type, workout_id, ts)
            new_count += 1
            if max_processed is None or ts > max_processed:
                max_processed = ts
    finally:
        # Advance only past what actually landed — never past a failure.
        if max_processed and max_processed != cursor:
            db.set_config_value(conn, CURSOR_KEY, max_processed)
    return new_count


# -- mapping gate -----------------------------------------------------------


def resolve_exercises(
    conn: sqlite3.Connection, workout_payload: dict
) -> list[ResolvedExercise]:
    """Resolve every exercise or raise MappingMiss on the first unmapped one."""
    resolved: list[ResolvedExercise] = []
    for exercise in workout_payload.get("exercises", []):
        category, subcategory, title = lookup_exercise(
            conn, exercise.get("title", ""), exercise.get("exercise_template_id"))
        resolved.append(ResolvedExercise(
            title=title, category=category, subcategory=subcategory,
            sets=exercise.get("sets", [])))
    return resolved


def apply_mapping_gate(
    conn: sqlite3.Connection, row: dict, strategy: str, hevy: HevyClient
) -> bool:
    """True when the workout may proceed under `strategy`. Strategies that
    push structured sets (merge/replace/passive) require every exercise
    mapped; a miss parks the workout as needs_mapping listing ALL missing
    titles and pulls unknown templates into the cache for the mappings UI.
    describe uses Hevy's own titles and always passes."""
    if strategy == "describe":
        return True

    payload = row["payload"]
    if isinstance(payload, str):
        payload = json.loads(payload)

    misses: list[MappingMiss] = []
    for exercise in payload.get("exercises", []):
        try:
            lookup_exercise(conn, exercise.get("title", ""),
                            exercise.get("exercise_template_id"))
        except MappingMiss as miss:
            misses.append(miss)
    if not misses:
        return True

    for miss in misses:
        template_id = miss.template_id
        if template_id and hevy_db.get_template(conn, template_id) is None:
            try:
                template = hevy.get_exercise_template(template_id)
            except Exception as exc:
                logger.warning("template fetch failed for %s: %s", template_id, exc)
                template = None
            if template:
                hevy_db.upsert_template(conn, {
                    "exercise_template_id": template.get("id", template_id),
                    "title": template.get("title", miss.title),
                    "primary_muscle_group": template.get("primary_muscle_group"),
                    "secondary_muscle_groups": template.get("secondary_muscle_groups", []),
                    "equipment_category": template.get("equipment_category")
                                          or template.get("equipment"),
                    "is_custom": template.get("is_custom", False),
                })

    titles = ", ".join(miss.title for miss in misses)
    hevy_db.set_workout_status(conn, row["hevy_id"], "needs_mapping",
                               error=f"unmapped exercises: {titles}")
    return False


# -- watch matching ---------------------------------------------------------


def find_watch_match(conn: sqlite3.Connection, row: dict):
    """Match the workout to a local watch activity (zero Garmin calls).

    Returns the activity id for exactly one unclaimed candidate, "multiple"
    for several, "claimed" when candidates exist but other workouts own them
    all, None for no candidate. A candidate this row already claimed counts
    as its own match (idempotent re-runs)."""
    hevy_start = _parse_ts(row["start_time"])
    hevy_end = _parse_ts(row["end_time"])
    if not hevy_start or not hevy_end or hevy_end <= hevy_start:
        return None
    hevy_duration = (hevy_end - hevy_start).total_seconds()

    claims: dict[int, str] = {}
    for claim_row in conn.execute(
        "SELECT hevy_id, source_garmin_activity_id, garmin_activity_id "
        "FROM hevy_workouts"
    ):
        for column in ("source_garmin_activity_id", "garmin_activity_id"):
            if claim_row[column] is not None:
                claims[claim_row[column]] = claim_row["hevy_id"]

    unclaimed: list[int] = []
    claimed_by_others = 0
    for activity in db.list_activities(conn):
        if activity["activity_type"] not in ("strength_training", "other"):
            continue
        act_start = _parse_ts(activity["start_time"])
        if act_start is None:
            continue
        try:
            duration = float(json.loads(activity["garmin_data"]).get("duration") or 0)
        except (ValueError, TypeError):
            duration = 0.0
        act_end = act_start + timedelta(seconds=duration)

        overlap_s = (min(hevy_end, act_end) - max(hevy_start, act_start)).total_seconds()
        if overlap_s <= 0 or overlap_s / hevy_duration < OVERLAP_MIN:
            continue
        drift_min = abs((act_start - hevy_start).total_seconds()) / 60
        if drift_min > DRIFT_MAX_MIN:
            continue

        activity_id = activity["garmin_activity_id"]
        owner = claims.get(activity_id)
        if owner is None or owner == row["hevy_id"]:
            unclaimed.append(activity_id)
        else:
            claimed_by_others += 1

    if len(unclaimed) == 1:
        return unclaimed[0]
    if len(unclaimed) > 1:
        return "multiple"
    if claimed_by_others:
        return "claimed"
    return None
