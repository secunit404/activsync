"""Garmin-synced user profile: 24 h cache, override precedence, fallbacks."""

from datetime import datetime, timedelta, timezone
from unittest.mock import MagicMock

import pytest

from activsync import db, hevy_profile

NOW = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)

FETCHED = {"weight_kg": 72.5, "birth_year": 1988, "sex": "female", "vo2max": 52.0}


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def _garmin(profile=None, fail=False):
    garmin = MagicMock()
    if fail:
        garmin.fetch_user_profile.side_effect = RuntimeError("garmin down")
    else:
        garmin.fetch_user_profile.return_value = dict(profile or FETCHED)
    return garmin


def _seed_cache(conn, fetched_at, **values):
    settings = db.get_config_value(conn, "settings", default={}) or {}
    settings["garmin_user_profile"] = {**values, "fetched_at": fetched_at.isoformat()}
    db.set_config_value(conn, "settings", settings)


def test_fetch_populates_cache_and_returns_profile(conn):
    garmin = _garmin()

    profile = hevy_profile.get_profile(conn, garmin, NOW)

    assert profile.weight_kg == 72.5
    assert profile.birth_year == 1988
    assert profile.sex == "female"
    assert profile.vo2max == 52.0
    cached = db.get_config_value(conn, "settings")["garmin_user_profile"]
    assert cached["fetched_at"] == NOW.isoformat()
    assert cached["weight_kg"] == 72.5


def test_fresh_cache_skips_the_garmin_fetch(conn):
    _seed_cache(conn, NOW - timedelta(hours=23), weight_kg=70.0)
    garmin = _garmin()

    profile = hevy_profile.get_profile(conn, garmin, NOW)

    assert garmin.fetch_user_profile.call_count == 0
    assert profile.weight_kg == 70.0


def test_stale_cache_refetches(conn):
    _seed_cache(conn, NOW - timedelta(hours=25), weight_kg=70.0)
    garmin = _garmin()

    profile = hevy_profile.get_profile(conn, garmin, NOW)

    assert garmin.fetch_user_profile.call_count == 1
    assert profile.weight_kg == 72.5


def test_override_wins_field_by_field(conn):
    _seed_cache(conn, NOW - timedelta(hours=1), weight_kg=70.0, birth_year=1985)
    settings = db.get_config_value(conn, "settings")
    settings["profile_override"] = {"weight_kg": 90.0}
    db.set_config_value(conn, "settings", settings)

    profile = hevy_profile.get_profile(conn, _garmin(), NOW)

    assert profile.weight_kg == 90.0, "override field must win"
    assert profile.birth_year == 1985, "non-overridden field comes from the cache"


def test_fetch_failure_falls_back_to_stale_cache(conn):
    _seed_cache(conn, NOW - timedelta(days=3), weight_kg=70.0, sex="male")

    profile = hevy_profile.get_profile(conn, _garmin(fail=True), NOW)

    assert profile.weight_kg == 70.0
    assert profile.sex == "male"


def test_fetch_failure_without_cache_falls_back_to_defaults(conn, caplog):
    import logging

    with caplog.at_level(logging.WARNING, logger="activsync.hevy_profile"):
        profile = hevy_profile.get_profile(conn, _garmin(fail=True), NOW)

    assert (profile.weight_kg, profile.birth_year, profile.vo2max, profile.sex) == (
        80.0, 1990, 45.0, "male")
    assert any("profile" in r.getMessage() for r in caplog.records), \
        "falling back to defaults must warn"


def test_partial_fetch_fills_missing_fields_with_defaults(conn):
    garmin = _garmin(profile={"weight_kg": 75.0, "birth_year": None,
                              "sex": None, "vo2max": None})

    profile = hevy_profile.get_profile(conn, garmin, NOW)

    assert profile.weight_kg == 75.0
    assert profile.birth_year == 1990
    assert profile.vo2max == 45.0
    assert profile.sex == "male"
