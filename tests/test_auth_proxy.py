import logging

import pytest
from fastapi.testclient import TestClient

from activsync import auth_core, db
from activsync.auth_middleware import ProxyAuthConfig
from activsync.server import create_app

OWNER_PASSWORD = "correct horse battery staple"
PROXY = "172.18.0.2"
OUTSIDER = "203.0.113.7"


@pytest.fixture
def proxy_env(monkeypatch):
    monkeypatch.setenv("ACTIVSYNC_AUTH_PROXY_HEADER", "Remote-User")
    monkeypatch.setenv("ACTIVSYNC_AUTH_PROXY_TRUSTED", "172.18.0.0/16")
    monkeypatch.setenv("ACTIVSYNC_AUTH_PROXY_USER", "andreas")


@pytest.fixture
def app(tmp_path, proxy_env):
    conn = db.connect(str(tmp_path / "proxy.db"))
    auth_core.set_password(conn, OWNER_PASSWORD, 1_800_000_000)
    return create_app(conn)


def _from(app, peer):
    return TestClient(app, client=(peer, 50000), follow_redirects=False)


def test_trusted_peer_asserting_the_owner_is_signed_in(app):
    client = _from(app, PROXY)

    response = client.get("/api/v1/app", headers={"Remote-User": "andreas"})
    status = client.get("/api/v1/auth/status", headers={"Remote-User": "andreas"}).json()

    assert response.status_code == 200
    assert status["method"] == "proxy"
    assert status["user"] == "andreas"
    assert "set-cookie" not in response.headers


def test_trusted_peer_asserting_someone_else_is_refused(app, caplog):
    with caplog.at_level(logging.WARNING, logger="activsync.auth"):
        response = _from(app, PROXY).get("/api/v1/app", headers={"Remote-User": "mallory"})

    assert response.status_code == 401
    assert "not the configured owner" in caplog.text


def test_trusted_peer_without_the_header_falls_through_to_cookies(app):
    response = _from(app, PROXY).get("/api/v1/app")

    assert response.status_code == 401


def test_untrusted_peer_header_is_ignored_and_logged_once_a_minute(app, caplog):
    client = _from(app, OUTSIDER)
    with caplog.at_level(logging.WARNING, logger="activsync.auth"):
        first = client.get("/api/v1/app", headers={"Remote-User": "andreas"})
        second = client.get("/api/v1/app", headers={"Remote-User": "andreas"})

    assert first.status_code == second.status_code == 401
    assert caplog.text.count("untrusted peer") == 1


def test_forwarded_for_cannot_impersonate_the_proxy(app, monkeypatch):
    monkeypatch.setenv("FORWARDED_ALLOW_IPS", "*")
    spoofed = create_app(app.state.db)

    response = _from(spoofed, OUTSIDER).get(
        "/api/v1/app",
        headers={"Remote-User": "andreas", "X-Forwarded-For": PROXY},
    )

    assert response.status_code == 401


def test_proxy_headers_still_resolve_the_client_for_the_login_limiter(app, monkeypatch):
    monkeypatch.setenv("FORWARDED_ALLOW_IPS", PROXY)
    proxied = create_app(app.state.db)
    client = _from(proxied, PROXY)
    client.headers.update({"X-ActivSync-Request": "1", "Origin": "http://testserver"})

    def login_from(address):
        # A trusted-device cookie skips the per-client limiter, so each
        # attempt arrives as a fresh browser.
        client.cookies.clear()
        return client.post(
            "/api/v1/auth/login",
            json={"password": OWNER_PASSWORD},
            headers={"X-Forwarded-For": address},
        )

    for _ in range(10):
        assert login_from(OUTSIDER).status_code == 200

    assert login_from(OUTSIDER).status_code == 429
    assert login_from("198.51.100.9").status_code == 200


def test_settings_describe_the_proxy_without_secrets(app):
    client = _from(app, PROXY)
    client.headers.update({"Remote-User": "andreas"})

    assert client.get("/api/v1/auth/settings").json()["proxy"] == {
        "header": "Remote-User",
        "trusted": ["172.18.0.0/16"],
        "user": "andreas",
    }


def test_logout_is_refused_for_proxy_sessions(app):
    client = _from(app, PROXY)
    client.headers.update({
        "Remote-User": "andreas", "X-ActivSync-Request": "1", "Origin": "http://testserver",
    })

    assert client.post("/api/v1/auth/logout").status_code == 409


@pytest.mark.parametrize(
    ("environ", "message"),
    [
        ({"ACTIVSYNC_AUTH_PROXY_HEADER": "Remote-User", "ACTIVSYNC_AUTH_PROXY_USER": "a"}, "TRUSTED"),
        ({"ACTIVSYNC_AUTH_PROXY_HEADER": "Remote-User", "ACTIVSYNC_AUTH_PROXY_TRUSTED": "10.0.0.0/8"}, "USER"),
        ({"ACTIVSYNC_AUTH_PROXY_HEADER": "Remote-User", "ACTIVSYNC_AUTH_PROXY_TRUSTED": "0.0.0.0/0", "ACTIVSYNC_AUTH_PROXY_USER": "a"}, "every address"),
        ({"ACTIVSYNC_AUTH_PROXY_HEADER": "Remote-User", "ACTIVSYNC_AUTH_PROXY_TRUSTED": "10.0.0.0/8,::/0", "ACTIVSYNC_AUTH_PROXY_USER": "a"}, "every address"),
        ({"ACTIVSYNC_AUTH_PROXY_HEADER": "Remote-User", "ACTIVSYNC_AUTH_PROXY_TRUSTED": "not-a-cidr", "ACTIVSYNC_AUTH_PROXY_USER": "a"}, "invalid"),
    ],
)
def test_startup_refuses_an_unsafe_proxy_config(environ, message):
    with pytest.raises(ValueError, match=message):
        ProxyAuthConfig.from_env(environ)


def test_no_header_means_the_method_is_off():
    assert ProxyAuthConfig.from_env({}) is None


def test_create_app_refuses_an_unsafe_proxy_config(tmp_path, monkeypatch):
    monkeypatch.setenv("ACTIVSYNC_AUTH_PROXY_HEADER", "Remote-User")

    with pytest.raises(ValueError):
        create_app(db.connect(str(tmp_path / "bad.db")))
