import asyncio
import json
import logging
import re
import time

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from starlette.routing import Mount

from activsync import auth_core, auth_middleware, db
from activsync.server import create_app
from auth_helpers import CSRF_HEADERS, OWNER_PASSWORD, owner_client

EXPECTED_PUBLIC_ROUTES = {
    ("GET", "/health"),
    ("GET", "/favicon.ico"),
    ("GET", "/apple-touch-icon.png"),
    ("GET", "/api/v1/auth/status"),
    ("POST", "/api/v1/auth/login"),
    ("POST", "/api/v1/auth/claim"),
    ("GET", "/auth/oidc/login"),
    ("GET", "/auth/oidc/callback"),
    ("GET", "/api/v1/auth/oidc/logo"),
}
SPA_ROUTE = "/{path:path}"
BOOT_SCRIPT = "window.boot = 1;"


def _web_build(tmp_path):
    web = tmp_path / "web"
    (web / "assets").mkdir(parents=True)
    (web / "index.html").write_text(
        f"<!doctype html><title>SPA</title><script>{BOOT_SCRIPT}</script>"
    )
    (web / "assets" / "app.js").write_text("window.app = true;")
    return web


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "auth.db"))


@pytest.fixture
def app(conn, tmp_path):
    return create_app(conn, web_dir=_web_build(tmp_path))


def _anonymous(app) -> TestClient:
    client = TestClient(app, follow_redirects=False)
    client.headers.update(CSRF_HEADERS)
    return client


def _flatten(routes, prefix=""):
    # FastAPI 0.139 keeps included routers as lazy wrappers in app.routes;
    # walk into them so a route added to any router is inventoried too.
    for route in routes:
        included = getattr(route, "original_router", None)
        if included is not None:
            yield from _flatten(included.routes, prefix + route.include_context.prefix)
        elif not isinstance(route, Mount):
            yield prefix, route


def test_allowlist_is_exactly_the_documented_public_surface():
    assert auth_middleware.PUBLIC_ROUTES == EXPECTED_PUBLIC_ROUTES
    assert auth_middleware.PUBLIC_PREFIXES == (("GET", "/assets/"),)
    assert auth_middleware.SPA_RESERVED_PREFIXES == ("api", "assets", "strava", "auth")


def test_every_route_outside_the_allowlist_requires_a_session(app):
    client = _anonymous(app)
    checked = 0
    for prefix, route in _flatten(app.routes):
        if route.path == SPA_ROUTE:
            continue
        for method in sorted(route.methods - {"HEAD"}):
            template = prefix + route.path
            if (method, template) in EXPECTED_PUBLIC_ROUTES:
                continue
            path = re.sub(r"\{[^}]+\}", "x", template)
            response = client.request(method, path)
            expected = 401 if path.startswith("/api/") else 303
            assert response.status_code == expected, (method, template)
            checked += 1
    assert checked > 40


def test_unauthenticated_api_gets_401_and_pages_redirect_to_login(app):
    client = _anonymous(app)

    api = client.get("/api/v1/app")
    page = client.get("/strava/callback?code=abc&state=xyz")

    assert api.status_code == 401
    assert api.json() == {"detail": "Not signed in"}
    assert page.status_code == 303
    assert page.headers["location"] == "/login?next=%2Fstrava%2Fcallback%3Fcode%3Dabc%26state%3Dxyz"


def test_spa_shell_and_static_files_stay_public(app):
    client = _anonymous(app)

    for path in ("/", "/login", "/setup", "/settings", "/assets/app.js", "/health"):
        assert client.get(path).status_code == 200, path
    assert client.get("/api/v1/not-a-route").status_code == 401
    assert client.get("/openapi.json").text.startswith("<!doctype html>")


def test_security_headers_allow_the_inline_bootstrap_by_hash(app):
    response = _anonymous(app).get("/")

    csp = response.headers["content-security-policy"]
    script_src = csp.split("script-src", 1)[1].split(";", 1)[0]
    expected_hash = auth_middleware.inline_script_hashes(f"<script>{BOOT_SCRIPT}</script>")[0]
    assert f"'sha256-{expected_hash}'" in script_src
    assert "'unsafe-inline'" not in script_src
    assert "frame-ancestors 'none'" in csp
    assert response.headers["referrer-policy"] == "same-origin"
    assert response.headers["x-content-type-options"] == "nosniff"
    assert "strict-transport-security" not in response.headers


