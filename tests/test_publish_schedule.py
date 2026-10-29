import re
import threading
import time
from datetime import datetime, timezone
from pathlib import Path
from unittest.mock import MagicMock

import pytest

from auth_helpers import owner_client

from activsync import db, hevy_db, publish_schedule, sync
from activsync.server import create_app

NOW = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)


class Clock:
    def __init__(self, start: float = 1_000_000.0):
        self.value = start

    def __call__(self) -> float:
        return self.value

    def advance(self, seconds: float) -> None:
        self.value += seconds


def _connect_services(conn):
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_tokens", {"refresh_token": "refresh"})


def _insert(conn, activity_id, status="pending", title=None):
    db.insert_activity(
        conn, activity_id, "running", title or f"Run {activity_id}", "",
        f"2026-07-{activity_id % 28 + 1:02d} 08:00:00", f"hash-{activity_id}", status, NOW,
    )


def _clients():
    garmin = MagicMock()
    garmin.download_fit.return_value = b"FIT"
    strava = MagicMock()
    strava.find_existing_activity.return_value = None
    strava.publish.side_effect = lambda activity_id, *args, **kwargs: 9_000 + activity_id
    return garmin, strava


@pytest.fixture
def env(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _connect_services(conn)
    garmin, strava = _clients()
    clock = Clock()
    scheduler = publish_schedule.PublishScheduler(
        conn,
        build_garmin_client=lambda conn: garmin,
        build_strava_client=lambda conn: strava,
        clock=clock,
    )
    client = owner_client(create_app(conn, publish_scheduler=scheduler))
    return conn, client, scheduler, clock, strava


def _activity_item(client, activity_id):
    items = client.get("/api/v1/activities").json()["items"]
    return next(item for item in items if item["garminActivityId"] == activity_id)


def test_scheduling_returns_the_deadline_and_sends_nothing_yet(env):
    conn, client, scheduler, clock, strava = env
    _insert(conn, 1)

    response = client.post("/api/v1/activities/1/publish/schedule")

    assert response.status_code == 200
    body = response.json()
    assert set(body) == {"batchId", "publishScheduledAt", "remainingMs", "undoWindowMs"}
    assert body["remainingMs"] == 5000
    assert body["undoWindowMs"] == 5000
    item = _activity_item(client, 1)
    assert item["publishPhase"] == "waiting"
    assert item["publishScheduledAt"] == body["publishScheduledAt"]
    assert scheduler.run_due() == 0
    strava.publish.assert_not_called()
    assert db.get_activity(conn, 1)["publish_status"] == "pending"


def test_an_unscheduled_activity_reports_no_publish_state(env):
    conn, client, *_ = env
    _insert(conn, 1)

    item = _activity_item(client, 1)

    assert item["publishScheduledAt"] is None
    assert item["publishPhase"] is None


def test_the_scheduler_publishes_once_the_deadline_passes(env):
    conn, client, scheduler, clock, strava = env
    _insert(conn, 1, title="Lunch run")
    batch_id = client.post("/api/v1/activities/1/publish/schedule").json()["batchId"]

    clock.advance(4.9)
    assert scheduler.run_due() == 0
    assert client.get(f"/api/v1/activities/publish/batches/{batch_id}").json()["state"] == "waiting"
    clock.advance(0.1)
    assert scheduler.run_due() == 1

    assert db.get_activity(conn, 1)["publish_status"] == "published"
    strava.publish.assert_called_once()
    batch = client.get(f"/api/v1/activities/publish/batches/{batch_id}").json()
    assert batch["state"] == "done"
    assert batch["result"]["message"] == "Published Lunch run to Strava"
    assert batch["error"] is None
    assert _activity_item(client, 1)["publishPhase"] is None


def test_a_missing_activity_is_reported_as_republished(env):
    conn, client, scheduler, clock, _ = env
    _insert(conn, 1, status="missing", title="Lost run")
    batch_id = client.post("/api/v1/activities/1/publish/schedule").json()["batchId"]

    clock.advance(5)
    scheduler.run_due()

    batch = client.get(f"/api/v1/activities/publish/batches/{batch_id}").json()
    assert batch["result"]["message"] == "Republished Lost run to Strava"


def test_undo_during_the_window_sends_nothing(env):
    conn, client, scheduler, clock, strava = env
    _insert(conn, 1)
    batch_id = client.post("/api/v1/activities/1/publish/schedule").json()["batchId"]

    response = client.post("/api/v1/activities/1/publish/cancel")
    clock.advance(10)

    assert response.status_code == 200
    assert scheduler.run_due() == 0
    strava.publish.assert_not_called()
    assert db.get_activity(conn, 1)["publish_status"] == "pending"
    assert client.get(f"/api/v1/activities/publish/batches/{batch_id}").status_code == 404
    assert _activity_item(client, 1)["publishPhase"] is None


def test_undo_after_the_scheduler_claimed_the_publish_is_a_conflict(env):
    conn, client, scheduler, clock, _ = env
    _insert(conn, 1)
    client.post("/api/v1/activities/1/publish/schedule")
    clock.advance(5)
    publish_schedule.claim_due(conn, clock())

    response = client.post("/api/v1/activities/1/publish/cancel")

    assert response.status_code == 409
    assert response.json()["detail"] == "Too late to undo: it is already being sent to Strava."
    assert _activity_item(client, 1)["publishPhase"] == "sending"


def test_undo_with_nothing_scheduled_is_a_conflict(env):
    conn, client, *_ = env
    _insert(conn, 1)

    response = client.post("/api/v1/activities/1/publish/cancel")

    assert response.status_code == 409
    assert response.json()["detail"] == "Nothing is waiting to publish."


def test_cancel_and_claim_never_both_win(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _insert(conn, 1)
    for _ in range(200):
        publish_schedule.schedule(
            conn, [db.get_activity(conn, 1)], 0.0, kind="single", window_seconds=0
        )
        barrier = threading.Barrier(2)
        outcome = {}

        def do_cancel():
            barrier.wait()
            outcome["cancelled"] = publish_schedule.cancel(conn, [1])

        def do_claim():
            barrier.wait()
            outcome["claimed"] = len(publish_schedule.claim_due(conn, 1.0))

        threads = [threading.Thread(target=do_cancel), threading.Thread(target=do_claim)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()

        assert outcome["cancelled"] + outcome["claimed"] == 1
        conn.execute("DELETE FROM publish_schedule")
        conn.commit()


def test_scheduling_an_activity_twice_is_a_conflict(env):
    conn, client, *_ = env
    _insert(conn, 1)
    client.post("/api/v1/activities/1/publish/schedule")

    again = client.post("/api/v1/activities/1/publish/schedule")
    bulk = client.post("/api/v1/activities/publish/schedule", json={"activityIds": [1]})

    assert again.status_code == 409
    assert bulk.status_code == 409


def test_scheduling_rejects_unpublishable_or_disconnected_activities(env):
    conn, client, *_ = env
    _insert(conn, 1, status="published")
    _insert(conn, 2)

    unpublishable = client.post("/api/v1/activities/1/publish/schedule")
    db.set_config_value(conn, "strava_tokens", None)
    disconnected = client.post("/api/v1/activities/2/publish/schedule")

    assert unpublishable.status_code == 409
    assert disconnected.status_code == 409
    assert "Both Garmin and Strava" in disconnected.json()["detail"]
    assert publish_schedule.active_by_activity(conn) == {}


def test_a_bulk_publish_reports_counts_and_can_be_undone_as_one(env):
    conn, client, scheduler, clock, strava = env
    for activity_id in (1, 2, 3):
        _insert(conn, activity_id, status="held")
    first = client.post("/api/v1/activities/publish/schedule", json={"activityIds": [1, 2]})
    second = client.post("/api/v1/activities/publish/schedule", json={"activityIds": [3]})

    undo = client.post("/api/v1/activities/publish/cancel", json={"activityIds": [3]})
    clock.advance(5)
    scheduler.run_due()

    assert undo.status_code == 200
    batch = client.get(f"/api/v1/activities/publish/batches/{first.json()['batchId']}").json()
    assert batch["result"] == {
        "message": "Published 2 activities to Strava",
        "severity": "success",
        "publishedCount": 2,
        "failedCount": 0,
        "blockedCount": 0,
    }
    assert client.get(
        f"/api/v1/activities/publish/batches/{second.json()['batchId']}"
    ).status_code == 404
    assert db.get_activity(conn, 3)["publish_status"] == "held"
    assert strava.publish.call_count == 2


def test_a_bulk_publish_reports_failures_and_hevy_blocks(env):
    conn, client, scheduler, clock, strava = env
    for activity_id in (1, 2, 3):
        _insert(conn, activity_id)
    hevy_db.upsert_workout(
        conn, "hevy-1", "Leg day", "2026-07-03T08:00:00+00:00",
        "2026-07-03T09:00:00+00:00", "2026-07-03T09:01:00+00:00",
        {"id": "hevy-1", "exercises": []},
    )
    conn.execute(
        "UPDATE hevy_workouts SET status = 'needs_review', source_garmin_activity_id = 2"
    )
    conn.commit()
    strava.publish.side_effect = lambda activity_id, *a, **k: (
        (_ for _ in ()).throw(RuntimeError("upload rejected")) if activity_id == 3 else 9_001
    )
    batch_id = client.post(
        "/api/v1/activities/publish/schedule", json={"activityIds": [1, 2, 3]}
    ).json()["batchId"]

    clock.advance(5)
    scheduler.run_due()

    result = client.get(f"/api/v1/activities/publish/batches/{batch_id}").json()["result"]
    assert result["severity"] == "warning"
    assert result["publishedCount"] == 1
    assert result["failedCount"] == 1
    assert result["blockedCount"] == 1
    assert db.get_activity(conn, 3)["publish_status"] == "pending"


def test_a_single_failure_comes_back_as_an_error(env):
    conn, client, scheduler, clock, strava = env
    _insert(conn, 1)
    strava.publish.side_effect = RuntimeError("Strava is down")
    batch_id = client.post("/api/v1/activities/1/publish/schedule").json()["batchId"]

    clock.advance(5)
    scheduler.run_due()

    batch = client.get(f"/api/v1/activities/publish/batches/{batch_id}").json()
    assert batch["state"] == "done"
    assert batch["result"] is None
    assert batch["error"] == "Could not publish activity: Strava is down"


def test_an_activity_excluded_during_the_window_is_not_sent(env):
    conn, client, scheduler, clock, strava = env
    _insert(conn, 1, title="Run")
    batch_id = client.post("/api/v1/activities/1/publish/schedule").json()["batchId"]
    sync.exclude(conn, 1)

    clock.advance(5)
    scheduler.run_due()

    strava.publish.assert_not_called()
    assert db.get_activity(conn, 1)["publish_status"] == "excluded"
    batch = client.get(f"/api/v1/activities/publish/batches/{batch_id}").json()
    assert batch["error"] == "Run was no longer publishable, so nothing was sent."


def test_the_poller_leaves_a_scheduled_activity_for_the_scheduler(env):
    conn, client, scheduler, clock, strava = env
    _insert(conn, 1)
    _insert(conn, 2)
    garmin, _ = _clients()
    client.post("/api/v1/activities/1/publish/schedule")

    stats = sync.publish_pending(conn, garmin, strava, NOW)

    assert stats.published == 1
    assert db.get_activity(conn, 1)["publish_status"] == "pending"
    assert db.get_activity(conn, 2)["publish_status"] == "published"
    clock.advance(5)
    scheduler.run_due()
    assert db.get_activity(conn, 1)["publish_status"] == "published"
    assert strava.publish.call_count == 2


def test_two_publishers_racing_on_one_activity_upload_it_once(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _insert(conn, 1)
    garmin, strava = _clients()
    started = threading.Event()

    def slow_publish(activity_id, *args, **kwargs):
        started.set()
        time.sleep(0.2)
        return 9_001

    strava.publish.side_effect = slow_publish
    poller = threading.Thread(target=sync.publish_pending, args=(conn, garmin, strava, NOW))
    poller.start()
    started.wait(1)
    sync.publish_now(conn, garmin, strava, 1, NOW)
    poller.join()

    assert strava.publish.call_count == 1
    assert db.get_activity(conn, 1)["strava_activity_id"] == 9_001


def test_a_restart_sends_publishes_that_were_waiting_or_mid_send(tmp_path):
    path = str(tmp_path / "test.db")
    conn = db.connect(path)
    _insert(conn, 1)
    _insert(conn, 2)
    _insert(conn, 3)
    past = time.time() - 60
    publish_schedule.schedule(conn, [db.get_activity(conn, 1)], past, kind="single")
    publish_schedule.schedule(conn, [db.get_activity(conn, 2)], past, kind="single")
    publish_schedule.claim_due(conn, past + 10)
    publish_schedule.schedule(conn, [db.get_activity(conn, 3)], past, kind="single")
    conn.close()

    conn = db.connect(path)
    garmin, strava = _clients()
    scheduler = publish_schedule.PublishScheduler(
        conn, build_garmin_client=lambda c: garmin, build_strava_client=lambda c: strava,
    )
    scheduler.start()
    try:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and publish_schedule.active_by_activity(conn):
            time.sleep(0.05)
    finally:
        scheduler.stop()

    assert publish_schedule.active_by_activity(conn) == {}
    assert {db.get_activity(conn, i)["publish_status"] for i in (1, 2, 3)} == {"published"}
    assert strava.publish.call_count == 3


def test_the_scheduler_thread_wakes_for_a_new_publish_at_its_deadline(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _insert(conn, 1)
    garmin, strava = _clients()
    scheduler = publish_schedule.PublishScheduler(
        conn, build_garmin_client=lambda c: garmin, build_strava_client=lambda c: strava,
        window_seconds=0.3,
    )
    scheduler.start()
    try:
        scheduler.schedule([db.get_activity(conn, 1)], kind="single")
        time.sleep(0.1)
        assert db.get_activity(conn, 1)["publish_status"] == "pending"
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and not db.is_published(db.get_activity(conn, 1)):
            time.sleep(0.05)
    finally:
        scheduler.stop()

    assert db.get_activity(conn, 1)["publish_status"] == "published"


def test_finished_outcomes_are_pruned_after_an_hour(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _insert(conn, 1)
    publish_schedule.schedule(conn, [db.get_activity(conn, 1)], 0.0, kind="single")
    publish_schedule.claim_due(conn, 10.0)
    publish_schedule.finish(conn, 1, publish_schedule.PUBLISHED, None, 10.0)

    publish_schedule.prune(conn, 10.0 + 3599)
    assert publish_schedule.get(conn, 1) is not None
    publish_schedule.prune(conn, 10.0 + 3601)
    assert publish_schedule.get(conn, 1) is None


def test_the_frontend_undo_window_matches_the_server():
    source = (
        Path(__file__).resolve().parents[1] / "app" / "hooks" / "use-publish-queue.tsx"
    ).read_text()
    match = re.search(r"export const PUBLISH_UNDO_WINDOW_MS = (\d+);", source)

    assert match is not None
    assert int(match.group(1)) == round(publish_schedule.UNDO_WINDOW_SECONDS * 1000)
