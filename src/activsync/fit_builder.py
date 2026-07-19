"""Generate strength-training FIT files from Hevy workout data.

Ported from hevy2garmin's fit.py (MIT) with ActivSync's deltas: the profile
and device identity are injected (no config reads — the builder is pure),
exercises arrive pre-resolved (the mapping gate ran first, so no mapper calls
and no UNKNOWN can slip in), and calories use sex-correct Keytel constants
(upstream hardcodes the male equation).

Device identity helpers live here too: identity_from_fit reads a watch FIT's
FileIdMessage so each install carries its *own* watch identity — never a
hardcoded serial.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

from fit_tool.fit_file import FitFile
from fit_tool.fit_file_builder import FitFileBuilder
from fit_tool.profile.messages.activity_message import ActivityMessage
from fit_tool.profile.messages.event_message import EventMessage
from fit_tool.profile.messages.exercise_title_message import ExerciseTitleMessage
from fit_tool.profile.messages.file_id_message import FileIdMessage
from fit_tool.profile.messages.lap_message import LapMessage
from fit_tool.profile.messages.record_message import RecordMessage
from fit_tool.profile.messages.session_message import SessionMessage
from fit_tool.profile.messages.set_message import SetMessage
from fit_tool.profile.messages.sport_message import SportMessage
from fit_tool.profile.profile_type import (
    Activity,
    Event,
    EventType,
    FileType,
    SetType,
    Sport,
    SubSport,
)

# -- timing/scaling constants (upstream's config defaults, now fixed) --------
WORKING_SET_S = 40
WARMUP_SET_S = 25
REST_BETWEEN_SETS_S = 75
REST_BETWEEN_EXERCISES_S = 120
_MIN_SCALE = 0.3
_MAX_SCALE = 2.0
DEFAULT_HR_BPM = 90  # calorie fallback when no HR data at all


@dataclass(frozen=True)
class DeviceIdentity:
    manufacturer: int
    product: int
    serial: int


# Fallback when no watch FIT has ever been seen. Spike-proven: a generic
# Garmin identity (manufacturer 1, product 0) is accepted and gets TE/Load.
# The serial is a placeholder — installs should persist new_fallback_identity()
# so each carries its own stable serial rather than a shared constant.
GENERIC_GARMIN_IDENTITY = DeviceIdentity(manufacturer=1, product=0, serial=424242)
# Upstream's choice, kept for reference/tests only — never the default.
DEVELOPMENT_IDENTITY = DeviceIdentity(manufacturer=255, product=0, serial=12345)

UNKNOWN_CATEGORY = 65534


def new_fallback_identity() -> DeviceIdentity:
    """A generic Garmin identity with a fresh random serial. Generated once
    per install and persisted (hevy_device_identity), so installs don't share
    a serial while still avoiding any hardcoded user value."""
    import secrets

    return DeviceIdentity(manufacturer=1, product=0,
                          serial=secrets.randbelow(2**31 - 1) + 1)


@dataclass(frozen=True)
class Profile:
    weight_kg: float
    birth_year: int
    vo2max: float
    sex: str  # "male" | "female"


@dataclass(frozen=True)
class ResolvedExercise:
    title: str
    category: int
    subcategory: int
    sets: list  # Hevy set dicts


def identity_from_fit(fit_bytes: bytes) -> DeviceIdentity | None:
    """Read the device identity from a watch FIT's FileIdMessage.

    This is the auto-detection primitive: each install derives its identity
    from the user's own watch recordings. None when the bytes are not a
    parseable FIT or carry no usable FileId."""
    import tempfile

    try:
        with tempfile.NamedTemporaryFile(suffix=".fit") as tmp:
            tmp.write(fit_bytes)
            tmp.flush()
            fit = FitFile.from_file(tmp.name)
        for record in fit.records:
            message = record.message
            if isinstance(message, FileIdMessage):
                manufacturer = message.manufacturer
                product = getattr(message, "product", None)
                serial = message.serial_number
                if manufacturer is None or product is None or serial is None:
                    return None
                return DeviceIdentity(int(manufacturer), int(product), int(serial))
    except Exception:
        return None
    return None


def identity_from_config(cfg: dict) -> DeviceIdentity:
    """The stored per-install identity, or the generic Garmin fallback."""
    stored = cfg.get("hevy_device_identity")
    if isinstance(stored, dict):
        try:
            return DeviceIdentity(
                int(stored["manufacturer"]), int(stored["product"]),
                int(stored["serial"]))
        except (KeyError, TypeError, ValueError):
            pass
    return GENERIC_GARMIN_IDENTITY


# -- time + calories --------------------------------------------------------


def _ms(dt: datetime) -> int:
    return round(dt.timestamp() * 1000)


def _parse_timestamp(raw: object) -> datetime | None:
    """ISO-8601 or Garmin space-separated timestamp → UTC datetime; None on
    null/malformed input."""
    if not raw or not isinstance(raw, str):
        return None
    cleaned = raw.strip()
    if not cleaned:
        return None
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


def keytel_kcal_per_min(hr: int, profile: Profile, age: int) -> float:
    """Keytel et al. 2005 energy expenditure, sex-correct constants."""
    if profile.sex == "female":
        return (-59.3954 + 0.45 * hr + 0.380 * profile.vo2max
                + 0.103 * profile.weight_kg + 0.274 * age) / 4.184
    return (-95.7735 + 0.634 * hr + 0.404 * profile.vo2max
            + 0.394 * profile.weight_kg + 0.271 * age) / 4.184


def _calc_calories(hr_bpm: list[int], duration_s: float, workout_year: int,
                   profile: Profile) -> int:
    age = workout_year - profile.birth_year
    samples = hr_bpm or [DEFAULT_HR_BPM]
    interval_min = (duration_s / len(samples)) / 60.0
    total = 0.0
    for hr in samples:
        total += max(0.0, keytel_kcal_per_min(hr, profile, age)) * interval_min
    return round(total)


# -- main generator ---------------------------------------------------------


def build_fit(
    workout: dict,
    resolved: list[ResolvedExercise],
    hr_samples: list | None,
    profile: Profile,
    identity: DeviceIdentity,
    output_path: str,
) -> dict:
    """Build a strength FIT file; returns a summary dict.

    hr_samples: either timestamped dicts ({"time": secs_from_start, "hr": bpm},
    placed at their real offsets) or plain bpm ints (distributed evenly). None
    or empty → no HR records, default-HR calories.
    """
    hr_timed: list[tuple[float, int]] | None = None
    if not hr_samples:
        hr_bpm: list[int] = []
    elif isinstance(hr_samples[0], dict):
        hr_timed = [
            (max(0.0, float(s.get("time", 0))), int(s["hr"]))
            for s in hr_samples
            if s.get("hr") is not None
        ]
        hr_bpm = [b for _, b in hr_timed]
    else:
        hr_bpm = [int(x) for x in hr_samples]

    start_dt = _parse_timestamp(workout.get("start_time"))
    end_dt = _parse_timestamp(workout.get("end_time"))
    if not start_dt or not end_dt:
        raise ValueError(
            f"Workout '{workout.get('title', '?')}' missing valid start/end time "
            f"(start={workout.get('start_time')!r}, end={workout.get('end_time')!r})"
        )
    duration_s = (end_dt - start_dt).total_seconds()
    if duration_s <= 0:
        raise ValueError(
            f"Workout '{workout.get('title', '?')}' has non-positive duration "
            f"({workout.get('start_time')!r} → {workout.get('end_time')!r})"
        )
    if hr_timed is not None:
        # Buffered source samples that cannot land in the activity must not
        # influence calories or avg/max summaries either.
        hr_timed = [(offset, bpm) for offset, bpm in hr_timed
                    if offset <= duration_s]
        hr_bpm = [bpm for _, bpm in hr_timed]
    for exercise in resolved:
        # Defense in depth behind the mapping gate: the sentinel must never
        # be written into a FIT, whatever upstream data slipped through.
        if exercise.category == UNKNOWN_CATEGORY:
            raise ValueError(f"exercise {exercise.title!r} resolved to UNKNOWN")
    start_ms = _ms(start_dt)
    end_ms = start_ms + round(duration_s * 1000)

    calories = _calc_calories(hr_bpm, duration_s, start_dt.year, profile)

    # -- gather sets and compute the synthetic timeline --
    num_exercises = len(resolved)
    total_distance_m = 0.0
    all_sets_info: list[dict] = []
    for ex_idx, exercise in enumerate(resolved):
        sets = exercise.sets
        for s_idx, s in enumerate(sets):
            is_warmup = s.get("type", "normal") == "warmup"
            explicit_dur = s.get("duration_seconds")
            if explicit_dur and explicit_dur > 0:
                set_dur = float(explicit_dur)
            else:
                set_dur = WARMUP_SET_S if is_warmup else WORKING_SET_S

            is_last_set_of_exercise = s_idx == len(sets) - 1
            is_last_exercise = ex_idx == num_exercises - 1
            if is_last_set_of_exercise and is_last_exercise:
                rest_dur = 0.0
            elif is_last_set_of_exercise:
                rest_dur = float(REST_BETWEEN_EXERCISES_S)
            else:
                rest_dur = float(REST_BETWEEN_SETS_S)

            all_sets_info.append({
                "ex_idx": ex_idx,
                "set_data": s,
                "set_dur": set_dur,
                "rest_dur": rest_dur,
            })

    total_sets = len(all_sets_info)
    ideal_total = sum(si["set_dur"] + si["rest_dur"] for si in all_sets_info)
    if ideal_total > 0:
        scale = max(_MIN_SCALE, min(_MAX_SCALE, duration_s / ideal_total))
    else:
        scale = 1.0

    cursor_s = 0.0
    for si in all_sets_info:
        si["start_offset_s"] = cursor_s
        scaled_set = si["set_dur"] * scale
        si["end_offset_s"] = cursor_s + scaled_set
        cursor_s += scaled_set + si["rest_dur"] * scale

    # -- build messages --
    builder = FitFileBuilder(auto_define=True, min_string_size=50)

    file_id = FileIdMessage()
    file_id.type = FileType.ACTIVITY
    file_id.manufacturer = identity.manufacturer
    file_id.product = identity.product
    file_id.serial_number = identity.serial
    file_id.time_created = start_ms
    builder.add(file_id)

    sport_msg = SportMessage()
    sport_msg.sport = Sport.TRAINING
    sport_msg.sub_sport = SubSport.STRENGTH_TRAINING
    builder.add(sport_msg)

    for ex_idx, exercise in enumerate(resolved):
        etm = ExerciseTitleMessage()
        etm.message_index = ex_idx
        etm.exercise_category = exercise.category
        etm.exercise_name = exercise.subcategory
        etm.workout_step_name = exercise.title
        builder.add(etm)

    event_start = EventMessage()
    event_start.timestamp = start_ms
    event_start.event = Event.TIMER
    event_start.event_type = EventType.START
    builder.add(event_start)

    # timeline of (ms, kind, message) — records sort before sets at equal ts
    timeline: list[tuple[int, str, object]] = []

    if hr_timed:
        for offset_s, hr_val in hr_timed:
            t_ms = start_ms + round(offset_s * 1000)
            rec = RecordMessage()
            rec.timestamp = t_ms
            rec.heart_rate = hr_val
            timeline.append((t_ms, "record", rec))
    elif hr_bpm:
        if len(hr_bpm) == 1:
            hr_interval_ms = 0
        else:
            hr_interval_ms = round(duration_s * 1000 / (len(hr_bpm) - 1))
        for i, hr_val in enumerate(hr_bpm):
            t_ms = start_ms + (i * hr_interval_ms if len(hr_bpm) > 1 else 0)
            rec = RecordMessage()
            rec.timestamp = t_ms
            rec.heart_rate = hr_val
            timeline.append((t_ms, "record", rec))

    msg_index = 0
    for si in all_sets_info:
        s = si["set_data"]
        exercise = resolved[si["ex_idx"]]

        set_start_ms = start_ms + round(si["start_offset_s"] * 1000)
        set_end_ms = start_ms + round(si["end_offset_s"] * 1000)

        active = SetMessage()
        active.timestamp = set_end_ms
        active.start_time = set_start_ms
        active.duration = si["end_offset_s"] - si["start_offset_s"]
        active.set_type = SetType.ACTIVE
        active.category = [exercise.category]
        active.category_subtype = [exercise.subcategory]
        active.message_index = msg_index
        active.workout_step_index = si["ex_idx"]

        reps = s.get("reps")
        if reps is not None:
            active.repetitions = int(reps)
        weight = s.get("weight_kg")
        if weight is not None:
            active.weight = max(0.0, float(weight))

        distance = s.get("distance_meters")
        if distance is not None and float(distance) > 0:
            total_distance_m += float(distance)
            dist_rec = RecordMessage()
            dist_rec.timestamp = set_end_ms
            dist_rec.distance = float(distance)
            timeline.append((set_end_ms, "record", dist_rec))

        timeline.append((set_end_ms, "set", active))
        msg_index += 1

        if si["rest_dur"] > 0:
            rest_start_ms = set_end_ms
            rest_dur_scaled = si["rest_dur"] * scale
            rest_end_ms = rest_start_ms + round(rest_dur_scaled * 1000)

            rest = SetMessage()
            rest.timestamp = rest_end_ms
            rest.start_time = rest_start_ms
            rest.duration = rest_dur_scaled
            rest.set_type = SetType.REST
            rest.message_index = msg_index
            rest.workout_step_index = si["ex_idx"]

            timeline.append((rest_end_ms, "set", rest))
            msg_index += 1

    # Nothing may be stamped after TIMER STOP_ALL: HR carries a ±60 s slice
    # buffer and the scale floor can push synthetic sets past the end, so
    # clamp set boundaries to end_ms and drop out-of-window records.
    clipped: list[tuple[int, str, object]] = []
    for ts, kind, msg in timeline:
        if kind == "record":
            if ts <= end_ms:
                clipped.append((ts, kind, msg))
            continue
        if msg.start_time >= end_ms:
            continue
        if ts > end_ms:
            msg.timestamp = end_ms
            msg.duration = (end_ms - msg.start_time) / 1000.0
            ts = end_ms
        clipped.append((ts, kind, msg))
    timeline = clipped

    timeline.sort(key=lambda x: (x[0], 0 if x[1] == "record" else 1))
    for _, _, msg in timeline:
        builder.add(msg)

    event_stop = EventMessage()
    event_stop.timestamp = end_ms
    event_stop.event = Event.TIMER
    event_stop.event_type = EventType.STOP_ALL
    builder.add(event_stop)

    lap = LapMessage()
    lap.timestamp = end_ms
    lap.start_time = start_ms
    lap.total_elapsed_time = duration_s
    lap.total_timer_time = duration_s
    lap.sport = Sport.TRAINING
    lap.sub_sport = SubSport.STRENGTH_TRAINING
    lap.message_index = 0
    lap.event = Event.LAP
    lap.event_type = EventType.STOP
    if hr_bpm:
        lap.avg_heart_rate = round(sum(hr_bpm) / len(hr_bpm))
        lap.max_heart_rate = max(hr_bpm)
    if total_distance_m > 0:
        lap.total_distance = total_distance_m
    lap.total_calories = calories
    builder.add(lap)

    session = SessionMessage()
    session.timestamp = end_ms
    session.start_time = start_ms
    session.total_elapsed_time = duration_s
    session.total_timer_time = duration_s
    session.sport = Sport.TRAINING
    session.sub_sport = SubSport.STRENGTH_TRAINING
    session.message_index = 0
    session.first_lap_index = 0
    session.num_laps = 1
    session.event = Event.LAP
    session.event_type = EventType.STOP
    if hr_bpm:
        session.avg_heart_rate = round(sum(hr_bpm) / len(hr_bpm))
        session.max_heart_rate = max(hr_bpm)
    if total_distance_m > 0:
        session.total_distance = total_distance_m
    session.total_calories = calories
    builder.add(session)

    activity = ActivityMessage()
    activity.timestamp = end_ms
    activity.total_timer_time = duration_s
    activity.num_sessions = 1
    activity.type = Activity.MANUAL
    activity.event = Event.ACTIVITY
    activity.event_type = EventType.STOP
    builder.add(activity)

    Path(output_path).parent.mkdir(parents=True, exist_ok=True)
    builder.build().to_file(output_path)

    avg_hr = round(sum(hr_bpm) / len(hr_bpm)) if hr_bpm else None
    return {
        "exercises": num_exercises,
        "total_sets": total_sets,
        "hr_samples": len(hr_bpm),
        "calories": calories,
        "avg_hr": avg_hr,
        "duration_s": duration_s,
        "output_path": output_path,
    }
