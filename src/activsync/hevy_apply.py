"""Hevy sync engine, part 2: strategy execution + operation journal.

Split from hevy_sync.py (file-size cap): the pure "apply this workout to
Garmin" machinery — the exerciseSets payload builder (ported from upstream
merge.py), description generation (ported from upstream garmin.py), the four
strategy executors, and the crash-safe operation state machine. hevy_sync
owns ingestion, matching, and the per-workout decision flow, and re-exports
this module's public names.
"""

from __future__ import annotations

import json
import logging
import sqlite3
from datetime import datetime, timedelta, timezone

from activsync import db, hevy_db, hr_sources
from activsync.fit_builder import (
    Profile,
    ResolvedExercise,
    build_fit,
    identity_from_config,
    identity_from_fit,
    new_fallback_identity,
)
from activsync.garmin_client import (
    ActivityGone,
    GarminClient,
    GarminUploadRejected,
    SubcategoryRejected,
    _is_not_found,
)
from activsync.hevy_mapper import (
    CATEGORY_NAMES,
    UNKNOWN_CATEGORY,
    MappingMiss,
    lookup_exercise,
    subcategory_name,
)

logger = logging.getLogger("activsync.hevy_apply")


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


# -- profile / identity helpers --------------------------------------------

_PROFILE_DEFAULTS = {"weight_kg": 80.0, "birth_year": 1990, "vo2max": 45.0,
                     "sex": "male"}


def _profile_from_cfg(cfg: dict) -> Profile:
    """Profile for calorie estimation: cached Garmin values overridden
    field-by-field by the user's manual override, defaults as last resort.
    (Task 11's hevy_profile refreshes the cache; this only reads.)"""
    merged = dict(_PROFILE_DEFAULTS)
    for source_key in ("garmin_user_profile", "profile_override"):
        stored = cfg.get(source_key) or {}
        for key in _PROFILE_DEFAULTS:
            if stored.get(key) is not None:
                merged[key] = stored[key]
    return Profile(weight_kg=float(merged["weight_kg"]),
                   birth_year=int(merged["birth_year"]),
                   vo2max=float(merged["vo2max"]), sex=str(merged["sex"]))


def _persist_identity(conn: sqlite3.Connection, identity) -> None:
    stored = db.get_config_value(conn, "settings", default={}) or {}
    stored["hevy_device_identity"] = {
        "manufacturer": identity.manufacturer, "product": identity.product,
        "serial": identity.serial}
    db.set_config_value(conn, "settings", stored)


def _identity_for_build(conn: sqlite3.Connection, cfg: dict):
    """The per-install device identity: stored settings first (preparing may
    have just persisted the watch identity), else a freshly generated
    per-install fallback, persisted so it stays stable."""
    stored = (db.get_config_value(conn, "settings", default={}) or {}).get(
        "hevy_device_identity") or cfg.get("hevy_device_identity")
    if stored:
        return identity_from_config({"hevy_device_identity": stored})
    identity = new_fallback_identity()
    _persist_identity(conn, identity)
    return identity


# -- exerciseSets payload (ported from upstream merge.py) --------------------


