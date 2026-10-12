import re

import pytest
from fastapi.testclient import TestClient
from starlette.routing import Mount

from activsync import auth_middleware, db
from activsync.server import create_app
from auth_helpers import CSRF_HEADERS, owner_client

EXPECTED_PUBLIC_ROUTES = {
    ("GET", "/health"),
    ("GET", "/favicon.ico"),
    ("GET", "/apple-touch-icon.png"),
    ("GET", "/api/v1/auth/status"),
    ("POST", "/api/v1/auth/login"),
    ("POST", "/api/v1/auth/claim"),
    ("GET", "/auth/oidc/login"),
    ("GET", "/auth/oidc/callback"),
    ("POST", "/api/v1/auth/strava/authorize"),
    ("GET", "/auth/strava/callback"),
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
    page = client.get("/strava/connect?from=settings")

    assert api.status_code == 401
    assert api.json() == {"detail": "Not signed in"}
    assert page.status_code == 303
    assert page.headers["location"] == "/login?next=%2Fstrava%2Fconnect%3Ffrom%3Dsettings"


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
    assert allowed_host.get("/").text == "Unknown host"
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
    ],
)
def test_host_is_allowed(host, allowed):
    allowed_hosts = ("testserver", ".example.lan")

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
