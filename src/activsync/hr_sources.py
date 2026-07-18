"""Heart-rate sourcing for generated FITs.

Ported from hevy2garmin's hr.py (MIT), reduced to ActivSync's needs: watch-FIT
1 s HR extraction (replace path), daily passive wrist HR sliced to the workout
window — fetching every calendar date the window touches, so midnight-spanning
sessions keep both halves (passive path) — and the priority merge. Everything
is best-effort: HR must never break a sync, so failures return [] and the
caller uploads without HR (default-HR calories, visibly flagged).

All functions return [{"time": secs_from_start, "hr": bpm}] sorted by time.
The Hevy-provided-HR seam from upstream is intentionally not ported — the
public Hevy API exposes no HR; merge_hr_sources keeps the slot open if that
ever changes.
"""

from __future__ import annotations

import logging
from datetime import datetime, timedelta

logger = logging.getLogger("activsync.hr_sources")

_WINDOW_BUFFER_MS = 60_000  # ±1 min around the workout window


def extract_fit_hr(fit_bytes: bytes, workout_start_ms: int) -> list[dict]:
    """High-resolution HR from a watch FIT's RecordMessages, as offsets from
    the workout start. [] on any parse failure."""
    from fit_tool.fit_file import FitFile
    from fit_tool.profile.messages.record_message import RecordMessage

    # Device FITs carry fields newer than fit_tool's bundled profile; they are
    # skipped safely but logged once per record, flooding sync output.
    fit_logger = logging.getLogger("fit_tool")
    previous_level = fit_logger.level
    fit_logger.setLevel(logging.ERROR)
    try:
        fit_file = FitFile.from_bytes(fit_bytes, check_crc=False)
    except Exception as exc:
        logger.debug("watch FIT parse failed: %s", exc)
        return []
    finally:
        fit_logger.setLevel(previous_level)

    samples: list[dict] = []
    for record in fit_file.records:
        message = record.message
        if not isinstance(message, RecordMessage):
            continue
        timestamp = getattr(message, "timestamp", None)
        bpm = getattr(message, "heart_rate", None)
        if timestamp is None or bpm is None:
            continue
        try:
            timestamp_ms = float(timestamp)
            # Defensive: some FIT implementations return Unix seconds rather
            # than fit_tool's milliseconds.
            if timestamp_ms < 100_000_000_000:
                timestamp_ms *= 1000
            bpm_int = int(bpm)
        except (TypeError, ValueError):
            continue
        if timestamp_ms >= workout_start_ms and 0 < bpm_int < 256:
            samples.append({
                "time": (timestamp_ms - workout_start_ms) / 1000.0,
                "hr": bpm_int,
            })
    samples.sort(key=lambda sample: sample["time"])
    return samples


def _dates_touched(start_dt: datetime, end_dt: datetime) -> list[str]:
    """Every calendar date the window touches, in order."""
    dates: list[str] = []
    day = start_dt.date()
    while day <= end_dt.date():
        dates.append(day.isoformat())
        day += timedelta(days=1)
    return dates


def passive_hr_for_window(
    garmin, start_dt: datetime, end_dt: datetime
) -> list[dict]:
    """Garmin daily wrist HR sliced to [start-60s, end+60s]. Fetches every
    calendar date the window touches so midnight-spanning workouts keep both
    halves. [] on any failure — passive HR is best-effort."""
    start_ms = int(start_dt.timestamp() * 1000)
    end_ms = int(end_dt.timestamp() * 1000)

    samples: list[dict] = []
    for date_str in _dates_touched(start_dt, end_dt):
        try:
            daily = garmin.get_daily_heart_rates(date_str)
        except Exception as exc:
            logger.debug("daily HR fetch failed for %s: %s", date_str, exc)
            return []
        values = daily.get("heartRateValues") if isinstance(daily, dict) else None
        if not isinstance(values, list):
            continue
        for entry in values:
            if (isinstance(entry, (list, tuple)) and len(entry) >= 2
                    and entry[1] is not None):
                ts, bpm = entry[0], entry[1]
                try:
                    ts = float(ts)
                    bpm_int = int(bpm)
                except (TypeError, ValueError):
                    continue
                if start_ms - _WINDOW_BUFFER_MS <= ts <= end_ms + _WINDOW_BUFFER_MS:
                    samples.append({
                        "time": max(0.0, (ts - start_ms) / 1000.0),
                        "hr": bpm_int,
                    })
    samples.sort(key=lambda sample: sample["time"])
    return samples


def merge_hr_sources(
    primary: list[dict] | None,
    secondary: list[dict] | None,
    bucket_s: float = 10.0,
) -> list[dict]:
    """Merge two timestamped series: per 10 s bucket the primary sample wins,
    the secondary fills gaps. Ported verbatim from upstream."""
    primary = primary or []
    secondary = secondary or []
    if not primary:
        return sorted(secondary, key=lambda s: s["time"])
    if not secondary:
        return sorted(primary, key=lambda s: s["time"])

    chosen: dict[int, dict] = {}
    # Secondary first, then overwrite with primary so primary wins ties.
    for s in secondary:
        chosen[int(s["time"] // bucket_s)] = s
    for s in primary:
        chosen[int(s["time"] // bucket_s)] = s
    return [chosen[k] for k in sorted(chosen)]
