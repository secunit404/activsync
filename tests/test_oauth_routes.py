from unittest.mock import MagicMock
from urllib.parse import parse_qs, urlparse

from fastapi.testclient import TestClient

from activsync import db
from activsync import server as server_module
from activsync.server import create_app


def _complete_setup(conn):
    db.set_config_value(
        conn,
        "garmin_credentials",
        {"email": "athlete@example.com", "password": "secret"},
    )
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(
        conn,
        "strava_credentials",
        {"client_id": "client-id", "client_secret": "client-secret"},
    )
    db.set_config_value(conn, "initial_sync_done", True)


def test_strava_connect_issues_and_persists_oauth_state(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "oauth.db"))
    _complete_setup(conn)
    fake_strava = MagicMock()
    fake_strava.authorize_url.side_effect = (
        lambda redirect_uri, state: f"https://strava.test/oauth?state={state}&redirect={redirect_uri}"
    )
    monkeypatch.setattr(server_module, "_build_strava_client", lambda _conn: fake_strava)

    response = TestClient(create_app(conn)).get(
        "/strava/connect",
        follow_redirects=False,
    )

    assert response.status_code == 307
    state = db.get_config_value(conn, "strava_oauth_state")
    assert state
    assert f"state={state}" in response.headers["location"]
    assert "strava/callback" in response.headers["location"]


def test_strava_callback_rejects_mismatched_and_replayed_state(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "oauth.db"))
    _complete_setup(conn)
    db.set_config_value(conn, "strava_oauth_state", "expected")
    fake_strava = MagicMock()
    monkeypatch.setattr(server_module, "_build_strava_client", lambda _conn: fake_strava)
    client = TestClient(create_app(conn))

    mismatch = client.get(
        "/strava/callback?code=code&state=wrong",
        follow_redirects=False,
    )
    replay = client.get(
        "/strava/callback?code=code&state=expected",
        follow_redirects=False,
    )

    for response in (mismatch, replay):
        assert response.status_code == 303
        parsed = urlparse(response.headers["location"])
        assert parsed.path == "/settings"
        assert "did not match" in parse_qs(parsed.query)["stravaError"][0]
    fake_strava.exchange_code.assert_not_called()
    assert db.get_config_value(conn, "strava_oauth_state") is None


def test_strava_callback_connects_and_runs_post_setup_catch_up(
    tmp_path,
    monkeypatch,
):
    conn = db.connect(str(tmp_path / "oauth.db"))
    _complete_setup(conn)
    db.set_config_value(conn, "strava_oauth_state", "expected")
    fake_strava = MagicMock()
    fake_garmin = MagicMock()
    catch_up = MagicMock()
    monkeypatch.setattr(server_module, "_build_strava_client", lambda _conn: fake_strava)
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda _conn: fake_garmin)
    monkeypatch.setattr(server_module.onboarding, "run_catch_up", catch_up)

    response = TestClient(create_app(conn)).get(
        "/strava/callback?code=code&state=expected",
        follow_redirects=False,
    )

    assert response.status_code == 303
    assert response.headers["location"] == "/settings"
    fake_strava.exchange_code.assert_called_once_with("code")
    catch_up.assert_called_once_with(conn, fake_garmin, fake_strava)


def test_strava_callback_returns_to_setup_and_surfaces_declines_there(
    tmp_path,
    monkeypatch,
):
    conn = db.connect(str(tmp_path / "oauth.db"))
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(
        conn,
        "strava_credentials",
        {"client_id": "client-id", "client_secret": "client-secret"},
    )
    fake_strava = MagicMock()
    monkeypatch.setattr(server_module, "_build_strava_client", lambda _conn: fake_strava)
    client = TestClient(create_app(conn))

    db.set_config_value(conn, "strava_oauth_state", "expected")
    connected = client.get(
        "/strava/callback?code=code&state=expected",
        follow_redirects=False,
    )
    db.set_config_value(conn, "strava_oauth_state", "expected-2")
    declined = client.get(
        "/strava/callback?error=access_denied&state=expected-2",
        follow_redirects=False,
    )

    assert connected.headers["location"] == "/setup"
    parsed = urlparse(declined.headers["location"])
    assert parsed.path == "/setup"
    assert "declined" in parse_qs(parsed.query)["stravaError"][0]
