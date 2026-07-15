"""Timezone conversion for displaying stored UTC activity times."""

from __future__ import annotations

from datetime import datetime, timezone
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError, available_timezones

GARMIN_TIME_FORMAT = "%Y-%m-%d %H:%M:%S"

# Timezones most likely to be useful, grouped by region and sorted roughly
# west-to-east. Full list at https://en.wikipedia.org/wiki/List_of_tz_database_time_zones
_COMMON_TIMEZONES = [
    "Pacific/Honolulu",
    "America/Anchorage",
    "America/Los_Angeles",
    "America/Denver",
    "America/Chicago",
    "America/New_York",
    "America/Halifax",
    "America/St_Johns",
    "America/Sao_Paulo",
    "Atlantic/Azores",
    "UTC",
    "Europe/London",
    "Europe/Paris",
    "Europe/Berlin",
    "Europe/Stockholm",
    "Europe/Helsinki",
    "Europe/Moscow",
    "Asia/Dubai",
    "Asia/Kolkata",
    "Asia/Bangkok",
    "Asia/Singapore",
    "Asia/Shanghai",
    "Asia/Tokyo",
    "Asia/Seoul",
    "Australia/Sydney",
    "Pacific/Auckland",
    "Pacific/Fiji",
]


def is_valid_timezone(tz_name: str) -> bool:
    try:
        ZoneInfo(tz_name)
        return True
    except (ZoneInfoNotFoundError, ValueError):
        return False


def common_timezones() -> list[str]:
    """Curated list of common IANA timezones for the Settings dropdown."""
    # Filter to only zones actually available in the running environment
    # (in case tzdata isn't installed on a bare system).
    return [tz for tz in _COMMON_TIMEZONES if is_valid_timezone(tz)]


def format_local_time(start_time: str, tz_name: str) -> str:
    """Convert a Garmin UTC start_time string ("%Y-%m-%d %H:%M:%S") to a
    display string in tz_name, e.g. "2026-07-09 11:00"."""
    local_dt = _to_local(start_time, tz_name)
    return local_dt.strftime("%Y-%m-%d %H:%M")


def _to_local(start_time: str, tz_name: str) -> datetime:
    dt_utc = datetime.strptime(start_time, GARMIN_TIME_FORMAT).replace(tzinfo=timezone.utc)
    return dt_utc.astimezone(ZoneInfo(tz_name))


def format_local_date(start_time: str, tz_name: str) -> str:
    """Format the local activity date compactly, e.g. ``9 Jul``."""
    local_dt = _to_local(start_time, tz_name)
    return f"{local_dt.day} {local_dt.strftime('%b')}"


def format_local_year(start_time: str, tz_name: str) -> str:
    """Format the local activity year."""
    return str(_to_local(start_time, tz_name).year)


def format_local_month_year(start_time: str, tz_name: str) -> str:
    """Format the local activity month and year, e.g. ``July 2026``."""
    return _to_local(start_time, tz_name).strftime("%B %Y")


def format_local_clock(start_time: str, tz_name: str) -> str:
    """Format the local activity time without its date, e.g. ``11:00``."""
    return _to_local(start_time, tz_name).strftime("%H:%M")
