"""Presentation helpers for the activity list: timezone display, external links."""

from __future__ import annotations

import json
from datetime import datetime, timezone

import sqlite3

from activsync import config, db, timeutil

GARMIN_ACTIVITY_URL = "https://connect.garmin.com/modern/activity/{}"


def _parse_garmin_data(row: dict) -> dict:
    """Parse the garmin_data JSON column into a dict, tolerating invalid JSON."""
    try:
        return json.loads(row.get("garmin_data", "{}") or "{}")
    except (json.JSONDecodeError, TypeError):
        return {}


def _fmt_duration(seconds: float | None) -> str:
    if seconds is None:
        return ""
    total = int(seconds)
    h, m = divmod(total, 3600)
    m, s = divmod(m, 60)
    if h:
        return f"{h}h {m:02d}m {s:02d}s"
    return f"{m}m {s:02d}s"


def _fmt_distance(metres: float | None) -> str:
    if metres is None:
        return ""
    km = metres / 1000
    return f"{km:.2f} km"


def _fmt_pace(distance_m: float | None, duration_s: float | None) -> str:
    if not distance_m or not duration_s:
        return ""
    km = distance_m / 1000
    if km < 0.01:
        return ""
    sec_per_km = duration_s / km
    m, s = divmod(int(sec_per_km), 60)
    return f"{m}:{s:02d} /km"


def _fmt_speed(distance_m: float | None, duration_s: float | None) -> str:
    if not distance_m or not duration_s:
        return ""
    km = distance_m / 1000
    h = duration_s / 3600
    if h < 0.001:
        return ""
    return f"{km / h:.1f} km/h"


def _fmt_hr(hr: float | None) -> str:
    if hr is None:
        return ""
    return f"{int(hr)} bpm"


def _fmt_elev(metres: float | None) -> str:
    if metres is None:
        return ""
    return f"{int(metres)} m"


def _hevy_badges(conn: sqlite3.Connection) -> dict[int, str]:
    """garmin_activity_id → applied strategy, for every applied Hevy link
    (both the final linked activity and the claimed watch source)."""
    from activsync import hevy_db

    badges: dict[int, str] = {}
    for workout in hevy_db.list_workouts(conn):
        strategy = workout.get("applied_strategy")
        if not strategy:
            continue
        for column in ("garmin_activity_id", "source_garmin_activity_id"):
            activity_id = workout.get(column)
            if activity_id is not None:
                badges[activity_id] = strategy
    return badges


def activities_view(
    conn: sqlite3.Connection,
    sort_order: str = "newest",
    status_filter: str = "",
) -> list[dict]:
    """Activity rows augmented with display-only fields."""
    cfg = config.load_config(conn)
    tz_name = cfg["display_timezone"]
    rows = db.list_activities(
        conn,
        status=status_filter or None,
        sort_order=sort_order,
    )
    hevy_badges = _hevy_badges(conn)
    result: list[dict] = []
    for row in rows:
        gd = _parse_garmin_data(row)
        duration = gd.get("duration")
        distance = gd.get("distance")
        result.append({
            **row,
            "start_time_display": timeutil.format_local_time(row["start_time"], tz_name),
            "start_date_display": timeutil.format_local_date(row["start_time"], tz_name),
            "start_year_display": timeutil.format_local_year(row["start_time"], tz_name),
            "start_month_year_display": timeutil.format_local_month_year(row["start_time"], tz_name),
            "start_clock_display": timeutil.format_local_clock(row["start_time"], tz_name),
            "garmin_url": GARMIN_ACTIVITY_URL.format(row["garmin_activity_id"]),
            "hevy_badge": hevy_badges.get(row["garmin_activity_id"]),
            "detail": {
                "description": row.get("description") or None,
                "distance": _fmt_distance(distance),
                "duration": _fmt_duration(duration),
                "moving_time": _fmt_duration(gd.get("moving_duration")),
                "elapsed_time": _fmt_duration(gd.get("elapsed_duration")),
                "pace": _fmt_pace(distance, duration),
                "speed": _fmt_speed(distance, duration),
                "elev_gain": _fmt_elev(gd.get("elevation_gain")),
                "elev_loss": _fmt_elev(gd.get("elevation_loss")),
                "calories": f"{int(gd['calories'])}" if gd.get("calories") else "",
                "avg_hr": _fmt_hr(gd.get("avg_hr")),
                "max_hr": _fmt_hr(gd.get("max_hr")),
                "avg_power": f"{int(gd['avg_power'])} W" if gd.get("avg_power") else "",
                "max_power": f"{int(gd['max_power'])} W" if gd.get("max_power") else "",
                "norm_power": f"{int(gd['norm_power'])} W" if gd.get("norm_power") else "",
                "aerobic_te": f"{gd['aerobic_te']:.1f}" if gd.get("aerobic_te") else "",
                "anaerobic_te": f"{gd['anaerobic_te']:.1f}" if gd.get("anaerobic_te") else "",
                "training_load": f"{gd['training_load']:.0f}" if gd.get("training_load") else "",
                "avg_cadence": f"{int(gd['avg_cadence'])} spm" if gd.get("avg_cadence") else "",
                "max_cadence": f"{int(gd['max_cadence'])} spm" if gd.get("max_cadence") else "",
                "total_sets": str(gd["total_sets"]) if gd.get("total_sets") else "",
                "total_reps": str(gd["total_reps"]) if gd.get("total_reps") else "",
                "total_volume": f"{gd['total_volume']:.0f} kg" if gd.get("total_volume") else "",
            },
        })
    return result


