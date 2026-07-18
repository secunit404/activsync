"""Tests for hevy_sync part 1: event ingestion, mapping gate, watch matching."""

import json
from datetime import datetime, timezone

import pytest

from activsync import db, hevy_db, hevy_sync
from activsync.hevy_sync import CURSOR_KEY

NOW = datetime(2026, 7, 18, 12, 0, 0, tzinfo=timezone.utc)
CURSOR_START = "2026-07-18T00:00:00+00:00"


def make_conn():
    return db.connect(":memory:")


def set_cursor(conn, value=CURSOR_START):
    db.set_config_value(conn, CURSOR_KEY, value)


def updated_event(workout_id, ts, title="Push Day", start="2026-07-18T10:00:00Z",
                  end="2026-07-18T11:00:00Z", exercises=None):
    return {"type": "updated", "workout": {
        "id": workout_id, "title": title, "start_time": start, "end_time": end,
        "updated_at": ts, "exercises": exercises or []}}


def deleted_event(workout_id, ts):
    return {"type": "deleted", "id": workout_id, "deleted_at": ts}


class FakeHevy:
    def __init__(self, events=None, templates=None):
        self.events = events or []
        self.templates = templates or {}
        self.since_calls = []

    def iter_events_since(self, since):
        self.since_calls.append(since)
        return list(self.events)

    def get_exercise_template(self, template_id):
        return self.templates.get(template_id)


# -- ingest_events ----------------------------------------------------------

def test_missing_cursor_initializes_forward_only():
    conn = make_conn()
    hevy = FakeHevy(events=[updated_event("w1", "2026-07-18T10:30:00Z")])
    assert hevy_sync.ingest_events(conn, hevy, NOW) == 0
    assert hevy.since_calls == []  # no poll before a cursor exists
    assert db.get_config_value(conn, CURSOR_KEY) == NOW.isoformat()


def test_ingest_upserts_and_advances_cursor():
    conn = make_conn()
    set_cursor(conn)
    hevy = FakeHevy(events=[updated_event("w1", "2026-07-18T10:30:00Z")])
    assert hevy_sync.ingest_events(conn, hevy, NOW) == 1
    assert hevy_db.get_workout(conn, "w1")["title"] == "Push Day"
    assert db.get_config_value(conn, CURSOR_KEY) == "2026-07-18T10:30:00Z"
    # the poll re-polled with the safety lag applied
    assert hevy.since_calls == ["2026-07-17T23:55:00+00:00"]


def test_cursor_not_advanced_when_upsert_raises(monkeypatch):
    conn = make_conn()
    set_cursor(conn)
    hevy = FakeHevy(events=[updated_event("w1", "2026-07-18T10:30:00Z")])

    def boom(*args, **kwargs):
        raise RuntimeError("db full")
    monkeypatch.setattr(hevy_sync.hevy_db, "upsert_workout", boom)
    with pytest.raises(RuntimeError):
        hevy_sync.ingest_events(conn, hevy, NOW)
    assert db.get_config_value(conn, CURSOR_KEY) == CURSOR_START


def test_repoll_after_failure_still_processes_event(monkeypatch):
    conn = make_conn()
    set_cursor(conn)
    hevy = FakeHevy(events=[updated_event("w1", "2026-07-18T10:30:00Z")])

    def boom(*args, **kwargs):
        raise RuntimeError("transient")
    monkeypatch.setattr(hevy_sync.hevy_db, "upsert_workout", boom)
    with pytest.raises(RuntimeError):
        hevy_sync.ingest_events(conn, hevy, NOW)
    monkeypatch.undo()

    # The failed event must not have been burned into the dedupe table.
    assert hevy_sync.ingest_events(conn, hevy, NOW) == 1
    assert hevy_db.get_workout(conn, "w1") is not None


def test_same_event_delivered_twice_processes_once():
    conn = make_conn()
    set_cursor(conn)
    event = updated_event("w1", "2026-07-18T10:30:00Z")
    hevy = FakeHevy(events=[event])
    assert hevy_sync.ingest_events(conn, hevy, NOW) == 1
    # safety-lag overlap re-delivers the same event next poll
    assert hevy_sync.ingest_events(conn, hevy, NOW) == 0


def test_partial_batch_failure_keeps_cursor_at_last_success(monkeypatch):
    conn = make_conn()
    set_cursor(conn)
    hevy = FakeHevy(events=[
        updated_event("w2", "2026-07-18T11:00:00Z"),  # newest first, as delivered
        updated_event("w1", "2026-07-18T10:00:00Z"),
    ])
    real_upsert = hevy_db.upsert_workout

    def fail_w2(conn_, hevy_id, *args, **kwargs):
        if hevy_id == "w2":
            raise RuntimeError("transient")
        return real_upsert(conn_, hevy_id, *args, **kwargs)
    monkeypatch.setattr(hevy_sync.hevy_db, "upsert_workout", fail_w2)
    with pytest.raises(RuntimeError):
        hevy_sync.ingest_events(conn, hevy, NOW)
    # oldest-first processing: w1 landed, cursor sits at w1's timestamp
    assert hevy_db.get_workout(conn, "w1") is not None
    assert db.get_config_value(conn, CURSOR_KEY) == "2026-07-18T10:00:00Z"