def test_hsts_only_over_https(app):
    response = TestClient(app, base_url="https://testserver").get("/health")

    assert response.headers["strict-transport-security"] == "max-age=31536000"


def test_a_session_cookie_opens_the_api(app):
    assert owner_client(app).get("/api/v1/app").status_code == 200


def test_csrf_requires_the_custom_header(app):
    client = owner_client(app)
    del client.headers["X-ActivSync-Request"]

    assert client.delete("/api/v1/catch-up-report").status_code == 403


def test_csrf_rejects_cross_site_fetch_metadata(app):
    response = owner_client(app).delete(
        "/api/v1/catch-up-report", headers={"Sec-Fetch-Site": "cross-site"}
    )

    assert response.status_code == 403


def test_csrf_always_rejects_an_opaque_origin(app):
    response = owner_client(app).delete(
        "/api/v1/catch-up-report",
        headers={"Origin": "null", "Sec-Fetch-Site": "same-origin"},
    )

    assert response.status_code == 403


def test_csrf_without_fetch_metadata_needs_a_matching_origin_or_referer(app):
    client = owner_client(app)
    del client.headers["Origin"]

    wrong = client.delete("/api/v1/catch-up-report", headers={"Origin": "https://evil.test"})
    missing = client.delete("/api/v1/catch-up-report")
    by_origin = client.delete(
        "/api/v1/catch-up-report", headers={"Origin": "http://testserver"}
    )
    by_referer = client.delete(
        "/api/v1/catch-up-report", headers={"Referer": "http://testserver/settings"}
    )

    assert wrong.status_code == 403
    assert missing.status_code == 403
    assert by_origin.status_code == 204
    assert by_referer.status_code == 204


def test_csrf_runs_before_public_routes_too(app):
    client = TestClient(app)

    assert client.post("/api/v1/auth/login", json={"password": "x"}).status_code == 403


def test_public_url_rejects_other_hosts_but_not_the_healthcheck(monkeypatch, conn, tmp_path):
    monkeypatch.setenv("ACTIVSYNC_PUBLIC_URL", "https://activsync.example.com")
    app = create_app(conn, web_dir=_web_build(tmp_path))

    local = TestClient(app, base_url="http://localhost:8381")
    public = TestClient(app, base_url="https://activsync.example.com")

    assert local.get("/").status_code == 400
    assert local.get("/health").status_code == 200
    assert public.get("/").status_code == 200


def test_public_url_is_the_expected_csrf_origin(monkeypatch, conn, tmp_path):
    monkeypatch.setenv("ACTIVSYNC_PUBLIC_URL", "https://activsync.example.com")
    client = owner_client(
        create_app(conn, web_dir=_web_build(tmp_path)),
        base_url="https://activsync.example.com",
    )
    del client.headers["Origin"]

    accepted = client.delete(
        "/api/v1/catch-up-report", headers={"Origin": "https://activsync.example.com"}
    )
    refused = client.delete(
        "/api/v1/catch-up-report", headers={"Origin": "http://activsync.example.com"}
    )

    assert accepted.status_code == 204
    assert refused.status_code == 403


def test_disabled_auth_opens_every_route(monkeypatch, conn, tmp_path):
    monkeypatch.setenv("ACTIVSYNC_AUTH_DISABLED", "1")
    client = TestClient(create_app(conn, web_dir=_web_build(tmp_path)))
    client.headers.update(CSRF_HEADERS)

    assert client.get("/api/v1/app").status_code == 200
    assert client.delete("/api/v1/catch-up-report").status_code == 204


UNKNOWN_HOST_MESSAGE = (
    "Unknown host — set ACTIVSYNC_PUBLIC_URL or add it to ACTIVSYNC_ALLOWED_HOSTS"
)
PUBLIC_HOST_MESSAGE = "Unknown host — ActivSync answers only on the host in ACTIVSYNC_PUBLIC_URL"


def test_unknown_host_is_rejected_without_a_public_url(app):
    response = TestClient(app, base_url="http://attacker.example:8381").get("/")

    assert response.status_code == 400
    assert response.headers["content-type"].startswith("text/plain")
    assert response.text == UNKNOWN_HOST_MESSAGE


def test_unknown_host_is_rejected_with_auth_disabled(monkeypatch, conn, tmp_path):
    monkeypatch.setenv("ACTIVSYNC_AUTH_DISABLED", "1")
    client = TestClient(
        create_app(conn, web_dir=_web_build(tmp_path)),
        base_url="http://attacker.example:8381",
    )
    client.headers.update(CSRF_HEADERS)

    assert client.get("/api/v1/app").status_code == 400
    assert client.delete("/api/v1/catch-up-report").status_code == 400