def build_exercise_sets_payload(
    resolved: list[ResolvedExercise],
    activity_start: str,
    activity_duration_s: float,
) -> dict:
    """Convert resolved exercises into a Garmin exerciseSets PUT payload,
    distributing synthetic set timing across the real activity window."""
    act_start = _parse_ts(activity_start)
    if act_start is None:
        raise ValueError(f"unparseable activity start: {activity_start!r}")
    for exercise in resolved:
        if exercise.category == UNKNOWN_CATEGORY:
            raise ValueError(f"exercise {exercise.title!r} resolved to UNKNOWN")

    working_set_s, warmup_set_s = 40, 25
    rest_sets_s, rest_exercises_s = 75, 120

    all_sets: list[dict] = []
    for ex_idx, exercise in enumerate(resolved):
        sets = exercise.sets
        for s_idx, s in enumerate(sets):
            is_warmup = s.get("type", "normal") == "warmup"
            explicit_dur = s.get("duration_seconds")
            if explicit_dur and explicit_dur > 0:
                set_dur = float(explicit_dur)
            else:
                set_dur = warmup_set_s if is_warmup else working_set_s
            is_last_set = s_idx == len(sets) - 1
            is_last_exercise = ex_idx == len(resolved) - 1
            if is_last_set and is_last_exercise:
                rest_dur = 0.0
            elif is_last_set:
                rest_dur = float(rest_exercises_s)
            else:
                rest_dur = float(rest_sets_s)
            all_sets.append({"ex_idx": ex_idx, "set_data": s,
                             "set_dur": set_dur, "rest_dur": rest_dur})

    ideal_total = sum(si["set_dur"] + si["rest_dur"] for si in all_sets)
    scale = activity_duration_s / ideal_total if ideal_total > 0 else 1.0
    scale = max(0.3, min(2.0, scale))

    exercise_sets: list[dict] = []
    msg_idx = 0
    cursor_s = 0.0
    for si in all_sets:
        s = si["set_data"]
        exercise = resolved[si["ex_idx"]]
        cat_str = CATEGORY_NAMES.get(exercise.category)
        if cat_str is None:
            raise ValueError(f"no category name for id {exercise.category}")
        # A null name under a valid category is accepted and rendered as the
        # generic label; an unrecognised name string renders as "Unknown".
        sub_name = subcategory_name(exercise.category, exercise.subcategory)

        set_start = act_start + timedelta(seconds=cursor_s)
        scaled_dur = si["set_dur"] * scale
        reps = s.get("reps")
        weight_kg = s.get("weight_kg")
        exercise_sets.append({
            "exercises": [{"category": cat_str, "name": sub_name,
                           "probability": None}],
            "duration": round(scaled_dur, 3),
            "repetitionCount": int(reps) if reps is not None else 0,
            "weight": float(round(weight_kg * 1000)) if weight_kg else 0.0,
            "setType": "ACTIVE",
            "startTime": set_start.strftime("%Y-%m-%dT%H:%M:%S.0"),
            "wktStepIndex": si["ex_idx"],
            "messageIndex": msg_idx,
        })
        msg_idx += 1
        cursor_s += scaled_dur

        if si["rest_dur"] > 0:
            rest_start = act_start + timedelta(seconds=cursor_s)
            scaled_rest = si["rest_dur"] * scale
            exercise_sets.append({
                "exercises": [],
                "duration": round(scaled_rest, 3),
                "setType": "REST",
                "startTime": rest_start.strftime("%Y-%m-%dT%H:%M:%S.0"),
                "wktStepIndex": si["ex_idx"],
                "messageIndex": msg_idx,
            })
            msg_idx += 1
            cursor_s += scaled_rest

    return {"exerciseSets": exercise_sets}


# -- description (ported from upstream generate_description) -----------------


