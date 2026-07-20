from fastapi.testclient import TestClient

from activsync import config, db, dev_mock, hevy_db
from activsync.server import create_app


def _client(tmp_path, monkeypatch, *, mock=True):
    monkeypatch.setenv("ACTIVSYNC_DEV_MOCK_DATA", "1" if mock else "0")
    conn = db.connect(str(tmp_path / "settings-api.db"))
    return conn, TestClient(create_app(conn), follow_redirects=False)


def _complete_setup(conn):
    db.set_config_value(
        conn,
        "garmin_credentials",
        {"email": "athlete@example.com", "password": "working-password"},
    )
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(
        conn,
        "strava_credentials",
        {"client_id": "client-id", "client_secret": "client-secret"},
    )
    db.set_config_value(
        conn,
        "strava_tokens",
        {
            "access_token": "access",
            "refresh_token": "refresh",
            "expires_at": 4102444800,
        },
    )
    db.set_config_value(conn, "initial_sync_done", True)


def test_settings_state_is_typed_and_never_exposes_secrets(tmp_path, monkeypatch):
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    db.set_config_value(
        conn,
        "garmin_activity_types",
        [
            {"type_key": "running", "label": "Running"},
            {"type_key": "cycling", "label": "Cycling"},
        ],
    )
    cfg = config.load_config(conn)
    cfg["held_activity_types"] = ["cycling"]
    config.save_config(conn, cfg)

    response = client.get("/api/v1/settings")

    assert response.status_code == 200
    body = response.json()
    assert body["setup"] == {
        "complete": True,
        "step": None,
        "mfaRequired": False,
    }
    assert body["credentials"] == {
        "garminEmail": "athlete@example.com",
        "garminPasswordSaved": True,
        "stravaClientId": "client-id",
        "stravaClientSecretSaved": True,
    }
    assert "working-password" not in response.text
    assert "client-secret" not in response.text
    assert body["activityTypes"] == [
        {"typeKey": "running", "label": "Running", "autosync": True},
        {"typeKey": "cycling", "label": "Cycling", "autosync": False},
    ]


def test_failed_garmin_reconnect_keeps_last_verified_password(
    tmp_path, monkeypatch
):
    conn, client = _client(tmp_path, monkeypatch, mock=False)
    _complete_setup(conn)

    def reject_login(*_args, **_kwargs):
        raise RuntimeError("401 Unauthorized")

    monkeypatch.setattr("activsync.server.garmin_begin_login", reject_login)

    response = client.post(
        "/api/v1/settings/garmin/reconnect",
        json={"email": "athlete@example.com", "password": "mistyped"},
    )

    assert response.status_code == 502
    assert db.get_config_value(conn, "garmin_credentials") == {
        "email": "athlete@example.com",
        "password": "working-password",
    }
    assert db.get_config_value(conn, "garmin_credentials_verified") is True


def test_mfa_credentials_are_only_committed_after_a_valid_code(
    tmp_path, monkeypatch
):
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)

    challenge = client.post(
        "/api/v1/settings/garmin/reconnect",
        json={
            "email": "new@example.com",
            "password": dev_mock.MFA_TRIGGER_PASSWORD,
        },
    )
    assert challenge.status_code == 200
    assert challenge.json()["mfaRequired"] is True
    assert db.get_config_value(conn, "garmin_credentials")["password"] == (
        "working-password"
    )

    rejected = client.post(
        "/api/v1/garmin/mfa", json={"code": dev_mock.MFA_REJECT_CODE}
    )
    assert rejected.status_code == 401
    assert db.get_config_value(conn, "garmin_credentials")["password"] == (
        "working-password"
    )

    accepted = client.post("/api/v1/garmin/mfa", json={"code": "123456"})
    assert accepted.status_code == 200
    assert accepted.json()["mfaRequired"] is False
    assert db.get_config_value(conn, "garmin_credentials") == {
        "email": "new@example.com",
        "password": dev_mock.MFA_TRIGGER_PASSWORD,
    }