_HEVY_IN_FLIGHT = ("waiting_watch", "syncing")
_HEVY_PROBLEMS = ("needs_mapping", "failed", "needs_review")


def _hevy_time_display(start_time: str, tz_name: str) -> str:
    try:
        return timeutil.format_local_time(start_time, tz_name)
    except Exception:
        return start_time or ""


def hevy_summary(conn: sqlite3.Connection) -> dict:
    """In-flight and problem Hevy workouts for the dashboard queue."""
    from activsync import hevy_db

    cfg = config.load_config(conn)
    tz_name = cfg["display_timezone"]

    def rows_for(statuses: tuple[str, ...]) -> list[dict]:
        rows: list[dict] = []
        for status in statuses:
            for row in hevy_db.list_workouts(conn, status=status):
                rows.append({
                    "hevy_id": row["hevy_id"],
                    "title": row["title"] or row["hevy_id"],
                    "status": status,
                    "error": row["error"],
                    "start_display": _hevy_time_display(row["start_time"], tz_name),
                    "needs_mapping": status == "needs_mapping",
                    "has_open_operation": (
                        hevy_db.get_open_operation(conn, row["hevy_id"]) is not None),
                    # The 404-tombstone case gets the "re-sync as fresh upload"
                    # action; everything else retries in place.
                    "resyncable": "deleted on Garmin" in (row["error"] or ""),
                })
        return rows

    in_flight = rows_for(_HEVY_IN_FLIGHT)
    problems = rows_for(_HEVY_PROBLEMS)
    skipped = rows_for(("skipped",))
    return {
        "enabled": bool(cfg.get("hevy_enabled")),
        "in_flight": in_flight,
        "problems": problems,
        "skipped": skipped,
        "counts": {"in_flight": len(in_flight), "problems": len(problems),
                   "skipped": len(skipped)},
    }


def hevy_settings_view(conn: sqlite3.Connection) -> dict:
    """Context for the settings page's Hevy card."""
    from activsync import hevy_profile

    settings = db.get_config_value(conn, "settings", default={}) or {}
    api_key = db.get_config_value(conn, "hevy_api_key")
    auth_ok = db.get_config_value(conn, "hevy_auth_ok")

    if not api_key:
        status = "Not connected"
    elif auth_ok is False:
        status = "Needs attention — Hevy rejected the API key"
    else:
        status = "Connected"

    cache = settings.get(hevy_profile.CACHE_KEY) or {}
    override = settings.get(hevy_profile.OVERRIDE_KEY) or {}
    profile = dict(hevy_profile.PROFILE_DEFAULTS)
    for source in (cache, override):
        for key in hevy_profile.PROFILE_DEFAULTS:
            if source.get(key) is not None:
                profile[key] = source[key]

    identity = settings.get("hevy_device_identity")
    if identity:
        numbers = "{}/{}/{}".format(identity.get("manufacturer"),
                                    identity.get("product"),
                                    identity.get("serial"))
        # product 0 is the generic-Garmin / per-install fallback shape; a real
        # watch FIT always carries a product number.
        if identity.get("product"):
            identity_display = f"detected from your watch: {numbers}"
        else:
            identity_display = f"generic Garmin fallback: {numbers}"
    else:
        identity_display = "not yet detected — set automatically from your watch"

    return {
        "api_key_saved": bool(api_key),
        "connected": bool(api_key) and auth_ok is not False,
        "status": status,
        "profile": profile,
        "profile_override": override,
        "profile_fetched_at": cache.get("fetched_at"),
        "identity": identity or {},
        "identity_display": identity_display,
        "last_success_at": db.get_config_value(conn, "hevy_last_success_at"),
    }