def generate_description(workout: dict, calories: int | None = None,
                         avg_hr: int | None = None) -> str:
    lines: list[str] = []
    title = workout.get("title", "Workout")
    duration_s = 0
    start_dt = _parse_ts(workout.get("start_time"))
    end_dt = _parse_ts(workout.get("end_time"))
    if start_dt and end_dt:
        duration_s = int((end_dt - start_dt).total_seconds())

    lines.append(f"\U0001F3CB️ {title}")
    if duration_s > 0:
        lines.append(f"⏱️ {duration_s // 60} min")
    if calories:
        lines.append(f"\U0001F525 {calories} kcal")
    if avg_hr:
        lines.append(f"❤️ avg {avg_hr} bpm")

    exercises = workout.get("exercises", [])
    if exercises:
        lines.append("")
        for ex in exercises:
            name = ex.get("title") or ex.get("name", "Unknown")
            all_sets = ex.get("sets", [])
            warmup = [s for s in all_sets if s.get("type") == "warmup"]
            working = [s for s in all_sets if s.get("type") != "warmup"]
            if working:
                n_label = "set" if len(working) == 1 else "sets"
                has_distance = any(s.get("distance_meters") for s in working)
                has_duration = any(s.get("duration_seconds") for s in working)
                has_weight = any(s.get("weight_kg") or s.get("weight") for s in working)
                if has_distance or (has_duration and not has_weight):
                    total_dist = sum(s.get("distance_meters", 0) or 0 for s in working)
                    total_dur = sum(s.get("duration_seconds", 0) or 0 for s in working)
                    parts = [f"{len(working)} {n_label}"]
                    if total_dist > 0:
                        parts.append(f"{total_dist / 1000:.1f}km")
                    if total_dur > 0:
                        parts.append(f"{int(total_dur // 60)}min")
                else:
                    weights = [s.get("weight_kg") or s.get("weight", 0) for s in working]
                    reps = [s.get("reps", 0) or 0 for s in working]
                    top_weight = max(weights) if weights else 0
                    top_reps = max(reps) if reps else 0
                    parts = [f"{len(working)} {n_label}",
                             f"{top_weight:.1f}kg × {top_reps}"]

                if warmup:
                    parts.append(f"{len(warmup)} warmup")
                for set_type in ("dropset", "failure"):
                    count = sum(s.get("type") == set_type for s in working)
                    if count:
                        parts.append(f"{count} {set_type}")
                for key, label in (("rpe", "RPE"), ("custom_metric", "metric")):
                    values = []
                    for set_data in all_sets:
                        value = set_data.get(key)
                        if value is not None and value not in values:
                            values.append(value)
                    if values:
                        parts.append(f"{label} {', '.join(str(v) for v in values)}")
                lines.append(f"• {name}: {' · '.join(parts)}")
            elif warmup:
                s_label = "set" if len(warmup) == 1 else "sets"
                lines.append(f"• {name}: {len(warmup)} warmup {s_label}")

    lines.append("\n— synced by activsync")
    return "\n".join(lines)


# -- strategy execution ------------------------------------------------------


def _payload_of(row: dict) -> dict:
    payload = row["payload"]
    return json.loads(payload) if isinstance(payload, str) else payload


def _activity_window(conn: sqlite3.Connection, activity_id: int,
                     row: dict) -> tuple[str, float]:
    """The linked activity's (start, duration) for set-timing distribution,
    falling back to the Hevy workout's own window."""
    activity = db.get_activity(conn, activity_id)
    if activity:
        try:
            duration = float(json.loads(activity["garmin_data"]).get("duration") or 0)
        except (ValueError, TypeError):
            duration = 0.0
        if duration > 0:
            return activity["start_time"], duration
    start_dt = _parse_ts(row["start_time"])
    end_dt = _parse_ts(row["end_time"])
    duration = (end_dt - start_dt).total_seconds() if start_dt and end_dt else 0.0
    return row["start_time"], duration


def _apply_metadata(garmin: GarminClient, activity_id: int, row: dict) -> None:
    payload = _payload_of(row)
    garmin.set_title(activity_id, row["title"] or payload.get("title", "Workout"))
    garmin.set_description(activity_id, generate_description(payload))


def execute_merge(conn: sqlite3.Connection, garmin: GarminClient, row: dict) -> None:
    source_id = row["source_garmin_activity_id"]
    resolved = resolve_exercises(conn, _payload_of(row))
    # Back up the activity's own sets before the atomic full replace.
    existing = garmin.get_exercise_sets(source_id)
    hevy_db.save_backup(conn, source_id, row["hevy_id"], existing, None)

    start, duration = _activity_window(conn, source_id, row)
    payload = {**build_exercise_sets_payload(resolved, start, duration),
               "activityId": source_id}
    try:
        garmin.put_exercise_sets(source_id, payload)
    except SubcategoryRejected:
        # The 400 does not say WHICH pair was rejected — list candidates
        # (non-generic subcategories), never blame or auto-reject one.
        candidates = [r.title for r in resolved
                      if r.subcategory not in (0, 65535)]
        hevy_db.set_workout_status(
            conn, row["hevy_id"], "needs_mapping",
            error="Garmin rejected an exercise pair; adjust one of: "
                  + ", ".join(candidates))
        return
    except ActivityGone:
        hevy_db.set_workout_status(conn, row["hevy_id"], "needs_review",
                                   error="watch activity deleted on Garmin")
        return
    _apply_metadata(garmin, source_id, row)
    hevy_db.link_target(conn, row["hevy_id"], source_id, "merge")
    hevy_db.set_workout_status(conn, row["hevy_id"], "merged")
    hevy_db.set_applied(conn, row["hevy_id"], "garmin", row["source_updated_at"])


