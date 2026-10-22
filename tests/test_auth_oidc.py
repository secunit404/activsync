import time
from urllib.parse import parse_qs, urlsplit

import pytest
from fastapi.testclient import TestClient

from activsync import auth_core, auth_oidc, db
from activsync.server import create_app
from auth_helpers import CSRF_HEADERS, OWNER_PASSWORD, owner_client
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


LINK_START = "/auth/oidc/login?intent=link&next=%2Fsettings"


def _grant(client, password=OWNER_PASSWORD):
    return client.post("/api/v1/auth/oidc/link", json={"currentPassword": password})


def _link(owner, idp):
    assert _grant(owner).status_code == 200
    response = _flow(owner, idp, LINK_START)
    assert response.headers["location"] == "/settings?oidc=linked"


def _oidc_visitor(app, idp):
    visitor = TestClient(app, follow_redirects=False)
    visitor.headers.update(CSRF_HEADERS)
    response = _flow(visitor, idp, "/auth/oidc/login")
    assert response.status_code == 303
    assert visitor.get("/api/v1/auth/status").json()["method"] == "oidc"
    return visitor


def _signed_in(client):
    return client.get("/api/v1/auth/status").json()["authenticated"]


def _error(response):
    return parse_qs(urlsplit(response.headers["location"]).query)["error"][0]


def _link_error(response):
    return parse_qs(urlsplit(response.headers["location"]).query)["oidcError"][0]


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


def test_link_grant_needs_a_session(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    anonymous = TestClient(app, follow_redirects=False)
    anonymous.headers.update(CSRF_HEADERS)

    response = _grant(anonymous)

    assert response.status_code == 401


def test_link_grant_needs_the_current_password(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)

    refused = _grant(owner, "wrong one")
    started = owner.get(LINK_START)

    assert refused.status_code == 400
    assert started.status_code == 303
    assert _link_error(started) == "link_password"
    assert idp.token_requests == []


def test_link_grant_needs_oidc_to_be_configured(tmp_path):
    conn = db.connect(str(tmp_path / "bare.db"))
    owner = owner_client(create_app(conn), follow_redirects=False)

    response = _grant(owner)

    assert response.status_code == 409


def test_link_grant_returns_the_link_start_and_the_flow_proceeds_to_the_provider(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)

    granted = _grant(owner)
    started = owner.get(granted.json()["url"])

    assert granted.status_code == 200
    assert granted.json() == {"url": LINK_START}
    assert started.status_code == 302
    assert started.headers["location"].startswith(f"{ISSUER}/authorize?")
    callback = owner.get("/auth/oidc/callback", params=idp.authorize(started.headers["location"]))
    assert callback.headers["location"] == "/settings?oidc=linked"


def test_starting_a_link_without_a_grant_is_sent_back_to_settings(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)

    response = owner.get("/auth/oidc/login?intent=link")

    assert response.status_code == 303
    assert _link_error(response) == "link_password"
    assert auth_oidc.load_config(conn).linked is None


def test_a_link_grant_is_single_use(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    assert _grant(owner).status_code == 200

    first = owner.get(LINK_START)
    second = owner.get(LINK_START)

    assert first.status_code == 302
    assert second.status_code == 303
    assert _link_error(second) == "link_password"


def test_an_expired_link_grant_is_refused(tmp_path, monkeypatch):
    conn, app, idp, owner = _setup(tmp_path)
    assert _grant(owner).status_code == 200
    later = time.time() + auth_oidc.LINK_GRANT_SECONDS + 1
    monkeypatch.setattr(time, "time", lambda: later)

    response = owner.get(LINK_START)

    assert response.status_code == 303
    assert _link_error(response) == "link_password"


def test_a_link_grant_does_not_carry_to_another_browser(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    assert _grant(owner).status_code == 200
    other_browser = owner_client(app, follow_redirects=False)

    response = other_browser.get(LINK_START)

    assert _link_error(response) == "link_password"


def test_a_grant_is_not_needed_to_sign_in(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)

    started = TestClient(app, follow_redirects=False).get("/auth/oidc/login")

    assert started.status_code == 302


@pytest.mark.parametrize(
    ("now", "stored", "granted"),
    [
        (1000.0, 1300, True),
        (1299.9, 1300, True),
        (1300.0, 1300, False),
        (2000.0, 1300, False),
        (1000.0, None, False),
        (1000.0, "1300", False),
    ],
)
def test_consuming_a_grant_checks_the_expiry_and_always_removes_it(now, stored, granted):
    session = {} if stored is None else {auth_oidc.LINK_GRANT_KEY: stored}

    assert auth_oidc.consume_link_grant(session, now) is granted
    assert auth_oidc.LINK_GRANT_KEY not in session


def test_granting_sets_a_five_minute_expiry():
    session = {}

    auth_oidc.grant_link(session, 1000.4)

    assert session == {auth_oidc.LINK_GRANT_KEY: 1000 + 300}


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

    assert _grant(owner).status_code == 200
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


def test_relinking_to_another_account_signs_out_the_old_accounts_sessions(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)
    visitor = _oidc_visitor(app, idp)
    idp.sub = "someone-else"

    _link(owner, idp)

    assert _signed_in(visitor) is False
    assert _signed_in(owner) is True
    assert auth_oidc.load_config(conn).linked["sub"] == "someone-else"


def test_relinking_to_another_account_keeps_the_session_doing_it(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)
    linking = _oidc_visitor(app, idp)
    other = _oidc_visitor(app, idp)
    idp.sub = "someone-else"

    _link(linking, idp)

    assert _signed_in(linking) is True
    assert _signed_in(other) is False
    assert [s.method for s in auth_core.list_sessions(conn)].count("oidc") == 1


def test_relinking_the_same_account_keeps_every_session(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    _link(owner, idp)
    visitor = _oidc_visitor(app, idp)

    _link(owner, idp)

    assert _signed_in(visitor) is True


def test_the_first_link_leaves_password_sessions_alone(tmp_path):
    conn, app, idp, owner = _setup(tmp_path)
    other_browser = owner_client(app, follow_redirects=False)

    _link(owner, idp)

    assert _signed_in(other_browser) is True
