"""Tests for the Hevy sync data model (hevy_db)."""

import sqlite3
import threading
import time
from datetime import datetime, timedelta, timezone

import pytest

from activsync import db, hevy_db


def make_conn():
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    hevy_db.init_schema(conn)
    return conn


def iso(dt: datetime) -> str:
    return dt.isoformat()


NOW = datetime(2026, 7, 18, 12, 0, 0, tzinfo=timezone.utc)


def seed_workout(conn, hevy_id="w1", updated_at=iso(NOW), title="Push Day"):
    hevy_db.upsert_workout(
        conn, hevy_id, title, "2026-07-18 10:00:00", "2026-07-18 11:00:00",
        updated_at, {"exercises": []},
    )


# -- init_schema ------------------------------------------------------------

def test_init_schema_is_idempotent():
    conn = make_conn()
    hevy_db.init_schema(conn)  # second call must not raise
    tables = {
        row["name"]
        for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")
    }
    assert {"hevy_workouts", "hevy_operations", "exercise_templates",
            "exercise_mappings", "merge_backups", "hevy_events_seen"} <= tables


# -- upsert_workout ---------------------------------------------------------

def test_upsert_inserts_with_waiting_watch_status():
    conn = make_conn()
    seed_workout(conn)
    row = hevy_db.get_workout(conn, "w1")
    assert row is not None
    assert row["status"] == "waiting_watch"
    assert row["title"] == "Push Day"


def test_upsert_newer_revision_updates_payload_and_title():
    conn = make_conn()
    seed_workout(conn, updated_at=iso(NOW))
    hevy_db.upsert_workout(
        conn, "w1", "Push Day v2", "2026-07-18 10:05:00", "2026-07-18 11:05:00",
        iso(NOW + timedelta(minutes=5)), {"exercises": [{"title": "Bench"}]},
    )
    row = hevy_db.get_workout(conn, "w1")
    assert row["title"] == "Push Day v2"
    assert row["source_updated_at"] == iso(NOW + timedelta(minutes=5))
    assert "Bench" in row["payload"]


def test_upsert_older_revision_is_ignored():
    conn = make_conn()
    seed_workout(conn, updated_at=iso(NOW))
    hevy_db.upsert_workout(
        conn, "w1", "Stale", "2026-07-18 09:00:00", "2026-07-18 10:00:00",
        iso(NOW - timedelta(minutes=5)), {"stale": True},
    )
    row = hevy_db.get_workout(conn, "w1")
    assert row["title"] == "Push Day"
    assert row["source_updated_at"] == iso(NOW)


def test_upsert_never_regresses_status():
    conn = make_conn()
    seed_workout(conn, updated_at=iso(NOW))
    hevy_db.set_workout_status(conn, "w1", "merged")
    hevy_db.upsert_workout(
        conn, "w1", "Push Day v2", "2026-07-18 10:00:00", "2026-07-18 11:00:00",
        iso(NOW + timedelta(minutes=5)), {},
    )
    assert hevy_db.get_workout(conn, "w1")["status"] == "merged"


def test_list_workouts_filters_by_status():
    conn = make_conn()
    seed_workout(conn, "w1")
    seed_workout(conn, "w2")
    hevy_db.set_workout_status(conn, "w2", "failed", error="boom")
    failed = hevy_db.list_workouts(conn, status="failed")
    assert [r["hevy_id"] for r in failed] == ["w2"]
    assert failed[0]["error"] == "boom"
    assert len(hevy_db.list_workouts(conn)) == 2


# -- claim_source -----------------------------------------------------------

def test_claim_source_is_unique():
    conn = make_conn()
    seed_workout(conn, "w1")
    seed_workout(conn, "w2")
    assert hevy_db.claim_source(conn, "w1", 111) is True
    assert hevy_db.claim_source(conn, "w2", 111) is False
    assert hevy_db.get_workout(conn, "w2")["source_garmin_activity_id"] is None


def test_claim_source_missing_row_and_reclaim_fail():
    conn = make_conn()
    seed_workout(conn, "w1")
    assert hevy_db.claim_source(conn, "missing", 111) is False
    assert hevy_db.claim_source(conn, "w1", 111) is True
    # a claim is immutable — no silent overwrite with a different source
    assert hevy_db.claim_source(conn, "w1", 222) is False
    assert hevy_db.get_workout(conn, "w1")["source_garmin_activity_id"] == 111
    # re-claiming the identical source is an idempotent success
    assert hevy_db.claim_source(conn, "w1", 111) is True


