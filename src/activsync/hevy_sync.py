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
from activsync.garmin_client import ActivityGone, GarminClient
from activsync.hevy_apply import (
    _parse_ts,
    _activity_window,
    _apply_metadata,
    _payload_of,
    advance_operation,
    build_exercise_sets_payload,
    execute_describe,
    execute_merge,
    execute_passive,
    execute_replace,
    generate_description,
    resolve_exercises,
)
from activsync.hevy_client import HevyAuthError, HevyClient
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


def _is_published_activity(activity: dict | None) -> bool:
    return bool(
        activity
        and activity.get("publish_status") == "published"
        and activity.get("strava_activity_id") is not None
    )


# -- event ingestion --------------------------------------------------------


def _event_key(event: dict) -> tuple[str, str, str]:
    """(event_type, workout_id, timestamp) — the dedupe key."""
    if event["type"] == "deleted":
        return ("deleted", event["id"], event["deleted_at"])
    workout = event["workout"]
    return ("updated", workout["id"], workout["updated_at"])


def _apply_updated(conn: sqlite3.Connection, event: dict) -> None:
    workout = event["workout"]
    before = hevy_db.get_workout(conn, workout["id"])
    hevy_db.upsert_workout(
        conn,
        workout["id"],
        workout.get("title", ""),
        workout.get("start_time", ""),
        workout.get("end_time", ""),
        workout["updated_at"],
        workout,
    )
    after = hevy_db.get_workout(conn, workout["id"])
    # A Hevy edit can itself remove or replace the exercise that required a
    # mapping. Wake this workout when this event is its current revision. The
    # equality also makes a crash after upsert but before wake safe on replay.
    if (
        before is not None
        and before["status"] == "needs_mapping"
        and after is not None
        and after["source_updated_at"] == workout["updated_at"]
    ):
        hevy_db.wake_needs_mapping(conn, hevy_id=workout["id"])


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
                # Processed on an earlier poll (possibly one that crashed
                # before persisting the cursor) — still move the cursor past
                # it, or it would lag until an unrelated newer event arrives.
                if max_processed is None or ts > max_processed:
                    max_processed = ts
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
            except HevyAuthError:
                raise  # the poller must see auth failures and pause the leg
            except Exception as exc:
                logger.warning("template fetch failed for %s: %s", template_id, exc)
                template = None
            # A transient fetch failure must not leave a parked workout with
            # no actionable mapping row. The payload still gives us the stable
            # id/title; a later catalog refresh enriches this placeholder.
            template = template or {"id": template_id, "title": miss.title}
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


# -- per-workout decision flow -----------------------------------------------


def process_workout(conn: sqlite3.Connection, garmin: GarminClient,
                    hevy: HevyClient, row: dict, cfg: dict,
                    now: datetime) -> None:
    hevy_id = row["hevy_id"]
    token = hevy_db.acquire_lease(conn, hevy_id, now)
    if not token:
        return
    try:
        if (
            row["status"] == "awaiting_match"
            and (
                cfg.get("hevy_match_mode", "automatic") == "review"
                or _is_published_activity(
                    db.get_activity(conn, row["source_garmin_activity_id"])
                    if row.get("source_garmin_activity_id")
                    else None
                )
            )
        ):
            return
        op = hevy_db.get_open_operation(conn, hevy_id)
        if op is not None:
            advance_operation(conn, garmin, row, op, cfg)
            return

        strategy = cfg.get("hevy_watch_strategy", "merge")
        match = find_watch_match(conn, row)
        if isinstance(match, int):
            if not hevy_db.claim_source(conn, hevy_id, match):
                hevy_db.set_workout_status(
                    conn, hevy_id, "needs_review",
                    error=f"watch activity {match} claim conflict")
                return
            row = hevy_db.get_workout(conn, hevy_id)
            matched_activity = db.get_activity(conn, match)
            if _is_published_activity(matched_activity):
                hevy_db.set_workout_status(
                    conn,
                    hevy_id,
                    "awaiting_match",
                    error=(
                        "Already published to Strava. Choose Merge or "
                        "Description only."
                    ),
                )
                return
            if cfg.get("hevy_match_mode", "automatic") == "review":
                hevy_db.set_workout_status(
                    conn,
                    hevy_id,
                    "awaiting_match",
                    error="Choose how to apply this Hevy workout.",
                )
                return
            if not apply_mapping_gate(conn, row, strategy, hevy):
                return
            if strategy == "describe":
                execute_describe(conn, garmin, row, cfg)
            elif strategy == "replace":
                execute_replace(conn, garmin, row, cfg)
            else:
                execute_merge(conn, garmin, row, cfg)
        elif match == "multiple":
            hevy_db.set_workout_status(conn, hevy_id, "needs_review",
                                       error="multiple matching watch activities")
        elif match == "claimed":
            hevy_db.set_workout_status(
                conn, hevy_id, "needs_review",
                error="matching watch activity already claimed by another workout")
        else:
            grace_min = int(cfg.get("hevy_grace_minutes", GRACE_DEFAULT_MIN))
            end_dt = _parse_ts(row["end_time"])
            if end_dt and now - end_dt < timedelta(minutes=grace_min):
                hevy_db.set_workout_status(conn, hevy_id, "waiting_watch")
            else:
                # The passive path pushes structured sets, so it is gated even
                # when the configured strategy (e.g. describe) is not.
                if not apply_mapping_gate(conn, row, "passive", hevy):
                    return
                execute_passive(conn, garmin, row, cfg)
    except HevyAuthError:
        raise
    except Exception as exc:
        logger.exception("processing hevy workout %s failed", hevy_id)
        status = "syncing" if hevy_db.get_open_operation(conn, hevy_id) else "failed"
        hevy_db.set_workout_status(conn, hevy_id, status, error=str(exc))
    finally:
        hevy_db.release_lease(conn, hevy_id, token)


