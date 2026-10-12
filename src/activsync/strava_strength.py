"""Strava's JSON strength upload for a Hevy-linked activity.

Strava reads a FIT file's sets through its own partial table of Garmin
exercise ids, so machine variants and some common exercises come out wrong or
"Unknown". The JSON format names each set's Strava exercise directly. The sets
come from the Hevy workout; time, heart rate and calories come from the
Garmin recording, which stays the source for everything the watch measured.
"""

from __future__ import annotations

import sqlite3
from dataclasses import dataclass
from datetime import datetime, timezone
from zoneinfo import ZoneInfo

from garmin_fit_sdk import Decoder, Stream

from activsync import timeutil
from activsync.strava_exercise_mapper import lookup_strava_exercise

_FIT_EPOCH_S = 631065600


@dataclass(frozen=True)
class _Recording:
    start: datetime
    elapsed_s: int
    active_s: int | None
    calories: int | None
    utc_offset_s: int | None
    heartrate: tuple[tuple[int, int], ...]
    set_timing: tuple[tuple[datetime, float], ...]
    device: str | None


def build_strength_upload(
    conn: sqlite3.Connection,
    workout: dict,
    fit_bytes: bytes,
    tz_name: str | None = None,
) -> dict:
    """The JSON upload document for a Hevy workout recorded as `fit_bytes`.

    Raises StravaMappingMiss when an exercise has no Strava exercise_type, and
    ValueError when the workout has no sets or no usable time window.
    """
    hevy_sets: list[tuple[str, dict]] = []
    for exercise in workout.get("exercises", []):
        exercise_type, _ = lookup_strava_exercise(
            conn, exercise.get("title", ""), exercise.get("exercise_template_id"))
        hevy_sets.extend((exercise_type, s) for s in exercise.get("sets", []))
    if not hevy_sets:
        raise ValueError("workout has no sets")

    recording = _read_fit(fit_bytes) or _from_workout(workout)
    timing = (recording.set_timing
              if len(recording.set_timing) == len(hevy_sets) else ())

    sets = []
    for index, (exercise_type, hevy_set) in enumerate(hevy_sets):
        entry: dict = {"exercise_type": exercise_type}
        reps = hevy_set.get("reps")
        if isinstance(reps, int) and reps >= 0:
            entry["repetitions"] = reps
        weight = hevy_set.get("weight_kg")
        if isinstance(weight, (int, float)) and weight > 0:
            entry["weight"] = float(weight)
        if timing:
            set_start, duration = timing[index]
            entry["start_time"] = _iso(set_start)
            entry["duration"] = round(duration)
        elif isinstance(hevy_set.get("duration_seconds"), (int, float)):
            entry["duration"] = round(hevy_set["duration_seconds"])
        sets.append(entry)

    offset = recording.utc_offset_s
    if offset is None:
        offset = _zone_offset(recording.start, tz_name)
    document: dict = {
        "version": "1.0",
        "start_time": _iso(recording.start),
        "utc_offset": offset,
        "elapsed_time": recording.elapsed_s,
        "sets": sets,
    }
    if recording.active_s is not None:
        document["active_time"] = recording.active_s
    if recording.calories:
        document["total_calories"] = recording.calories
    if recording.device:
        document["creator"] = {"name": recording.device}
    if recording.heartrate:
        document["streams"] = {
            "time": [t for t, _ in recording.heartrate],
            "heartrate": [bpm for _, bpm in recording.heartrate],
        }
    return document


def _read_fit(fit_bytes: bytes) -> _Recording | None:
    """What the watch measured, or None when the file can't be read."""
    if not fit_bytes:
        return None
    try:
        messages, errors = Decoder(
            Stream.from_byte_array(bytearray(fit_bytes))).read()
    except Exception:
        return None
    sessions = messages.get("session_mesgs") or []
    if errors or not sessions or not isinstance(sessions[0].get("start_time"), datetime):
        return None
    session = sessions[0]
    start = session["start_time"].astimezone(timezone.utc)
    elapsed = session.get("total_elapsed_time")
    if not isinstance(elapsed, (int, float)) or elapsed <= 0:
        return None
    timer = session.get("total_timer_time")
    calories = session.get("total_calories")

    offset = None
    activity = (messages.get("activity_mesgs") or [{}])[0]
    local, utc = activity.get("local_timestamp"), activity.get("timestamp")
    if isinstance(local, int) and isinstance(utc, datetime):
        # local_timestamp is wall-clock seconds since the FIT epoch, so its
        # distance from the UTC timestamp is the athlete's offset.
        offset = round(_FIT_EPOCH_S + local - utc.timestamp())

    heartrate: list[tuple[int, int]] = []
    for record in messages.get("record_mesgs") or []:
        bpm, stamp = record.get("heart_rate"), record.get("timestamp")
        if isinstance(bpm, int) and isinstance(stamp, datetime):
            second = int((stamp - start).total_seconds())
            if second >= 0 and (not heartrate or second > heartrate[-1][0]):
                heartrate.append((second, bpm))

    set_timing = tuple(
        (s["start_time"].astimezone(timezone.utc), float(s["duration"]))
        for s in messages.get("set_mesgs") or []
        if s.get("set_type") == "active"
        and isinstance(s.get("start_time"), datetime)
        and isinstance(s.get("duration"), (int, float))
    )

    file_id = (messages.get("file_id_mesgs") or [{}])[0]
    product = file_id.get("garmin_product")
    device = (f"Garmin {str(product).replace('_', ' ')}"
              if file_id.get("manufacturer") == "garmin" and isinstance(product, str)
              else None)

    return _Recording(
        start=start,
        elapsed_s=round(elapsed),
        active_s=round(timer) if isinstance(timer, (int, float)) and timer > 0 else None,
        calories=round(calories) if isinstance(calories, (int, float)) else None,
        utc_offset_s=offset,
        heartrate=tuple(heartrate),
        set_timing=set_timing,
        device=device,
    )


def _from_workout(workout: dict) -> _Recording:
    start = timeutil.parse_timestamp(workout.get("start_time"))
    end = timeutil.parse_timestamp(workout.get("end_time"))
    if start is None or end is None or end <= start:
        raise ValueError("workout has no usable time window")
    return _Recording(
        start=start,
        elapsed_s=round((end - start).total_seconds()),
        active_s=None,
        calories=None,
        utc_offset_s=None,
        heartrate=(),
        set_timing=(),
        device=None,
    )


def _zone_offset(moment: datetime, tz_name: str | None) -> int:
    if tz_name and timeutil.is_valid_timezone(tz_name):
        offset = moment.astimezone(ZoneInfo(tz_name)).utcoffset()
        if offset is not None:
            return round(offset.total_seconds())
    return 0


def _iso(moment: datetime) -> str:
    return moment.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
