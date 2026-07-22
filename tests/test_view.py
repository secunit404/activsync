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


def test_activities_view_exposes_duration_seconds(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn, 45, "running", "Run", "", "2026-07-09 09:00:00", "h", "pending", now,
        garmin_data='{"duration": 1500}',
    )

    rows = view.activities_view(conn)

    assert rows[0]["duration_seconds"] == 1500


def test_activities_view_duration_seconds_none_when_missing(conn):
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 46, "running", "Run", "", "2026-07-09 09:00:00", "h", "pending", now)

    rows = view.activities_view(conn)

    assert rows[0]["duration_seconds"] is None


def test_fmt_duration_coarse_drops_seconds():
    assert view.fmt_duration_coarse(27720) == "7h 42m"


def test_fmt_duration_coarse_under_an_hour():
    assert view.fmt_duration_coarse(2520) == "42m"


def test_fmt_duration_coarse_empty_week():
    assert view.fmt_duration_coarse(0) == "0m"
    assert view.fmt_duration_coarse(None) == "0m"


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


def _template(conn, template_id, title, *, is_custom=False, muscle="chest"):
    from activsync import hevy_db

    hevy_db.upsert_template(conn, {
        "exercise_template_id": template_id,
        "title": title,
        "primary_muscle_group": muscle,
        "secondary_muscle_groups": [],
        "equipment_category": "barbell",
        "is_custom": is_custom,
    })


def test_mappings_view_includes_exercises_garmin_resolves_on_its_own(conn):
    """Every Hevy exercise ends up somewhere in Garmin, so the list has to
    show every one — including built-ins resolved by the ported tables, which
    were previously skipped and so could never be inspected or overridden."""
    _template(conn, "tpl-bench", "Bench Press (Barbell)")

    rows = view.hevy_mappings_view(conn)

    assert [row["template_id"] for row in rows] == ["tpl-bench"]
    row = rows[0]
    assert row["mapped"] is True
    assert row["unmapped"] is False
    assert row["source"] == "automatic"
    # It resolves to a real Garmin pair, and the view reports which.
    assert row["category"] is not None
    assert row["subcategory"] is not None
    assert row["category_name"]
    assert row["subcategory_name"]


def test_mappings_view_marks_a_user_override_as_such(conn):
    """A saved mapping wins over the built-in table, and the row says so —
    that distinction is what tells a user whether they set it or Garmin did."""
    from activsync import hevy_db

    _template(conn, "tpl-bench", "Bench Press (Barbell)")
    hevy_db.save_mapping(conn, "tpl-bench", 0, 1)

    row = next(r for r in view.hevy_mappings_view(conn) if r["template_id"] == "tpl-bench")

    assert row["source"] == "user"
    assert row["mapped"] is True
    assert (row["category"], row["subcategory"]) == (0, 1)


def test_mappings_view_still_flags_what_needs_action(conn):
    """The needs-mapping case is unchanged: a custom exercise no table can
    resolve stays unmapped, which is what the hub's summary counts."""
    _template(conn, "tpl-odd", "Bulgarian Ring Row", is_custom=True, muscle="upper_back")

    row = next(r for r in view.hevy_mappings_view(conn) if r["template_id"] == "tpl-odd")

    assert row["unmapped"] is True
    assert row["mapped"] is False
    assert row["source"] == ""


def test_mappings_view_sorts_actionable_rows_first(conn):
    """The list opens on what needs doing, without needing the filter."""
    _template(conn, "tpl-bench", "Bench Press (Barbell)")
    _template(conn, "tpl-odd", "Bulgarian Ring Row", is_custom=True, muscle="upper_back")

    rows = view.hevy_mappings_view(conn)

    assert rows[0]["template_id"] == "tpl-odd"
    assert rows[0]["unmapped"] is True
