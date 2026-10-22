import asyncio
import json
import time

import pytest
from fastapi.testclient import TestClient

from activsync import auth_body_cap, auth_core, auth_oidc, db
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


OVERSIZED = auth_body_cap.PUBLIC_BODY_LIMIT + 1


def _padded_login(password="nope", size=OVERSIZED):
    return json.dumps({"password": password, "padding": "x" * size}).encode()


def _throttle(conn):
    return db.get_config_value(conn, auth_core.THROTTLE_KEY)


def test_an_oversized_login_is_refused_before_it_reaches_the_route(app, conn):
    auth_core.set_password(conn, OWNER_PASSWORD, time.time())

    response = _anonymous(app).post(
        "/api/v1/auth/login",
        content=_padded_login(),
        headers={"Content-Type": "application/json"},
    )

    assert response.status_code == 413
    assert response.json() == {"detail": auth_body_cap.TOO_LARGE}
    assert _throttle(conn) is None
    assert auth_core.list_sessions(conn) == []


def test_an_oversized_chunked_login_is_refused_before_it_reaches_the_route(app, conn):
    auth_core.set_password(conn, OWNER_PASSWORD, time.time())

    def chunks():
        yield _padded_login()

    response = _anonymous(app).post(
        "/api/v1/auth/login",
        content=chunks(),
        headers={"Content-Type": "application/json"},
    )

    assert response.status_code == 413
    assert response.json() == {"detail": auth_body_cap.TOO_LARGE}
    assert _throttle(conn) is None


def test_an_oversized_claim_leaves_the_install_unclaimed(app, conn):
    code = app.state.setup_code.current
    body = json.dumps(
        {"setupCode": code, "password": OWNER_PASSWORD, "padding": "x" * OVERSIZED}
    ).encode()

    response = _anonymous(app).post(
        "/api/v1/auth/claim", content=body, headers={"Content-Type": "application/json"}
    )

    assert response.status_code == 413
    assert auth_core.owner_claimed(conn) is False
    assert app.state.setup_code.current == code


def test_a_body_at_the_cap_still_reaches_the_route(app, conn):
    auth_core.set_password(conn, OWNER_PASSWORD, time.time())
    empty = len(_padded_login(OWNER_PASSWORD, size=0))
    body = _padded_login(OWNER_PASSWORD, size=auth_body_cap.PUBLIC_BODY_LIMIT - empty)
    assert len(body) == auth_body_cap.PUBLIC_BODY_LIMIT

    response = _anonymous(app).post(
        "/api/v1/auth/login", content=body, headers={"Content-Type": "application/json"}
    )

    assert response.status_code == 200


def test_a_small_chunked_login_is_replayed_to_the_route(app, conn):
    auth_core.set_password(conn, OWNER_PASSWORD, time.time())

    def chunks():
        yield b'{"password": '
        yield json.dumps(OWNER_PASSWORD).encode()
        yield b"}"

    response = _anonymous(app).post(
        "/api/v1/auth/login",
        content=chunks(),
        headers={"Content-Type": "application/json"},
    )

    assert response.status_code == 200
    assert response.json()["authenticated"] is True


def _stream(app, method, path, chunks, *, headers=None):
    """Drive the app with a chunked body, returning the status, how many body
    messages the app side consumed, and whether the route was reached."""
    request_headers = {
        "host": "localhost:8381",
        "x-activsync-request": "1",
        "origin": "http://localhost:8381",
        **(headers or {}),
    }
    scope = {
        "type": "http",
        "asgi": {"version": "3.0"},
        "http_version": "1.1",
        "method": method,
        "scheme": "http",
        "path": path,
        "raw_path": path.encode(),
        "root_path": "",
        "query_string": b"",
        "headers": [(k.encode(), v.encode()) for k, v in request_headers.items()],
        "client": ("127.0.0.1", 50000),
        "server": ("localhost", 8381),
    }
    pending = list(chunks)
    pulled = []
    sent = []

    async def receive():
        if not pending:
            return {"type": "http.disconnect"}
        pulled.append(1)
        body = pending.pop(0)
        return {"type": "http.request", "body": body, "more_body": bool(pending)}

    async def send(message):
        sent.append(message)

    asyncio.run(app(scope, receive, send))
    start = next(m for m in sent if m["type"] == "http.response.start")
    return start["status"], len(pulled), len(pending)


def test_a_chunked_body_is_read_only_as_far_as_the_cap(app, conn):
    auth_core.set_password(conn, OWNER_PASSWORD, time.time())
    chunk = b"x" * 4096

    status, pulled, unread = _stream(
        app, "POST", "/api/v1/auth/login", [chunk] * 50,
        headers={"content-type": "application/json"},
    )

    assert status == 413
    assert pulled == auth_body_cap.PUBLIC_BODY_LIMIT // len(chunk) + 1
    assert unread == 50 - pulled
    assert _throttle(conn) is None


def test_a_signed_in_request_is_not_capped(app, conn):
    client = owner_client(app)
    auth_oidc.save_config(
        conn,
        display_name="Pocket ID",
        issuer="https://id.example.com",
        client_id="activsync",
        client_secret="very-secret-value",
        scopes="openid",
        auto_redirect=False,
    )

    response = client.put(
        "/api/v1/auth/oidc/logo",
        content=b"x" * (4 * auth_body_cap.PUBLIC_BODY_LIMIT),
        headers={"Content-Type": "image/png"},
    )

    assert response.status_code == 400
