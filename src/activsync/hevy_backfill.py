"""Shared Hevy history preview and ingestion logic."""

from __future__ import annotations

import logging
import sqlite3
from datetime import datetime, timedelta, timezone

from activsync import config, db, hevy_db, hevy_sync
from activsync.hevy_apply import _parse_ts
from activsync.hevy_mapper import MappingMiss, lookup_exercise

logger = logging.getLogger("activsync.hevy_backfill")


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
        hevy_id = workout.get("id")
        if hevy_id and hevy_db.get_workout(conn, hevy_id) is not None:
            items.append(
                {"workout": workout, "twin": None, "action": "already tracked"}
            )
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
            action = "needs_review" if owner else "linked_existing"
        else:
            strategy = cfg["hevy_watch_strategy"]
            exercises = workout.get("exercises", []) or []

            def has_mapping_miss() -> bool:
                for exercise in exercises:
                    try:
                        lookup_exercise(
                            conn,
                            exercise.get("title", ""),
                            exercise.get("exercise_template_id"),
                        )
                    except MappingMiss:
                        return True
                return False

            missing_mapping = strategy != "describe" and has_mapping_miss()
            match = hevy_sync.find_watch_match(
                conn,
                {
                    "hevy_id": f"__preview_{workout.get('id')}__",
                    "start_time": workout.get("start_time"),
                    "end_time": workout.get("end_time"),
                },
            )
            if missing_mapping:
                action = "needs_mapping"
            elif isinstance(match, int):
                action = strategy
            elif match in ("multiple", "claimed"):
                action = "needs_review"
            else:
                end = _parse_ts(workout.get("end_time"))
                grace = timedelta(minutes=int(cfg["hevy_grace_minutes"]))
                if end is not None and now - end < grace:
                    action = "waiting_watch"
                elif strategy == "describe" and has_mapping_miss():
                    action = "needs_mapping"
                else:
                    action = "passive"
        items.append({"workout": workout, "twin": twin, "action": action})
    return items


def run_items(conn: sqlite3.Connection, items: list[dict]) -> int:
    """Ingest a preview, safely linking exact Garmin twins when unclaimed."""
    linked = 0
    for item in items:
        workout = item["workout"]
        hevy_id = workout.get("id")
        if not hevy_id:
            continue
        if item["action"] == "already tracked" or hevy_db.get_workout(
            conn, hevy_id
        ) is not None:
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
            continue
        if item["action"] != "linked_existing":
            hevy_db.set_workout_status(
                conn,
                hevy_id,
                "needs_review",
                error="backfill twin is already claimed by another workout",
            )
            continue
        try:
            hevy_db.link_target(
                conn,
                hevy_id,
                item["twin"]["garmin_activity_id"],
                applied_strategy="external",
                provenance="backfill",
            )
        except sqlite3.IntegrityError:
            logger.warning("backfill twin for %s already claimed", hevy_id)
            continue
        hevy_db.set_workout_status(conn, hevy_id, "linked_existing")
        linked += 1
    return linked
