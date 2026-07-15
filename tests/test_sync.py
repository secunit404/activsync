from datetime import datetime, timedelta, timezone
from unittest.mock import MagicMock

import pytest

from activsync import config, db, sync
from activsync.garmin_client import ActivityRecord

NOW = datetime(2026, 7, 9, 12, 0, tzinfo=timezone.utc)


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


@pytest.fixture
def cfg():
    return dict(config.DEFAULT_CONFIG)


def _record(gid, activity_type, title="Activity", description="", start_time="2026-07-09 09:00:00"):
    return ActivityRecord(
        garmin_activity_id=gid, activity_type=activity_type, title=title,
        description=description, start_time=start_time,
    )


def _fake_garmin(activities, fit_bytes=b"FIT"):
    garmin = MagicMock()
    garmin.fetch_recent_activities.return_value = activities
    garmin.download_fit.return_value = fit_bytes
    return garmin


def _garmin_connected(conn):
    """catch_up_sync only runs its Garmin half when Garmin is actually
    connected — a user who reconnects Strava first must still get the Strava
    half rather than having the whole catch-up abort on the Garmin fetch."""
    db.set_config_value(conn, "garmin_credentials_verified", True)


def _fake_strava(strava_activity_id=1001, existing_activity_id=None):
    strava = MagicMock()
    strava.publish.return_value = strava_activity_id
    strava.find_existing_activity.return_value = existing_activity_id
    strava.activity_exists.return_value = True
    return strava


def test_publish_now_force_publishes_a_held_activity(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=10, activity_type="strength_training", title="Strength Training",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="held", now=NOW,
    )
    garmin = _fake_garmin([], fit_bytes=b"FIT-10")
    strava = _fake_strava(strava_activity_id=4004)

    sync.publish_now(conn, garmin, strava, garmin_activity_id=10, now=NOW)

    row = db.get_activity(conn, 10)
    assert row["publish_status"] == "published"
    assert row["strava_activity_id"] == 4004
    strava.publish.assert_called_once_with(10, b"FIT-10", name="Strength Training", description=None)


def test_exclude_sets_status_excluded(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=11, activity_type="strength_training", title="Strength Training",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="held", now=NOW,
    )

    sync.exclude(conn, 11)

    assert db.get_activity(conn, 11)["publish_status"] == "excluded"


def test_unexclude_returns_activity_to_pending_when_type_not_held(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=12, activity_type="strength_training", title="Strength Training",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="excluded", now=NOW,
    )

    sync.unexclude(conn, 12, cfg)

    assert db.get_activity(conn, 12)["publish_status"] == "pending"


def test_unexclude_returns_held_activity_with_marker_to_pending(conn, cfg):
    cfg["held_activity_types"] = ["strength_training"]
    cfg["hevy2garmin_marker_enabled"] = True
    db.insert_activity(
        conn, garmin_activity_id=13, activity_type="strength_training", title="Leg Day",
        description="— synced by hevy2garmin", start_time="2026-07-09 09:00:00",
        content_hash="h", publish_status="excluded", now=NOW,
    )

    sync.unexclude(conn, 13, cfg)

    assert db.get_activity(conn, 13)["publish_status"] == "pending"


def test_unexclude_keeps_held_activity_held_when_marker_override_disabled(conn, cfg):
    cfg["held_activity_types"] = ["strength_training"]
    cfg["hevy2garmin_marker_enabled"] = False
    db.insert_activity(
        conn, garmin_activity_id=15, activity_type="strength_training", title="Leg Day",
        description="— synced by hevy2garmin", start_time="2026-07-09 09:00:00",
        content_hash="h", publish_status="excluded", now=NOW,
    )

    sync.unexclude(conn, 15, cfg)

    assert db.get_activity(conn, 15)["publish_status"] == "held"


def test_unexclude_non_strength_activity_returns_to_pending(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=14, activity_type="running", title="Run",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="excluded", now=NOW,
    )

    sync.unexclude(conn, 14, cfg)

    assert db.get_activity(conn, 14)["publish_status"] == "pending"


def test_publish_now_links_existing_strava_activity_without_re_uploading(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=16, activity_type="strength_training", title="Strength Training",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="held", now=NOW,
    )
    garmin = _fake_garmin([])
    strava = _fake_strava(existing_activity_id=7007)

    sync.publish_now(conn, garmin, strava, garmin_activity_id=16, now=NOW)

    row = db.get_activity(conn, 16)
    assert row["publish_status"] == "published"
    assert row["strava_activity_id"] == 7007
    garmin.download_fit.assert_not_called()
    strava.publish.assert_not_called()


