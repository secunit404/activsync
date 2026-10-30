import re
import threading
from datetime import datetime, timezone
from pathlib import Path

import pytest

from auth_helpers import owner_client

from activsync import db, hevy_db, hevy_match_schedule, hevy_sync
from activsync.hevy_client import HevyAuthError
from activsync.server import create_app

NOW = datetime(2026, 7, 22, 12, 0, tzinfo=timezone.utc)


class Clock:
    def __init__(self, start: float = 1_000_000.0):
        self.value = start

    def __call__(self) -> float:
        return self.value

    def advance(self, seconds: float) -> None:
        self.value += seconds


def _seed_workout(conn, hevy_id="hevy-1", status="awaiting_match", title="Leg Day"):
    hevy_db.upsert_workout(
        conn, hevy_id, title, "2026-07-22T10:00:00Z", "2026-07-22T11:00:00Z",
        "2026-07-22T11:05:00Z", {"exercises": []},
    )
    hevy_db.set_workout_status(conn, hevy_id, status)


def _seed_activity(conn, activity_id=123, status="held"):
    db.insert_activity(
        conn, activity_id, "strength_training", "Morning strength", "",
        "2026-07-22 10:00:00", f"hash-{activity_id}", status, NOW,
        garmin_data='{"duration": 3600}',
    )


class Applier:
    def __init__(self, result="merged", error=None):
        self.result = result
        self.error = error
        self.calls = []

    def __call__(self, hevy_id, strategy):
        self.calls.append((hevy_id, strategy))
        if self.error is not None:
            raise self.error
        return self.result