def test_link_target_records_strategy_and_provenance():
    conn = make_conn()
    seed_workout(conn, "w1")
    hevy_db.link_target(conn, "w1", 222, "merge")
    row = hevy_db.get_workout(conn, "w1")
    assert row["garmin_activity_id"] == 222
    assert row["applied_strategy"] == "merge"
    assert row["provenance"] == "activsync"


def test_link_target_strategy_is_immutable_and_missing_row_raises():
    conn = make_conn()
    seed_workout(conn, "w1")
    hevy_db.link_target(conn, "w1", 222, "merge")
    hevy_db.link_target(conn, "w1", 222, "replace")  # later strategy ignored
    assert hevy_db.get_workout(conn, "w1")["applied_strategy"] == "merge"
    import pytest
    with pytest.raises(ValueError):
        hevy_db.link_target(conn, "missing", 1, "merge")


# -- leases -----------------------------------------------------------------

def test_acquire_lease_blocks_second_acquire_until_expiry():
    conn = make_conn()
    seed_workout(conn, "w1")
    token = hevy_db.acquire_lease(conn, "w1", NOW)
    assert token
    assert hevy_db.acquire_lease(conn, "w1", NOW + timedelta(seconds=10)) is None
    # after expiry (default 300 s) the lease can be taken again
    assert hevy_db.acquire_lease(conn, "w1", NOW + timedelta(seconds=301))


def test_release_lease_frees_immediately():
    conn = make_conn()
    seed_workout(conn, "w1")
    token = hevy_db.acquire_lease(conn, "w1", NOW)
    hevy_db.release_lease(conn, "w1", token)
    assert hevy_db.acquire_lease(conn, "w1", NOW + timedelta(seconds=1))


def test_stale_owner_cannot_release_new_owners_lease():
    conn = make_conn()
    seed_workout(conn, "w1")
    token_a = hevy_db.acquire_lease(conn, "w1", NOW)
    # A's lease expires; B acquires
    token_b = hevy_db.acquire_lease(conn, "w1", NOW + timedelta(seconds=301))
    assert token_b and token_b != token_a
    # A finishes late and releases — must NOT clear B's lease
    hevy_db.release_lease(conn, "w1", token_a)
    assert hevy_db.acquire_lease(conn, "w1", NOW + timedelta(seconds=310)) is None


# -- set_applied ------------------------------------------------------------

def test_set_applied_tracks_sides_separately():
    conn = make_conn()
    seed_workout(conn, "w1")
    hevy_db.set_applied(conn, "w1", "garmin", iso(NOW))
    row = hevy_db.get_workout(conn, "w1")
    assert row["garmin_applied_updated_at"] == iso(NOW)
    assert row["strava_applied_updated_at"] is None


# -- operations journal -----------------------------------------------------

def test_open_operation_unique_per_hevy_id():
    conn = make_conn()
    seed_workout(conn, "w1")
    op_id = hevy_db.open_operation(conn, "w1", "replace", 111, [1, 2])
    assert op_id is not None
    assert hevy_db.open_operation(conn, "w1", "replace", 333, []) is None


def test_open_operation_unique_per_source():
    conn = make_conn()
    seed_workout(conn, "w1")
    seed_workout(conn, "w2")
    assert hevy_db.open_operation(conn, "w1", "replace", 111, []) is not None
    assert hevy_db.open_operation(conn, "w2", "replace", 111, []) is None


def test_closed_operation_frees_the_unique_slots():
    conn = make_conn()
    seed_workout(conn, "w1")
    op_id = hevy_db.open_operation(conn, "w1", "replace", 111, [])
    hevy_db.close_operation(conn, op_id, "done")
    assert hevy_db.get_open_operation(conn, "w1") is None
    assert hevy_db.open_operation(conn, "w1", "replace", 111, []) is not None


def test_update_operation_sets_fields():
    conn = make_conn()
    seed_workout(conn, "w1")
    op_id = hevy_db.open_operation(conn, "w1", "upload_passive", None, [])
    hevy_db.update_operation(conn, op_id, phase="uploading", upload_id="u-9",
                             attempt_count=2)
    op = hevy_db.get_open_operation(conn, "w1")
    assert op["phase"] == "uploading"
    assert op["upload_id"] == "u-9"
    assert op["attempt_count"] == 2


