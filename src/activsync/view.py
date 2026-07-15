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


def garmin_status(conn: sqlite3.Connection, now: datetime | None = None) -> dict:
    """Garmin connection status for the Settings page, derived from the
    outcome of the most recent sync_garmin() attempt (poller or manual)."""
    last_sync_at = db.get_config_value(conn, "garmin_last_sync_at")
    if last_sync_at is None:
        return {"state": "not_synced", "message": "Not yet synced"}

    synced_at = datetime.fromisoformat(last_sync_at)
    now = now or datetime.now(timezone.utc)

    if db.get_config_value(conn, "garmin_last_sync_ok"):
        age_minutes = max(int((now - synced_at).total_seconds() // 60), 0)
        return {"state": "connected", "message": f"Connected — last synced {age_minutes} min ago"}

    error = db.get_config_value(conn, "garmin_last_sync_error") or "unknown error"
    return {
        "state": "needs_attention",
        "message": f"Needs attention — last attempt at {synced_at.strftime('%H:%M')} failed: {error}",
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

    garmin_detail = (
        garmin_status(conn, now)["message"] if garmin_connected
        else "Disconnected — sync paused"
    )
    broken = [
        name for name, ok in (("garmin", garmin_connected), ("strava", strava_connected))
        if not ok
    ]
    return {
        "garmin": {
            "connected": garmin_connected,
            "detail": garmin_detail,
            "email": creds.get("email", ""),
        },
        "strava": {
            "connected": strava_connected,
            "detail": "Connected" if strava_connected else "Disconnected — publishing paused",
        },
        "broken": broken,
    }
