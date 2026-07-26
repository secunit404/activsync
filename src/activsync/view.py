"""Presentation helpers for the activity list: timezone display, external links."""

from __future__ import annotations

import json
from datetime import datetime, timezone

import sqlite3

from activsync import config, db, timeutil

GARMIN_ACTIVITY_URL = "https://connect.garmin.com/modern/activity/{}"
STRAVA_ACTIVITY_URL = "https://www.strava.com/activities/{}"

_GARMIN_EXERCISE_ACRONYMS = {"BOSU", "EZ", "GHD", "HIIT", "KBS", "RDL", "TRX"}


def garmin_exercise_label(enum_name: str) -> str:
    """Turn a Garmin FIT enum identifier into a sentence-case UI label.

    The raw enum name must stay unchanged where it is sent back to Garmin.
    FIT prefixes identifiers that begin with a number with ``N`` so they are
    valid Python names (for example ``N45_DEGREE_PLANK``); that implementation
    detail should not leak into the UI either.
    """
    words = enum_name.split("_")
    if words and len(words[0]) > 1 and words[0][0] == "N" and words[0][1:].isdigit():
        words[0] = words[0][1:]

    words = [word if word in _GARMIN_EXERCISE_ACRONYMS else word.lower() for word in words]
    label = " ".join(words)
    return label[:1].upper() + label[1:]


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


def fmt_duration_coarse(seconds: float | None) -> str:
    """Whole-minute duration for summary tiles: '7h 42m', '42m', '0m'."""
    total = int(seconds or 0)
    h, m = divmod(total // 60, 60)
    if h:
        return f"{h}h {m:02d}m"
    return f"{m}m"


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
    tz_name: str | None = None,
) -> list[dict]:
    """Activity rows augmented with display-only fields.

    ``tz_name`` lets a caller that already loaded config for the same
    request pass the display timezone through instead of triggering a
    second ``config.load_config`` call.
    """
    if tz_name is None:
        tz_name = config.load_config(conn)["display_timezone"]
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
            "duration_seconds": duration,
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


_HEVY_IN_FLIGHT = ("waiting_watch", "awaiting_match", "syncing")
_HEVY_PROBLEMS = ("needs_mapping", "failed", "needs_review")


def _hevy_time_display(start_time: str, tz_name: str) -> str:
    """Hevy rows carry ISO timestamps; activities carry Garmin's space-separated
    form. Normalize to the latter so both render through the same formatter."""
    cleaned = (start_time or "").replace("T", " ").split("+")[0].rstrip("Z")
    try:
        return timeutil.format_local_time(cleaned, tz_name)
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
                matched_activity = (
                    db.get_activity(conn, row["source_garmin_activity_id"])
                    if row["source_garmin_activity_id"] is not None
                    else None
                )
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
                    "awaiting_match": status == "awaiting_match",
                    "matched_garmin_activity_id": row["source_garmin_activity_id"],
                    "matched_garmin_title": (
                        matched_activity["title"] if matched_activity else None
                    ),
                    "matched_strava_activity_id": (
                        matched_activity["strava_activity_id"]
                        if matched_activity
                        and matched_activity.get("publish_status") == "published"
                        else None
                    ),
                    "matched_strava_url": (
                        STRAVA_ACTIVITY_URL.format(
                            matched_activity["strava_activity_id"]
                        )
                        if matched_activity
                        and matched_activity.get("publish_status") == "published"
                        and matched_activity.get("strava_activity_id") is not None
                        else None
                    ),
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

    cache = db.get_config_value(conn, hevy_profile.CACHE_KEY, default={}) or {}
    override = settings.get(hevy_profile.OVERRIDE_KEY) or {}
    # What applies when a field is left blank: Garmin's cached values, else
    # the built-in defaults. The UI shows these as the inputs' placeholders.
    baseline = dict(hevy_profile.PROFILE_DEFAULTS)
    for key in hevy_profile.PROFILE_DEFAULTS:
        if cache.get(key) is not None:
            baseline[key] = cache[key]
    from_garmin = any(cache.get(key) is not None
                      for key in hevy_profile.PROFILE_DEFAULTS)

    # Two distinct things: what the app detected (or generated) for itself,
    # and what the user pinned by hand. The detected value must survive a
    # settings save that leaves the override blank.
    detected = settings.get("hevy_device_identity") or {}
    if detected:
        numbers = "{}/{}/{}".format(detected.get("manufacturer"),
                                    detected.get("product"),
                                    detected.get("serial"))
        # product 0 is the generic-Garmin / per-install fallback shape; a real
        # watch FIT always carries a product number.
        if detected.get("product"):
            identity_display = f"detected from your watch: {numbers}"
        else:
            identity_display = f"generic Garmin fallback: {numbers}"
    else:
        identity_display = "not yet detected — set automatically from your watch"

    return {
        "api_key_saved": bool(api_key),
        "connected": bool(api_key) and auth_ok is not False,
        "status": status,
        "profile_baseline": baseline,
        "profile_from_garmin": from_garmin,
        "profile_override": override,
        "profile_fetched_at": cache.get("fetched_at"),
        "identity": settings.get("hevy_device_identity_override") or {},
        "identity_display": identity_display,
        "last_success_at": db.get_config_value(conn, "hevy_last_success_at"),
    }


