"""Build a TestClient that is already signed in as the owner."""

import time

from fastapi.testclient import TestClient

from activsync import auth_core

OWNER_PASSWORD = "correct horse battery staple"
CSRF_HEADERS = {"X-ActivSync-Request": "1", "Origin": "http://testserver"}


def owner_client(app, **kwargs) -> TestClient:
    conn = app.state.db
    if not auth_core.owner_claimed(conn):
        auth_core.set_password(conn, OWNER_PASSWORD, time.time())
    token = auth_core.create_session(
        conn, method="password", ip="testclient", user_agent="pytest", now=time.time()
    )
    client = TestClient(app, **kwargs)
    scheme = client.base_url.scheme
    client.headers.update({
        "X-ActivSync-Request": "1",
        "Origin": f"{scheme}://{client.base_url.netloc.decode()}",
    })
    cookie = "__Host-activsync_session" if scheme == "https" else "activsync_session"
    client.cookies.set(cookie, token)
    return client
