import json
from datetime import datetime, timedelta, timezone
from unittest.mock import MagicMock

import pytest

from activsync import config, db, hevy_db, sync
from activsync.garmin_client import ActivityRecord


NOW = datetime(2026, 7, 18, 12, 0, tzinfo=timezone.utc)


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def _activity(conn, activity_id=101, *, activity_type="strength_training"):
    db.insert_activity(
        conn,
        activity_id,
        activity_type,
        "Workout",
        "",
        "2026-07-18 09:00:00",
        "hash",
        "pending",
        NOW,
        garmin_data=json.dumps({"duration": 3600}),
    )


def _hevy_workout(conn, hevy_id="hevy-1"):
    hevy_db.upsert_workout(
        conn,
        hevy_id,
        "Leg day",
        "2026-07-18T09:00:00+00:00",
        "2026-07-18T10:00:00+00:00",
        "2026-07-18T10:01:00+00:00",
        {"id": hevy_id, "exercises": []},
    )


def _clients():
    garmin = MagicMock()
    garmin.download_fit.return_value = b"FIT"
    strava = MagicMock()
    strava.find_existing_activity.return_value = None
    strava.publish.return_value = 9001
    return garmin, strava


def _enable_hevy(conn):
    settings = dict(config.DEFAULT_CONFIG)
    settings.update({"hevy_enabled": True, "hevy_poll_interval_minutes": 10})
    db.set_config_value(conn, "settings", settings)


def test_source_of_open_operation_is_blocked_from_automatic_publish(conn):
    _activity(conn)
    _hevy_workout(conn)
    hevy_db.set_workout_status(conn, "hevy-1", "merged")
    assert hevy_db.open_operation(conn, "hevy-1", "replace", 101, []) is not None
    garmin, strava = _clients()

    stats = sync.publish_pending(conn, garmin, strava, NOW)

    assert stats.published == 0
    assert stats.failed == 0
    assert stats.blocked == 1
    assert stats.blocked_reasons == ("hevy workout hevy-1 is merged",)
    strava.publish.assert_not_called()


def test_target_of_open_operation_is_blocked_even_after_workout_is_terminal(conn):
    _activity(conn)
    _hevy_workout(conn)
    hevy_db.set_workout_status(conn, "hevy-1", "replaced")
    op_id = hevy_db.open_operation(conn, "hevy-1", "replace", 202, [])
    assert op_id is not None
    hevy_db.update_operation(conn, op_id, target_activity_id=101)
    garmin, strava = _clients()

    stats = sync.publish_pending(conn, garmin, strava, NOW)

    assert stats.published == 0
    assert stats.failed == 0
    assert stats.blocked == 1
    strava.publish.assert_not_called()


def test_target_of_unresolved_workout_blocks_manual_publish(conn):
    _activity(conn)
    _hevy_workout(conn)
    hevy_db.link_target(conn, "hevy-1", 101, "replace")
    hevy_db.set_workout_status(conn, "hevy-1", "needs_review")
    garmin, strava = _clients()

    with pytest.raises(sync.PublishBlocked, match="hevy workout hevy-1 is needs_review") as exc:
        sync.publish_now(conn, garmin, strava, 101, NOW)

    assert exc.value.reason == "hevy workout hevy-1 is needs_review"
    strava.publish.assert_not_called()


def test_settlement_hold_skips_automatic_publish_without_recording_failure(conn):
    _activity(conn)
    _enable_hevy(conn)
    garmin, strava = _clients()

    stats = sync.publish_pending(conn, garmin, strava, NOW)

    assert stats.published == 0
    assert stats.failed == 0
    assert stats.blocked == 1
    assert stats.blocked_reasons == ("settlement hold",)
    assert db.get_activity(conn, 101)["publish_status"] == "pending"
    strava.publish.assert_not_called()


@pytest.mark.parametrize("entrypoint", ["bulk", "single"])
def test_manual_publish_bypasses_only_the_settlement_hold(conn, entrypoint):
    _activity(conn)
    _enable_hevy(conn)
    garmin, strava = _clients()

    if entrypoint == "bulk":
        stats = sync.publish_pending(conn, garmin, strava, NOW, {101})
        assert stats.published == 1
    else:
        sync.publish_now(conn, garmin, strava, 101, NOW)

    assert db.get_activity(conn, 101)["publish_status"] == "published"
    strava.publish.assert_called_once()