def execute_describe(conn: sqlite3.Connection, garmin: GarminClient, row: dict) -> None:
    source_id = row["source_garmin_activity_id"]
    try:
        _apply_metadata(garmin, source_id, row)
    except ActivityGone:
        hevy_db.set_workout_status(conn, row["hevy_id"], "needs_review",
                                   error="watch activity deleted on Garmin")
        return
    hevy_db.link_target(conn, row["hevy_id"], source_id, "describe")
    hevy_db.set_workout_status(conn, row["hevy_id"], "described")
    hevy_db.set_applied(conn, row["hevy_id"], "garmin", row["source_updated_at"])


def execute_replace(conn: sqlite3.Connection, garmin: GarminClient, row: dict,
                    cfg: dict) -> None:
    hevy_db.set_workout_status(conn, row["hevy_id"], "syncing")
    op = hevy_db.get_open_operation(conn, row["hevy_id"])
    if op is None:
        op_id = hevy_db.open_operation(conn, row["hevy_id"], "replace",
                                       row["source_garmin_activity_id"], [])
        if op_id is None:
            return  # another open operation owns this workout or source
        op = hevy_db.get_open_operation(conn, row["hevy_id"])
    advance_operation(conn, garmin, row, op, cfg)


def execute_passive(conn: sqlite3.Connection, garmin: GarminClient, row: dict,
                    cfg: dict) -> None:
    hevy_db.set_workout_status(conn, row["hevy_id"], "syncing")
    op = hevy_db.get_open_operation(conn, row["hevy_id"])
    if op is None:
        op_id = hevy_db.open_operation(conn, row["hevy_id"], "upload_passive",
                                       None, [])
        if op_id is None:
            return
        op = hevy_db.get_open_operation(conn, row["hevy_id"])
    advance_operation(conn, garmin, row, op, cfg)


# -- operation journal state machine -----------------------------------------


def _overlapping_any_type(conn: sqlite3.Connection, row: dict) -> str | None:
    """Type of any local activity overlapping the workout window, or None.
    The passive path's wrong-type guard: a blind upload would duplicate a
    session that WAS recorded, just under another type."""
    start = _parse_ts(row["start_time"])
    end = _parse_ts(row["end_time"])
    if not start or not end:
        return None
    for activity in db.list_activities(conn):
        act_start = _parse_ts(activity["start_time"])
        if act_start is None:
            continue
        try:
            duration = float(json.loads(activity["garmin_data"]).get("duration") or 0)
        except (ValueError, TypeError):
            duration = 0.0
        act_end = act_start + timedelta(seconds=duration)
        if act_start < end and act_end > start:
            return activity["activity_type"]
    return None


def _park_operation(conn: sqlite3.Connection, op: dict, row: dict,
                    error: str) -> None:
    hevy_db.set_operation_outcome(
        conn, op["id"], row["hevy_id"], "needs_review", "needs_review", error)


