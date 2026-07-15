from datetime import datetime, timezone
from unittest.mock import MagicMock

from fastapi.testclient import TestClient

from activsync import db
from activsync import server as server_module
from activsync.server import create_app

NOW = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)


def _logged_in_client(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))
    return conn, client


def test_publish_action_publishes_held_activity(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    db.insert_activity(conn, 1, "strength_training", "Leg Day", "", "2026-07-09 09:00:00", "h", "held", NOW)

    fake_garmin = MagicMock()
    fake_garmin.download_fit.return_value = b"FIT"
    fake_strava = MagicMock()
    fake_strava.publish.return_value = 7001
    fake_strava.find_existing_activity.return_value = None
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.post("/api/activities/1/publish")

    assert response.status_code == 200
    row = db.get_activity(conn, 1)
    assert row["publish_status"] == "published"
    assert row["strava_activity_id"] == 7001


def test_exclude_action_excludes_activity(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    db.insert_activity(conn, 2, "strength_training", "Leg Day", "", "2026-07-09 09:00:00", "h", "held", NOW)

    response = client.post("/api/activities/2/exclude")

    assert response.status_code == 200
    assert db.get_activity(conn, 2)["publish_status"] == "excluded"


def test_unexclude_action_returns_activity_to_pending(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    db.insert_activity(conn, 3, "strength_training", "Leg Day", "", "2026-07-09 09:00:00", "h", "excluded", NOW)

    response = client.post("/api/activities/3/unexclude")

    assert response.status_code == 200
    assert db.get_activity(conn, 3)["publish_status"] == "pending"


def test_edit_action_updates_garmin_and_local_activity(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    db.insert_activity(conn, 5, "running", "Old title", "Old desc", "2026-07-09 09:00:00", "old", "pending", NOW)

    fake_garmin = MagicMock()
    fake_strava = MagicMock()
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.post(
        "/api/activities/5/edit",
        data={"title": "New title", "description": "New desc"},
    )

    assert response.status_code == 200
    fake_garmin.update_activity_metadata.assert_called_once_with(5, "New title", "New desc")
    fake_strava.update_activity_metadata.assert_not_called()
    row = db.get_activity(conn, 5)
    assert row["title"] == "New title"
    assert row["description"] == "New desc"
    assert row["content_hash"] != "old"


def test_edit_action_updates_strava_when_activity_is_published(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    db.insert_activity(conn, 6, "running", "Old title", "Old desc", "2026-07-09 09:00:00", "old", "published", NOW)
    db.set_published(conn, 6, strava_activity_id=7001, now=NOW)

    fake_garmin = MagicMock()
    fake_strava = MagicMock()
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.post(
        "/api/activities/6/edit",
        data={"title": "Published title", "description": "Published desc"},
    )

    assert response.status_code == 200
    fake_garmin.update_activity_metadata.assert_called_once_with(6, "Published title", "Published desc")
    fake_strava.update_activity_metadata.assert_called_once_with(7001, "Published title", "Published desc")
    assert db.get_activity(conn, 6)["title"] == "Published title"
    # Saving state is shown on the save button itself (spinner + label), not a
    # separate success message.
    assert 'class="button-loading"' in response.text
    assert "Changes saved successfully." not in response.text


def test_edit_action_rejects_blank_title(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    db.insert_activity(conn, 7, "running", "Old title", "Old desc", "2026-07-09 09:00:00", "old", "pending", NOW)

    fake_garmin = MagicMock()
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())

    response = client.post(
        "/api/activities/7/edit",
        data={"title": "   ", "description": "New desc"},
    )

    assert response.status_code == 502
    assert "Activity title cannot be blank" in response.text
    fake_garmin.update_activity_metadata.assert_not_called()
    assert db.get_activity(conn, 7)["title"] == "Old title"


def test_actions_are_available_without_password_auth(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))
    db.insert_activity(conn, 4, "strength_training", "Leg Day", "", "2026-07-09 09:00:00", "h", "held", NOW)

    response = client.post("/api/activities/4/exclude")

    assert response.status_code == 200
    assert db.get_activity(conn, 4)["publish_status"] == "excluded"

    response = client.post(
        "/api/activities/4/edit",
        data={"title": "New", "description": "New"},
    )

    assert response.status_code == 502
    assert "Garmin credentials are not configured" in response.text
