"""Adaptive Garmin polling: fast right after training, configured interval otherwise."""

from datetime import datetime, timedelta, timezone
from unittest.mock import MagicMock

import pytest

from activsync import config, db, hevy_db, poll_activity
from activsync.poller import Poller

NOW = datetime(2026, 9, 29, 18, 0, tzinfo=timezone.utc)


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def _garmin_time(dt: datetime) -> str:
    return dt.strftime("%Y-%m-%d %H:%M:%S")


def _seed_activity(conn, activity_id, start, duration_s):
    db.insert_activity(
        conn, activity_id, "running", "Run", "", _garmin_time(start),
        f"hash-{activity_id}", "pending", NOW,
        garmin_data=f'{{"duration": {duration_s}}}',
    )


def _seed_hevy_workout(conn, hevy_id, end, status=None):
    start = end - timedelta(hours=1)
    hevy_db.upsert_workout(
        conn, hevy_id, "Push", start.isoformat(), end.isoformat(),
        end.isoformat(), {"id": hevy_id},
    )
    if status is not None:
        hevy_db.set_workout_status(conn, hevy_id, status)


def test_idle_without_any_recent_training(conn):
    _seed_activity(conn, 1, NOW - timedelta(days=2), 3600)

    assert poll_activity.recently_active(conn, NOW) is False


def test_activity_that_ended_within_the_window_is_active(conn):
    _seed_activity(conn, 1, NOW - timedelta(hours=2), 3600)

    assert poll_activity.recently_active(conn, NOW) is True


def test_long_activity_that_started_long_ago_but_just_ended_is_active(conn):
    _seed_activity(conn, 1, NOW - timedelta(hours=7), 6.5 * 3600)

    assert poll_activity.recently_active(conn, NOW) is True


def test_activity_that_ended_before_the_window_is_idle(conn):
    _seed_activity(conn, 1, NOW - timedelta(hours=4), 3600)

    assert poll_activity.recently_active(conn, NOW) is False


def test_recent_hevy_workout_is_active(conn):
    _seed_hevy_workout(conn, "w1", NOW - timedelta(minutes=30), status="merged")

    assert poll_activity.recently_active(conn, NOW) is True


def test_hevy_workout_waiting_for_its_watch_activity_is_active(conn):
    _seed_hevy_workout(conn, "w1", NOW - timedelta(hours=5), status="waiting_watch")

    assert poll_activity.recently_active(conn, NOW) is True


def test_old_finished_hevy_workout_is_idle(conn):
    _seed_hevy_workout(conn, "w1", NOW - timedelta(hours=5), status="merged")

    assert poll_activity.recently_active(conn, NOW) is False


def _poller(conn, override=None):
    return Poller(
        conn,
        garmin_factory=lambda: MagicMock(),
        strava_factory=lambda: MagicMock(),
        garmin_interval_seconds_override=override,
    )


def test_garmin_interval_drops_to_the_active_interval_after_training(conn):
    _seed_activity(conn, 1, NOW - timedelta(hours=1), 1800)

    interval = _poller(conn)._garmin_interval_seconds(NOW)

    assert interval == poll_activity.ACTIVE_GARMIN_INTERVAL_SECONDS


def test_garmin_interval_uses_the_configured_value_when_idle(conn):
    cfg = config.load_config(conn)

    interval = _poller(conn)._garmin_interval_seconds(NOW)

    assert interval == cfg["garmin_poll_interval_minutes"] * 60


def test_active_interval_never_slows_a_faster_configured_interval(conn):
    _seed_activity(conn, 1, NOW - timedelta(hours=1), 1800)
    cfg = config.load_config(conn)
    cfg["garmin_poll_interval_minutes"] = 2
    config.save_config(conn, cfg)

    assert _poller(conn)._garmin_interval_seconds(NOW) == 120


def test_explicit_override_bypasses_adaptive_polling(conn):
    _seed_activity(conn, 1, NOW - timedelta(hours=1), 1800)

    assert _poller(conn, override=100000)._garmin_interval_seconds(NOW) == 100000
