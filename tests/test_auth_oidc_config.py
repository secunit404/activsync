import pytest
from fastapi.testclient import TestClient

from activsync import auth_cli, auth_oidc, db
from activsync.server import create_app
from auth_helpers import OWNER_PASSWORD, owner_client

ISSUER = "https://id.example.com"
CONFIG = {
    "displayName": "Pocket ID",
    "issuer": ISSUER + "/",
    "clientId": "activsync",
    "clientSecret": "very-secret-value",
    "scopes": "openid profile email",
    "autoRedirect": False,
    "currentPassword": OWNER_PASSWORD,
}


@pytest.fixture
def conn(tmp_path, monkeypatch):
    path = tmp_path / "oidc.db"
    monkeypatch.setenv("ACTIVSYNC_DB_PATH", str(path))
    return db.connect(str(path))


@pytest.fixture
def owner(conn):
    return owner_client(create_app(conn))


def _link(conn):
    auth_oidc.set_linked(conn, iss=ISSUER, sub="owner-sub", label="andreas")


def test_saving_needs_the_current_password(owner, conn):
    rejected = owner.put("/api/v1/auth/oidc", json={**CONFIG, "currentPassword": "wrong one"})

    assert rejected.status_code == 400
    assert auth_oidc.load_config(conn) is None


def test_config_round_trip_hides_the_secret(owner):
    assert owner.put("/api/v1/auth/oidc", json=CONFIG).status_code == 200

    state = owner.get("/api/v1/auth/oidc").json()

    assert state == {
        "configured": True,
        "displayName": "Pocket ID",
        "issuer": ISSUER,
        "clientId": "activsync",
        "hasSecret": True,
        "scopes": "openid profile email",
        "autoRedirect": False,
        "redirectUri": "http://testserver/auth/oidc/callback",
        "linkedLabel": None,
        "logoUrl": None,
    }


def test_redirect_uri_uses_the_public_url(conn, monkeypatch):
    monkeypatch.setenv("ACTIVSYNC_PUBLIC_URL", "https://activsync.example.com")
    owner = owner_client(create_app(conn), base_url="https://activsync.example.com")

    state = owner.get("/api/v1/auth/oidc").json()

    assert state["redirectUri"] == "https://activsync.example.com/auth/oidc/callback"


def test_an_issuer_must_be_a_url(owner):
    response = owner.put("/api/v1/auth/oidc", json={**CONFIG, "issuer": "id.example.com"})

    assert response.status_code == 400


def test_a_new_issuer_or_client_drops_the_link(owner, conn):
    owner.put("/api/v1/auth/oidc", json=CONFIG)
    _link(conn)

    owner.put("/api/v1/auth/oidc", json={**CONFIG, "displayName": "Renamed"})
    assert auth_oidc.load_config(conn).linked is not None
    owner.put("/api/v1/auth/oidc", json={**CONFIG, "clientId": "other"})
    assert auth_oidc.load_config(conn).linked is None


def test_saving_without_a_secret_keeps_the_stored_one(owner, conn):
    owner.put("/api/v1/auth/oidc", json=CONFIG)

    owner.put("/api/v1/auth/oidc", json={**CONFIG, "clientSecret": None})

    assert auth_oidc.load_config(conn).client_secret == "very-secret-value"


def test_removing_needs_the_current_password(owner, conn):
    owner.put("/api/v1/auth/oidc", json=CONFIG)

    rejected = owner.request("DELETE", "/api/v1/auth/oidc", json={"currentPassword": "wrong one"})
    removed = owner.request("DELETE", "/api/v1/auth/oidc", json={"currentPassword": OWNER_PASSWORD})

    assert rejected.status_code == 400
    assert removed.status_code == 200
    assert auth_oidc.load_config(conn) is None


def test_login_status_offers_oidc_only_once_linked(owner, conn):
    owner.put("/api/v1/auth/oidc", json={**CONFIG, "autoRedirect": True})
    anonymous = TestClient(owner.app)

    assert anonymous.get("/api/v1/auth/status").json()["login"]["oidc"] is None
    _link(conn)
    assert anonymous.get("/api/v1/auth/status").json()["login"]["oidc"] == {
        "label": "Pocket ID",
        "autoRedirect": True,
        "logoUrl": None,
    }
    assert owner.get("/api/v1/auth/oidc").json()["linkedLabel"] == "andreas"


def test_cli_disables_oidc_and_reports_it_without_the_secret(owner, conn, capsys):
    owner.put("/api/v1/auth/oidc", json=CONFIG)

    assert auth_cli.main(["status"]) == 0
    out = capsys.readouterr().out
    assert f"Single sign-on:       Pocket ID ({ISSUER}), not linked" in out
    assert "very-secret-value" not in out

    assert auth_cli.main(["disable-oidc"]) == 0
    assert auth_oidc.load_config(conn) is None