def test_republish_via_publish_now_clears_missing_status(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=20, activity_type="running", title="Morning Run",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="missing", now=NOW - timedelta(hours=1),
    )
    garmin = _fake_garmin([], fit_bytes=b"FIT-20")
    strava = _fake_strava(strava_activity_id=5005)

    sync.publish_now(conn, garmin, strava, garmin_activity_id=20, now=NOW)

    row = db.get_activity(conn, 20)
    assert row["publish_status"] == "published"
    assert row["strava_activity_id"] == 5005


def test_sync_garmin_new_non_strength_activity_is_pending(conn, cfg):
    garmin = _fake_garmin([_record(101, "running")])

    stats = sync.sync_garmin(conn, garmin, cfg, NOW)

    row = db.get_activity(conn, 101)
    assert row["publish_status"] == "pending"
    assert stats.new == 1


def test_sync_garmin_new_strength_activity_is_pending_by_default(conn, cfg):
    garmin = _fake_garmin([_record(102, "strength_training")])

    stats = sync.sync_garmin(conn, garmin, cfg, NOW)

    row = db.get_activity(conn, 102)
    assert row["publish_status"] == "pending"
    assert stats.new == 1


def test_sync_garmin_held_activity_flips_to_pending_when_marker_appears(conn, cfg):
    cfg["hevy2garmin_marker_enabled"] = True
    db.insert_activity(
        conn, garmin_activity_id=103, activity_type="strength_training", title="Strength Training",
        description="", start_time="2026-07-09 09:00:00",
        content_hash=sync.compute_content_hash("Strength Training", "", "strength_training"),
        publish_status="held", now=NOW - timedelta(hours=1),
    )
    updated = _record(
        103, "strength_training", title="Leg Day",
        description="Squats 5x5\n— synced by hevy2garmin",
    )
    garmin = _fake_garmin([updated])

    stats = sync.sync_garmin(conn, garmin, cfg, NOW)

    row = db.get_activity(conn, 103)
    assert row["publish_status"] == "pending"
    assert stats.updated == 1


def test_sync_garmin_held_activity_stays_held_when_marker_override_disabled(conn, cfg):
    cfg["hevy2garmin_marker_enabled"] = False
    db.insert_activity(
        conn, garmin_activity_id=105, activity_type="strength_training", title="Strength Training",
        description="", start_time="2026-07-09 09:00:00",
        content_hash=sync.compute_content_hash("Strength Training", "", "strength_training"),
        publish_status="held", now=NOW - timedelta(hours=1),
    )
    updated = _record(
        105, "strength_training", title="Leg Day",
        description="Squats 5x5\n— synced by hevy2garmin",
    )
    garmin = _fake_garmin([updated])

    sync.sync_garmin(conn, garmin, cfg, NOW)

    assert db.get_activity(conn, 105)["publish_status"] == "held"


def test_sync_garmin_activity_missing_from_garmin_is_deleted(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=104, activity_type="strength_training", title="Strength Training",
        description="", start_time="2026-07-09 09:00:00",
        content_hash="h", publish_status="held", now=NOW - timedelta(hours=1),
    )
    garmin = _fake_garmin([])

    stats = sync.sync_garmin(conn, garmin, cfg, NOW)

    assert db.get_activity(conn, 104) is None
    assert stats.removed == 1


def test_sync_garmin_does_not_remove_activities_older_than_the_window(conn, cfg):
    # Published 30 days ago; the 7-day fetch window cannot see it.
    db.insert_activity(
        conn, 1, "running", "Old race", "", "2026-06-09 09:00:00",
        "hash-old", "published", NOW, "{}",
    )
    # Inside the window, and genuinely gone from Garmin.
    db.insert_activity(
        conn, 2, "running", "Deleted run", "", "2026-07-08 09:00:00",
        "hash-gone", "pending", NOW, "{}",
    )
    cfg["lookback_days"] = 7
    garmin = _fake_garmin([])

    stats = sync.sync_garmin(conn, garmin, cfg, NOW)

    assert db.get_activity(conn, 1)["publish_status"] == "published"
    assert db.get_activity(conn, 2) is None
    assert stats.removed == 1


