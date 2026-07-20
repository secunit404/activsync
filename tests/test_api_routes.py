from datetime import datetime, timezone
from unittest.mock import MagicMock

from fastapi.testclient import TestClient

from activsync import db, hevy_db
from activsync import server as server_module
from activsync.server import create_app


def _connect_publish_services(conn):
    db.set_config_value(
        conn,
        "garmin_credentials",
        {"email": "athlete@example.com", "password": "secret"},
    )
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_tokens", {"refresh_token": "refresh"})


def test_app_state_starts_at_garmin_setup(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    response = TestClient(create_app(conn)).get("/api/v1/app")

    assert response.status_code == 200
    payload = response.json()
    assert payload["name"] == "ActivSync"
    assert payload["setup"] == {"complete": False, "step": "garmin"}
    assert payload["connections"]["broken"] == ["garmin", "strava"]
    assert payload["connections"]["garmin"]["connected"] is False
    assert payload["connections"]["strava"]["connected"] is False


def test_app_state_reports_connected_services_and_hevy(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(
        conn,
        "garmin_credentials",
        {"email": "athlete@example.com", "password": "secret"},
    )
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_tokens", {"refresh_token": "refresh"})
    db.set_config_value(conn, "hevy_api_key", "hevy-key")
    db.set_config_value(conn, "hevy_auth_ok", True)
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(conn, "settings", {"hevy_enabled": True})
    monkeypatch.setenv("ACTIVSYNC_DEV_MOCK_DATA", "1")

    response = TestClient(create_app(conn)).get("/api/v1/app")

    assert response.status_code == 200
    payload = response.json()
    assert payload["development"] is True
    assert payload["setup"] == {"complete": True, "step": None}
    assert payload["connections"]["broken"] == []
    assert payload["connections"]["garmin"]["email"] == "athlete@example.com"
    assert payload["hevy"] == {
        "enabled": True,
        "connected": True,
        "status": "Connected",
    }


def test_app_state_exposes_and_dismisses_reconnect_report(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(
        conn,
        "catch_up_report",
        {"new": 4, "held": 3, "linked": 1, "days": 12},
    )
    client = TestClient(create_app(conn))

    before = client.get("/api/v1/app")
    dismissed = client.delete("/api/v1/catch-up-report")
    after = client.get("/api/v1/app")

    assert before.json()["catchUpReport"] == {
        "new": 4,
        "held": 3,
        "linked": 1,
        "days": 12,
    }
    assert before.json()["update"]["repoUrl"].endswith("/activsync")
    assert dismissed.status_code == 204
    assert after.json()["catchUpReport"] is None


def test_hevy_queue_json_actions_preserve_workout_state_machine(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    for hevy_id, status, error in (
        ("needs-map", "needs_mapping", "Map an exercise"),
        ("skip-me", "failed", "Temporary failure"),
        ("restore-me", "skipped", None),
        ("fresh-me", "needs_review", "linked activity deleted on Garmin"),
    ):
        hevy_db.upsert_workout(
            conn,
            hevy_id,
            hevy_id.replace("-", " ").title(),
            "2026-07-19T08:00:00Z",
            "2026-07-19T09:00:00Z",
            "2026-07-19T10:00:00Z",
            {"exercises": []},
        )
        hevy_db.set_workout_status(conn, hevy_id, status, error=error)
    client = TestClient(create_app(conn))

    queue = client.get("/api/v1/hevy/queue")
    retried = client.post("/api/v1/hevy/needs-map/retry")
    skipped = client.post("/api/v1/hevy/skip-me/skip")
    restored = client.post("/api/v1/hevy/restore-me/unskip")
    fresh = client.post("/api/v1/hevy/fresh-me/resync-fresh")

    assert queue.status_code == 200
    assert queue.json()["counts"] == {
        "inFlight": 0,
        "problems": 3,
        "skipped": 1,
    }
    assert queue.json()["problems"][0]["hevyId"]
    assert retried.status_code == 200
    assert skipped.status_code == 200
    assert restored.status_code == 200
    assert fresh.status_code == 200
    assert hevy_db.get_workout(conn, "needs-map")["status"] == "waiting_watch"
    assert hevy_db.get_workout(conn, "skip-me")["status"] == "skipped"
    assert hevy_db.get_workout(conn, "restore-me")["status"] == "waiting_watch"
    assert hevy_db.get_workout(conn, "fresh-me")["status"] == "waiting_watch"


def test_activities_page_filters_sorts_and_uses_camel_case(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    now = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn,
        1,
        "running",
        "Older run",
        "Easy pace",
        "2026-07-18 08:00:00",
        "hash-1",
        "held",
        now,
        garmin_data='{"distance": 5000, "duration": 1500}',
    )
    db.insert_activity(
        conn,
        2,
        "cycling",
        "Newer ride",
        "",
        "2026-07-19 09:00:00",
        "hash-2",
        "published",
        now,
    )
    db.set_published(conn, 2, 987654, now)

    response = TestClient(create_app(conn)).get(
        "/api/v1/activities?sort=oldest&status=held&pageSize=10"
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["sort"] == "oldest"
    assert payload["status"] == "held"
    assert payload["counts"] == {
        "pending": 0,
        "held": 1,
        "published": 1,
        "missing": 0,
        "excluded": 0,
    }
    assert payload["pagination"] == {
        "page": 1,
        "pageSize": 10,
        "pageCount": 1,
        "totalCount": 1,
        "firstItem": 1,
        "lastItem": 1,
    }
    assert payload["items"][0]["garminActivityId"] == 1
    assert payload["items"][0]["startDateDisplay"] == "18 Jul"
    assert payload["items"][0]["detail"]["distance"] == "5.00 km"
    assert payload["items"][0]["detail"]["duration"] == "25m 00s"


def test_activities_page_clamps_to_last_page_and_builds_strava_link(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    now = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)
    for activity_id in range(1, 12):
        db.insert_activity(
            conn,
            activity_id,
            "running",
            f"Run {activity_id}",
            "",
            f"2026-07-{activity_id:02d} 09:00:00",
            f"hash-{activity_id}",
            "published",
            now,
        )
        db.set_published(conn, activity_id, 9000 + activity_id, now)

    response = TestClient(create_app(conn)).get(
        "/api/v1/activities?page=99&pageSize=10"
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["pagination"]["page"] == 2
    assert payload["pagination"]["pageCount"] == 2
    assert payload["pagination"]["firstItem"] == 11
    assert payload["pagination"]["lastItem"] == 11
    assert payload["items"][0]["stravaUrl"].startswith(
        "https://www.strava.com/activities/"
    )


def test_activities_page_rejects_unknown_status_and_page_size(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))

    assert client.get("/api/v1/activities?status=queued").status_code == 422
    assert client.get("/api/v1/activities?pageSize=25").status_code == 422


def test_publish_activity_returns_json_and_updates_status(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    _connect_publish_services(conn)
    now = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn,
        21,
        "running",
        "Lunch run",
        "",
        "2026-07-19 12:00:00",
        "hash-21",
        "pending",
        now,
    )
    fake_garmin = MagicMock()
    fake_garmin.download_fit.return_value = b"FIT"
    fake_strava = MagicMock()
    fake_strava.find_existing_activity.return_value = None
    fake_strava.publish.return_value = 9021
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda conn: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda conn: fake_strava)

    response = TestClient(create_app(conn)).post("/api/v1/activities/21/publish")

    assert response.status_code == 200
    assert response.json() == {
        "message": "Published Lunch run to Strava",
        "severity": "success",
        "publishedCount": 0,
        "failedCount": 0,
        "blockedCount": 0,
    }
    assert db.get_activity(conn, 21)["publish_status"] == "published"


def test_bulk_publish_returns_counts(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    _connect_publish_services(conn)
    now = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)
    for activity_id in (31, 32):
        db.insert_activity(
            conn,
            activity_id,
            "running",
            f"Run {activity_id}",
            "",
            f"2026-07-{activity_id - 20:02d} 08:00:00",
            f"hash-{activity_id}",
            "held",
            now,
        )
    fake_garmin = MagicMock()
    fake_garmin.download_fit.return_value = b"FIT"
    fake_strava = MagicMock()
    fake_strava.find_existing_activity.return_value = None
    fake_strava.publish.side_effect = [9031, 9032]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda conn: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda conn: fake_strava)

    response = TestClient(create_app(conn)).post(
        "/api/v1/activities/publish", json={"activityIds": [31, 32]}
    )

    assert response.status_code == 200
    assert response.json()["message"] == "Published 2 activities to Strava"
    assert response.json()["publishedCount"] == 2


def test_edit_exclude_and_restore_activity_json_actions(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    now = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn,
        41,
        "running",
        "Old title",
        "Old description",
        "2026-07-19 12:00:00",
        "hash-41",
        "pending",
        now,
    )
    fake_garmin = MagicMock()
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda conn: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda conn: MagicMock())
    client = TestClient(create_app(conn))

    edit_response = client.put(
        "/api/v1/activities/41",
        json={"title": "New title", "description": "New description"},
    )
    exclude_response = client.post("/api/v1/activities/41/exclude")
    restore_response = client.post("/api/v1/activities/41/restore")

    assert edit_response.status_code == 200
    assert edit_response.json()["message"] == "Saved changes to Old title"
    assert exclude_response.json()["message"] == "Excluded New title"
    assert restore_response.json()["message"] == "Restored New title"
    assert db.get_activity(conn, 41)["publish_status"] == "pending"
    fake_garmin.update_activity_metadata.assert_called_once_with(
        41, "New title", "New description"
    )


def test_json_actions_return_actionable_errors(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    now = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn,
        51,
        "running",
        "Disconnected run",
        "",
        "2026-07-19 12:00:00",
        "hash-51",
        "pending",
        now,
    )
    client = TestClient(create_app(conn))

    publish_response = client.post("/api/v1/activities/51/publish")
    blank_edit_response = client.put(
        "/api/v1/activities/51", json={"title": "   ", "description": ""}
    )
    missing_response = client.post("/api/v1/activities/999/exclude")

    assert publish_response.status_code == 409
    assert "Both Garmin and Strava" in publish_response.json()["detail"]
    assert blank_edit_response.status_code == 422
    assert blank_edit_response.json()["detail"] == "Activity title cannot be blank"
    assert missing_response.status_code == 404