def test_update_then_deletion_deletion_wins_any_delivery_order():
    for delivery in (
        [deleted_event("w1", "2026-07-18T10:05:00Z"),
         updated_event("w1", "2026-07-18T10:00:00Z")],   # newest first
        [updated_event("w1", "2026-07-18T10:00:00Z"),
         deleted_event("w1", "2026-07-18T10:05:00Z")],   # oldest first
    ):
        conn = make_conn()
        set_cursor(conn)
        hevy_sync.ingest_events(conn, FakeHevy(events=delivery), NOW)
        assert hevy_db.get_workout(conn, "w1") is None  # never synced → gone


def test_deletion_wins_equal_timestamps():
    conn = make_conn()
    set_cursor(conn)
    ts = "2026-07-18T10:00:00Z"
    events = [updated_event("w1", ts), deleted_event("w1", ts)]
    hevy_sync.ingest_events(conn, FakeHevy(events=events), NOW)
    assert hevy_db.get_workout(conn, "w1") is None


def test_deleted_event_on_synced_row_parks_needs_review():
    conn = make_conn()
    set_cursor(conn)
    hevy_db.upsert_workout(conn, "w1", "T", "2026-07-18T10:00:00Z",
                           "2026-07-18T11:00:00Z", "2026-07-18T09:00:00Z", {})
    hevy_db.set_workout_status(conn, "w1", "merged")
    hevy_sync.ingest_events(
        conn, FakeHevy(events=[deleted_event("w1", "2026-07-18T11:30:00Z")]), NOW)
    row = hevy_db.get_workout(conn, "w1")
    assert row["status"] == "needs_review"
    assert "deleted in Hevy" in row["error"]


# -- mapping gate -----------------------------------------------------------

def seed_row(conn, hevy_id="w1", exercises=None):
    payload = {"id": hevy_id, "title": "Push Day",
               "start_time": "2026-07-18T10:00:00Z",
               "end_time": "2026-07-18T11:00:00Z",
               "exercises": exercises if exercises is not None else []}
    hevy_db.upsert_workout(conn, hevy_id, "Push Day", "2026-07-18T10:00:00Z",
                           "2026-07-18T11:00:00Z", "2026-07-18T11:00:00Z", payload)
    return hevy_db.get_workout(conn, hevy_id)


MAPPED = {"title": "Bench Press (Barbell)", "exercise_template_id": "79D0BB3A",
          "sets": [{"type": "normal", "reps": 5}]}
UNMAPPED = {"title": "Custom Blaster", "exercise_template_id": "CUSTOM01",
            "sets": [{"type": "normal", "reps": 5}]}


def test_gate_passes_when_all_mapped():
    conn = make_conn()
    row = seed_row(conn, exercises=[MAPPED])
    assert hevy_sync.apply_mapping_gate(conn, row, "merge", FakeHevy()) is True


def test_gate_parks_with_titles_and_fetches_template():
    conn = make_conn()
    row = seed_row(conn, exercises=[MAPPED, UNMAPPED])
    hevy = FakeHevy(templates={"CUSTOM01": {
        "id": "CUSTOM01", "title": "Custom Blaster",
        "primary_muscle_group": "chest", "is_custom": True}})
    assert hevy_sync.apply_mapping_gate(conn, row, "merge", hevy) is False
    updated = hevy_db.get_workout(conn, "w1")
    assert updated["status"] == "needs_mapping"
    assert "Custom Blaster" in updated["error"]
    # unknown template got fetched into the cache for the mappings UI
    assert hevy_db.get_template(conn, "CUSTOM01")["title"] == "Custom Blaster"


def test_gate_describe_always_passes():
    conn = make_conn()
    row = seed_row(conn, exercises=[UNMAPPED])
    assert hevy_sync.apply_mapping_gate(conn, row, "describe", FakeHevy()) is True
    assert hevy_db.get_workout(conn, "w1")["status"] == "waiting_watch"


def test_resolve_exercises_raises_on_miss():
    conn = make_conn()
    row = seed_row(conn, exercises=[UNMAPPED])
    from activsync.hevy_mapper import MappingMiss
    with pytest.raises(MappingMiss):
        hevy_sync.resolve_exercises(conn, json.loads(row["payload"]))


def test_resolve_exercises_returns_resolved():
    conn = make_conn()
    row = seed_row(conn, exercises=[MAPPED])
    resolved = hevy_sync.resolve_exercises(conn, json.loads(row["payload"]))
    assert len(resolved) == 1
    assert (resolved[0].category, resolved[0].subcategory) == (0, 1)
    assert resolved[0].sets == MAPPED["sets"]


