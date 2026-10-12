import time
from unittest.mock import MagicMock
from urllib.parse import parse_qs, urlsplit

import pytest
from fastapi.testclient import TestClient

from activsync import auth_core, db, dev_mock, view
from activsync import server as server_module
from activsync.server import create_app
from activsync.strava_client import StravaClient
from auth_helpers import CSRF_HEADERS, owner_client

OWNER_ATHLETE = dev_mock.DEV_STRAVA_ATHLETE_ID
OLD_TOKENS = {
    "access_token": "old-access",
    "refresh_token": "old-refresh",
    "expires_at": 1,
    "athlete_id": OWNER_ATHLETE,
}


class RecordingStrava(dev_mock.FakeStravaClient):
    def __init__(self, conn, *, athlete_id=OWNER_ATHLETE):
        super().__init__(conn)
        self.athlete_id = athlete_id
        self.disconnect_calls = 0

    def fetch_tokens(self, code):
        return {
            "access_token": "new-access",
            "refresh_token": "new-refresh",
            "expires_at": int(time.time()) + 3600,
            "athlete": {"id": self.athlete_id},
        }

    def disconnect(self):
        self.disconnect_calls += 1


@pytest.fixture
def conn(tmp_path):
    conn = db.connect(str(tmp_path / "strava-login.db"))
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(
        conn, "strava_credentials", {"client_id": "cid", "client_secret": "secret"}
    )
    db.set_config_value(conn, "strava_tokens", OLD_TOKENS)
    auth_core.save_settings(conn, strava_login=True)
    return conn


@pytest.fixture
def strava(conn, monkeypatch):
    fake = RecordingStrava(conn)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda _conn: fake)
    return fake


LOGIN_URL = "/api/v1/auth/strava/authorize"


def _anonymous(app) -> TestClient:
    client = TestClient(app, follow_redirects=False)
    client.headers.update(CSRF_HEADERS)
    return client


def _login(
    app, *, scope="read,activity:write,activity:read_all", state=None, next="/settings"
):
    client = _anonymous(app)
    started = client.post(LOGIN_URL, json={"next": next})
    assert started.status_code == 200
    callback = urlsplit(started.json()["url"])
    params = {key: values[0] for key, values in parse_qs(callback.query).items()}
    if state is not None:
        params["state"] = state
    if scope is not None:
        params["scope"] = scope
    return client, client.get(callback.path, params=params)


def test_login_url_goes_straight_to_strava_with_the_shared_scope_and_the_auth_callback(
    conn, monkeypatch
):
    real = StravaClient(conn, "cid", "secret")
    monkeypatch.setattr(server_module, "_build_strava_client", lambda _conn: real)

    started = _anonymous(create_app(conn)).post(LOGIN_URL, json={})

    assert started.status_code == 200
    location = started.json()["url"]
    assert location.startswith("https://www.strava.com/oauth/authorize?")
    assert "client_id=cid" in location
    assert "scope=read,activity:write,activity:read_all" in location
    assert "approval_prompt=auto" in location
    assert "redirect_uri=http://testserver/auth/strava/callback" in location


def test_the_owner_athlete_signs_in_and_the_tokens_are_replaced(conn, strava):
    client, response = _login(create_app(conn))

    assert response.status_code == 303
    assert response.headers["location"] == "/settings"
    assert client.get("/api/v1/auth/status").json()["method"] == "strava"
    tokens = db.get_config_value(conn, "strava_tokens")
    assert tokens["access_token"] == "new-access"
    assert tokens["refresh_token"] == "new-refresh"
    assert tokens["athlete_id"] == OWNER_ATHLETE


def test_another_athlete_is_refused_and_tokens_are_untouched(conn, strava):
    strava.athlete_id = OWNER_ATHLETE + 1

    client, response = _login(create_app(conn))

    assert response.headers["location"] == "/login?error=strava_mismatch"
    assert db.get_config_value(conn, "strava_tokens") == OLD_TOKENS
    assert client.get("/api/v1/auth/status").json()["authenticated"] is False


def test_a_forged_state_is_refused_before_the_code_is_exchanged(conn, strava, monkeypatch):
    exchanged = MagicMock()
    monkeypatch.setattr(strava, "fetch_tokens", exchanged)

    _, response = _login(create_app(conn), state="forged")

    assert response.headers["location"] == "/login?error=strava_state"
    exchanged.assert_not_called()


def test_a_narrowed_scope_keeps_the_tokens_but_flags_a_reconnect(conn, strava):
    _, response = _login(create_app(conn), scope="read")

    assert response.headers["location"] == "/settings"
    assert db.get_config_value(conn, "strava_tokens")["refresh_token"] == "new-refresh"
    connections = view.connection_status(conn)
    assert connections["strava"]["status"].startswith("Reconnect needed")
    assert "strava" in connections["broken"]


def test_logout_never_calls_strava(conn, strava):
    client, _ = _login(create_app(conn))

    client.headers.update({"X-ActivSync-Request": "1", "Origin": "http://testserver"})
    assert client.post("/api/v1/auth/logout").status_code == 204
    assert strava.disconnect_calls == 0
    assert db.get_config_value(conn, "strava_tokens")["refresh_token"] == "new-refresh"


def test_the_button_is_offered_only_when_enabled_and_possible(conn, strava):
    app = create_app(conn)
    anonymous = TestClient(app)

    assert anonymous.get("/api/v1/auth/status").json()["login"]["strava"] is True
    db.set_config_value(conn, "strava_tokens", {**OLD_TOKENS, "athlete_id": None})
    assert anonymous.get("/api/v1/auth/status").json()["login"]["strava"] is False
    started = _anonymous(app).post(LOGIN_URL, json={"next": "/settings"})
    assert started.status_code == 409
    assert started.json() == {
        "detail": "Sign in with Strava is not available right now. Use your password instead."
    }


def test_enabling_waits_for_the_athlete_id(conn, strava):
    db.set_config_value(conn, "strava_tokens", {**OLD_TOKENS, "athlete_id": None})
    auth_core.save_settings(conn, strava_login=False)
    owner = owner_client(create_app(conn))

    refused = owner.put("/api/v1/auth/settings", json={"stravaLogin": True})
    settings = owner.get("/api/v1/auth/settings").json()

    assert refused.status_code == 409
    assert refused.json()["detail"] == "Waiting for the next Strava sync."
    assert settings["stravaLoginBlocker"] == "Waiting for the next Strava sync."
    assert settings["stravaLogin"] is False


@pytest.mark.parametrize("unsafe", ["//evil.test", "https://evil.test", "/\\evil.test"])
def test_next_is_made_safe_on_the_server(conn, strava, unsafe):
    _, response = _login(create_app(conn), next=unsafe)

    assert response.headers["location"] == "/"


def test_starting_a_strava_login_needs_the_csrf_header(conn, strava):
    response = TestClient(create_app(conn)).post(LOGIN_URL, json={"next": "/"})

    assert response.status_code == 403


def test_the_old_login_redirect_is_gone(conn, strava):
    response = owner_client(create_app(conn)).get("/auth/strava/login")

    assert response.status_code == 404