def test_removed_then_reappeared_activity_links_to_existing_strava(conn, cfg):
    # A published activity removed from Garmin is hard-deleted locally, but its
    # Strava copy is left in place. If Garmin later returns it again, the publish
    # pass must link back to that existing Strava activity by start time rather
    # than uploading a duplicate — this is the safety net that replaces the old
    # 'removed' tombstone.
    start = "2026-07-08 09:00:00"
    db.insert_activity(
        conn, 3, "running", "Flaky run", "", start, "hash-3", "published", NOW, "{}",
    )
    db.set_published(conn, 3, strava_activity_id=8008, now=NOW)

    # Vanishes from Garmin → hard-deleted.
    sync.sync_garmin(conn, _fake_garmin([]), cfg, NOW)
    assert db.get_activity(conn, 3) is None

    # Reappears in the next fetch → re-inserted as pending.
    garmin = _fake_garmin([_record(3, "running", start_time=start)])
    sync.sync_garmin(conn, garmin, cfg, NOW)
    assert db.get_activity(conn, 3)["publish_status"] == "pending"

    # Publish pass finds the existing Strava copy and links it — no re-upload.
    strava = _fake_strava(existing_activity_id=8008)
    sync.publish_pending(conn, garmin, strava, NOW)

    row = db.get_activity(conn, 3)
    assert row["publish_status"] == "published"
    assert row["strava_activity_id"] == 8008
    strava.publish.assert_not_called()


def test_hold_before_holds_out_of_window_inserts_regardless_of_category(conn, cfg):
    cfg["held_activity_types"] = []          # running would normally autosync
    garmin = _fake_garmin([
        _record(1, "running", start_time="2026-06-20 09:00:00"),   # outside normal window
        _record(2, "running", start_time="2026-07-09 09:00:00"),   # inside normal window
    ])
    normal_window_start = NOW - timedelta(days=cfg["lookback_days"])

    stats = sync.sync_garmin(
        conn, garmin, cfg, NOW, lookback_days=30, hold_before=normal_window_start,
    )

    assert db.get_activity(conn, 1)["publish_status"] == "held"
    assert db.get_activity(conn, 2)["publish_status"] == "pending"
    assert stats.held_backlog == 1


def test_hold_before_never_demotes_an_existing_activity(conn, cfg):
    db.insert_activity(
        conn, 1, "running", "Old race", "", "2026-06-20 09:00:00",
        "hash-old", "published", NOW, "{}",
    )
    garmin = _fake_garmin([_record(1, "running", start_time="2026-06-20 09:00:00")])

    sync.sync_garmin(
        conn, garmin, cfg, NOW,
        lookback_days=30, hold_before=NOW - timedelta(days=7),
    )

    assert db.get_activity(conn, 1)["publish_status"] == "published"


def test_lookback_override_widens_the_garmin_fetch(conn, cfg):
    cfg["lookback_days"] = 7
    garmin = _fake_garmin([])
    sync.sync_garmin(conn, garmin, cfg, NOW, lookback_days=30)
    garmin.fetch_recent_activities.assert_called_once_with(30)


def test_sync_garmin_records_success_status(conn, cfg):
    garmin = _fake_garmin([])

    sync.sync_garmin(conn, garmin, cfg, NOW)

    assert db.get_config_value(conn, "garmin_last_sync_ok") is True
    assert db.get_config_value(conn, "garmin_last_sync_at") == NOW.isoformat()
    assert db.get_config_value(conn, "garmin_last_sync_error") is None


def test_sync_garmin_records_failure_status_and_reraises(conn, cfg):
    garmin = MagicMock()
    garmin.fetch_recent_activities.side_effect = RuntimeError("MFA required")

    with pytest.raises(RuntimeError, match="MFA required"):
        sync.sync_garmin(conn, garmin, cfg, NOW)

    assert db.get_config_value(conn, "garmin_last_sync_ok") is False
    assert db.get_config_value(conn, "garmin_last_sync_error") == "MFA required"
    assert db.get_config_value(conn, "garmin_last_sync_at") == NOW.isoformat()