# -- watch matching ---------------------------------------------------------

def seed_activity(conn, activity_id, start="2026-07-18 10:00:00",
                  duration=3600.0, activity_type="strength_training"):
    db.insert_activity(
        conn, activity_id, activity_type, "Strength", "", start,
        "hash", "held", NOW, garmin_data=json.dumps({"duration": duration}))


def test_match_single_unclaimed():
    conn = make_conn()
    row = seed_row(conn)  # hevy 10:00–11:00 UTC
    seed_activity(conn, 111, start="2026-07-18 10:05:00")
    assert hevy_sync.find_watch_match(conn, row) == 111


def test_match_ignores_wrong_type_and_far_activities():
    conn = make_conn()
    row = seed_row(conn)
    seed_activity(conn, 111, activity_type="running")            # wrong type
    seed_activity(conn, 222, start="2026-07-18 14:00:00")        # too far
    assert hevy_sync.find_watch_match(conn, row) is None


def test_match_rejects_high_drift_even_with_overlap():
    conn = make_conn()
    # hevy 10:00–12:00 (2 h)
    row = seed_row(conn)
    hevy_db.upsert_workout(conn, "w1", "Push Day", "2026-07-18T10:00:00Z",
                           "2026-07-18T12:00:00Z", "2026-07-18T12:30:00Z",
                           json.loads(row["payload"]))
    row = hevy_db.get_workout(conn, "w1")
    # activity starts 25 min early: overlap is fine, drift is not
    seed_activity(conn, 111, start="2026-07-18 09:35:00", duration=9000)
    assert hevy_sync.find_watch_match(conn, row) is None


def test_match_multiple_parks():
    conn = make_conn()
    row = seed_row(conn)
    seed_activity(conn, 111, start="2026-07-18 10:02:00")
    seed_activity(conn, 222, start="2026-07-18 10:04:00")
    assert hevy_sync.find_watch_match(conn, row) == "multiple"


def test_match_only_claimed_candidates():
    conn = make_conn()
    row = seed_row(conn)
    seed_activity(conn, 111, start="2026-07-18 10:05:00")
    seed_row(conn, hevy_id="other")
    assert hevy_db.claim_source(conn, "other", 111) is True
    assert hevy_sync.find_watch_match(conn, row) == "claimed"


def test_match_own_claim_counts_as_match():
    conn = make_conn()
    row = seed_row(conn)
    seed_activity(conn, 111, start="2026-07-18 10:05:00")
    assert hevy_db.claim_source(conn, "w1", 111) is True
    row = hevy_db.get_workout(conn, "w1")
    assert hevy_sync.find_watch_match(conn, row) == 111


# -- Checkpoint C.1 additions ------------------------------------------------

def test_seen_events_still_advance_cursor():
    """Crash after dedupe-record but before cursor persist: the re-poll skips
    the seen event but must still move the cursor past it."""
    conn = make_conn()
    set_cursor(conn)
    event = updated_event("w1", "2026-07-18T10:30:00Z")
    hevy_db.record_event_seen(conn, "updated", "w1", "2026-07-18T10:30:00Z")
    hevy_db.upsert_workout(conn, "w1", "Push Day", "2026-07-18T10:00:00Z",
                           "2026-07-18T11:00:00Z", "2026-07-18T10:30:00Z", {})
    assert hevy_sync.ingest_events(conn, FakeHevy(events=[event]), NOW) == 0
    assert db.get_config_value(conn, CURSOR_KEY) == "2026-07-18T10:30:00Z"


def test_gate_template_fetch_auth_error_propagates():
    from activsync.hevy_client import HevyAuthError

    class AuthFailingHevy(FakeHevy):
        def get_exercise_template(self, template_id):
            raise HevyAuthError("bad key")

    conn = make_conn()
    row = seed_row(conn, exercises=[UNMAPPED])
    with pytest.raises(HevyAuthError):
        hevy_sync.apply_mapping_gate(conn, row, "merge", AuthFailingHevy())


def test_newer_hevy_edit_wakes_its_needs_mapping_workout():
    """Removing the unmapped exercise in Hevy can itself resolve the gate."""
    conn = make_conn()
    set_cursor(conn)
    seed_row(conn, exercises=[UNMAPPED])
    hevy_db.set_workout_status(conn, "w1", "needs_mapping",
                               error="unmapped exercises: Custom Blaster")

    event = updated_event(
        "w1", "2026-07-18T12:00:00Z", exercises=[MAPPED])
    assert hevy_sync.ingest_events(conn, FakeHevy(events=[event]), NOW) == 1

    updated = hevy_db.get_workout(conn, "w1")
    assert updated["status"] == "waiting_watch"
    assert updated["error"] is None
