"""Shared Hevy history preview and ingestion logic."""

from __future__ import annotations

import logging
import sqlite3
from datetime import datetime, timedelta, timezone

from activsync import config, db, hevy_db, hevy_sync
from activsync.hevy_apply import _parse_ts
from activsync.hevy_mapper import MappingMiss, lookup_exercise

logger = logging.getLogger("activsync.hevy_backfill")


def _is_published(activity: dict | None) -> bool:
    return bool(
        activity
        and activity.get("publish_status") == "published"
        and activity.get("strava_activity_id") is not None
    )


def parse_since(raw: str) -> datetime | None:
    try:
        parsed = datetime.fromisoformat(raw.strip())
    except (ValueError, TypeError):
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def workouts_since(client, since_dt: datetime) -> list[dict]:
    """Page through newest-first Hevy workouts to the requested boundary."""
    collected: list[dict] = []
    page = 1
    while True:
        data = client.get_workouts_page(page, page_size=10)
        workouts = data.get("workouts", []) or []
        older_seen = False
        for workout in workouts:
            start = _parse_ts(workout.get("start_time"))
            if start is None:
                continue
            if start < since_dt:
                older_seen = True
                continue
            collected.append(workout)
        page_count = data.get("page_count", page)
        if older_seen or page >= page_count or not workouts:
            break
        page += 1
    return collected


def twin_activity(conn: sqlite3.Connection, workout: dict) -> dict | None:
    """Return an existing strength activity at exactly the workout start."""
    start = _parse_ts(workout.get("start_time"))
    if start is None:
        return None
    for activity in db.list_activities(conn):
        if activity["activity_type"] not in ("strength_training", "other"):
            continue
        if _parse_ts(activity["start_time"]) == start:
            return activity
    return None


def preview_items(conn: sqlite3.Connection, client, since_dt: datetime) -> list[dict]:
    """Describe the backfill plan without writing any state."""
    cfg = config.load_config(conn)
    now = datetime.now(timezone.utc)
    items: list[dict] = []
    for workout in workouts_since(client, since_dt):
        has_mapping_miss = False
        missing_ids: list[str] = []
        hevy_id = workout.get("id")
        tracked = hevy_db.get_workout(conn, hevy_id) if hevy_id else None
        if tracked is not None and tracked["status"] != "skipped":
            # Once imported, the queue is the source of truth for this workout.
            # A deliberate queue Skip releases it back to this picker.
            continue
        twin = twin_activity(conn, workout)
        if twin is not None:
            owner = conn.execute(
                """SELECT hevy_id FROM hevy_workouts
                   WHERE source_garmin_activity_id = ?
                      OR garmin_activity_id = ?
                   LIMIT 1""",
                (twin["garmin_activity_id"], twin["garmin_activity_id"]),
            ).fetchone()
            action = (
                "needs_review"
                if owner
                else (
                    "awaiting_match"
                    if _is_published(twin)
                    or cfg.get("hevy_match_mode") == "review"
                    else cfg["hevy_watch_strategy"]
                )
            )
        else:
            strategy = cfg["hevy_watch_strategy"]
            exercises = workout.get("exercises", []) or []

            def collect_mapping_misses() -> tuple[bool, list[str]]:
                """Whether any exercise failed to resolve, and which distinct
                template ids that involved. The two are tracked separately:
                a miss can occur for an exercise with no (falsy) template id,
                in which case the boolean is True but the id list stays
                empty — callers must branch on the boolean, not the list."""
                any_miss = False
                missing: list[str] = []
                for exercise in exercises:
                    try:
                        lookup_exercise(
                            conn,
                            exercise.get("title", ""),
                            exercise.get("exercise_template_id"),
                        )
                    except MappingMiss:
                        any_miss = True
                        template_id = exercise.get("exercise_template_id")
                        if template_id and template_id not in missing:
                            missing.append(template_id)
                return any_miss, missing

            match = hevy_sync.find_watch_match(
                conn,
                {
                    "hevy_id": f"__preview_{workout.get('id')}__",
                    "start_time": workout.get("start_time"),
                    "end_time": workout.get("end_time"),
                },
            )
            if isinstance(match, int):
                twin = db.get_activity(conn, match)
                if _is_published(twin) or cfg.get("hevy_match_mode") == "review":
                    # Review happens before the mapping gate: Description only
                    # remains a valid choice even when structured sets cannot
                    # yet be mapped.
                    action = "awaiting_match"
                else:
                    has_mapping_miss, missing_ids = (
                        (False, [])
                        if strategy == "describe"
                        else collect_mapping_misses()
                    )
                    action = "needs_mapping" if has_mapping_miss else strategy
            elif match in ("multiple", "claimed"):
                action = "needs_review"
            else:
                if strategy == "describe":
                    has_mapping_miss, missing_ids = False, []
                else:
                    has_mapping_miss, missing_ids = collect_mapping_misses()
                if has_mapping_miss:
                    action = "needs_mapping"
                    items.append(
                        {
                            "workout": workout,
                            "twin": None,
                            "action": action,
                            "missing_template_ids": missing_ids,
                        }
                    )
                    continue
                end = _parse_ts(workout.get("end_time"))
                grace = timedelta(minutes=int(cfg["hevy_grace_minutes"]))
                if end is not None and now - end < grace:
                    action = "waiting_watch"
                elif strategy == "describe":
                    has_mapping_miss, missing_ids = collect_mapping_misses()
                    action = "needs_mapping" if has_mapping_miss else "passive"
                else:
                    action = "passive"
        items.append(
            {
                "workout": workout,
                "twin": twin,
                "action": action,
                "missing_template_ids": missing_ids,
            }
        )
    return items


def run_items(conn: sqlite3.Connection, items: list[dict]) -> int:
    """Ingest a preview and claim unique Garmin matches for processing."""
    matched = 0
    for item in items:
        workout = item["workout"]
        hevy_id = workout.get("id")
        if not hevy_id:
            continue
        existing = hevy_db.get_workout(conn, hevy_id)
        if existing is not None and existing["status"] != "skipped":
            continue
        hevy_db.upsert_workout(
            conn,
            hevy_id,
            workout.get("title", ""),
            workout.get("start_time", ""),
            workout.get("end_time", ""),
            workout.get("updated_at", ""),
            workout,
        )
        if item["twin"] is None:
            if existing is not None:
                hevy_db.set_workout_status(conn, hevy_id, "waiting_watch")
            continue
        if item["action"] == "needs_review":
            hevy_db.set_workout_status(
                conn,
                hevy_id,
                "needs_review",
                error="backfill twin is already claimed by another workout",
            )
            continue
        source_id = item["twin"]["garmin_activity_id"]
        if not hevy_db.claim_source(conn, hevy_id, source_id):
            logger.warning("backfill twin for %s already claimed", hevy_id)
            hevy_db.set_workout_status(
                conn,
                hevy_id,
                "needs_review",
                error="backfill match was claimed by another workout",
            )
            continue
        if item["action"] == "awaiting_match":
            published = _is_published(item["twin"])
            hevy_db.set_workout_status(
                conn,
                hevy_id,
                "awaiting_match",
                error=(
                    "Already published to Strava. Choose Merge or Description only."
                    if published
                    else "Choose how to apply this Hevy workout."
                ),
            )
        else:
            # Automatic mode processes the durable source claim on the next
            # Hevy pass; review mode above needs no polling handoff at all.
            hevy_db.set_workout_status(conn, hevy_id, "waiting_watch")
        matched += 1
    return matched