def test_publish_pending_publishes_all_pending_activities(conn, cfg):
    db.insert_activity(conn, 201, "running", "Run A", "", "2026-07-09 09:00:00", "h1", "pending", NOW)
    db.insert_activity(conn, 202, "running", "Run B", "", "2026-07-09 09:30:00", "h2", "pending", NOW)
    garmin = _fake_garmin([], fit_bytes=b"FIT")
    strava = _fake_strava(strava_activity_id=5001)

    stats = sync.publish_pending(conn, garmin, strava, NOW)

    assert db.get_activity(conn, 201)["publish_status"] == "published"
    assert db.get_activity(conn, 202)["publish_status"] == "published"
    assert stats.published == 2


def test_publish_pending_only_publishes_selected_ids(conn, cfg):
    db.insert_activity(conn, 203, "running", "Run C", "", "2026-07-09 09:00:00", "h3", "pending", NOW)
    db.insert_activity(conn, 204, "running", "Run D", "", "2026-07-09 09:30:00", "h4", "pending", NOW)
    garmin = _fake_garmin([], fit_bytes=b"FIT")
    strava = _fake_strava(strava_activity_id=5002)

    stats = sync.publish_pending(conn, garmin, strava, NOW, garmin_activity_ids={203})

    assert db.get_activity(conn, 203)["publish_status"] == "published"
    assert db.get_activity(conn, 204)["publish_status"] == "pending"
    assert stats.published == 1


def test_publish_pending_records_failure_and_continues(conn, cfg):
    db.insert_activity(conn, 205, "running", "Run E", "", "2026-07-09 09:00:00", "h5", "pending", NOW)
    db.insert_activity(conn, 206, "running", "Run F", "", "2026-07-09 09:30:00", "h6", "pending", NOW)
    garmin = _fake_garmin([], fit_bytes=b"FIT")
    strava = MagicMock()
    strava.find_existing_activity.return_value = None
    strava.publish.side_effect = [RuntimeError("network error"), 5003]

    stats = sync.publish_pending(conn, garmin, strava, NOW)

    assert stats.published == 1
    assert stats.failed == 1


def test_check_strava_status_flags_missing_activity(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=301, activity_type="running", title="Morning Run",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="published", now=NOW - timedelta(hours=1),
    )
    db.set_published(conn, 301, strava_activity_id=9101, now=NOW - timedelta(hours=1))
    strava = _fake_strava()
    strava.activity_exists.return_value = False

    stats = sync.check_strava_status(conn, strava, cfg, NOW)

    assert db.get_activity(conn, 301)["publish_status"] == "missing"
    assert stats.flagged_missing == 1


def test_check_strava_status_leaves_existing_activity_published(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=302, activity_type="running", title="Morning Run",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="published", now=NOW - timedelta(hours=1),
    )
    db.set_published(conn, 302, strava_activity_id=9102, now=NOW - timedelta(hours=1))
    strava = _fake_strava()
    strava.activity_exists.return_value = True

    stats = sync.check_strava_status(conn, strava, cfg, NOW)

    assert db.get_activity(conn, 302)["publish_status"] == "published"
    assert stats.flagged_missing == 0


def test_check_strava_status_skips_activities_older_than_lookback(conn, cfg):
    old_start = (NOW - timedelta(days=cfg["lookback_days"] + 1)).strftime("%Y-%m-%d %H:%M:%S")
    db.insert_activity(
        conn, garmin_activity_id=303, activity_type="running", title="Old Run",
        description="", start_time=old_start, content_hash="h",
        publish_status="published", now=NOW - timedelta(days=10),
    )
    db.set_published(conn, 303, strava_activity_id=9103, now=NOW - timedelta(days=10))
    strava = _fake_strava()
    strava.activity_exists.return_value = False

    stats = sync.check_strava_status(conn, strava, cfg, NOW)

    assert db.get_activity(conn, 303)["publish_status"] == "published"
    assert stats.flagged_missing == 0
    strava.activity_exists.assert_not_called()


def test_check_strava_status_links_held_activity_already_on_strava(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=304, activity_type="strength_training", title="Leg Day",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="held", now=NOW - timedelta(hours=1),
    )
    strava = _fake_strava(existing_activity_id=9201)

    stats = sync.check_strava_status(conn, strava, cfg, NOW)

    row = db.get_activity(conn, 304)
    assert row["publish_status"] == "published"
    assert row["strava_activity_id"] == 9201
    assert stats.linked_existing == 1


