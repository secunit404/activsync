"""User physiology profile for FIT calorie estimation, synced from Garmin.

Cached in its own app_config row under `garmin_user_profile`, refreshed daily.
Manual override values in `profile_override` win field-by-field; hard-coded
defaults are the last resort.
"""

from __future__ import annotations

import logging
import sqlite3
from datetime import datetime, timedelta, timezone

from activsync import db
from activsync.fit_builder import Profile

logger = logging.getLogger("activsync.hevy_profile")

CACHE_KEY = "garmin_user_profile"
OVERRIDE_KEY = "profile_override"
CACHE_MAX_AGE = timedelta(hours=24)

PROFILE_DEFAULTS = {"weight_kg": 80.0, "birth_year": 1990, "vo2max": 45.0,
                    "sex": "male"}


def _parse_fetched_at(raw: str | None) -> datetime | None:
    if not raw:
        return None
    try:
        parsed = datetime.fromisoformat(raw)
    except (ValueError, TypeError):
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def _refresh_cache(conn: sqlite3.Connection, garmin, now: datetime) -> dict:
    """Fetch from Garmin and persist; returns the new cache entry.
    Raises on fetch failure — the caller decides the fallback."""
    fetched = garmin.fetch_user_profile()
    entry = {key: fetched.get(key) for key in PROFILE_DEFAULTS
             if fetched.get(key) is not None}
    entry["fetched_at"] = now.isoformat()
    # Cache state has its own row: a slow Garmin request must never write an
    # old snapshot of the user's settings over a concurrent form save.
    db.set_config_value(conn, CACHE_KEY, entry)
    return entry


def get_profile(conn: sqlite3.Connection, garmin, now: datetime) -> Profile:
    """The profile used for calorie estimation. Never raises: falls back to
    the cached values, then to defaults, logging a warning on the way down."""
    cache = db.get_config_value(conn, CACHE_KEY, default={}) or {}
    fetched_at = _parse_fetched_at(cache.get("fetched_at"))

    if fetched_at is None or now - fetched_at >= CACHE_MAX_AGE:
        try:
            cache = _refresh_cache(conn, garmin, now)
        except Exception as exc:
            if cache:
                logger.warning(
                    "garmin profile fetch failed (%s); using cached profile", exc)
            else:
                logger.warning(
                    "garmin profile fetch failed (%s); using default profile", exc)

    # Read overrides after the potentially slow fetch, so a settings save that
    # completed while Garmin was responding applies to this very build.
    settings = db.get_config_value(conn, "settings", default={}) or {}
    override = settings.get(OVERRIDE_KEY) or {}
    merged = dict(PROFILE_DEFAULTS)
    for source in (cache, override):
        for key in PROFILE_DEFAULTS:
            if source.get(key) is not None:
                merged[key] = source[key]

    return Profile(weight_kg=float(merged["weight_kg"]),
                   birth_year=int(merged["birth_year"]),
                   vo2max=float(merged["vo2max"]),
                   sex=str(merged["sex"]))