def test_unknown_host_still_passes_the_healthcheck(app):
    client = TestClient(app, base_url="http://attacker.example:8381")

    assert client.get("/health").status_code == 200


@pytest.mark.parametrize(
    "host",
    ["192.168.1.20:8381", "[::1]:8381", "localhost:8381", "app.localhost", "LOCALHOST:8381"],
)
def test_local_hosts_are_accepted_without_a_public_url(app, host):
    assert TestClient(app).get("/", headers={"Host": host}).status_code == 200


def test_allowed_hosts_match_exactly_and_by_leading_dot(monkeypatch, conn, tmp_path):
    monkeypatch.setenv("ACTIVSYNC_ALLOWED_HOSTS", "activsync.home, .Example.LAN")
    app = create_app(conn, web_dir=_web_build(tmp_path))

    def status(base_url):
        return TestClient(app, base_url=base_url).get("/").status_code

    assert status("http://ActivSync.home:8381") == 200
    assert status("http://example.lan") == 200
    assert status("http://activsync.example.lan:8381") == 200
    assert status("http://other.home") == 400
    assert status("http://notexample.lan") == 400
    assert status("http://testserver") == 400


def test_public_url_mode_ignores_the_allowed_hosts(monkeypatch, conn, tmp_path):
    monkeypatch.setenv("ACTIVSYNC_PUBLIC_URL", "https://activsync.example.com")
    monkeypatch.setenv("ACTIVSYNC_ALLOWED_HOSTS", "attacker.example")
    app = create_app(conn, web_dir=_web_build(tmp_path))

    allowed_host = TestClient(app, base_url="http://attacker.example")
    public = TestClient(app, base_url="https://activsync.example.com")

    assert allowed_host.get("/").status_code == 400
    assert allowed_host.get("/").text == PUBLIC_HOST_MESSAGE
    assert public.get("/").status_code == 200


@pytest.mark.parametrize(
    ("host", "allowed"),
    [
        (None, False),
        ("", False),
        ("10.0.0.1", True),
        ("10.0.0.1:80", True),
        ("[::1]", True),
        ("[fe80::1]:8381", True),
        ("::1", False),
        ("[not-an-ip]", False),
        ("[::1]x", False),
        ("999.0.0.1", False),
        ("localhost", True),
        ("a.b.localhost:5173", True),
        ("evil-localhost", False),
        ("attacker.example", False),
        ("testserver", True),
        ("app.example.lan", True),
        ("example.lan", True),
        ("badexample.lan", False),
        ("TESTSERVER", True),
        ("testserver.", True),
        ("TestServer.:8381", True),
        ("app.example.lan.", True),
        ("localhost.:8381", True),
        ("nas.local", True),
        ("NAS.LOCAL.:80", True),
        ("nas.local.attacker.example", False),
        (".", False),
        (":8381", False),
    ],
)
def test_host_is_allowed(host, allowed):
    allowed_hosts = ("testserver", ".example.lan", "Nas.Local.")

    assert auth_middleware.host_is_allowed(host, allowed_hosts) is allowed


@pytest.mark.parametrize(
    "value",
    [
        None,
        "",
        "hevy",
        "//evil.test",
        "https://evil.test",
        "/\\evil.test",
        "/\t/evil.test",
        "/\r/evil.test",
        "/\n/evil.test",
        "/.//evil.test",
        "/a/..//evil.test",
        "/..//evil.test",
        "/%2e//evil.test",
        "/a/%2E%2e//evil.test",
    ],
)
def test_safe_next_refuses_anything_that_could_leave_the_site(value):
    assert auth_middleware.safe_next(value) == "/"


@pytest.mark.parametrize("value", ["/", "/hevy", "/settings?tab=access", "/a/../b", "/./hevy"])
def test_safe_next_keeps_same_site_paths(value):
    assert auth_middleware.safe_next(value) == value


@pytest.mark.parametrize(
    ("root_path", "path", "expected"),
    [
        ("", "/api/v1/app", "/api/v1/app"),
        ("/activsync", "/activsync/api/v1/app", "/api/v1/app"),
        ("/activsync", "/activsync", ""),
        ("/activsync", "/activsyncx/api/v1/app", "/activsyncx/api/v1/app"),
        ("/activsync", "/api/v1/app", "/api/v1/app"),
    ],
)
def test_route_path_strips_the_root_path_the_way_starlette_does(root_path, path, expected):
    scope = {"path": path, "root_path": root_path}

    assert auth_middleware.route_path(scope) == expected


