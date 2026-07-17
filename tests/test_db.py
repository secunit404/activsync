import sqlite3
from datetime import datetime, timezone

import pytest

from activsync import db


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def test_insert_and_get_activity(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)

    db.insert_activity(
        conn,
        garmin_activity_id=1,
        activity_type="strength_training",
        title="Strength Training",
        description="",
        start_time="2026-07-09 09:00:00",
        content_hash="abc123",
        publish_status="held",
        now=now,
    )

    row = db.get_activity(conn, 1)

    assert row["garmin_activity_id"] == 1
    assert row["activity_type"] == "strength_training"
    assert row["publish_status"] == "held"
    assert row["held_since"] == now.isoformat()
    assert row["first_seen_at"] == now.isoformat()
    assert row["strava_activity_id"] is None


def test_get_activity_missing_returns_none(conn):
    assert db.get_activity(conn, 999) is None


def test_insert_pending_activity_has_no_held_since(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)

    db.insert_activity(
        conn, garmin_activity_id=2, activity_type="running", title="Run",
        description="", start_time="2026-07-09 09:00:00", content_hash="x",
        publish_status="pending", now=now,
    )

    row = db.get_activity(conn, 2)
    assert row["held_since"] is None


def test_update_activity_content(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn, garmin_activity_id=3, activity_type="strength_training",
        title="Strength Training", description="", start_time="2026-07-09 09:00:00",
        content_hash="old", publish_status="held", now=now,
    )

    db.update_activity_content(
        conn, garmin_activity_id=3, title="Leg Day",
        description="— synced by hevy2garmin", activity_type="strength_training",
        content_hash="new", publish_status="pending",
    )

    row = db.get_activity(conn, 3)
    assert row["title"] == "Leg Day"
    assert row["description"] == "— synced by hevy2garmin"
    assert row["content_hash"] == "new"
    assert row["publish_status"] == "pending"


def test_set_publish_status(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn, garmin_activity_id=4, activity_type="running", title="Run",
        description="", start_time="2026-07-09 09:00:00", content_hash="x",
        publish_status="held", now=now,
    )

    db.set_publish_status(conn, 4, "pending")

    assert db.get_activity(conn, 4)["publish_status"] == "pending"


def test_mark_removed_deletes_the_row(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn, garmin_activity_id=5, activity_type="running", title="Run",
        description="", start_time="2026-07-09 09:00:00", content_hash="x",
        publish_status="pending", now=now,
    )

    db.mark_removed(conn, 5)

    assert db.get_activity(conn, 5) is None


def test_set_published(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    published_at = datetime(2026, 7, 9, 12, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn, garmin_activity_id=6, activity_type="running", title="Run",
        description="", start_time="2026-07-09 09:00:00", content_hash="x",
        publish_status="pending", now=now,
    )

    db.set_published(conn, 6, strava_activity_id=999, now=published_at)

    row = db.get_activity(conn, 6)
    assert row["publish_status"] == "published"
    assert row["strava_activity_id"] == 999
    assert row["published_at"] == published_at.isoformat()


def test_list_activities_filters_by_status(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 7, "running", "Run", "", "2026-07-09 09:00:00", "x", "pending", now)
    db.insert_activity(conn, 8, "strength_training", "Lift", "", "2026-07-09 09:00:00", "y", "held", now)

    held = db.list_activities(conn, status="held")

    assert [row["garmin_activity_id"] for row in held] == [8]


def test_list_active_ids_since_excludes_removed(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 9, "running", "Run", "", "2026-07-09 09:00:00", "x", "pending", now)
    db.insert_activity(conn, 10, "running", "Run", "", "2026-07-09 09:00:00", "y", "pending", now)
    db.mark_removed(conn, 10)

    assert db.get_activity(conn, 10) is None
    assert db.list_active_ids_since(conn, "2026-01-01 00:00:00") == {9}


def test_list_active_ids_since_excludes_activities_before_window(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 11, "running", "Old Run", "", "2026-06-01 09:00:00", "x", "pending", now)
    db.insert_activity(conn, 12, "running", "Recent Run", "", "2026-07-08 09:00:00", "y", "pending", now)

    assert db.list_active_ids_since(conn, "2026-07-02 00:00:00") == {12}


def test_config_value_roundtrip(conn):
    assert db.get_config_value(conn, "settings") is None
    assert db.get_config_value(conn, "settings", default={}) == {}

    db.set_config_value(conn, "settings", {"poll_interval_minutes": 5})

    assert db.get_config_value(conn, "settings") == {"poll_interval_minutes": 5}


def test_connect_purges_the_dead_password_auth_state(tmp_path):
    """Password auth was removed, but databases created before that still hold
    its session table and its stored password hash. Nothing reads either, and a
    stale credential is not worth keeping at rest, so connect() clears both.
    Unrelated config must survive untouched."""
    path = str(tmp_path / "legacy.db")
    legacy = sqlite3.connect(path)
    legacy.executescript(
        """
        CREATE TABLE app_config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE sessions (
            token_hash TEXT PRIMARY KEY,
            created_at TEXT NOT NULL,
            expires_at TEXT NOT NULL
        );
        """
    )
    legacy.execute(
        "INSERT INTO app_config (key, value) VALUES ('auth', ?)",
        ('{"password_hash": "deadbeef", "salt": "cafe"}',),
    )
    legacy.execute(
        "INSERT INTO app_config (key, value) VALUES ('settings', ?)",
        ('{"lookback_days": 30}',),
    )
    legacy.execute("INSERT INTO sessions VALUES ('h', '2026-01-01', '2026-02-01')")
    legacy.commit()
    legacy.close()

    conn = db.connect(path)

    assert db.get_config_value(conn, "auth") is None
    tables = {r["name"] for r in conn.execute("SELECT name FROM sqlite_master WHERE type = 'table'")}
    assert "sessions" not in tables
    assert db.get_config_value(conn, "settings") == {"lookback_days": 30}


def test_connect_purge_is_idempotent_on_a_fresh_database(conn):
    """A database that never had password auth must not trip the purge."""
    assert db.get_config_value(conn, "auth") is None
    tables = {r["name"] for r in conn.execute("SELECT name FROM sqlite_master WHERE type = 'table'")}
    assert "sessions" not in tables
    assert "activities" in tables
