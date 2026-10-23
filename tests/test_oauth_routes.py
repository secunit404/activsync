from unittest.mock import MagicMock
from urllib.parse import parse_qs, urlparse

from fastapi.testclient import TestClient

from auth_helpers import CSRF_HEADERS, owner_client

from activsync import db, dev_mock, onboarding, view
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


CONNECT_URL = "/api/v1/settings/strava/authorize"


def test_strava_connect_issues_and_persists_oauth_state(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "oauth.db"))
    _complete_setup(conn)
    fake_strava = MagicMock()
    fake_strava.authorize_url.side_effect = (
        lambda redirect_uri, state: f"https://strava.test/oauth?state={state}&redirect={redirect_uri}"
    )
    monkeypatch.setattr(server_module, "_build_strava_client", lambda _conn: fake_strava)

    response = owner_client(create_app(conn)).post(CONNECT_URL)

    assert response.status_code == 200
    state = db.get_config_value(conn, "strava_oauth_state")
    assert state
    assert f"state={state}" in response.json()["url"]
    assert "strava/callback" in response.json()["url"]


def test_strava_connect_url_points_straight_at_strava_with_the_shared_scope(tmp_path):
    conn = db.connect(str(tmp_path / "oauth.db"))
    _complete_setup(conn)

    url = owner_client(create_app(conn)).post(CONNECT_URL).json()["url"]

    parsed = urlparse(url)
    assert f"{parsed.scheme}://{parsed.netloc}{parsed.path}" == (
        "https://www.strava.com/oauth/authorize"
    )
    assert "client_id=client-id" in url
    assert "scope=read,activity:write,activity:read_all" in url
    assert "redirect_uri=http://testserver/strava/callback" in url


def test_strava_connect_without_saved_credentials_is_a_json_error(tmp_path):
    conn = db.connect(str(tmp_path / "oauth.db"))

    response = owner_client(create_app(conn)).post(CONNECT_URL)

    assert response.status_code == 409
    assert response.json() == {
        "detail": "Save your Strava client ID and client secret before connecting."
    }
    assert db.get_config_value(conn, "strava_oauth_state") is None


def test_strava_connect_requires_a_session_and_the_csrf_header(tmp_path):
    conn = db.connect(str(tmp_path / "oauth.db"))
    _complete_setup(conn)
    app = create_app(conn)
    owner = owner_client(app)

    anonymous = TestClient(app).post(CONNECT_URL, headers=CSRF_HEADERS)
    cross_site = owner.post(CONNECT_URL, headers={"X-ActivSync-Request": ""})

    assert anonymous.status_code == 401
    assert cross_site.status_code == 403
    assert db.get_config_value(conn, "strava_oauth_state") is None


def test_the_old_connect_redirect_is_gone(tmp_path):
    conn = db.connect(str(tmp_path / "oauth.db"))
    _complete_setup(conn)

    response = owner_client(create_app(conn)).get("/strava/connect", follow_redirects=False)

    assert response.status_code == 404


def test_strava_callback_rejects_mismatched_and_replayed_state(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "oauth.db"))
    _complete_setup(conn)
    db.set_config_value(conn, "strava_oauth_state", "expected")
    fake_strava = MagicMock()
    monkeypatch.setattr(server_module, "_build_strava_client", lambda _conn: fake_strava)
    client = owner_client(create_app(conn))

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

    response = owner_client(create_app(conn)).get(
        "/strava/callback?code=code&state=expected",
        follow_redirects=False,
    )

    assert response.status_code == 303
    assert response.headers["location"] == "/settings"
    fake_strava.exchange_code.assert_called_once_with("code", granted_scope=None)
    catch_up.assert_called_once_with(conn, fake_garmin, fake_strava)


def test_strava_callback_flags_a_narrowed_grant_for_reconnect(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "oauth.db"))
    _complete_setup(conn)
    db.set_config_value(conn, "strava_oauth_state", "expected")
    catch_up = MagicMock()
    monkeypatch.setattr(server_module, "_build_strava_client", dev_mock.FakeStravaClient)
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda _conn: MagicMock())
    monkeypatch.setattr(server_module.onboarding, "run_catch_up", catch_up)

    response = owner_client(create_app(conn)).get(
        "/strava/callback?code=code&state=expected&scope=read",
        follow_redirects=False,
    )

    connections = view.connection_status(conn)
    assert connections["strava"]["status"].startswith("Reconnect needed")
    assert "strava" in connections["broken"]
    assert response.status_code == 303
    parsed = urlparse(response.headers["location"])
    assert parsed.path == "/settings"
    assert "didn't grant upload access" in parse_qs(parsed.query)["stravaError"][0]
    catch_up.assert_not_called()


def test_strava_callback_keeps_setup_on_the_strava_step_for_a_narrowed_grant(
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
    db.set_config_value(conn, "strava_oauth_state", "expected")
    monkeypatch.setattr(server_module, "_build_strava_client", dev_mock.FakeStravaClient)

    response = owner_client(create_app(conn)).get(
        "/strava/callback?code=code&state=expected&scope=read",
        follow_redirects=False,
    )

    assert response.status_code == 303
    parsed = urlparse(response.headers["location"])
    assert parsed.path == "/setup"
    assert "didn't grant upload access" in parse_qs(parsed.query)["stravaError"][0]
    assert onboarding.setup_step(conn) == "strava"
    assert db.get_config_value(conn, "strava_tokens")["scope"] == "read"


def test_strava_callback_moves_setup_on_for_the_full_grant(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "oauth.db"))
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(
        conn,
        "strava_credentials",
        {"client_id": "client-id", "client_secret": "client-secret"},
    )
    db.set_config_value(conn, "strava_oauth_state", "expected")
    monkeypatch.setattr(server_module, "_build_strava_client", dev_mock.FakeStravaClient)

    response = owner_client(create_app(conn)).get(
        "/strava/callback?code=code&state=expected"
        "&scope=read,activity:write,activity:read_all",
        follow_redirects=False,
    )

    assert response.status_code == 303
    assert response.headers["location"] == "/setup"
    assert onboarding.setup_step(conn) == "hevy"


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
    client = owner_client(create_app(conn))

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


def test_strava_connect_builds_the_callback_from_the_public_url(tmp_path, monkeypatch):
    monkeypatch.setenv("ACTIVSYNC_PUBLIC_URL", "https://activsync.example.com/")
    conn = db.connect(str(tmp_path / "oauth.db"))
    _complete_setup(conn)
    fake_strava = MagicMock()
    fake_strava.authorize_url.side_effect = (
        lambda redirect_uri, state: f"https://strava.test/oauth?redirect={redirect_uri}"
    )
    monkeypatch.setattr(server_module, "_build_strava_client", lambda _conn: fake_strava)
    client = owner_client(create_app(conn), base_url="https://activsync.example.com")

    response = client.post(CONNECT_URL)

    assert response.json()["url"] == (
        "https://strava.test/oauth?redirect=https://activsync.example.com/strava/callback"
    )