def test_preferences_and_categories_save_through_json_api(tmp_path, monkeypatch):
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    db.set_config_value(
        conn,
        "garmin_activity_types",
        [
            {"type_key": "running", "label": "Running"},
            {"type_key": "cycling", "label": "Cycling"},
        ],
    )
    timezone_updates = []
    monkeypatch.setattr(
        "activsync.settings_api_routes.logging_setup.set_log_timezone",
        timezone_updates.append,
    )

    preferences = client.put(
        "/api/v1/settings/preferences",
        json={
            "displayTimezone": "Europe/Oslo",
            "garminPollIntervalMinutes": 30,
            "stravaPollIntervalMinutes": 8,
            "lookbackDays": 14,
            "hevy2garminMarker": "— via hevy",
            "hevy2garminMarkerEnabled": True,
        },
    )
    categories = client.put(
        "/api/v1/settings/activity-types",
        json={"autosyncTypes": ["running"]},
    )

    assert preferences.status_code == 200
    assert categories.status_code == 200
    cfg = config.load_config(conn)
    assert cfg["display_timezone"] == "Europe/Oslo"
    assert cfg["lookback_days"] == 14
    assert cfg["hevy2garmin_marker_enabled"] is True
    assert cfg["held_activity_types"] == ["cycling"]
    assert timezone_updates == ["Europe/Oslo"]


def test_mock_setup_can_complete_through_json_contract(tmp_path, monkeypatch):
    conn, client = _client(tmp_path, monkeypatch)

    garmin = client.post(
        "/api/v1/setup/garmin",
        json={
            "email": "someone@example.com",
            "password": "safe-mock-password",
            "lookbackDays": 7,
            "detectedTimezone": "Europe/Oslo",
        },
    )
    assert garmin.status_code == 200
    assert garmin.json()["setupStep"] == "strava"

    credentials = client.put(
        "/api/v1/settings/strava-credentials",
        json={"clientId": "dev-id", "clientSecret": "dev-secret"},
    )
    assert credentials.status_code == 200

    connect = client.get("/strava/connect")
    assert connect.status_code == 307
    callback = client.get(connect.headers["location"])
    assert callback.status_code == 303

    skipped = client.post("/api/v1/setup/hevy/skip")
    assert skipped.status_code == 200
    assert skipped.json()["setupStep"] == "syncing"

    initial = client.post("/api/v1/setup/initial-sync")
    assert initial.status_code == 200
    assert initial.json()["setupStep"] is None
    assert db.get_config_value(conn, "initial_sync_done") is True


def test_hevy_mapping_tools_save_remove_and_wake_waiting_workouts(
    tmp_path, monkeypatch
):
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    hevy_db.upsert_template(
        conn,
        {
            "exercise_template_id": "custom-1",
            "title": "Landmine Press",
            "primary_muscle_group": "chest",
            "secondary_muscle_groups": [],
            "equipment_category": "barbell",
            "is_custom": True,
        },
    )
    hevy_db.upsert_workout(
        conn,
        "workout-1",
        "Push",
        "2026-07-18T10:00:00Z",
        "2026-07-18T11:00:00Z",
        "2026-07-18T12:00:00Z",
        {
            "exercises": [
                {
                    "exercise_template_id": "custom-1",
                    "title": "Landmine Press",
                }
            ]
        },
    )
    hevy_db.set_workout_status(
        conn, "workout-1", "needs_mapping", error="unmapped"
    )

    tools = client.get("/api/v1/settings/hevy/tools")
    saved = client.put(
        "/api/v1/settings/hevy/mappings/custom-1",
        json={"category": 0, "subcategory": 1},
    )

    assert tools.status_code == 200
    assert tools.json()["mappings"][0]["templateId"] == "custom-1"
    assert saved.status_code == 200
    assert "1 waiting workout resumed" in saved.json()["message"]
    assert hevy_db.get_mapping(conn, "custom-1")["subcategory"] == 1
    assert hevy_db.get_workout(conn, "workout-1")["status"] == "waiting_watch"

    removed = client.delete("/api/v1/settings/hevy/mappings/custom-1")
    assert removed.status_code == 200
    assert hevy_db.get_mapping(conn, "custom-1") is None


def test_hevy_backfill_json_preview_is_read_only_and_run_ingests(
    tmp_path, monkeypatch
):
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    db.set_config_value(conn, "hevy_api_key", "safe-mock-key")

    preview = client.post(
        "/api/v1/settings/hevy/backfill/preview",
        json={"since": "2020-01-01"},
    )

    assert preview.status_code == 200
    assert preview.json()["ran"] is False
    assert len(preview.json()["items"]) == 5
    assert hevy_db.list_workouts(conn) == []

    run = client.post(
        "/api/v1/settings/hevy/backfill/run",
        json={"since": "2020-01-01"},
    )
    assert run.status_code == 200
    assert run.json()["ran"] is True
    assert len(hevy_db.list_workouts(conn)) == 5