def test_settlement_hold_uses_successful_poll_time_and_persists_during_outage(conn):
    _activity(conn)
    _enable_hevy(conn)
    db.set_config_value(conn, "hevy_last_success_at", "2026-07-18T10:14:59+00:00")
    garmin, strava = _clients()

    stale_stats = sync.publish_pending(conn, garmin, strava, NOW + timedelta(days=30))

    assert stale_stats.published == 0
    assert stale_stats.failed == 0
    assert stale_stats.blocked == 1
    strava.publish.assert_not_called()

    db.set_config_value(conn, "hevy_last_success_at", "2026-07-18T10:15:00+00:00")
    released_stats = sync.publish_pending(conn, garmin, strava, NOW + timedelta(days=30))

    assert released_stats.published == 1
    strava.publish.assert_called_once()


def test_settlement_hold_is_inactive_when_hevy_is_disabled(conn):
    _activity(conn)
    garmin, strava = _clients()

    stats = sync.publish_pending(conn, garmin, strava, NOW)

    assert stats.published == 1
    strava.publish.assert_called_once()


def test_terminal_activsync_link_promotes_existing_category_held_activity(conn):
    garmin = MagicMock()
    activity = ActivityRecord(
        101, "strength_training", "Workout", "", "2026-07-18 09:00:00"
    )
    garmin.fetch_recent_activities.return_value = [activity]
    cfg = {**config.DEFAULT_CONFIG, "held_activity_types": ["strength_training"]}

    sync.sync_garmin(conn, garmin, cfg, NOW)
    assert db.get_activity(conn, 101)["publish_status"] == "held"

    _hevy_workout(conn)
    hevy_db.link_target(conn, "hevy-1", 101, "merge", provenance="activsync")
    hevy_db.set_workout_status(conn, "hevy-1", "merged")
    # The second Garmin response is intentionally content-identical. Promotion
    # is a lifecycle transition and must not depend on a metadata edit.
    stats = sync.sync_garmin(conn, garmin, cfg, NOW + timedelta(minutes=1))

    assert db.get_activity(conn, 101)["publish_status"] == "pending"
    assert stats.updated == 1


def test_terminal_activsync_link_does_not_promote_backlog_held_activity(conn):
    garmin = MagicMock()
    activity = ActivityRecord(
        101, "strength_training", "Workout", "", "2026-07-18 09:00:00"
    )
    garmin.fetch_recent_activities.return_value = [activity]
    cfg = {**config.DEFAULT_CONFIG, "held_activity_types": ["strength_training"]}

    sync.sync_garmin(
        conn,
        garmin,
        cfg,
        NOW,
        hold_before=NOW - timedelta(hours=1),
    )
    assert db.get_activity(conn, 101)["hold_reason"] == sync.HOLD_BACKLOG

    _hevy_workout(conn)
    hevy_db.link_target(conn, "hevy-1", 101, "merge", provenance="activsync")
    hevy_db.set_workout_status(conn, "hevy-1", "merged")
    sync.sync_garmin(conn, garmin, cfg, NOW + timedelta(minutes=1))

    row = db.get_activity(conn, 101)
    assert row["publish_status"] == "held"
    assert row["hold_reason"] == sync.HOLD_BACKLOG


def test_backfill_link_does_not_promote_new_activity_past_category_hold(conn):
    _hevy_workout(conn)
    hevy_db.link_target(conn, "hevy-1", 101, "external", provenance="backfill")
    hevy_db.set_workout_status(conn, "hevy-1", "linked_existing")
    garmin = MagicMock()
    garmin.fetch_recent_activities.return_value = [
        ActivityRecord(101, "strength_training", "Workout", "", "2026-07-18 09:00:00")
    ]
    cfg = {**config.DEFAULT_CONFIG, "held_activity_types": ["strength_training"]}

    sync.sync_garmin(conn, garmin, cfg, NOW)

    row = db.get_activity(conn, 101)
    assert row["publish_status"] == "held"
    assert row["hold_reason"] == sync.HOLD_CATEGORY