def test_check_strava_status_links_pending_activity_to_existing_strava(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=305, activity_type="running", title="Morning Run",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="pending", now=NOW - timedelta(hours=1),
    )
    strava = _fake_strava(existing_activity_id=9202)

    stats = sync.check_strava_status(conn, strava, cfg, NOW)

    row = db.get_activity(conn, 305)
    assert row["publish_status"] == "published"
    assert row["strava_activity_id"] == 9202
    assert stats.linked_existing == 1


def test_reconcile_held_activities_flips_activities_no_longer_held(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=401, activity_type="bouldering", title="Bouldering",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="held", now=NOW,
    )

    flipped = sync.reconcile_held_activities(conn, held_activity_types=["strength_training"])

    assert db.get_activity(conn, 401)["publish_status"] == "pending"
    assert flipped == 1


def test_reconcile_held_activities_leaves_still_held_activities_alone(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=402, activity_type="strength_training", title="Strength Training",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="held", now=NOW,
    )

    flipped = sync.reconcile_held_activities(conn, held_activity_types=["strength_training"])

    assert db.get_activity(conn, 402)["publish_status"] == "held"
    assert flipped == 0


def test_reconcile_held_activities_ignores_non_held_rows(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=403, activity_type="running", title="Run",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="published", now=NOW,
    )

    flipped = sync.reconcile_held_activities(conn, held_activity_types=[])

    assert db.get_activity(conn, 403)["publish_status"] == "published"
    assert flipped == 0


def test_reconcile_held_activities_never_unholds_the_outage_backlog(conn, cfg):
    # The disaster this feature exists to prevent: a three-week backlog is held
    # regardless of category, so an autosync-category change — which says
    # nothing about whether a three-week-old activity should be published —
    # must not promote it to pending for the poller to blast-publish.
    db.insert_activity(
        conn, garmin_activity_id=404, activity_type="running", title="Old Run",
        description="", start_time="2026-06-20 09:00:00", content_hash="h",
        publish_status="held", now=NOW, hold_reason="backlog",
    )

    flipped = sync.reconcile_held_activities(conn, held_activity_types=["strength_training"])

    assert db.get_activity(conn, 404)["publish_status"] == "held"
    assert flipped == 0


def test_reconcile_held_activities_unholds_a_category_held_row(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=405, activity_type="running", title="Run",
        description="", start_time="2026-07-09 09:00:00", content_hash="h",
        publish_status="held", now=NOW, hold_reason="category",
    )

    flipped = sync.reconcile_held_activities(conn, held_activity_types=["strength_training"])

    assert db.get_activity(conn, 405)["publish_status"] == "pending"
    assert flipped == 1


def test_backlog_row_can_still_be_published_deliberately(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=406, activity_type="running", title="Old Run",
        description="", start_time="2026-06-20 09:00:00", content_hash="h",
        publish_status="held", now=NOW, hold_reason="backlog",
    )
    db.insert_activity(
        conn, garmin_activity_id=407, activity_type="running", title="Older Run",
        description="", start_time="2026-06-21 09:00:00", content_hash="h2",
        publish_status="held", now=NOW, hold_reason="backlog",
    )
    garmin = _fake_garmin([], fit_bytes=b"FIT")
    strava = _fake_strava(strava_activity_id=6006)

    sync.publish_now(conn, garmin, strava, garmin_activity_id=406, now=NOW)
    stats = sync.publish_pending(conn, garmin, strava, NOW, garmin_activity_ids={407})

    assert db.get_activity(conn, 406)["publish_status"] == "published"
    assert db.get_activity(conn, 407)["publish_status"] == "published"
    assert stats.published == 1


def test_publishing_a_backlog_row_clears_its_hold_reason(conn, cfg):
    db.insert_activity(
        conn, garmin_activity_id=408, activity_type="running", title="Old Run",
        description="", start_time="2026-06-20 09:00:00", content_hash="h",
        publish_status="held", now=NOW, hold_reason="backlog",
    )
    garmin = _fake_garmin([], fit_bytes=b"FIT")

    sync.publish_now(conn, garmin, _fake_strava(strava_activity_id=6007), 408, NOW)

    assert db.get_activity(conn, 408)["hold_reason"] is None


