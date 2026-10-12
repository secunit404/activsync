import threading
import time

import pytest
from fastapi.testclient import TestClient

from activsync import auth_core, db, onboarding
from activsync.server import create_app
from auth_helpers import CSRF_HEADERS, OWNER_PASSWORD, owner_client


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "auth.db"))


@pytest.fixture
def app(conn):
    return create_app(conn)


def _anonymous(app) -> TestClient:
    client = TestClient(app, follow_redirects=False)
    client.headers.update(CSRF_HEADERS)
    return client


def _claim(conn):
    auth_core.set_password(conn, OWNER_PASSWORD, time.time())


def _login(client, password=OWNER_PASSWORD):
    return client.post("/api/v1/auth/login", json={"password": password})


def test_status_before_claim(app):
    body = _anonymous(app).get("/api/v1/auth/status").json()

    assert body == {
        "authenticated": False,
        "method": None,
        "user": None,
        "ownerClaimed": False,
        "authDisabled": False,
        "login": {"passwordForm": "shown", "oidc": None, "strava": False},
    }


def test_claim_needs_the_setup_code_then_signs_in(app):
    client = _anonymous(app)
    code = app.state.setup_code.current

    wrong = client.post(
        "/api/v1/auth/claim", json={"setupCode": "AAAA-AAAA-AAAA-AAAA", "password": OWNER_PASSWORD}
    )
    short = client.post("/api/v1/auth/claim", json={"setupCode": code, "password": "short"})
    claimed = client.post(
        "/api/v1/auth/claim", json={"setupCode": code, "password": OWNER_PASSWORD}
    )

    assert wrong.status_code == 403
    assert short.status_code == 400
    assert claimed.status_code == 200
    assert claimed.json()["authenticated"] is True
    assert "activsync_session" in claimed.cookies
    assert app.state.setup_code.current is None
    assert client.get("/api/v1/app").status_code == 200
    again = client.post(
        "/api/v1/auth/claim", json={"setupCode": code, "password": OWNER_PASSWORD}
    )
    assert again.status_code == 409


def test_session_cookie_flags_over_http(app, conn):
    _claim(conn)

    cookie = _login(_anonymous(app)).headers["set-cookie"]

    assert cookie.startswith("activsync_session=")
    assert "HttpOnly" in cookie
    assert "SameSite=lax" in cookie
    assert "Path=/" in cookie
    assert "Secure" not in cookie


def test_https_uses_a_host_prefixed_secure_cookie(app, conn):
    _claim(conn)
    client = TestClient(app, base_url="https://testserver")
    client.headers.update({**CSRF_HEADERS, "Origin": "https://testserver"})

    cookie = _login(client).headers["set-cookie"]

    assert cookie.startswith("__Host-activsync_session=")
    assert "Secure" in cookie
    assert client.get("/api/v1/app").status_code == 200


def test_login_rotates_the_session_token(app, conn):
    _claim(conn)
    client = _anonymous(app)

    _login(client)
    first = client.cookies["activsync_session"]
    _login(client)
    second = client.cookies["activsync_session"]

    assert first != second
    assert auth_core.resolve_session(conn, first, time.time()) is None
    assert len(auth_core.list_sessions(conn)) == 1


def test_wrong_password_is_one_generic_message(app, conn):
    _claim(conn)

    response = _login(_anonymous(app), "nope")

    assert response.status_code == 401
    assert response.json() == {"detail": "Wrong password."}


def test_login_before_claim_is_refused(app):
    assert _login(_anonymous(app)).status_code == 409


def test_throttled_login_answers_429_without_verifying(app, conn, monkeypatch):
    _claim(conn)
    for _ in range(6):
        auth_core.record_failure(conn, time.time())
    verified = []
    monkeypatch.setattr(
        auth_core, "verify_password", lambda *args: verified.append(args) or True
    )

    response = _login(_anonymous(app))

    assert response.status_code == 429
    assert int(response.headers["retry-after"]) >= 1
    assert verified == []


def test_successful_login_resets_the_backoff(app, conn):
    _claim(conn)
    client = _anonymous(app)
    for _ in range(4):
        _login(client, "nope")

    assert _login(client).status_code == 200
    assert db.get_config_value(conn, "auth_login_throttle")["failures"] == 0


def test_one_client_is_limited_to_ten_attempts_a_minute(app, conn):
    _claim(conn)
    client = _anonymous(app)
    for _ in range(10):
        client.cookies.clear()
        _login(client)

    client.cookies.clear()
    response = _login(client)

    assert response.status_code == 429
    assert "retry-after" in response.headers