def hevy_mappings_view(conn: sqlite3.Connection) -> list[dict]:
    """Exercise templates joined with user mappings for the mappings section.

    Only rows a user can act on: custom templates, templates with a user
    mapping, and templates no built-in table resolves. Built-in templates that
    already resolve have nothing to configure and would drown the list."""
    from activsync import hevy_db
    from activsync.hevy_mapper import (
        CATEGORY_NAMES,
        SUBCATEGORY_NAMES,
        MappingMiss,
        lookup_exercise,
        suggest_mapping,
    )

    mappings = {m["exercise_template_id"]: m for m in hevy_db.list_mappings(conn)}
    rows: list[dict] = []
    for template in hevy_db.list_templates(conn):
        template_id = template["exercise_template_id"]
        mapping = mappings.get(template_id)
        resolves = True
        if mapping is None:
            try:
                lookup_exercise(conn, template["title"], template_id)
            except MappingMiss:
                resolves = False
        if not template["is_custom"] and mapping is None and resolves:
            continue

        suggestion = (suggest_mapping(template["title"], template)
                      if mapping is None else None)
        category = mapping["category"] if mapping else (
            suggestion[0] if suggestion else None)
        subcategory = mapping["subcategory"] if mapping else (
            suggestion[1] if suggestion else None)
        rows.append({
            "template_id": template_id,
            "title": template["title"],
            "is_custom": bool(template["is_custom"]),
            "muscle_group": template.get("primary_muscle_group") or "",
            "mapped": mapping is not None,
            "unmapped": mapping is None and not resolves,
            "garmin_rejected": bool(mapping and mapping["garmin_rejected"]),
            "suggested": mapping is None and suggestion is not None,
            "category": category,
            "subcategory": subcategory,
            "category_name": (CATEGORY_NAMES.get(category)
                              if category is not None else None),
            "subcategory_name": (SUBCATEGORY_NAMES.get(category, {}).get(subcategory)
                                 if category is not None and subcategory is not None
                                 else None),
        })
    rows.sort(key=lambda r: (not r["is_custom"], r["mapped"], r["title"].lower()))
    return rows


def garmin_status(conn: sqlite3.Connection, now: datetime | None = None) -> dict:
    """Garmin connection status for the Settings page, derived from the
    outcome of the most recent sync_garmin() attempt (poller or manual).

    `status` is the short label shown on the connection row; `meta` is the
    extra detail (sync age, failure info) shown in the smaller row beneath it,
    or "" when there's nothing more to say.
    """
    last_sync_at = db.get_config_value(conn, "garmin_last_sync_at")
    if last_sync_at is None:
        return {"state": "not_synced", "status": "Not yet synced", "meta": ""}

    synced_at = datetime.fromisoformat(last_sync_at)
    now = now or datetime.now(timezone.utc)

    if db.get_config_value(conn, "garmin_last_sync_ok"):
        age_minutes = max(int((now - synced_at).total_seconds() // 60), 0)
        return {"state": "connected", "status": "Connected", "meta": f"last synced {age_minutes} min ago"}

    error = db.get_config_value(conn, "garmin_last_sync_error") or "unknown error"
    return {
        "state": "needs_attention",
        "status": "Needs attention",
        "meta": f"last attempt at {synced_at.strftime('%H:%M')} failed: {error}",
    }


def connection_status(conn: sqlite3.Connection, now: datetime | None = None) -> dict:
    """The single source of truth for whether each service is usable.

    garmin_credentials_verified is the flag — sync_garmin sets it True on a
    successful fetch and False when the fetch is rejected, so a working sync is
    itself the proof. garmin_status() supplies human detail, never the verdict.
    """
    creds = db.get_config_value(conn, "garmin_credentials") or {}
    garmin_connected = bool(
        db.get_config_value(conn, "garmin_credentials_verified", default=False)
    )
    tokens = db.get_config_value(conn, "strava_tokens") or {}
    strava_connected = bool(tokens.get("refresh_token"))

    if garmin_connected:
        gs = garmin_status(conn, now)
        garmin_line = {"status": gs["status"], "meta": gs["meta"]}
    else:
        garmin_line = {"status": "Disconnected — sync paused", "meta": ""}

    broken = [
        name for name, ok in (("garmin", garmin_connected), ("strava", strava_connected))
        if not ok
    ]
    return {
        "garmin": {
            "connected": garmin_connected,
            "status": garmin_line["status"],
            "meta": garmin_line["meta"],
            "email": creds.get("email", ""),
        },
        "strava": {
            "connected": strava_connected,
            "status": "Connected" if strava_connected else "Disconnected — publishing paused",
            "meta": "",
        },
        "broken": broken,
    }