def test_sync_garmin_marker_does_not_promote_a_backlog_held_row(conn, cfg):
    # The Hevy marker rule promotes a category-held row to pending on update.
    # A backlog-held row is held because it predates the normal window, not
    # because of its category — an edit must not sneak it past the hold.
    cfg["hevy2garmin_marker_enabled"] = True
    db.insert_activity(
        conn, garmin_activity_id=409, activity_type="strength_training", title="Strength Training",
        description="", start_time="2026-06-20 09:00:00",
        content_hash=sync.compute_content_hash("Strength Training", "", "strength_training"),
        publish_status="held", now=NOW - timedelta(days=19), hold_reason="backlog",
    )
    updated = _record(
        409, "strength_training", title="Leg Day",
        description="Squats 5x5\n— synced by hevy2garmin", start_time="2026-06-20 09:00:00",
    )

    sync.sync_garmin(conn, _fake_garmin([updated]), cfg, NOW, lookback_days=30)

    assert db.get_activity(conn, 409)["publish_status"] == "held"


def test_sync_garmin_records_why_each_new_activity_is_held(conn, cfg):
    cfg["lookback_days"] = 7
    cfg["held_activity_types"] = ["strength_training"]
    garmin = _fake_garmin([
        _record(1, "running", start_time="2026-06-20 09:00:00"),          # outage backlog
        _record(2, "strength_training", start_time="2026-07-09 09:00:00"),  # held by category
        _record(3, "running", start_time="2026-07-09 09:00:00"),          # autosync
    ])

    sync.sync_garmin(
        conn, garmin, cfg, NOW,
        lookback_days=30, hold_before=NOW - timedelta(days=7),
    )

    assert db.get_activity(conn, 1)["hold_reason"] == "backlog"
    assert db.get_activity(conn, 2)["hold_reason"] == "category"
    assert db.get_activity(conn, 3)["hold_reason"] is None


def test_catch_up_widens_the_window_to_cover_the_outage(conn, cfg):
    _garmin_connected(conn)
    cfg["lookback_days"] = 7
    db.set_config_value(
        conn, "garmin_last_sync_ok_at", (NOW - timedelta(days=21)).isoformat(),
    )
    garmin = _fake_garmin([])
    strava = _fake_strava()

    stats = sync.catch_up_sync(conn, garmin, strava, cfg, NOW)

    assert stats.lookback_days == 22          # 21 days of outage + 1 buffer
    garmin.fetch_recent_activities.assert_called_once_with(22)


def test_catch_up_holds_the_out_of_window_backlog(conn, cfg):
    _garmin_connected(conn)
    cfg["lookback_days"] = 7
    cfg["held_activity_types"] = []           # running would normally autosync
    db.set_config_value(
        conn, "garmin_last_sync_ok_at", (NOW - timedelta(days=21)).isoformat(),
    )
    garmin = _fake_garmin([
        _record(1, "running", start_time="2026-06-25 09:00:00"),   # during the outage
        _record(2, "running", start_time="2026-07-09 09:00:00"),   # inside normal window
    ])

    stats = sync.catch_up_sync(conn, garmin, _fake_strava(), cfg, NOW)

    assert db.get_activity(conn, 1)["publish_status"] == "held"
    assert db.get_activity(conn, 2)["publish_status"] == "pending"
    assert stats.garmin.held_backlog == 1


def test_catch_up_caps_the_window(conn, cfg):
    _garmin_connected(conn)
    db.set_config_value(
        conn, "garmin_last_sync_ok_at", (NOW - timedelta(days=400)).isoformat(),
    )
    garmin = _fake_garmin([])
    stats = sync.catch_up_sync(conn, garmin, _fake_strava(), cfg, NOW)
    assert stats.lookback_days == sync.CATCH_UP_MAX_DAYS


def test_catch_up_without_a_prior_success_uses_the_normal_window(conn, cfg):
    _garmin_connected(conn)
    cfg["lookback_days"] = 7
    garmin = _fake_garmin([])
    stats = sync.catch_up_sync(conn, garmin, _fake_strava(), cfg, NOW)
    assert stats.lookback_days == 7


def test_hold_before_boundary_is_exclusive(conn, cfg):
    # An activity landing exactly on hold_before is NOT held by the outage
    # backlog rule (comparison is `<`, not `<=`) — it falls through to the
    # normal category check, which for "running" with no held types is pending.
    cfg["held_activity_types"] = []
    hold_before = NOW - timedelta(days=7)
    garmin = _fake_garmin([
        _record(1, "running", start_time=hold_before.strftime("%Y-%m-%d %H:%M:%S")),
    ])

    sync.sync_garmin(conn, garmin, cfg, NOW, lookback_days=30, hold_before=hold_before)

    assert db.get_activity(conn, 1)["publish_status"] == "pending"