def test_logout_ends_only_this_session(app):
    client = owner_client(app)
    other = owner_client(app)

    assert client.post("/api/v1/auth/logout").status_code == 204
    assert client.get("/api/v1/app").status_code == 401
    assert other.get("/api/v1/app").status_code == 200


def test_sessions_list_marks_the_current_one_and_hides_hashes(app, conn):
    client = owner_client(app)
    owner_client(app)

    sessions = client.get("/api/v1/auth/sessions").json()

    assert len(sessions) == 2
    assert [entry["current"] for entry in sessions].count(True) == 1
    assert set(sessions[0]) == {
        "publicId", "method", "createdAt", "lastSeenAt", "ip", "userAgent", "current",
    }
    hashes = {session.token_hash for session in auth_core.list_sessions(conn)}
    assert not hashes & {entry["publicId"] for entry in sessions}


def test_revoke_one_and_revoke_all_others(app, conn):
    client = owner_client(app)
    other = owner_client(app)
    third = owner_client(app)
    other_id = next(
        entry["publicId"]
        for entry in client.get("/api/v1/auth/sessions").json()
        if not entry["current"]
    )

    assert client.delete(f"/api/v1/auth/sessions/{other_id}").status_code == 204
    assert client.delete(f"/api/v1/auth/sessions/{other_id}").status_code == 404
    assert client.delete("/api/v1/auth/sessions").json()["message"].startswith("Signed out")
    assert client.get("/api/v1/app").status_code == 200
    assert other.get("/api/v1/app").status_code == 401
    assert third.get("/api/v1/app").status_code == 401
    assert len(auth_core.list_sessions(conn)) == 1


def test_password_change_verifies_current_and_revokes_other_sessions(app, conn):
    client = owner_client(app)
    other = owner_client(app)
    new_password = "a different long passphrase"

    wrong = client.put("/api/v1/auth/password", json={"current": "nope", "new": new_password})
    short = client.put(
        "/api/v1/auth/password", json={"current": OWNER_PASSWORD, "new": "short"}
    )
    changed = client.put(
        "/api/v1/auth/password", json={"current": OWNER_PASSWORD, "new": new_password}
    )

    assert wrong.status_code == 400
    assert short.status_code == 400
    assert changed.status_code == 200
    assert client.get("/api/v1/app").status_code == 200
    assert other.get("/api/v1/app").status_code == 401
    assert auth_core.verify_password(conn, new_password) is True


def test_hidden_password_form_is_reported_in_status(app):
    client = owner_client(app)

    assert client.put("/api/v1/auth/settings", json={"passwordForm": "hidden"}).status_code == 200

    assert _anonymous(app).get("/api/v1/auth/status").json()["login"]["passwordForm"] == "hidden"
    assert client.get("/api/v1/auth/settings").json()["passwordForm"] == "hidden"


def test_upgraded_install_is_gated_to_the_account_step(app, conn):
    db.set_config_value(conn, "initial_sync_done", True)

    assert onboarding.setup_step(conn) == "account"
    assert _anonymous(app).get("/api/v1/auth/status").json()["ownerClaimed"] is False
    _claim(conn)
    assert onboarding.setup_step(conn) is None


def test_disabled_auth_reports_itself_and_skips_the_account_step(monkeypatch, conn):
    monkeypatch.setenv("ACTIVSYNC_AUTH_DISABLED", "1")
    client = TestClient(create_app(conn))

    status = client.get("/api/v1/auth/status").json()

    assert status["authDisabled"] is True
    assert status["authenticated"] is True
    assert status["method"] == "disabled"
    assert client.get("/api/v1/app").json()["setup"]["step"] == "garmin"


def test_dev_unclaim_is_mock_only(app, monkeypatch):
    client = owner_client(app)

    assert client.post("/api/v1/dev/unclaim").status_code == 404
    monkeypatch.setenv("ACTIVSYNC_DEV_MOCK_DATA", "1")
    response = client.post("/api/v1/dev/unclaim")

    assert response.status_code == 200
    assert response.json()["setupCode"] == app.state.setup_code.current
    assert _anonymous(app).get("/api/v1/auth/status").json()["ownerClaimed"] is False


def _devices(conn):
    return db.get_config_value(conn, "auth_trusted_devices") or []


def _cookie_header(response, name):
    return next(
        value
        for key, value in response.headers.multi_items()
        if key == "set-cookie" and value.startswith(f"{name}=")
    )


