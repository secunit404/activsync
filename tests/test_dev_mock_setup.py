"""End-to-end walk of the first-run setup wizard in mock mode.

Verifies that with ACTIVSYNC_DEV_MOCK_DATA on, the whole onboarding flow —
Garmin login (incl. MFA), Strava OAuth, and the initial sync — completes without
any real account or network call, landing on a usable dashboard.
"""

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
    # Don't auto-follow redirects: the wizard leans on 303/307 hops we assert on.
    return TestClient(create_app(conn), follow_redirects=False)


def test_fresh_start_redirects_to_setup(mock_env, seeded_conn):
    client = _client(seeded_conn)
    resp = client.get("/", follow_redirects=False)
    assert resp.status_code == 303
    assert resp.headers["location"] == "/setup"


def test_full_wizard_completes_end_to_end(mock_env, seeded_conn):
    conn = seeded_conn
    client = _client(conn)

    # 1. Garmin connect — any credentials succeed in mock mode.
    resp = client.post("/setup/garmin/connect", data={
        "garmin_email": "someone@example.com",
        "garmin_password": "whatever",
        "lookback_days": 7,
        "detected_timezone": "Europe/Oslo",
    })
    assert resp.status_code == 303
    assert view.connection_status(conn)["garmin"]["connected"] is True

    # 2. Strava connect — persists creds, then /strava/connect bounces through
    #    the faked OAuth callback and stores a token.
    resp = client.post("/setup/strava/connect", data={
        "strava_client_id": "dev-id",
        "strava_client_secret": "dev-secret",
    })
    assert resp.status_code == 303
    assert resp.headers["location"] == "/strava/connect"

    resp = client.get("/strava/connect")
    assert resp.status_code == 307
    # Fake authorize_url loops straight back to our own callback.
    assert "/strava/callback?code=dev-mock-code" in resp.headers["location"]

    resp = client.get("/strava/callback", params={"code": "dev-mock-code"})
    assert resp.status_code == 303
    assert view.connection_status(conn)["strava"]["connected"] is True

    # 3. Initial sync — completes against the seeded data and finishes the wizard,
    #    then shows the completion screen (no auto-redirect) with a button into the app.
    resp = client.post("/setup/initial-sync")
    assert resp.status_code == 200
    assert "hx-redirect" not in resp.headers
    assert 'href="/"' in resp.text
    assert "Go to ActivSync" in resp.text
    # The completion screen confirms each step of the wizard.
    for label in ("Garmin connected", "Strava connected", "Activities synced"):
        assert label in resp.text
    assert db.get_config_value(conn, "initial_sync_done") is True

    # 4. Following that button reaches the dashboard instead of bouncing to /setup.
    resp = client.get("/")
    assert resp.status_code == 200


def test_mfa_challenge_flow(mock_env, seeded_conn):
    conn = seeded_conn
    client = _client(conn)

    # The magic password triggers a simulated MFA challenge instead of an
    # immediate success, so the code-entry modal can be exercised.
    resp = client.post("/setup/garmin/connect", data={
        "garmin_email": "someone@example.com",
        "garmin_password": dev_mock.MFA_TRIGGER_PASSWORD,
        "lookback_days": 7,
        "detected_timezone": "",
    })
    assert resp.status_code == 303
    assert view.connection_status(conn)["garmin"]["connected"] is False

    # A rejected code surfaces the error screen and does not connect.
    resp = client.post("/setup/garmin-mfa", data={"mfa_code": dev_mock.MFA_REJECT_CODE})
    assert resp.status_code == 401
    assert view.connection_status(conn)["garmin"]["connected"] is False

    # Any other code completes the login and connects.
    resp = client.post("/setup/garmin-mfa", data={"mfa_code": "123456"})
    assert resp.status_code == 303
    assert view.connection_status(conn)["garmin"]["connected"] is True


_BANNER_MARKUP = 'aria-label="Development mode"'


def test_dev_banner_shown_in_mock_mode(mock_env, seeded_conn):
    body = _client(seeded_conn).get("/setup").text
    assert _BANNER_MARKUP in body
    assert "<title>[DEV] " in body


def test_dev_banner_hidden_when_mock_off(monkeypatch, seeded_conn):
    monkeypatch.setenv("ACTIVSYNC_DEV_MOCK_DATA", "0")
    body = _client(seeded_conn).get("/setup").text
    assert _BANNER_MARKUP not in body
    assert "[DEV]" not in body


def test_mock_off_uses_real_login_path(monkeypatch, seeded_conn):
    # With mock mode off, the setup path must go through the real Garmin login
    # (stubbed here to avoid the network), never the dev fake. If the fake were
    # used, this bogus login would "succeed"; the real path raising proves it is
    # taken, and the connect surfaces the error screen instead of connecting.
    monkeypatch.setenv("ACTIVSYNC_DEV_MOCK_DATA", "0")

    def _fake_dev_login(*args, **kwargs):
        raise AssertionError("dev_mock.begin_login must not run when mock is off")

    def _real_login_fails(email, password, token_dir):
        raise RuntimeError("invalid credentials")

    monkeypatch.setattr(dev_mock, "begin_login", _fake_dev_login)
    monkeypatch.setattr(server_module, "garmin_begin_login", _real_login_fails)

    client = _client(seeded_conn)
    resp = client.post("/setup/garmin/connect", data={
        "garmin_email": "someone@example.com",
        "garmin_password": "whatever",
        "lookback_days": 7,
        "detected_timezone": "",
    })
    assert resp.status_code == 502
    assert view.connection_status(seeded_conn)["garmin"]["connected"] is False