def advance_operation(conn: sqlite3.Connection, garmin: GarminClient, row: dict,
                      op: dict, cfg: dict) -> None:
    """Drive one operation as far as it can go this tick. Every transition is
    persisted before the next side effect, so a crash resumes exactly where
    it stopped. submission_unknown is entered ONLY from the upload call
    itself and is never auto-resubmitted."""
    hevy_id = row["hevy_id"]
    while True:
        op = hevy_db.get_open_operation(conn, hevy_id)
        if op is None:
            return
        phase = op["phase"]
        kind = op["kind"]
        source = op["source_activity_id"]

        if phase == "preparing":
            if kind == "replace":
                fit_bytes = garmin.download_fit(source)
                existing = garmin.get_exercise_sets(source)
                hevy_db.save_backup(conn, source, hevy_id, existing, fit_bytes)
                settings = db.get_config_value(conn, "settings", default={}) or {}
                stored_identity = settings.get("hevy_device_identity") or {}
                # Product 0 is the generic no-watch fallback. Upgrade it as
                # soon as a real watch FIT makes per-user detection possible.
                if not stored_identity or stored_identity.get("product") in (0, "0"):
                    detected = identity_from_fit(fit_bytes)
                    if detected:
                        _persist_identity(conn, detected)
                activity = db.get_activity(conn, source)
                if activity and activity.get("strava_activity_id"):
                    _park_operation(conn, op, row,
                                    "source activity already published to Strava")
                    return
            else:
                wrong_type = _overlapping_any_type(conn, row)
                if wrong_type:
                    _park_operation(
                        conn, op, row,
                        f"overlapping {wrong_type} activity exists — passive "
                        "upload would duplicate it")
                    return
            snapshot = garmin.list_activity_ids_near(row["start_time"])
            hevy_db.update_operation(conn, op["id"], phase="uploading",
                                     pre_upload_ids=snapshot)
            continue

        if phase == "uploading":
            # Everything before the upload call is pre-submission: a
            # deterministic failure here closes the operation instead of
            # wedging it open in `uploading` forever.
            import tempfile
            try:
                resolved = resolve_exercises(conn, _payload_of(row))
                start_dt = _parse_ts(row["start_time"])
                end_dt = _parse_ts(row["end_time"])
                if kind == "replace":
                    backup = hevy_db.get_backup(conn, source)
                    fit_bytes = backup["original_fit"] if backup else None
                    hr = (hr_sources.extract_fit_hr(
                        fit_bytes, round(start_dt.timestamp() * 1000))
                        if fit_bytes and start_dt else [])
                else:
                    hr = (hr_sources.passive_hr_for_window(garmin, start_dt, end_dt)
                          if start_dt and end_dt else [])
                profile = _profile_from_cfg(cfg)
                identity = _identity_for_build(conn, cfg)
                tmp_dir_ctx = tempfile.TemporaryDirectory(prefix="activsync-fit-")
                with tmp_dir_ctx as tmp_dir:
                    fit_path = f"{tmp_dir}/hevy_{hevy_id}.fit"
                    build_fit(_payload_of(row), resolved, hr or None, profile,
                              identity, fit_path)
                    upload_error: Exception | None = None
                    try:
                        result = garmin.upload_fit(fit_path)
                    except Exception as exc:
                        upload_error = exc
            except MappingMiss as miss:
                hevy_db.set_operation_outcome(
                    conn, op["id"], hevy_id, "failed", "needs_mapping",
                    f"unmapped exercises: {miss.title}")
                return
            except Exception as exc:
                hevy_db.set_operation_outcome(
                    conn, op["id"], hevy_id, "failed", "failed",
                    f"FIT build failed: {exc}")
                return

            if upload_error is not None:
                if isinstance(upload_error, GarminUploadRejected):
                    hevy_db.set_operation_outcome(
                        conn, op["id"], hevy_id, "failed", "failed",
                        str(upload_error))
                    return
                # Outcome unknown — record and wait; NEVER resubmit.
                hevy_db.update_operation(conn, op["id"],
                                         phase="submission_unknown",
                                         next_step="resolve",
                                         last_error=str(upload_error))
                return

            new_id = result.get("activity_id")
            pre_ids = set(op["pre_upload_ids"])
            if new_id and new_id not in pre_ids and new_id != source:
                hevy_db.update_operation(conn, op["id"], phase="finalizing",
                                         next_step="metadata",
                                         target_activity_id=new_id,
                                         upload_id=result.get("upload_id"))
                continue
            # Accepted but not yet resolvable — let Garmin finish processing.
            hevy_db.update_operation(conn, op["id"], phase="submission_unknown",
                                     next_step="resolve",
                                     upload_id=result.get("upload_id"))
            return

        if phase == "submission_unknown":
            candidates = _resolution_candidates(
                garmin.list_activities_near(row["start_time"]), row,
                set(op["pre_upload_ids"]), source)
            if len(candidates) == 1:
                hevy_db.update_operation(conn, op["id"], phase="finalizing",
                                         next_step="metadata",
                                         target_activity_id=candidates.pop())
                continue
            if len(candidates) > 1:
                _park_operation(
                    conn, op, row,
                    f"ambiguous upload result: candidates {sorted(candidates)}")
                return
            attempts = op["attempt_count"] + 1
            if attempts >= 5:
                _park_operation(conn, op, row,
                                "upload outcome unresolved after 5 checks")
                return
            hevy_db.update_operation(conn, op["id"], attempt_count=attempts)
            return

        if phase == "finalizing":
            # Sub-steps persist through next_step so a crash resumes exactly
            # where it stopped: metadata → delete → finalize.
            target = op["target_activity_id"]
            next_step = op["next_step"] or "metadata"

            if next_step == "metadata":
                _apply_metadata(garmin, target, row)
                hevy_db.update_operation(conn, op["id"], next_step="delete")
                next_step = "delete"

            if next_step == "delete":
                if kind == "replace" and source and target != source:
                    try:
                        garmin.delete_activity(source)
                    except Exception as exc:
                        if _is_not_found(exc):
                            # Already gone — a crash after a successful delete
                            # replays here; 404 IS the success signal.
                            pass
                        else:
                            deletes = op["delete_attempt_count"] + 1
                            if deletes >= 3:
                                _park_operation(
                                    conn, op, row,
                                    f"could not delete watch activity "
                                    f"{source}: {exc}")
                                return
                            hevy_db.update_operation(
                                conn, op["id"], delete_attempt_count=deletes,
                                last_error=str(exc))
                            return
                hevy_db.update_operation(conn, op["id"], next_step="finalize")

            strategy = "replace" if kind == "replace" else "passive"
            status = "replaced" if kind == "replace" else "uploaded_passive"
            # Terminal transition is atomic: op done + link + status + applied
            # land in one transaction (a split would strand an unlinked
            # replacement behind a closed journal → duplicate upload later).
            hevy_db.complete_operation(conn, op["id"], hevy_id, target,
                                       strategy, status,
                                       row["source_updated_at"])
            return

        return  # done/failed/needs_review — nothing to drive