def hevy_mappings_view(conn: sqlite3.Connection) -> list[dict]:
    """Every known exercise template, with where it lands in Garmin.

    The point of the integration is that every Hevy exercise maps to a Garmin
    one, so this returns them all — including built-ins the ported tables
    resolve without help. Those used to be skipped as "nothing to configure",
    which also meant they could never be inspected or overridden; the mapping
    screen's search and filter are what keep the longer list usable.

    `source` says who decided the pair:
      "user"      — a saved mapping (an override, or the answer to a miss)
      "automatic" — resolved by the ported template/name tables
      ""          — nothing resolves it yet; this is the needs-mapping case
    """
    from activsync import hevy_db
    from activsync.hevy_mapper import (
        CATEGORY_NAMES,
        SUBCATEGORY_NAMES,
        MappingMiss,
        lookup_exercise,
        lookup_standard_mapping,
        suggest_mapping,
    )

    mappings = {m["exercise_template_id"]: m for m in hevy_db.list_mappings(conn)}
    rows: list[dict] = []
    for template in hevy_db.list_templates(conn):
        template_id = template["exercise_template_id"]
        mapping = mappings.get(template_id)
        standard_mapping = lookup_standard_mapping(
            template["title"], template_id
        )

        # Ask the resolver rather than reimplementing its precedence here —
        # it is the same call the sync path makes, so the screen can never
        # disagree with what actually gets written.
        try:
            category, subcategory, _ = lookup_exercise(
                conn, template["title"], template_id
            )
            resolves = True
        except MappingMiss:
            category = subcategory = None
            resolves = False

        if resolves and mapping is not None:
            source = "user"
        elif resolves:
            source = "automatic"
        else:
            source = ""

        # Only offer a suggestion where nothing resolves — it is a hint for
        # filling the gap, not a competing answer.
        suggestion = (
            suggest_mapping(template["title"], template) if not resolves else None
        )
        if suggestion is not None:
            category, subcategory = suggestion

        rows.append({
            "template_id": template_id,
            "title": template["title"],
            "is_custom": bool(template["is_custom"]),
            "muscle_group": template.get("primary_muscle_group") or "",
            "mapped": resolves,
            "unmapped": not resolves,
            "suggested": suggestion is not None,
            "source": source,
            "has_standard_mapping": standard_mapping is not None,
            "standard_category": (
                standard_mapping[0] if standard_mapping is not None else None
            ),
            "standard_subcategory": (
                standard_mapping[1] if standard_mapping is not None else None
            ),
            "category": category,
            "subcategory": subcategory,
            "category_name": (
                garmin_exercise_label(CATEGORY_NAMES[category])
                if category is not None and category in CATEGORY_NAMES
                else None
            ),
            "subcategory_name": (
                garmin_exercise_label(SUBCATEGORY_NAMES[category][subcategory])
                if (
                    category is not None
                    and subcategory is not None
                    and subcategory in SUBCATEGORY_NAMES.get(category, {})
                )
                else None
            ),
        })
    # What needs doing first, then custom exercises (the ones a user actually
    # invented), then alphabetical.
    rows.sort(key=lambda r: (
        not r["unmapped"],
        not r["is_custom"],
        r["title"].lower(),
    ))
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
