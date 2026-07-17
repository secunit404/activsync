"""The dev fakes must stay drop-in replacements for the real clients.

Every other test mocks the clients with MagicMock, which answers any call at
all — so nothing here was catching the fakes drifting out of sync with the
interface `sync` actually uses. That drift is invisible until `make dev`
raises AttributeError at runtime.
"""

import os
from datetime import datetime, timedelta, timezone
from unittest.mock import MagicMock

import pytest

from activsync import config, db, dev_mock, dev_seed, sync
from activsync import server as server_module
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


def test_fake_publish_keeps_the_dev_request_visible(monkeypatch, conn):
    sleep = MagicMock()
    monkeypatch.setattr(dev_mock.time, "sleep", sleep)

    dev_mock.FakeStravaClient(conn).publish(5, b"")

    sleep.assert_called_once_with(dev_mock.DEV_PUBLISH_DELAY_SECONDS)


def test_fake_garmin_derives_activity_types_exactly_as_the_real_client_does(conn):
    """The fake ships keys and derives labels; the real client derives them from
    what Garmin sends. Run the real derivation over the same keys: if the two
    ever disagree, dev is rendering categories production never would."""
    raw_client = MagicMock()
    raw_client.get_activity_types.return_value = [
        {"typeKey": key} for key in dev_mock.GARMIN_ACTIVITY_TYPE_KEYS
    ]

    real = GarminClient(raw_client).fetch_activity_types()

    assert dev_mock.FakeGarminClient(conn).fetch_activity_types() == real


def test_fake_garmin_reports_the_taxonomy_rather_than_the_stored_list(conn):
    """Echoing the stored list back would make Refresh categories a no-op in
    dev: it could never disagree with the DB, so the one thing the button does
    would go unexercised."""
    db.set_config_value(conn, "garmin_activity_types", [
        {"type_key": "running", "label": "Running"},
    ])

    types = dev_mock.FakeGarminClient(conn).fetch_activity_types()

    assert len(types) > 1
    assert {"type_key": "yoga", "label": "Yoga"} in types


def test_dev_seed_stores_what_the_fake_garmin_reports(conn):
    dev_seed.seed(conn)

    assert (db.get_config_value(conn, "garmin_activity_types")
            == dev_mock.FakeGarminClient(conn).fetch_activity_types())


def test_dev_taxonomy_is_large_enough_to_exercise_the_collapsed_picker(conn):
    """The picker only collapses past 18 categories. A dev list under that
    renders a shape production never shows."""
    assert len(dev_mock.garmin_activity_types()) > 18


def test_dev_busts_the_stylesheet_cache_when_css_changes(tmp_path, monkeypatch):
    """Dev keys stylesheet URLs off file mtime, not the release version. The
    version cannot move while CSS is being edited and uvicorn's reloader only
    watches Python, so a version-keyed URL serves the stylesheet cached at the
    start of the session — and the browser renders something other than what is
    on disk, which quietly invalidates any visual check made against it."""
    monkeypatch.setattr(server_module, "_mock_mode", lambda: True)
    monkeypatch.setattr(server_module, "STATIC_DIR", tmp_path)
    css = tmp_path / "css" / "sheet.css"
    css.parent.mkdir()
    css.write_text("a{}")

    before = server_module._asset_version("sheet")
    os.utime(css, (1_000_000_000, 1_000_000_000))
    after = server_module._asset_version("sheet")

    assert before != after
    assert after != server_module.__version__


def test_asset_version_falls_back_to_the_version_for_an_unknown_sheet(monkeypatch, tmp_path):
    """A missing file must not 500 the whole page over a cache-busting token."""
    monkeypatch.setattr(server_module, "_mock_mode", lambda: True)
    monkeypatch.setattr(server_module, "STATIC_DIR", tmp_path)

    assert server_module._asset_version("nope") == server_module.__version__


def test_production_keys_stylesheets_to_the_release_version(monkeypatch):
    """Released images are immutable, so the version is the honest token there —
    and it stays stable across restarts instead of busting every deploy's cache
    for no reason."""
    monkeypatch.setattr(server_module, "_mock_mode", lambda: False)

    assert server_module._asset_version("buttons") == server_module.__version__