_RESOLUTION_DRIFT_MIN = 10
_RESOLUTION_TYPES = ("strength_training", "other")


def _resolution_candidates(activities: list[dict], row: dict,
                           pre_upload_ids: set, source: int | None) -> set[int]:
    """Strict matching for submission_unknown resolution: an unknown-outcome
    upload may only be adopted if the candidate looks like OUR upload — new
    id, strength/other type, start near the workout's start, plausible
    duration. A new run appearing in the 3-day window must never be adopted,
    renamed, and have the watch activity deleted under it."""
    hevy_start = _parse_ts(row["start_time"])
    hevy_end = _parse_ts(row["end_time"])
    if not hevy_start or not hevy_end:
        return set()
    hevy_duration = (hevy_end - hevy_start).total_seconds()
    if hevy_duration <= 0:
        return set()

    normalized_pre_ids: set[int] = set()
    for raw_id in pre_upload_ids:
        try:
            normalized_pre_ids.add(int(raw_id))
        except (TypeError, ValueError):
            continue
    try:
        normalized_source = int(source) if source is not None else None
    except (TypeError, ValueError):
        normalized_source = None

    candidates: set[int] = set()
    for act in activities:
        try:
            activity_id = int(act.get("activityId"))
        except (TypeError, ValueError):
            continue
        if activity_id in normalized_pre_ids or activity_id == normalized_source:
            continue
        act_type = (act.get("activityType") or {}).get("typeKey", "")
        if act_type not in _RESOLUTION_TYPES:
            continue
        act_start = _parse_ts(act.get("startTimeGMT", ""))
        if act_start is None:
            continue
        drift_s = abs((act_start - hevy_start).total_seconds())
        if drift_s > _RESOLUTION_DRIFT_MIN * 60:
            continue
        try:
            duration = float(act.get("duration"))
        except (TypeError, ValueError):
            continue
        if duration <= 0:
            continue
        ratio = duration / hevy_duration
        if not (0.25 <= ratio <= 4.0):
            continue
        candidates.add(activity_id)
    return candidates
