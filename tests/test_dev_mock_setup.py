"""End-to-end first-run setup through the typed API in mock mode."""

import pytest
from fastapi.testclient import TestClient

from activsync import db, dev_mock, view
from activsync import server as server_module
from activsync.dev_seed import seed as seed_dev_data
from activsync.server import create_app


@pytest.fixture
def mock_env(monkeypatch):
    monkeypatch.setenv("ACTIVSYNC_DEV_MOCK_DATA", "1")


@pytest.fixture
def seeded_conn(tmp_path):
    conn = db.connect(str(tmp_path / "dev.db"))
    seed_dev_data(conn)
    return conn


def _client(conn):
    return TestClient(create_app(conn), follow_redirects=False)


def _garmin_payload(password: str = "whatever") -> dict:
    return {
        "email": "someone@example.com",
        "password": password,
        "lookbackDays": 7,
        "detectedTimezone": "Europe/Oslo",
    }


def test_fresh_start_reports_garmin_setup_step(mock_env, seeded_conn):
    response = _client(seeded_conn).get("/api/v1/app")

    assert response.status_code == 200
    assert response.json()["setup"] == {"complete": False, "step": "garmin"}


def test_seed_with_e2e_onboarded_flag_reports_setup_complete(mock_env, tmp_path):
    """The E2E server (npm run dev:e2e) opts into this via mark_onboarded=True
    so every spec lands on the activities dashboard, not the setup wizard."""
    conn = db.connect(str(tmp_path / "e2e.db"))
    seed_dev_data(conn, mark_onboarded=True)

    response = _client(conn).get("/api/v1/app")

    assert response.status_code == 200
    assert response.json()["setup"] == {"complete": True, "step": None}


def test_seed_without_e2e_onboarded_flag_stays_incomplete(mock_env, tmp_path):
    """`make dev-fresh` must keep landing on the first-run wizard: seeding
    without the flag (the default) must never mark setup complete."""
    conn = db.connect(str(tmp_path / "dev-fresh.db"))
    seed_dev_data(conn)

    response = _client(conn).get("/api/v1/app")

    assert response.status_code == 200
    assert response.json()["setup"]["complete"] is False


def test_full_setup_completes_end_to_end(mock_env, seeded_conn):
    conn = seeded_conn
    client = _client(conn)

    garmin = client.post("/api/v1/setup/garmin", json=_garmin_payload())
    assert garmin.status_code == 200
    assert garmin.json()["setupStep"] == "strava"
    assert view.connection_status(conn)["garmin"]["connected"] is True

    credentials = client.put(
        "/api/v1/settings/strava-credentials",
        json={"clientId": "dev-id", "clientSecret": "dev-secret"},
    )
    assert credentials.status_code == 200

    connect = client.get("/strava/connect")
    assert connect.status_code == 307
    assert "/strava/callback?code=dev-mock-code" in connect.headers["location"]
    callback = client.get(connect.headers["location"])
    assert callback.status_code == 303
    assert callback.headers["location"] == "/setup"
    assert view.connection_status(conn)["strava"]["connected"] is True

    skipped = client.post("/api/v1/setup/hevy/skip")
    assert skipped.status_code == 200
    assert skipped.json()["setupStep"] == "syncing"

    initial = client.post("/api/v1/setup/initial-sync")
    assert initial.status_code == 200
    assert initial.json()["setupStep"] is None
    assert db.get_config_value(conn, "initial_sync_done") is True
    assert client.get("/api/v1/app").json()["setup"]["complete"] is True


def test_mfa_challenge_flow(mock_env, seeded_conn):
    conn = seeded_conn
    client = _client(conn)

    challenge = client.post(
        "/api/v1/setup/garmin",
        json=_garmin_payload(dev_mock.MFA_TRIGGER_PASSWORD),
    )
    assert challenge.status_code == 200
    assert challenge.json()["mfaRequired"] is True
    assert view.connection_status(conn)["garmin"]["connected"] is False

    rejected = client.post(
        "/api/v1/garmin/mfa",
        json={"code": dev_mock.MFA_REJECT_CODE},
    )
    assert rejected.status_code == 401
    assert view.connection_status(conn)["garmin"]["connected"] is False

    accepted = client.post("/api/v1/garmin/mfa", json={"code": "123456"})
    assert accepted.status_code == 200
    assert view.connection_status(conn)["garmin"]["connected"] is True


def test_app_state_marks_mock_mode_without_frontend_markup(mock_env, seeded_conn):
    assert _client(seeded_conn).get("/api/v1/app").json()["development"] is True


def test_app_state_hides_mock_mode_when_disabled(monkeypatch, seeded_conn):
    monkeypatch.setenv("ACTIVSYNC_DEV_MOCK_DATA", "0")

    assert _client(seeded_conn).get("/api/v1/app").json()["development"] is False


def test_mock_off_uses_real_login_path(monkeypatch, seeded_conn):
    monkeypatch.setenv("ACTIVSYNC_DEV_MOCK_DATA", "0")

    def fake_dev_login(*_args, **_kwargs):
        raise AssertionError("dev_mock.begin_login must not run when mock is off")

    def real_login_fails(_email, _password, _token_dir):
        raise RuntimeError("invalid credentials")

    monkeypatch.setattr(dev_mock, "begin_login", fake_dev_login)
    monkeypatch.setattr(server_module, "garmin_begin_login", real_login_fails)

    response = _client(seeded_conn).post(
        "/api/v1/setup/garmin",
        json=_garmin_payload(),
    )

    assert response.status_code == 502
    assert view.connection_status(seeded_conn)["garmin"]["connected"] is False