def test_login_issues_a_one_year_device_cookie_over_http(app, conn):
    _claim(conn)

    response = _login(_anonymous(app))

    cookie = _cookie_header(response, "activsync_device")
    assert "HttpOnly" in cookie
    assert "SameSite=lax" in cookie
    assert "Path=/" in cookie
    assert "Max-Age=31536000" in cookie
    assert "Secure" not in cookie
    device_id = response.cookies["activsync_device"]
    assert [entry["hash"] for entry in _devices(conn)] == [auth_core.hash_token(device_id)]


def test_https_uses_a_host_prefixed_secure_device_cookie(app, conn):
    _claim(conn)
    client = TestClient(app, base_url="https://testserver")
    client.headers.update({**CSRF_HEADERS, "Origin": "https://testserver"})

    cookie = _cookie_header(_login(client), "__Host-activsync_device")

    assert "Secure" in cookie
    assert "Max-Age=31536000" in cookie


def test_claim_issues_a_device_cookie(app, conn):
    client = _anonymous(app)

    response = client.post(
        "/api/v1/auth/claim",
        json={"setupCode": app.state.setup_code.current, "password": OWNER_PASSWORD},
    )

    assert response.status_code == 200
    device_id = response.cookies["activsync_device"]
    assert _devices(conn)[0]["hash"] == auth_core.hash_token(device_id)


def test_signing_in_again_replaces_this_browsers_device(app, conn):
    _claim(conn)
    client = _anonymous(app)

    _login(client)
    first = client.cookies["activsync_device"]
    _login(client)

    assert client.cookies["activsync_device"] != first
    assert len(_devices(conn)) == 1


def test_exhausted_shared_bucket_does_not_block_a_trusted_device(app, conn):
    _claim(conn)
    known = _anonymous(app)
    assert _login(known).status_code == 200
    for _ in range(20):
        auth_core.record_failure(conn, time.time())

    assert _login(_anonymous(app)).status_code == 429
    assert _login(known, "nope").status_code == 401
    assert _login(known).status_code == 200


def test_trusted_device_skips_the_per_client_limiter(app, conn):
    _claim(conn)
    known = _anonymous(app)
    assert _login(known).status_code == 200
    stranger = _anonymous(app)
    for _ in range(9):
        stranger.cookies.clear()
        _login(stranger)
    stranger.cookies.clear()

    assert _login(stranger).status_code == 429
    assert _login(known).status_code == 200


def test_forged_device_cookie_is_limited_like_any_client(app, conn):
    _claim(conn)
    for _ in range(20):
        auth_core.record_failure(conn, time.time())
    client = _anonymous(app)
    client.cookies.set("activsync_device", "forged")

    assert _login(client).status_code == 429


def test_ten_device_failures_drop_trust(app, conn):
    _claim(conn)
    client = _anonymous(app)
    _login(client)
    device_id = client.cookies["activsync_device"]
    db.set_config_value(
        conn,
        "auth_trusted_devices",
        [{**entry, "failures": 9, "next_allowed_at": None} for entry in _devices(conn)],
    )

    assert _login(client, "nope").status_code == 401

    assert _devices(conn) == []
    assert client.cookies["activsync_device"] == device_id
    for _ in range(20):
        auth_core.record_failure(conn, time.time())
    assert _login(client).status_code == 429


def test_password_change_clears_trusted_devices(app, conn):
    _claim(conn)
    client = _anonymous(app)
    _login(client)
    other = _anonymous(app)
    _login(other)
    assert len(_devices(conn)) == 2

    changed = client.put(
        "/api/v1/auth/password",
        json={"current": OWNER_PASSWORD, "new": "a different long passphrase"},
    )

    assert changed.status_code == 200
    assert _devices(conn) == []


def test_signing_out_everywhere_clears_trusted_devices(app, conn):
    _claim(conn)
    client = _anonymous(app)
    _login(client)

    assert client.delete("/api/v1/auth/sessions").status_code == 200

    assert _devices(conn) == []


def test_concurrent_wrong_logins_are_all_counted(app, conn, monkeypatch):
    _claim(conn)
    attempts = 10
    verified = []
    lock = threading.Lock()

    def slow_wrong(*_args):
        with lock:
            verified.append(1)
        time.sleep(0.05)
        return False

    monkeypatch.setattr(auth_core, "verify_password", slow_wrong)
    barrier = threading.Barrier(attempts)
    statuses = []

    def guess():
        client = _anonymous(app)
        barrier.wait()
        status = _login(client, "nope").status_code
        with lock:
            statuses.append(status)

    threads = [threading.Thread(target=guess) for _ in range(attempts)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    failures = db.get_config_value(conn, "auth_login_throttle")["failures"]
    assert sorted(statuses) == [401] * 5 + [429] * 5
    assert failures == len(verified) == statuses.count(401)