def _call(app, method, path, *, root_path="", headers=None, body=b""):
    request_headers = {"host": "localhost:8381", **(headers or {})}
    scope = {
        "type": "http",
        "asgi": {"version": "3.0"},
        "http_version": "1.1",
        "method": method,
        "scheme": "http",
        "path": path,
        "raw_path": path.encode(),
        "root_path": root_path,
        "query_string": b"",
        "headers": [(k.encode(), v.encode()) for k, v in request_headers.items()],
        "client": ("127.0.0.1", 50000),
        "server": ("localhost", 8381),
    }
    sent = []
    # A real client sends its body once and then disconnects; a receive that
    # never disconnects makes Starlette's FileResponse wait on it forever.
    messages = iter([{"type": "http.request", "body": body, "more_body": False}])

    async def receive():
        return next(messages, {"type": "http.disconnect"})

    async def send(message):
        sent.append(message)

    asyncio.run(app(scope, receive, send))
    start = next(m for m in sent if m["type"] == "http.response.start")
    payload = b"".join(m.get("body", b"") for m in sent if m["type"] == "http.response.body")
    return start["status"], start["headers"], payload


ROOT = "/activsync"


def test_a_root_path_does_not_open_the_api_without_a_session(app):
    for path in ("/api/v1/app", "/api/v1/settings"):
        status, _, _ = _call(app, "GET", ROOT + path, root_path=ROOT)
        assert status == 401, path


def test_a_root_path_still_redirects_protected_pages_to_the_login(app):
    status, headers, _ = _call(app, "GET", ROOT + "/strava/callback", root_path=ROOT)

    assert status == 303
    assert dict(headers)[b"location"].startswith(b"/login?next=")


def test_a_root_path_keeps_the_public_surface_public(app):
    for path in ("/health", "/api/v1/auth/status", "/", "/login", "/assets/app.js"):
        status, _, _ = _call(app, "GET", ROOT + path, root_path=ROOT)
        assert status == 200, path


def test_a_root_path_still_exempts_the_healthcheck_from_the_host_guard(app):
    status, _, _ = _call(
        app, "GET", ROOT + "/health", root_path=ROOT, headers={"host": "attacker.example"}
    )
    blocked, _, _ = _call(
        app, "GET", ROOT + "/api/v1/app", root_path=ROOT, headers={"host": "attacker.example"}
    )

    assert status == 200
    assert blocked == 400


def test_login_and_the_session_work_under_a_root_path(app, conn):
    auth_core.set_password(conn, OWNER_PASSWORD, time.time())
    csrf = {"x-activsync-request": "1", "origin": "http://localhost:8381"}

    status, headers, _ = _call(
        app,
        "POST",
        ROOT + "/api/v1/auth/login",
        root_path=ROOT,
        headers={**csrf, "content-type": "application/json"},
        body=json.dumps({"password": OWNER_PASSWORD}).encode(),
    )
    cookie = next(
        value.decode().split(";", 1)[0]
        for name, value in headers
        if name == b"set-cookie" and value.startswith(b"activsync_session=")
    )
    signed_in, _, _ = _call(
        app, "GET", ROOT + "/api/v1/app", root_path=ROOT, headers={"cookie": cookie}
    )

    assert status == 200
    assert signed_in == 200


PUBLIC_HTTPS = "https://activsync.example.com"


def _login(client, origin):
    return client.post(
        "/api/v1/auth/login",
        json={"password": OWNER_PASSWORD},
        headers={"X-ActivSync-Request": "1", "Origin": origin},
    )


def _set_cookie(response, name):
    return next(
        value
        for value in response.headers.get_list("set-cookie")
        if value.startswith(f"{name}=")
    )