def apply_match_choice(
    conn: sqlite3.Connection,
    garmin: GarminClient,
    hevy: HevyClient,
    hevy_id: str,
    strategy: str,
    cfg: dict,
    now: datetime,
) -> dict:
    """Apply one explicit choice to a durably claimed watch match."""
    if strategy not in ("merge", "replace", "describe"):
        raise ValueError(f"unsupported Hevy match strategy: {strategy}")
    token = hevy_db.acquire_lease(conn, hevy_id, now)
    if not token:
        raise RuntimeError("Workout is being processed right now.")
    try:
        row = hevy_db.get_workout(conn, hevy_id)
        if row is None:
            raise LookupError("Unknown Hevy workout.")
        if row["status"] != "awaiting_match" or not row["source_garmin_activity_id"]:
            raise ValueError("This workout is not waiting for a match decision.")
        if hevy_db.get_open_operation(conn, hevy_id) is not None:
            raise ValueError("This workout already has an operation in progress.")
        source_activity = db.get_activity(conn, row["source_garmin_activity_id"])
        if strategy == "replace" and _is_published_activity(source_activity):
            raise ValueError(
                "Replace is unavailable because this activity is already on Strava. "
                "Choose Merge or Description only."
            )
        if not apply_mapping_gate(conn, row, strategy, hevy):
            return hevy_db.get_workout(conn, hevy_id)
        if strategy == "describe":
            execute_describe(conn, garmin, row, cfg)
        elif strategy == "replace":
            execute_replace(conn, garmin, row, cfg)
        else:
            execute_merge(conn, garmin, row, cfg)
        return hevy_db.get_workout(conn, hevy_id)
    except HevyAuthError:
        raise
    except Exception as exc:
        logger.exception("applying Hevy match choice for %s failed", hevy_id)
        row = hevy_db.get_workout(conn, hevy_id)
        status = "syncing" if hevy_db.get_open_operation(conn, hevy_id) else "failed"
        if row is not None and row["status"] == "awaiting_match":
            status = "awaiting_match"
        hevy_db.set_workout_status(conn, hevy_id, status, error=str(exc))
        raise
    finally:
        hevy_db.release_lease(conn, hevy_id, token)


# -- post-sync edits ---------------------------------------------------------

_TERMINAL_APPLIED = ("merged", "described", "replaced", "uploaded_passive")


