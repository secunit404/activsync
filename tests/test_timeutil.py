from datetime import datetime, timezone

import pytest

from activsync import timeutil


def test_to_local_converts_utc_to_configured_timezone():
    result = timeutil.to_local("2026-07-09 09:00:00", "Europe/Stockholm")
    assert result.hour == 11


def test_to_local_now_returns_current_time_in_target_zone():
    before = datetime.now(timezone.utc)
    result = timeutil.to_local_now("Europe/Stockholm")
    after = datetime.now(timezone.utc)

    assert before <= result.astimezone(timezone.utc) <= after
    assert result.tzinfo is not None
    assert result.utcoffset() != timezone.utc.utcoffset(None)


def test_format_local_time_converts_to_stockholm_summer_time():
    result = timeutil.format_local_time("2026-07-09 09:00:00", "Europe/Stockholm")
    assert result == "2026-07-09 11:00"


def test_format_local_time_converts_to_stockholm_winter_time():
    result = timeutil.format_local_time("2026-01-09 09:00:00", "Europe/Stockholm")
    assert result == "2026-01-09 10:00"


def test_format_local_time_converts_to_other_timezone():
    result = timeutil.format_local_time("2026-07-09 20:00:00", "America/New_York")
    assert result == "2026-07-09 16:00"


def test_is_valid_timezone_accepts_known_zone():
    assert timeutil.is_valid_timezone("Europe/Stockholm") is True


def test_is_valid_timezone_rejects_unknown_zone():
    assert timeutil.is_valid_timezone("Not/AZone") is False


@pytest.mark.parametrize("raw,expected", [
    ("2026-07-18T10:00:00Z", datetime(2026, 7, 18, 10, 0, tzinfo=timezone.utc)),
    ("2026-07-18T10:00:00+00:00", datetime(2026, 7, 18, 10, 0, tzinfo=timezone.utc)),
    ("2026-07-18T12:00:00+02:00", datetime(2026, 7, 18, 10, 0, tzinfo=timezone.utc)),
    # Naive input is read as UTC, not as local time.
    ("2026-07-18T10:00:00", datetime(2026, 7, 18, 10, 0, tzinfo=timezone.utc)),
    # Garmin's space-separated form, always UTC.
    ("2026-07-18 10:00:00", datetime(2026, 7, 18, 10, 0, tzinfo=timezone.utc)),
    # A bare date, which the backfill start field accepts.
    ("2026-07-18", datetime(2026, 7, 18, 0, 0, tzinfo=timezone.utc)),
    ("  2026-07-18T10:00:00Z  ", datetime(2026, 7, 18, 10, 0, tzinfo=timezone.utc)),
])
def test_parse_timestamp_normalises_every_accepted_form(raw, expected):
    assert timeutil.parse_timestamp(raw) == expected


@pytest.mark.parametrize("raw", ["", "   ", None, 123, [], "garbage", "2026-13-45"])
def test_parse_timestamp_returns_none_for_anything_unusable(raw):
    """None means "no value" — every caller treats it that way rather than
    raising, so a malformed field must never propagate an exception."""
    assert timeutil.parse_timestamp(raw) is None