@pytest.fixture
def env(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _seed_workout(conn)
    apply = Applier()
    clock = Clock()
    scheduler = hevy_match_schedule.HevyMatchScheduler(
        conn, apply_hevy_match=apply, clock=clock
    )
    client = owner_client(create_app(conn, hevy_match_scheduler=scheduler))
    return conn, client, scheduler, clock, apply


def _choose(client, strategy="merge", hevy_id="hevy-1"):
    return client.post(f"/api/v1/hevy/{hevy_id}/match", json={"strategy": strategy})


def _state(conn, hevy_id="hevy-1"):
    row = hevy_match_schedule.get(conn, hevy_id)
    return row["state"] if row else None


def test_choosing_a_match_schedules_it_and_applies_nothing_yet(env):
    conn, client, scheduler, clock, apply = env

    response = _choose(client)

    assert response.status_code == 200
    body = response.json()
    assert set(body) == {"deadline", "strategy", "remainingMs", "undoWindowMs"}
    assert body["strategy"] == "merge"
    assert body["remainingMs"] == 5000
    assert body["undoWindowMs"] == 5000
    assert body["deadline"] == hevy_match_schedule.deadline_iso(clock.value + 5)
    assert scheduler.run_due() == 0
    assert apply.calls == []
    assert _state(conn) == "waiting"


def test_the_queue_reports_the_waiting_and_applying_phases(env):
    conn, client, scheduler, clock, _ = env
    _choose(client)

    waiting = client.get("/api/v1/hevy/queue").json()["inFlight"][0]
    assert waiting["matchPhase"] == "waiting"
    assert waiting["matchScheduledAt"] == hevy_match_schedule.deadline_iso(clock.value + 5)

    hevy_match_schedule.claim_due(conn, clock.value + 5)
    applying = client.get("/api/v1/hevy/queue").json()["inFlight"][0]
    assert applying["matchPhase"] == "applying"


def test_an_unscheduled_workout_has_no_match_phase(env):
    _, client, *_ = env

    item = client.get("/api/v1/hevy/queue").json()["inFlight"][0]

    assert item["matchPhase"] is None
    assert item["matchScheduledAt"] is None


def test_the_deadline_applies_the_choice_and_reports_it(env):
    conn, client, scheduler, clock, apply = env
    _choose(client, "describe")

    clock.advance(5)
    assert scheduler.run_due() == 1

    assert apply.calls == [("hevy-1", "describe")]
    status = client.get("/api/v1/hevy/hevy-1/match/status").json()
    assert status == {"state": "done", "message": "Merged Leg Day."}


def test_nothing_is_applied_before_the_deadline(env):
    conn, client, scheduler, clock, apply = env
    _choose(client)

    clock.advance(4.9)

    assert scheduler.run_due() == 0
    assert apply.calls == []
    assert client.get("/api/v1/hevy/hevy-1/match/status").json()["state"] == "waiting"


def test_only_due_decisions_are_claimed(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    hevy_match_schedule.schedule(conn, "early", "merge", 100.0)
    hevy_match_schedule.schedule(conn, "late", "describe", 110.0)

    claimed = hevy_match_schedule.claim_due(conn, 106.0)

    assert [row["hevy_id"] for row in claimed] == ["early"]
    assert _state(conn, "early") == "applying"
    assert _state(conn, "late") == "waiting"


def test_undo_before_the_deadline_drops_the_decision(env):
    conn, client, scheduler, clock, apply = env
    _choose(client)

    response = client.post("/api/v1/hevy/hevy-1/match/cancel")

    assert response.status_code == 200
    assert response.json() == {"cancelled": True}
    clock.advance(10)
    assert scheduler.run_due() == 0
    assert apply.calls == []
    assert hevy_match_schedule.get(conn, "hevy-1") is None
    assert client.get("/api/v1/hevy/hevy-1/match/status").status_code == 404


def test_undo_after_the_scheduler_claimed_it_is_too_late(env):
    conn, client, scheduler, clock, _ = env
    _choose(client)
    hevy_match_schedule.claim_due(conn, clock.value + 5)

    response = client.post("/api/v1/hevy/hevy-1/match/cancel")

    assert response.status_code == 409
    assert response.json()["detail"] == "Too late to undo: Garmin is already being updated."
    assert _state(conn) == "applying"


def test_undo_with_nothing_scheduled_is_a_404(env):
    _, client, *_ = env

    assert client.post("/api/v1/hevy/hevy-1/match/cancel").status_code == 404


def test_scheduling_twice_is_refused_while_a_decision_is_waiting_or_applying(env):
    conn, client, scheduler, clock, _ = env
    _choose(client)

    waiting = _choose(client, "describe")
    hevy_match_schedule.claim_due(conn, clock.value + 5)
    applying = _choose(client, "describe")

    for response in (waiting, applying):
        assert response.status_code == 409
        assert response.json()["detail"] == "This workout is already being matched."


def test_a_workout_can_be_decided_again_after_an_undo_or_a_failure(env):
    conn, client, scheduler, clock, apply = env
    _choose(client)
    client.post("/api/v1/hevy/hevy-1/match/cancel")
    apply.error = RuntimeError("boom")
    assert _choose(client, "describe").status_code == 200
    clock.advance(5)
    scheduler.run_due()
    assert _state(conn) == "failed"

    assert _choose(client).status_code == 200

    assert _state(conn) == "waiting"
    assert hevy_match_schedule.get(conn, "hevy-1")["message"] is None


def test_cancel_and_claim_race_has_exactly_one_winner(tmp_path):
    for round_number in range(25):
        conn = db.connect(str(tmp_path / f"race-{round_number}.db"))
        hevy_match_schedule.schedule(conn, "hevy-1", "merge", 0.0)
        results = {}
        barrier = threading.Barrier(2)

        def do_cancel():
            barrier.wait()
            results["cancel"] = hevy_match_schedule.cancel(conn, "hevy-1")

        def do_claim():
            barrier.wait()
            results["claim"] = len(hevy_match_schedule.claim_due(conn, 10.0))

        threads = [threading.Thread(target=do_cancel), threading.Thread(target=do_claim)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()

        assert results["cancel"] + results["claim"] == 1


def test_a_cancel_that_wins_leaves_nothing_to_claim(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    hevy_match_schedule.schedule(conn, "hevy-1", "merge", 0.0)

    assert hevy_match_schedule.cancel(conn, "hevy-1") == 1
    assert hevy_match_schedule.claim_due(conn, 10.0) == []


def test_a_claim_that_wins_cannot_be_cancelled(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    hevy_match_schedule.schedule(conn, "hevy-1", "merge", 0.0)

    assert len(hevy_match_schedule.claim_due(conn, 10.0)) == 1
    assert hevy_match_schedule.cancel(conn, "hevy-1") == 0


def test_a_workout_that_left_awaiting_match_is_skipped_not_applied(env):
    conn, client, scheduler, clock, apply = env
    _choose(client)
    hevy_db.set_workout_status(conn, "hevy-1", "merged")

    clock.advance(5)
    scheduler.run_due()

    assert apply.calls == []
    assert client.get("/api/v1/hevy/hevy-1/match/status").json() == {
        "state": "skipped", "message": None,
    }


def test_a_deleted_workout_is_skipped(env):
    conn, client, scheduler, clock, apply = env
    _choose(client)
    hevy_db.delete_workout(conn, "hevy-1")

    clock.advance(5)
    scheduler.run_due()

    assert apply.calls == []
    assert _state(conn) == "skipped"


def test_a_rejected_key_fails_the_decision_and_marks_hevy_for_reconnect(env):
    conn, client, scheduler, clock, apply = env
    apply.error = HevyAuthError("bad key")
    _choose(client)

    clock.advance(5)
    scheduler.run_due()

    assert client.get("/api/v1/hevy/hevy-1/match/status").json() == {
        "state": "failed",
        "message": "Hevy rejected the stored API key — reconnect Hevy.",
    }
    assert db.get_config_value(conn, "hevy_auth_ok") is False


def test_missing_exercise_mappings_fail_the_decision(env):
    conn, client, scheduler, clock, apply = env
    apply.result = "needs_mapping"
    _choose(client)

    clock.advance(5)
    scheduler.run_due()

    assert client.get("/api/v1/hevy/hevy-1/match/status").json() == {
        "state": "failed",
        "message": "Map the missing exercises before choosing Merge or Replace.",
    }


def test_an_unexpected_error_fails_the_decision_with_a_generic_message(env, caplog):
    conn, client, scheduler, clock, apply = env
    apply.error = RuntimeError("secret detail")
    _choose(client)

    clock.advance(5)
    scheduler.run_due()

    status = client.get("/api/v1/hevy/hevy-1/match/status").json()
    assert status == {
        "state": "failed",
        "message": "Could not apply that Hevy match. Check the server log for details.",
    }
    assert "secret detail" in caplog.text


@pytest.mark.parametrize(
    ("result", "message"),
    [
        ("merged", "Merged Leg Day."),
        ("replaced", "Replaced Leg Day."),
        ("described", "Described Leg Day."),
    ],
)
def test_success_messages_name_what_happened(env, result, message):
    conn, client, scheduler, clock, apply = env
    apply.result = result
    _choose(client)

    clock.advance(5)
    scheduler.run_due()

    assert client.get("/api/v1/hevy/hevy-1/match/status").json()["message"] == message


def test_a_scheduler_without_an_apply_function_claims_nothing(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    clock = Clock()
    scheduler = hevy_match_schedule.HevyMatchScheduler(
        conn, apply_hevy_match=None, clock=clock
    )
    hevy_match_schedule.schedule(conn, "hevy-1", "merge", clock.value - 10)

    assert scheduler.run_due() == 0
    assert _state(conn) == "waiting"


def test_choosing_without_an_apply_function_is_a_503(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _seed_workout(conn)
    client = owner_client(create_app(conn))

    response = _choose(client)

    assert response.status_code == 503
    assert hevy_match_schedule.get(conn, "hevy-1") is None


def test_choosing_keeps_the_existing_validation(env):
    conn, client, *_ = env
    _seed_workout(conn, "hevy-done", status="merged")
    _seed_workout(conn, "hevy-strava")
    _seed_activity(conn)
    db.set_published(conn, 123, 456, NOW)
    assert hevy_db.claim_source(conn, "hevy-strava", 123)

    assert _choose(client, hevy_id="missing").status_code == 404
    assert _choose(client, hevy_id="hevy-done").status_code == 409
    replace = _choose(client, "replace", hevy_id="hevy-strava")
    assert replace.status_code == 409
    assert "already on Strava" in replace.json()["detail"]
    assert hevy_match_schedule.get(conn, "hevy-strava") is None
    assert _choose(client, "merge", hevy_id="hevy-strava").status_code == 200


def test_skipping_during_the_window_cancels_the_decision(env):
    conn, client, scheduler, clock, apply = env
    _choose(client)

    response = client.post("/api/v1/hevy/hevy-1/skip")

    assert response.status_code == 200
    assert hevy_match_schedule.get(conn, "hevy-1") is None
    assert hevy_db.get_workout(conn, "hevy-1")["status"] == "skipped"
    clock.advance(10)
    scheduler.run_due()
    assert apply.calls == []


def test_skipping_while_garmin_is_being_updated_is_refused(env):
    conn, client, scheduler, clock, _ = env
    _choose(client)
    hevy_match_schedule.claim_due(conn, clock.value + 5)

    response = client.post("/api/v1/hevy/hevy-1/skip")

    assert response.status_code == 409
    assert hevy_db.get_workout(conn, "hevy-1")["status"] == "awaiting_match"


def test_the_hevy_poller_leaves_a_workout_with_a_pending_decision_alone(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _seed_workout(conn)
    hevy_match_schedule.schedule(conn, "hevy-1", "merge", 0.0)
    row = hevy_db.get_workout(conn, "hevy-1")

    class Boom:
        def __getattr__(self, name):
            raise AssertionError(f"touched {name}")

    hevy_sync.process_workout(conn, Boom(), Boom(), row, {}, NOW)

    assert hevy_db.get_workout(conn, "hevy-1")["status"] == "awaiting_match"
    assert hevy_db.get_workout(conn, "hevy-1")["locked_until"] is None


def test_recover_requeues_decisions_a_dead_process_was_applying(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    hevy_match_schedule.schedule(conn, "applying", "merge", 0.0)
    hevy_match_schedule.schedule(conn, "waiting", "merge", 100.0)
    hevy_match_schedule.schedule(conn, "finished", "merge", 0.0)
    hevy_match_schedule.claim_due(conn, 10.0)
    hevy_match_schedule.finish(conn, "finished", hevy_match_schedule.DONE, "ok", 10.0)

    assert hevy_match_schedule.recover(conn) == 1

    assert _state(conn, "applying") == "waiting"
    assert _state(conn, "waiting") == "waiting"
    assert _state(conn, "finished") == "done"


def test_a_recovered_decision_is_applied_on_the_next_pass(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _seed_workout(conn)
    apply = Applier()
    clock = Clock()
    scheduler = hevy_match_schedule.HevyMatchScheduler(
        conn, apply_hevy_match=apply, clock=clock
    )
    hevy_match_schedule.schedule(conn, "hevy-1", "merge", clock.value - 10)
    hevy_match_schedule.claim_due(conn, clock.value)

    hevy_match_schedule.recover(conn)
    scheduler.run_due()

    assert apply.calls == [("hevy-1", "merge")]
    assert _state(conn) == "done"


def test_finished_results_are_pruned_after_an_hour(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    hevy_match_schedule.schedule(conn, "hevy-1", "merge", 0.0)
    hevy_match_schedule.claim_due(conn, 10.0)
    hevy_match_schedule.finish(conn, "hevy-1", hevy_match_schedule.DONE, "ok", 10.0)

    hevy_match_schedule.prune(conn, 10.0 + 3599)
    assert hevy_match_schedule.get(conn, "hevy-1") is not None
    hevy_match_schedule.prune(conn, 10.0 + 3601)
    assert hevy_match_schedule.get(conn, "hevy-1") is None


def test_the_table_is_created_with_the_rest_of_the_schema(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))

    columns = {row["name"] for row in conn.execute("PRAGMA table_info(hevy_match_schedule)")}

    assert columns == {"hevy_id", "strategy", "deadline", "state", "message", "finished_at"}


def test_the_frontend_undo_window_matches_the_server():
    source = (
        Path(__file__).resolve().parents[1] / "app" / "hooks" / "use-hevy-match-queue.tsx"
    ).read_text()
    match = re.search(r"export const HEVY_MATCH_UNDO_WINDOW_MS = (\d+);", source)

    assert match is not None
    assert int(match.group(1)) == round(hevy_match_schedule.UNDO_WINDOW_SECONDS * 1000)