def test_removal_window_boundary_activity_survives_when_still_fetched(conn, cfg):
    # An activity exactly on the (buffered) removal-window boundary is inside
    # the inclusive `>=` candidate set; if Garmin still returns it, it must
    # not be marked removed.
    cfg["lookback_days"] = 7
    window_start = NOW - timedelta(days=cfg["lookback_days"]) + timedelta(minutes=5)
    boundary_str = window_start.strftime("%Y-%m-%d %H:%M:%S")
    db.insert_activity(
        conn, 1, "running", "Edge run", "", boundary_str,
        "hash-edge", "pending", NOW, "{}",
    )
    garmin = _fake_garmin([_record(1, "running", start_time=boundary_str)])

    sync.sync_garmin(conn, garmin, cfg, NOW)

    assert db.get_activity(conn, 1) is not None


def test_catch_up_outage_of_exactly_the_cap_uses_the_cap(conn, cfg):
    _garmin_connected(conn)
    db.set_config_value(
        conn, "garmin_last_sync_ok_at",
        (NOW - timedelta(days=sync.CATCH_UP_MAX_DAYS - 1)).isoformat(),
    )
    garmin = _fake_garmin([])

    stats = sync.catch_up_sync(conn, garmin, _fake_strava(), cfg, NOW)

    assert stats.lookback_days == sync.CATCH_UP_MAX_DAYS


def test_catch_up_cap_never_narrows_below_the_configured_lookback(conn, cfg):
    # Regression: a short 5-day outage must never shrink the catch-up window
    # below the user's own configured lookback (120 days here) — that would
    # make a reconnect fetch LESS than a plain sync, and would put hold_before
    # (now - 120d) before the entire fetched span, so nothing is ever held.
    _garmin_connected(conn)
    cfg["lookback_days"] = 120
    db.set_config_value(
        conn, "garmin_last_sync_ok_at", (NOW - timedelta(days=5)).isoformat(),
    )
    garmin = _fake_garmin([])

    stats = sync.catch_up_sync(conn, garmin, _fake_strava(), cfg, NOW)

    assert stats.lookback_days == 120


def test_catch_up_removal_loop_respects_widened_window(conn, cfg):
    _garmin_connected(conn)
    cfg["lookback_days"] = 7
    db.set_config_value(
        conn, "garmin_last_sync_ok_at", (NOW - timedelta(days=21)).isoformat(),
    )
    # Published 60 days ago — outside even the widened (~22-day) catch-up span.
    old_published_start = (NOW - timedelta(days=60)).strftime("%Y-%m-%d %H:%M:%S")
    db.insert_activity(
        conn, 1, "running", "Old race", "", old_published_start,
        "hash-old", "published", NOW, "{}",
    )
    # Pending, 15 days old — inside the widened span, but absent from the fetch.
    gone_start = (NOW - timedelta(days=15)).strftime("%Y-%m-%d %H:%M:%S")
    db.insert_activity(
        conn, 2, "running", "Deleted run", "", gone_start,
        "hash-gone", "pending", NOW, "{}",
    )
    garmin = _fake_garmin([])

    stats = sync.catch_up_sync(conn, garmin, _fake_strava(), cfg, NOW)

    assert stats.lookback_days == 22
    assert db.get_activity(conn, 1)["publish_status"] == "published"
    assert db.get_activity(conn, 2) is None
    assert stats.garmin.removed == 1


def test_catch_up_links_out_of_window_held_activity_to_existing_strava(conn, cfg):
    _garmin_connected(conn)
    cfg["lookback_days"] = 7
    db.set_config_value(
        conn, "garmin_last_sync_ok_at", (NOW - timedelta(days=21)).isoformat(),
    )
    # Held, 15 days old — outside the normal 7-day window but inside the
    # widened (~22-day) catch-up span, so check_strava_status must see it.
    held_start = (NOW - timedelta(days=15)).strftime("%Y-%m-%d %H:%M:%S")
    db.insert_activity(
        conn, garmin_activity_id=501, activity_type="strength_training", title="Leg Day",
        description="", start_time=held_start,
        content_hash=sync.compute_content_hash("Leg Day", "", "strength_training"),
        publish_status="held", now=NOW - timedelta(days=15),
    )
    garmin = _fake_garmin([
        _record(501, "strength_training", title="Leg Day", description="", start_time=held_start),
    ])
    strava = _fake_strava(existing_activity_id=555)

    stats = sync.catch_up_sync(conn, garmin, strava, cfg, NOW)

    row = db.get_activity(conn, 501)
    assert row["publish_status"] == "published"
    assert row["strava_activity_id"] == 555
    assert stats.status.linked_existing == 1


