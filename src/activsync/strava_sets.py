"""Carry Garmin's current exercise sets into the FIT that goes to Strava.

Strava reads strength sets straight from the FIT ``set`` messages
(``category``/``category_subtype``, reps, weight, timing). Garmin's ORIGINAL
download is the file the watch recorded, so it carries the watch's own set
guesses: neither a Hevy merge (an ``exerciseSets`` PUT) nor an edit made in
Garmin Connect ever reaches it, and Strava shows those exercises as
"Unknown". This module swaps the file's ``set`` messages for the sets Garmin
holds now, leaving every other byte of the recording intact.
"""

from __future__ import annotations

import logging
import struct
from datetime import datetime, timezone

from garmin_fit_sdk import Encoder
from garmin_fit_sdk.crc_calculator import CrcCalculator

from activsync.fit_profile import CATEGORY_NAMES, SUBCATEGORY_NAMES, mesg

logger = logging.getLogger(__name__)

SET_MESG_NUM = 225
_INVALID_UINT16 = 0xFFFF

_CATEGORY_IDS: dict[str, int] = {name: value for value, name in CATEGORY_NAMES.items()}
_SUBCATEGORY_IDS: dict[int, dict[str, int]] = {
    category: {name: value for value, name in names.items()}
    for category, names in SUBCATEGORY_NAMES.items()
}


def _parse_start(raw: object) -> datetime | None:
    if not isinstance(raw, str) or not raw.strip():
        return None
    try:
        parsed = datetime.fromisoformat(raw.strip().replace("Z", "+00:00"))
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)  # Garmin sends GMT
    return parsed.astimezone(timezone.utc)


def _best_exercise(exercises: object) -> dict | None:
    """The watch can attach several guesses; keep the most probable one."""
    if not isinstance(exercises, list):
        return None
    candidates = [e for e in exercises if isinstance(e, dict) and e.get("category")]
    if not candidates:
        return None
    return max(candidates, key=lambda e: e.get("probability") or 0)


def fit_sets_from_garmin(exercise_sets: object) -> list[dict]:
    """Garmin's ``exerciseSets`` JSON as FIT ``set`` message payloads.

    Returns [] when the payload holds nothing usable, so callers can keep
    the original file untouched.
    """
    if not isinstance(exercise_sets, dict):
        return []
    raw_sets = exercise_sets.get("exerciseSets")
    if not isinstance(raw_sets, list):
        return []

    sets: list[dict] = []
    for index, raw in enumerate(raw_sets):
        if not isinstance(raw, dict):
            continue
        start = _parse_start(raw.get("startTime"))
        duration = raw.get("duration")
        if start is None or not isinstance(duration, (int, float)) or duration < 0:
            continue
        is_active = raw.get("setType") == "ACTIVE"
        payload: dict = {
            "timestamp": datetime.fromtimestamp(
                round(start.timestamp() + duration), tz=timezone.utc),
            "start_time": start,
            "duration": float(duration),
            "set_type": "active" if is_active else "rest",
            "message_index": index,
        }
        step = raw.get("wktStepIndex")
        if isinstance(step, int):
            payload["wkt_step_index"] = step
        if is_active:
            exercise = _best_exercise(raw.get("exercises"))
            category = _CATEGORY_IDS.get(str(exercise["category"]).upper()) if exercise else None
            if category is not None:
                name = exercise.get("name")
                subcategory = (
                    _SUBCATEGORY_IDS.get(category, {}).get(str(name).upper())
                    if name else None
                )
                payload["category"] = [category]
                payload["category_subtype"] = [
                    subcategory if subcategory is not None else _INVALID_UINT16]
            reps = raw.get("repetitionCount")
            if isinstance(reps, int) and reps >= 0:
                payload["repetitions"] = reps
            weight_g = raw.get("weight")
            if isinstance(weight_g, (int, float)) and weight_g > 0:
                payload["weight"] = weight_g / 1000.0  # Garmin JSON is grams
        sets.append(payload)
    return sets


def _encoded_messages(sets: list[dict]) -> bytes:
    """The definition + data records for `sets`, without header or CRC."""
    encoder = Encoder()
    for payload in sets:
        encoder.write_mesg(mesg("set", **payload))
    data = bytes(encoder.close())
    return data[data[0]:-2]


def _strip_sets(records: bytes) -> bytes | None:
    """`records` without its ``set`` data messages; None if unparseable."""
    out = bytearray()
    sizes: dict[int, int] = {}
    globals_: dict[int, int] = {}
    pos = 0
    while pos < len(records):
        header = records[pos]
        if header & 0x80:  # compressed-timestamp data message
            local = (header >> 5) & 0x03
            is_definition = False
        else:
            local = header & 0x0F
            is_definition = bool(header & 0x40)
        if is_definition:
            if pos + 6 > len(records):
                return None
            big_endian = records[pos + 2] == 1
            global_num = struct.unpack(">H" if big_endian else "<H",
                                       records[pos + 3:pos + 5])[0]
            n_fields = records[pos + 5]
            end = pos + 6 + 3 * n_fields
            if end > len(records):
                return None
            size = sum(records[pos + 6 + 3 * i + 1] for i in range(n_fields))
            if header & 0x20:  # developer fields follow
                if end >= len(records):
                    return None
                n_dev = records[end]
                dev_end = end + 1 + 3 * n_dev
                if dev_end > len(records):
                    return None
                size += sum(records[end + 1 + 3 * i + 1] for i in range(n_dev))
                end = dev_end
            sizes[local] = size
            globals_[local] = global_num
            out += records[pos:end]
            pos = end
            continue
        if local not in sizes:
            return None
        end = pos + 1 + sizes[local]
        if end > len(records):
            return None
        if globals_[local] != SET_MESG_NUM:
            out += records[pos:end]
        pos = end
    return bytes(out)


def replace_sets(fit_bytes: bytes, sets: list[dict]) -> bytes:
    """`fit_bytes` with its ``set`` messages replaced by `sets`.

    Any file this can't safely rewrite (not FIT, chained files, a truncated
    record) comes back unchanged — Strava then gets the watch's own sets,
    exactly as before.
    """
    if not sets or len(fit_bytes) < 14:
        return fit_bytes
    header_size = fit_bytes[0]
    if header_size not in (12, 14) or fit_bytes[8:12] != b".FIT":
        return fit_bytes
    data_size = struct.unpack("<I", fit_bytes[4:8])[0]
    if header_size + data_size + 2 != len(fit_bytes):
        return fit_bytes  # chained or truncated — leave it alone
    stripped = _strip_sets(fit_bytes[header_size:header_size + data_size])
    if stripped is None:
        return fit_bytes

    records = stripped + _encoded_messages(sets)
    header = bytearray(fit_bytes[:header_size])
    header[4:8] = struct.pack("<I", len(records))
    if header_size == 14:
        header[12:14] = struct.pack(
            "<H", CrcCalculator.calculate_crc(header, 0, 12))
    body = bytes(header) + records
    return body + struct.pack("<H", CrcCalculator.calculate_crc(body, 0, len(body)))


def with_current_sets(fit_bytes: bytes, exercise_sets: object) -> bytes:
    """The Strava upload for a strength activity: Garmin's current sets in
    place of the watch's originals, or the original bytes when there are
    none to carry."""
    sets = fit_sets_from_garmin(exercise_sets)
    if not any(s["set_type"] == "active" for s in sets):
        return fit_bytes
    return replace_sets(fit_bytes, sets)