def test_an_https_public_url_makes_cookies_secure_without_forwarded_headers(
    monkeypatch, conn, tmp_path
):
    monkeypatch.setenv("ACTIVSYNC_PUBLIC_URL", PUBLIC_HTTPS)
    auth_core.set_password(conn, OWNER_PASSWORD, time.time())
    app = create_app(conn, web_dir=_web_build(tmp_path))
    plain_http = "http://activsync.example.com"

    response = _login(TestClient(app, base_url=plain_http), PUBLIC_HTTPS)

    assert response.status_code == 200
    session = _set_cookie(response, "__Host-activsync_session")
    assert "Secure" in session
    assert "Secure" in _set_cookie(response, "__Host-activsync_device")
    assert not any(
        value.startswith(("activsync_session=", "activsync_device="))
        for value in response.headers.get_list("set-cookie")
    )
    assert response.headers["strict-transport-security"] == "max-age=31536000"

    token = session.split("=", 1)[1].split(";", 1)[0]
    reader = TestClient(app, base_url=plain_http)
    assert reader.get(
        "/api/v1/app", headers={"Cookie": f"__Host-activsync_session={token}"}
    ).status_code == 200
    assert reader.get(
        "/api/v1/app", headers={"Cookie": f"activsync_session={token}"}
    ).status_code == 401


def test_an_https_public_url_puts_hsts_on_every_response(monkeypatch, conn, tmp_path):
    monkeypatch.setenv("ACTIVSYNC_PUBLIC_URL", PUBLIC_HTTPS)
    app = create_app(conn, web_dir=_web_build(tmp_path))

    for path in ("/", "/health", "/api/v1/app"):
        response = TestClient(app, base_url="http://activsync.example.com").get(path)
        assert response.headers["strict-transport-security"] == "max-age=31536000", path


def test_an_http_public_url_keeps_plain_cookies_and_no_hsts(monkeypatch, conn, tmp_path):
    monkeypatch.setenv("ACTIVSYNC_PUBLIC_URL", "http://activsync.example.com")
    auth_core.set_password(conn, OWNER_PASSWORD, time.time())
    app = create_app(conn, web_dir=_web_build(tmp_path))

    response = _login(
        TestClient(app, base_url="http://activsync.example.com"), "http://activsync.example.com"
    )

    assert response.status_code == 200
    assert "Secure" not in _set_cookie(response, "activsync_session")
    assert "strict-transport-security" not in response.headers


def test_without_a_public_url_the_cookie_follows_the_request_scheme(conn, tmp_path):
    auth_core.set_password(conn, OWNER_PASSWORD, time.time())
    app = create_app(conn, web_dir=_web_build(tmp_path))

    over_http = _login(TestClient(app), "http://testserver")
    over_https = _login(TestClient(app, base_url="https://testserver"), "https://testserver")

    assert "Secure" not in _set_cookie(over_http, "activsync_session")
    assert "strict-transport-security" not in over_http.headers
    assert "Secure" in _set_cookie(over_https, "__Host-activsync_session")
    assert "strict-transport-security" in over_https.headers


@pytest.mark.parametrize(
    ("value", "scheme", "expected"),
    [
        ("Activsync.Example.com", "https", "activsync.example.com"),
        ("activsync.example.com.", "https", "activsync.example.com"),
        ("activsync.example.com:443", "https", "activsync.example.com"),
        ("activsync.example.com:80", "http", "activsync.example.com"),
        ("activsync.example.com:80", "https", "activsync.example.com:80"),
        ("activsync.example.com:443", "http", "activsync.example.com:443"),
        ("activsync.example.com.:8443", "https", "activsync.example.com:8443"),
        ("activsync.example.com:", "https", "activsync.example.com"),
        ("activsync.example.com:80", None, "activsync.example.com"),
        ("activsync.example.com:443", None, "activsync.example.com"),
        ("activsync.example.com:8381", None, "activsync.example.com:8381"),
        ("[::1]:443", "https", "[::1]"),
        ("[::1]:8381", "https", "[::1]:8381"),
        ("[::1]x", None, "[::1]x"),
        ("::1", None, "::1"),
    ],
)
def test_normalize_host(value, scheme, expected):
    assert auth_middleware.normalize_host(value, scheme) == expected


