"""The dev fakes must stay drop-in replacements for the real clients.

Every other test mocks the clients with MagicMock, which answers any call at
all — so nothing here was catching the fakes drifting out of sync with the
interface `sync` actually uses. That drift is invisible until `make dev`
raises AttributeError at runtime.
"""

from datetime import datetime, timedelta, timezone

import pytest

from activsync import config, db, dev_mock, sync
from activsync.garmin_client import GarminClient
from activsync.strava_client import StravaClient

NOW = datetime(2026, 7, 9, 12, 0, tzinfo=timezone.utc)


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def _public_methods(cls) -> set[str]:
    return {
        name for name, value in vars(cls).items()
        if not name.startswith("_") and callable(value)
    }


def test_fake_strava_client_implements_the_real_strava_surface():
    missing = _public_methods(StravaClient) - _public_methods(dev_mock.FakeStravaClient)

    assert not missing, f"FakeStravaClient is missing {sorted(missing)} — mock mode will crash"


def test_fake_garmin_client_implements_the_real_garmin_surface():
    missing = _public_methods(GarminClient) - _public_methods(dev_mock.FakeGarminClient)

    assert not missing, f"FakeGarminClient is missing {sorted(missing)} — mock mode will crash"


def test_check_strava_status_runs_against_the_fake_client(conn):
    """The regression: check_strava_status called list_activities_between, which
    the fake did not have, so every poll in mock mode raised AttributeError."""
    db.insert_activity(conn, 1, "running", "Run", "", "2026-07-09 09:00:00", "h", "pending", NOW)

    stats = sync.check_strava_status(
        conn, dev_mock.FakeStravaClient(conn), dict(config.DEFAULT_CONFIG), NOW,
    )

    assert stats.flagged_missing == 0
    assert stats.linked_existing == 0


def test_fake_client_never_flags_a_published_activity_as_missing(conn):
    """Nothing published through the mock exists on real Strava, so a window
    fetch that forgot about them would flag the lot as deleted."""
    db.insert_activity(conn, 2, "running", "Run", "", "2026-07-09 09:00:00", "h", "published", NOW)
    db.set_published(conn, 2, strava_activity_id=9_000_002, now=NOW)

    stats = sync.check_strava_status(
        conn, dev_mock.FakeStravaClient(conn), dict(config.DEFAULT_CONFIG), NOW,
    )

    assert db.get_activity(conn, 2)["publish_status"] == "published"
    assert stats.flagged_missing == 0


def test_fake_client_window_excludes_activities_outside_the_range(conn):
    old_start = (NOW - timedelta(days=30)).strftime("%Y-%m-%d %H:%M:%S")
    db.insert_activity(conn, 3, "running", "Old", "", old_start, "h", "published", NOW)
    db.set_published(conn, 3, strava_activity_id=9_000_003, now=NOW)
    db.insert_activity(conn, 4, "running", "New", "", "2026-07-09 09:00:00", "h2", "published", NOW)
    db.set_published(conn, 4, strava_activity_id=9_000_004, now=NOW)

    window = dev_mock.FakeStravaClient(conn).list_activities_between(
        NOW - timedelta(days=7), NOW,
    )

    assert [a["id"] for a in window] == [9_000_004]
    assert all(isinstance(a["start_date"], datetime) for a in window)


def test_publish_pending_runs_end_to_end_against_the_fakes(conn):
    db.insert_activity(conn, 5, "running", "Run", "", "2026-07-09 09:00:00", "h", "pending", NOW)

    stats = sync.publish_pending(
        conn, dev_mock.FakeGarminClient(conn), dev_mock.FakeStravaClient(conn), NOW,
    )

    assert stats.published == 1
    assert stats.failed == 0
    assert db.get_activity(conn, 5)["publish_status"] == "published"