def _reapply(conn: sqlite3.Connection, garmin: GarminClient, hevy: HevyClient,
             row: dict, cfg: dict, now: datetime) -> None:
    """A newer Hevy revision for an already-applied workout: re-apply
    following the ORIGINALLY applied strategy, never the current setting."""
    hevy_id = row["hevy_id"]
    token = hevy_db.acquire_lease(conn, hevy_id, now)
    if not token:
        return
    try:
        if not apply_mapping_gate(
            conn, row, row["applied_strategy"] or "merge", hevy
        ):
            return
        target = row["garmin_activity_id"]
        if row["applied_strategy"] != "describe":
            try:
                resolved = resolve_exercises(conn, _payload_of(row))
            except MappingMiss as miss:
                hevy_db.set_workout_status(
                    conn, hevy_id, "needs_mapping",
                    error=f"unmapped exercises: {miss.title}")
                return
            start, duration = _activity_window(conn, target, row)
            payload = {**build_exercise_sets_payload(resolved, start, duration),
                       "activityId": target}
            garmin.put_exercise_sets(target, payload)
        strategy = row["applied_strategy"] or "merge"
        _apply_metadata(
            garmin,
            target,
            row,
            cfg,
            write_summary=(
                strategy == "describe"
                or strategy == "passive"
                or bool(cfg.get("hevy_summary_on_structured", True))
            ),
        )
        hevy_db.set_applied(conn, hevy_id, "garmin", row["source_updated_at"])
    except ActivityGone:
        hevy_db.set_workout_status(
            conn, hevy_id, "needs_review",
            error="linked activity deleted on Garmin — re-sync as fresh upload")
    except Exception as exc:
        logger.exception("re-applying hevy workout %s failed", hevy_id)
        hevy_db.set_workout_status(conn, hevy_id, row["status"], error=str(exc))
    finally:
        hevy_db.release_lease(conn, hevy_id, token)


# -- the leg -----------------------------------------------------------------

# failed and needs_mapping are deliberately NOT here: a definite rejection
# must not re-upload every tick. failed re-enters via the user's explicit retry
# (Task 13); needs_mapping via hevy_db.wake_needs_mapping when a mapping is saved.
_ACTIONABLE = ("waiting_watch", "awaiting_match", "syncing")


def _row_fingerprint(row: dict | None) -> tuple:
    if row is None:
        return ()
    return (row["status"], row["garmin_activity_id"],
            row["garmin_applied_updated_at"], row["error"])


def run_hevy_leg(conn: sqlite3.Connection, garmin: GarminClient,
                 hevy: HevyClient, cfg: dict, now: datetime,
                 strava=None) -> bool:
    """One tick of the Hevy leg. Returns whether anything changed (drives the
    poller's Garmin-leg handoff and the SSE refresh)."""
    changed = ingest_events(conn, hevy, now) > 0

    for status in _ACTIONABLE:
        for row in hevy_db.list_workouts(conn, status=status):
            before = _row_fingerprint(row)
            process_workout(conn, garmin, hevy, row, cfg, now)
            if _row_fingerprint(hevy_db.get_workout(conn, row["hevy_id"])) != before:
                changed = True

    for status in _TERMINAL_APPLIED:
        for row in hevy_db.list_workouts(conn, status=status):
            applied = row["garmin_applied_updated_at"] or ""
            if row["source_updated_at"] and row["source_updated_at"] > applied:
                _reapply(conn, garmin, hevy, row, cfg, now)
                changed = True

    if strava is not None:
        changed = _strava_catchup(conn, strava, cfg) or changed
    return changed


def _strava_catchup(conn: sqlite3.Connection, strava, cfg: dict) -> bool:
    """Push edited titles/descriptions to already-published Strava copies.

    The Garmin and Strava sides are applied separately: the Garmin update can
    succeed while Strava is down, so this retries every tick until the Strava
    side has caught up to the applied Garmin revision."""
    changed = False
    for status in _TERMINAL_APPLIED:
        for row in hevy_db.list_workouts(conn, status=status):
            garmin_applied = row["garmin_applied_updated_at"] or ""
            strava_applied = row["strava_applied_updated_at"] or ""
            if not garmin_applied or garmin_applied <= strava_applied:
                continue
            activity = (db.get_activity(conn, row["garmin_activity_id"])
                        if row["garmin_activity_id"] else None)
            if not activity or not activity.get("strava_activity_id"):
                continue  # not published yet — publish carries current content
            payload = _payload_of(row)
            strategy = row["applied_strategy"] or "merge"
            write_summary = (
                strategy in ("describe", "passive")
                or bool(cfg.get("hevy_summary_on_structured", True))
            )
            try:
                strava.update_activity_metadata(
                    activity["strava_activity_id"],
                    row["title"] or payload.get("title", "Workout"),
                    (
                        generate_description(
                            payload,
                            template=cfg.get("hevy_description_template"),
                        )
                        if write_summary
                        else activity.get("description", "")
                    ))
            except Exception as exc:
                logger.warning("strava catch-up failed for %s: %s",
                               row["hevy_id"], exc)
                continue
            hevy_db.set_applied(conn, row["hevy_id"], "strava", garmin_applied)
            changed = True
    return changed