def test_operation_outcome_rolls_back_if_workout_transition_fails():
    """The op must not close if its paired workout update cannot land."""
    conn = make_conn()
    seed_workout(conn, "w1")
    op_id = hevy_db.open_operation(conn, "w1", "replace", 111, [])
    # Simulate a corrupt/missing paired row after the operation was opened.
    conn.execute("DELETE FROM hevy_workouts WHERE hevy_id = 'w1'")
    conn.commit()

    with pytest.raises(ValueError):
        hevy_db.set_operation_outcome(
            conn, op_id, "w1", "failed", "failed", "definite rejection")

    op = conn.execute(
        "SELECT phase FROM hevy_operations WHERE id = ?", (op_id,)
    ).fetchone()
    assert op["phase"] == "preparing"


def test_complete_operation_is_atomic_under_concurrent_commit(tmp_path):
    """CRITICAL 2 regression test.

    `complete_operation` issues two UPDATEs before its commit. With
    `isolation_level=""`, the implicit BEGIN opened by the first UPDATE is
    connection-global — so on the shared connection this app actually uses
    (one Connection, many threads), an unrelated thread's `commit()` landing
    between the two UPDATEs would flush the first one early, leaving
    `hevy_operations.phase='done'` durable while `hevy_workouts
    .garmin_activity_id` is still NULL. That torn state is exactly the
    "closed journal with an unlinked replacement" that causes a duplicate
    re-upload on the next poller tick.

    This uses `db.connect` (not the bare in-memory `make_conn` helper used
    elsewhere in this file) because the bug only reproduces on the real
    shared, lock-guarded connection this app hands to both the poller and
    the request thread pool.
    """
    conn = db.connect(str(tmp_path / "concurrency.db"))
    now = datetime.now(timezone.utc)
    hevy_db.upsert_workout(
        conn, "w1", "Push Day", "2026-01-01 10:00:00", "2026-01-01 11:00:00",
        now.isoformat(), {"exercises": []},
    )
    op_id = hevy_db.open_operation(conn, "w1", "replace", 111, [])
    assert op_id is not None

    first_statement_done = threading.Event()
    observed = {}

    real_execute = conn.execute

    def spying_execute(sql, *args, **kwargs):
        result = real_execute(sql, *args, **kwargs)
        if "hevy_operations SET phase = 'done'" in sql:
            # Simulate the window between complete_operation's two
            # statements in which a concurrent thread could act.
            first_statement_done.set()
            time.sleep(0.3)
        return result

    conn.execute = spying_execute

    def do_complete():
        hevy_db.complete_operation(
            conn, op_id, "w1", 999, "replace", "replaced", now.isoformat())

    def do_unrelated_write():
        assert first_statement_done.wait(timeout=2), "first statement never ran"
        # An unrelated write+commit from another thread, exactly the kind
        # that FastAPI's worker threadpool or the poller would issue.
        hevy_db.upsert_workout(
            conn, "w2", "Other", "2026-01-01 09:00:00", "2026-01-01 09:30:00",
            now.isoformat(), {"exercises": []},
        )
        op_row = conn.execute(
            "SELECT phase FROM hevy_operations WHERE id = ?", (op_id,)
        ).fetchone()
        workout_row = conn.execute(
            "SELECT garmin_activity_id FROM hevy_workouts WHERE hevy_id = ?", ("w1",)
        ).fetchone()
        observed["phase"] = op_row["phase"]
        observed["garmin_activity_id"] = workout_row["garmin_activity_id"]

    t_complete = threading.Thread(target=do_complete)
    t_other = threading.Thread(target=do_unrelated_write)
    t_complete.start()
    t_other.start()
    t_complete.join(timeout=5)
    t_other.join(timeout=5)
    conn.execute = real_execute

    assert not t_complete.is_alive() and not t_other.is_alive(), "threads did not finish"

    # The torn state: the operation journal reads 'done' (closed) while the
    # workout it should have linked is still unlinked. A crash at this exact
    # moment is what produces the duplicate re-upload.
    torn = observed["phase"] == "done" and observed["garmin_activity_id"] is None
    assert not torn, (
        "observed a torn transaction: hevy_operations.phase='done' became "
        "durable before hevy_workouts.garmin_activity_id was set"
    )

    # Sanity: the operation itself still completes correctly once both
    # threads are done.
    final_op = conn.execute(
        "SELECT phase FROM hevy_operations WHERE id = ?", (op_id,)
    ).fetchone()
    final_workout = conn.execute(
        "SELECT garmin_activity_id FROM hevy_workouts WHERE hevy_id = ?", ("w1",)
    ).fetchone()
    assert final_op["phase"] == "done"
    assert final_workout["garmin_activity_id"] == 999