def test_catch_up_status_checks_stale_pending_rows_after_a_strava_only_outage(conn, cfg):
    """During a STRAVA outage Garmin sync keeps succeeding, so the Garmin-derived
    outage measurement stays at ~0 days. But pending rows piled up for three
    weeks, and publish_pending has no window at all — so without widening the
    STATUS window from the actual backlog, the first tick after a Strava
    reconnect re-uploads every stale row that Garmin's native sync already
    pushed, duplicating them.
    """
    cfg["lookback_days"] = 7
    db.set_config_value(conn, "garmin_last_sync_ok_at", NOW.isoformat())  # Garmin never broke
    stale_start = (NOW - timedelta(days=18)).strftime("%Y-%m-%d %H:%M:%S")
    db.insert_activity(
        conn, 601, "running", "Stale Run", "", stale_start, "h",
        "pending", NOW - timedelta(days=18),
    )
    garmin = _fake_garmin([_record(601, "running", title="Stale Run", start_time=stale_start)])
    strava = _fake_strava(existing_activity_id=888)

    stats = sync.catch_up_sync(conn, garmin, strava, cfg, NOW)

    row = db.get_activity(conn, 601)
    assert row["publish_status"] == "published"      # linked, not re-uploaded
    assert row["strava_activity_id"] == 888
    assert stats.status.linked_existing == 1
    strava.publish.assert_not_called()


def test_catch_up_status_window_is_capped_and_floored(conn, cfg):
    cfg["lookback_days"] = 7
    db.set_config_value(conn, "garmin_last_sync_ok_at", NOW.isoformat())
    ancient = (NOW - timedelta(days=400)).strftime("%Y-%m-%d %H:%M:%S")
    db.insert_activity(conn, 602, "running", "Ancient", "", ancient, "h", "pending", NOW)

    assert sync.strava_status_lookback_days(conn, cfg, NOW) == sync.CATCH_UP_MAX_DAYS

    # No pending/held work at all -> never narrower than the configured lookback.
    db.set_publish_status(conn, 602, "excluded")
    assert sync.strava_status_lookback_days(conn, cfg, NOW) == 7


def test_catch_up_skips_the_garmin_half_when_garmin_is_not_connected(conn, cfg):
    """A user who reconnects STRAVA first must still get their reconciliation:
    the Garmin fetch raising must not abort the Strava half of the catch-up."""
    cfg["lookback_days"] = 7
    db.set_config_value(conn, "garmin_credentials_verified", False)
    db.insert_activity(
        conn, 603, "running", "Stale Run", "", "2026-07-08 09:00:00", "h", "pending", NOW,
    )
    garmin = MagicMock()
    garmin.fetch_recent_activities.side_effect = RuntimeError("garmin still broken")
    strava = _fake_strava(existing_activity_id=999)

    stats = sync.catch_up_sync(conn, garmin, strava, cfg, NOW)

    garmin.fetch_recent_activities.assert_not_called()
    assert db.get_activity(conn, 603)["publish_status"] == "published"
    assert stats.status.linked_existing == 1
    assert stats.garmin.new == 0


def test_garmin_last_sync_ok_at_advances_only_on_success(conn, cfg):
    garmin = _fake_garmin([_record(1, "running")])
    sync.sync_garmin(conn, garmin, cfg, NOW)
    assert db.get_config_value(conn, "garmin_last_sync_ok_at") == NOW.isoformat()

    later = NOW + timedelta(days=1)
    failing = MagicMock()
    failing.fetch_recent_activities.side_effect = RuntimeError("401 Unauthorized")
    with pytest.raises(RuntimeError):
        sync.sync_garmin(conn, failing, cfg, later)

    # The failed attempt advances last_sync_at but must not touch last_sync_ok_at.
    assert db.get_config_value(conn, "garmin_last_sync_at") == later.isoformat()
    assert db.get_config_value(conn, "garmin_last_sync_ok_at") == NOW.isoformat()
