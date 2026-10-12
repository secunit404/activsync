from urllib.parse import parse_qs, urlsplit

import pytest
from fastapi.testclient import TestClient

from activsync import auth_oidc, db
from activsync.server import create_app
from auth_helpers import OWNER_PASSWORD, owner_client
from fake_idp import CLIENT_ID, CLIENT_SECRET, ISSUER, FakeIdp, unreachable_transport

OIDC_CONFIG = {
    "displayName": "Authentik",
    "issuer": ISSUER + "/",
    "clientId": CLIENT_ID,
    "clientSecret": CLIENT_SECRET,
    "scopes": "openid profile email",
    "autoRedirect": False,
    "currentPassword": OWNER_PASSWORD,
}


def _setup(tmp_path, *, alg="RS256"):
    idp = FakeIdp(alg=alg)
    conn = db.connect(str(tmp_path / "oidc.db"))
    app = create_app(conn, oidc_transport=idp.transport)
    owner = owner_client(app, follow_redirects=False)
    assert owner.put("/api/v1/auth/oidc", json=OIDC_CONFIG).status_code == 200
    return conn, app, idp, owner


def _flow(client, idp, start, *, tamper=None):
    started = client.get(start)
    assert started.status_code == 302, started.text
    params = idp.authorize(started.headers["location"])
    if tamper:
        params = tamper(params)
    return client.get("/auth/oidc/callback", params=params)


def _link(owner, idp):
    response = _flow(owner, idp, "/auth/oidc/login?intent=link")
    assert response.headers["location"] == "/settings?oidc=linked"


def _error(response):
    return parse_qs(urlsplit(response.headers["location"]).query)["error"][0]


def test_link_stores_iss_and_sub_with_a_display_label(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)

    _link(owner, idp)

    assert auth_oidc.load_config(conn).linked == {
        "iss": ISSUER,
        "sub": "owner-sub",
        "label": "andreas",
    }
    assert owner.get("/api/v1/auth/oidc").json()["linkedLabel"] == "andreas"


def test_link_requires_a_session(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)

    response = TestClient(app, follow_redirects=False).get("/auth/oidc/login?intent=link")

    assert response.status_code == 303
    assert response.headers["location"].startswith("/login")


def test_login_with_the_linked_account_starts_a_session(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)
    visitor = TestClient(app, follow_redirects=False)

    response = _flow(visitor, idp, "/auth/oidc/login?next=%2Fhevy")

    assert response.status_code == 303
    assert response.headers["location"] == "/hevy"
    assert visitor.get("/api/v1/auth/status").json()["method"] == "oidc"


def test_a_different_subject_is_refused(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)
    idp.sub = "someone-else"
    visitor = TestClient(app, follow_redirects=False)

    response = _flow(visitor, idp, "/auth/oidc/login")

    assert _error(response) == "oidc_mismatch"
    assert visitor.get("/api/v1/auth/status").json()["authenticated"] is False


def test_a_forged_state_is_refused(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)
    visitor = TestClient(app, follow_redirects=False)

    response = _flow(visitor, idp, "/auth/oidc/login", tamper=lambda p: {**p, "state": "forged"})

    assert _error(response) == "oidc_state"


def test_a_callback_without_a_started_flow_is_refused(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)

    response = TestClient(app, follow_redirects=False).get(
        "/auth/oidc/callback", params={"code": "x", "state": "y"}
    )

    assert _error(response) == "oidc_state"


def test_a_wrong_nonce_is_refused(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)
    idp.nonce_override = "replayed-nonce"

    response = _flow(TestClient(app, follow_redirects=False), idp, "/auth/oidc/login")

    assert _error(response) == "oidc_state"


def test_a_token_for_another_audience_is_refused(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)
    idp.audience = "another-client"

    response = _flow(TestClient(app, follow_redirects=False), idp, "/auth/oidc/login")

    assert _error(response) == "oidc_state"


def test_a_token_for_another_audience_naming_us_as_azp_is_refused(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)
    idp.audience = "another-client"
    idp.azp = CLIENT_ID

    response = _flow(TestClient(app, follow_redirects=False), idp, "/auth/oidc/login")

    assert _error(response) == "oidc_state"


def test_a_token_from_another_issuer_is_refused(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)
    idp.issuer_claim = "https://evil.test"

    response = _flow(TestClient(app, follow_redirects=False), idp, "/auth/oidc/login")

    assert _error(response) == "oidc_state"


def test_pkce_s256_challenge_is_sent_and_the_verifier_redeemed(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)

    started = owner.get("/auth/oidc/login?intent=link")
    query = parse_qs(urlsplit(started.headers["location"]).query)
    owner.get("/auth/oidc/callback", params=idp.authorize(started.headers["location"]))

    assert query["code_challenge_method"] == ["S256"]
    assert query["code_challenge"][0]
    assert query["nonce"][0]
    assert query["scope"] == ["openid profile email"]
    assert idp.token_requests[0]["code_verifier"]


def test_es256_signed_tokens_are_accepted(tmp_path):
    conn, app, idp, owner = _setup(tmp_path, alg="ES256")
    _link(owner, idp)

    response = _flow(TestClient(app, follow_redirects=False), idp, "/auth/oidc/login")

    assert response.headers["location"] == "/"


def test_an_unreachable_provider_sends_the_browser_back_with_a_code(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)
    app_without_idp = create_app(conn, oidc_transport=unreachable_transport())

    response = TestClient(app_without_idp, follow_redirects=False).get("/auth/oidc/login")

    assert _error(response) == "oidc_unavailable"


@pytest.mark.parametrize(
    "next_path",
    ["//evil.test", "https://evil.test", "/\\evil.test", "/.//evil.test", "/a/..//evil.test"],
)
def test_next_must_stay_on_this_site(tmp_path, next_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)
    visitor = TestClient(app, follow_redirects=False)

    response = _flow(visitor, idp, f"/auth/oidc/login?next={next_path}")

    assert response.headers["location"] == "/"