def test_pre_upload_ids_round_trip_as_list():
    conn = make_conn()
    seed_workout(conn, "w1")
    op_id = hevy_db.open_operation(conn, "w1", "replace", 111, [1, 2])
    assert hevy_db.get_open_operation(conn, "w1")["pre_upload_ids"] == [1, 2]
    hevy_db.update_operation(conn, op_id, pre_upload_ids=[3, 4, 5])
    assert hevy_db.get_open_operation(conn, "w1")["pre_upload_ids"] == [3, 4, 5]


# -- merge backups ----------------------------------------------------------

def test_save_backup_is_immutable():
    conn = make_conn()
    hevy_db.save_backup(conn, 111, "w1", {"sets": [1]}, b"fitbytes")
    hevy_db.save_backup(conn, 111, "w2", {"sets": [2]}, None)
    backup = hevy_db.get_backup(conn, 111)
    assert backup["hevy_id"] == "w1"
    assert "1" in backup["original_sets"]
    assert backup["original_fit"] == b"fitbytes"


def test_get_backup_missing_returns_none():
    conn = make_conn()
    assert hevy_db.get_backup(conn, 999) is None


# -- exercise templates + mappings ------------------------------------------

def test_template_upsert_and_listing():
    conn = make_conn()
    hevy_db.upsert_template(conn, {
        "exercise_template_id": "t1", "title": "Bench Press (Barbell)",
        "primary_muscle_group": "chest", "secondary_muscle_groups": ["triceps"],
        "equipment_category": "barbell", "is_custom": False,
    })
    hevy_db.upsert_template(conn, {
        "exercise_template_id": "t2", "title": "My Custom Curl",
        "primary_muscle_group": "biceps", "secondary_muscle_groups": [],
        "equipment_category": "dumbbell", "is_custom": True,
    })
    assert len(hevy_db.list_templates(conn)) == 2
    custom = hevy_db.list_templates(conn, only_custom=True)
    assert [t["exercise_template_id"] for t in custom] == ["t2"]
    assert hevy_db.get_template(conn, "t1")["title"] == "Bench Press (Barbell)"


def test_mapping_crud_and_rejected_flag():
    conn = make_conn()
    hevy_db.save_mapping(conn, "t1", 0, 1)
    m = hevy_db.get_mapping(conn, "t1")
    assert (m["category"], m["subcategory"]) == (0, 1)
    assert m["garmin_rejected"] == 0

    hevy_db.mark_mapping_rejected(conn, "t1")
    assert hevy_db.get_mapping(conn, "t1")["garmin_rejected"] == 1

    # re-saving clears the rejected flag
    hevy_db.save_mapping(conn, "t1", 0, 2)
    m = hevy_db.get_mapping(conn, "t1")
    assert m["subcategory"] == 2
    assert m["garmin_rejected"] == 0

    assert len(hevy_db.list_mappings(conn)) == 1
    hevy_db.delete_mapping(conn, "t1")
    assert hevy_db.get_mapping(conn, "t1") is None


@pytest.mark.parametrize(("strategy", "expected_status"), [
    ("merge", "merged"),
    ("describe", "described"),
    ("replace", "replaced"),
    ("passive", "uploaded_passive"),
    (None, "waiting_watch"),
])
def test_waking_mapping_row_preserves_applied_strategy(strategy, expected_status):
    conn = make_conn()
    seed_workout(conn)
    if strategy is not None:
        hevy_db.link_target(conn, "w1", 901, strategy)
    hevy_db.set_workout_status(conn, "w1", "needs_mapping")

    assert hevy_db.wake_needs_mapping(conn, hevy_id="w1") == 1
    assert hevy_db.get_workout(conn, "w1")["status"] == expected_status


# -- events dedupe ----------------------------------------------------------

def test_record_event_seen_dedupes():
    conn = make_conn()
    assert hevy_db.record_event_seen(conn, "updated", "w1", iso(NOW)) is True
    assert hevy_db.record_event_seen(conn, "updated", "w1", iso(NOW)) is False
    # different timestamp or type is a different event
    assert hevy_db.record_event_seen(conn, "updated", "w1",
                                     iso(NOW + timedelta(minutes=1))) is True
    assert hevy_db.record_event_seen(conn, "deleted", "w1", iso(NOW)) is True


# -- statuses constant ------------------------------------------------------

def test_statuses_constant_matches_spec():
    assert set(hevy_db.STATUSES) == {
        "needs_mapping", "waiting_watch", "syncing", "merged", "described",
        "replaced", "uploaded_passive", "linked_existing", "failed",
        "needs_review", "skipped",
    }
