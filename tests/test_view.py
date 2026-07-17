from datetime import datetime, timedelta, timezone

import pytest

from activsync import config, db, view


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def test_activities_view_adds_display_time_and_garmin_link(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 42, "running", "Morning Run", "", "2026-07-09 09:00:00", "h", "pending", now)

    rows = view.activities_view(conn)

    assert len(rows) == 1
    row = rows[0]
    assert row["garmin_activity_id"] == 42
    assert row["start_time_display"] == "2026-07-09 11:00"
    assert row["garmin_url"] == "https://connect.garmin.com/modern/activity/42"


def test_activities_view_uses_configured_display_timezone(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 43, "running", "Evening Run", "", "2026-07-09 20:00:00", "h", "pending", now)
    cfg = config.load_config(conn)
    cfg["display_timezone"] = "America/New_York"
    config.save_config(conn, cfg)

    rows = view.activities_view(conn)

    assert rows[0]["start_time_display"] == "2026-07-09 16:00"


def test_activities_view_does_not_mutate_db_rows(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 44, "running", "Run", "", "2026-07-09 09:00:00", "h", "pending", now)

    view.activities_view(conn)
    raw_row = db.get_activity(conn, 44)

    assert "start_time_display" not in raw_row
    assert "garmin_url" not in raw_row


def test_garmin_status_not_synced_when_never_attempted(conn):
    status = view.garmin_status(conn)

    assert status == {"state": "not_synced", "status": "Not yet synced", "meta": ""}


def test_garmin_status_connected_after_successful_sync(conn):
    now = datetime(2026, 7, 9, 10, 5, tzinfo=timezone.utc)
    db.set_config_value(conn, "garmin_last_sync_at", (now - timedelta(minutes=5)).isoformat())
    db.set_config_value(conn, "garmin_last_sync_ok", True)
    db.set_config_value(conn, "garmin_last_sync_error", None)

    status = view.garmin_status(conn, now=now)

    assert status["state"] == "connected"
    assert status["status"] == "Connected"
    assert status["meta"] == "last synced 5 min ago"


def test_garmin_status_needs_attention_after_failed_sync(conn):
    now = datetime(2026, 7, 9, 14, 2, tzinfo=timezone.utc)
    db.set_config_value(conn, "garmin_last_sync_at", now.isoformat())
    db.set_config_value(conn, "garmin_last_sync_ok", False)
    db.set_config_value(conn, "garmin_last_sync_error", "MFA required")

    status = view.garmin_status(conn, now=now)

    assert status["state"] == "needs_attention"
    assert status["status"] == "Needs attention"
    assert status["meta"] == "last attempt at 14:02 failed: MFA required"


def test_connection_status_reports_garmin_broken_after_a_failed_sync(conn):
    db.set_config_value(conn, "garmin_credentials", {"email": "me@example.com", "password": "x"})
    db.set_config_value(conn, "garmin_credentials_verified", False)
    # A stale successful sync must not resurrect the connection.
    db.set_config_value(conn, "garmin_last_sync_at", "2026-07-14 11:00:00+00:00")
    db.set_config_value(conn, "garmin_last_sync_ok", True)

    status = view.connection_status(conn)

    assert status["garmin"]["connected"] is False
    assert status["garmin"]["email"] == "me@example.com"
    assert status["broken"] == ["garmin", "strava"]


def test_connection_status_is_healthy_when_both_are_connected(conn):
    db.set_config_value(conn, "garmin_credentials", {"email": "me@example.com", "password": "x"})
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_tokens", {"refresh_token": "r"})

    status = view.connection_status(conn)

    assert status["garmin"]["connected"] is True
    assert status["strava"]["connected"] is True
    assert status["broken"] == []