@pytest.mark.parametrize(
    ("public_url", "host", "status"),
    [
        ("https://activsync.example.com", "activsync.example.com", 200),
        ("https://ActivSync.example.com", "activsync.example.com", 200),
        ("https://activsync.example.com:443", "activsync.example.com", 200),
        ("https://activsync.example.com.", "activsync.example.com", 200),
        ("https://activsync.example.com", "activsync.example.com:443", 200),
        ("https://activsync.example.com", "ACTIVSYNC.Example.com.", 200),
        ("http://activsync.example.com:80", "activsync.example.com", 200),
        ("http://activsync.example.com", "activsync.example.com:80", 200),
        ("https://activsync.example.com:8443", "activsync.example.com:8443", 200),
        ("https://activsync.example.com", "activsync.example.com:80", 400),
        ("http://activsync.example.com", "activsync.example.com:443", 400),
        ("https://activsync.example.com:8443", "activsync.example.com", 400),
        ("https://activsync.example.com", "activsync.example.com:8381", 400),
        ("https://activsync.example.com", "other.example.com", 400),
        ("https://activsync.example.com", "activsync.example.com.attacker.example", 400),
    ],
)
def test_public_url_host_matching_is_normalized(
    monkeypatch, conn, tmp_path, public_url, host, status
):
    monkeypatch.setenv("ACTIVSYNC_PUBLIC_URL", public_url)
    app = create_app(conn, web_dir=_web_build(tmp_path))

    response = TestClient(app).get("/", headers={"Host": host})

    assert response.status_code == status
    if status == 400:
        assert response.text == PUBLIC_HOST_MESSAGE


def test_a_normalized_public_url_is_also_the_csrf_origin(monkeypatch, conn, tmp_path):
    monkeypatch.setenv("ACTIVSYNC_PUBLIC_URL", "https://ActivSync.example.com:443")
    client = owner_client(
        create_app(conn, web_dir=_web_build(tmp_path)), base_url=PUBLIC_HTTPS
    )
    del client.headers["Origin"]

    accepted = client.delete("/api/v1/catch-up-report", headers={"Origin": PUBLIC_HTTPS})
    spelled_out = client.delete(
        "/api/v1/catch-up-report", headers={"Origin": PUBLIC_HTTPS + ":443"}
    )
    refused = client.delete(
        "/api/v1/catch-up-report", headers={"Origin": "http://activsync.example.com"}
    )

    assert accepted.status_code == 204
    assert spelled_out.status_code == 204
    assert refused.status_code == 403


def test_a_malformed_origin_or_referer_is_blocked_not_a_server_error(app):
    client = owner_client(app)

    origin = client.delete("/api/v1/catch-up-report", headers={"Origin": "http://[bad"})
    del client.headers["Origin"]
    referer = client.delete("/api/v1/catch-up-report", headers={"Referer": "http://[bad"})

    assert origin.status_code == 403
    assert referer.status_code == 403


def test_allowed_hosts_entries_are_normalized(monkeypatch, conn, tmp_path):
    monkeypatch.setenv("ACTIVSYNC_ALLOWED_HOSTS", "NAS.Local., tailnet.example.ts.net:80")
    app = create_app(conn, web_dir=_web_build(tmp_path))

    def status(host):
        return TestClient(app).get("/", headers={"Host": host}).status_code

    assert status("nas.local:8381") == 200
    assert status("nas.local.") == 200
    assert status("tailnet.example.ts.net") == 200
    assert status("other.local") == 400


def _guarded(conn, clock, public_url=None):
    return TestClient(
        auth_middleware.AuthenticateMiddleware(
            FastAPI(), conn=conn, public_url=public_url, allowed_hosts=(), clock=clock
        ),
        base_url="http://nas.local:8381",
    )


def test_a_rejected_host_is_logged_once_a_minute_with_both_settings(conn, caplog):
    now = [1_000_000.0]
    client = _guarded(conn, lambda: now[0])

    with caplog.at_level(logging.WARNING, logger="activsync.auth"):
        first = client.get("/")
        now[0] += 30
        client.get("/")
        client.get("/health")
        now[0] += 31
        client.get("/")

    assert first.status_code == 400
    assert first.text == UNKNOWN_HOST_MESSAGE
    assert caplog.text.count("rejected a request for Host") == 2
    assert "'nas.local:8381'" in caplog.text
    assert "ACTIVSYNC_PUBLIC_URL" in caplog.text
    assert "ACTIVSYNC_ALLOWED_HOSTS" in caplog.text


def test_a_host_rejected_under_a_public_url_is_logged_and_carries_the_hint(conn, caplog):
    client = _guarded(conn, lambda: 1_000_000.0, public_url=PUBLIC_HTTPS)

    with caplog.at_level(logging.WARNING, logger="activsync.auth"):
        response = client.get("/")

    assert response.status_code == 400
    assert response.text == PUBLIC_HOST_MESSAGE
    assert "nas.local:8381" in caplog.text
    assert "ACTIVSYNC_PUBLIC_URL allows only" in caplog.text
    assert "ACTIVSYNC_ALLOWED_HOSTS" not in caplog.text
