"""Which journal phase a retry resumes from.

Re-entering a parked operation at the wrong phase is the expensive mistake in
this codebase: resuming at `preparing` for a workout whose FIT already reached
Garmin uploads it a second time. The route picks the phase from what the
journal row carries, and each branch is exercised here.
"""

import pytest
from fastapi.testclient import TestClient

from activsync import db, hevy_db
from activsync.server import create_app


def _seed_parked(conn, hevy_id="w1", **operation_fields):
    """A needs_review workout with an open operation parked at needs_review."""
    hevy_db.upsert_workout(
        conn, hevy_id, "Push Day", "2026-07-19T08:00:00Z", "2026-07-19T09:00:00Z",
        "2026-07-19T10:00:00Z", {"exercises": []})
    hevy_db.set_workout_status(conn, hevy_id, "needs_review", error="parked")
    op_id = hevy_db.open_operation(conn, hevy_id, "replace", 111, [500])
    hevy_db.update_operation(
        conn, op_id, phase="needs_review", attempt_count=4,
        delete_attempt_count=2, last_error="parked", **operation_fields)
    return op_id


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def test_retry_resumes_at_finalizing_when_a_target_activity_exists(conn):
    """The upload already landed and was identified — never re-upload it."""
    _seed_parked(conn, target_activity_id=999)

    response = TestClient(create_app(conn)).post("/api/v1/hevy/w1/retry")

    assert response.status_code == 200
    operation = hevy_db.get_open_operation(conn, "w1")
    assert operation["phase"] == "finalizing"
    assert hevy_db.get_workout(conn, "w1")["status"] == "syncing"


def test_retry_resumes_at_submission_unknown_when_an_upload_id_is_recorded(conn):
    """The FIT was submitted but the resulting activity was never resolved."""
    _seed_parked(conn, upload_id="u1")

    TestClient(create_app(conn)).post("/api/v1/hevy/w1/retry")

    assert hevy_db.get_open_operation(conn, "w1")["phase"] == "submission_unknown"


def test_retry_resumes_at_submission_unknown_when_the_next_step_is_resolve(conn):
    _seed_parked(conn, next_step="resolve")

    TestClient(create_app(conn)).post("/api/v1/hevy/w1/retry")

    assert hevy_db.get_open_operation(conn, "w1")["phase"] == "submission_unknown"


def test_retry_resumes_at_preparing_only_when_nothing_was_submitted(conn):
    _seed_parked(conn)

    TestClient(create_app(conn)).post("/api/v1/hevy/w1/retry")

    assert hevy_db.get_open_operation(conn, "w1")["phase"] == "preparing"


def test_retry_clears_the_attempt_counters_and_error(conn):
    """Otherwise a resumed operation parks again after one tick."""
    _seed_parked(conn, target_activity_id=999)

    TestClient(create_app(conn)).post("/api/v1/hevy/w1/retry")

    operation = hevy_db.get_open_operation(conn, "w1")
    assert operation["attempt_count"] == 0
    assert operation["delete_attempt_count"] == 0
    assert operation["last_error"] is None
