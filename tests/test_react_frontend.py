from fastapi.testclient import TestClient

from activsync import db
from activsync.server import create_app


def _web_build(tmp_path):
    web = tmp_path / "web"
    assets = web / "assets"
    assets.mkdir(parents=True)
    (web / "index.html").write_text(
        "<!doctype html><title>React cutover</title><div id='app'></div>"
    )
    (assets / "app.js").write_text("window.activsyncReact = true;")
    return web


def test_react_build_serves_all_spa_entry_routes_and_assets(tmp_path):
    conn = db.connect(str(tmp_path / "app.db"))
    client = TestClient(create_app(conn, web_dir=_web_build(tmp_path)))

    for path in ("/", "/setup", "/settings"):
        response = client.get(path, follow_redirects=False)
        assert response.status_code == 200
        assert "React cutover" in response.text
        assert response.headers["cache-control"] == "no-cache"

    asset = client.get("/assets/app.js")
    assert asset.status_code == 200
    assert "activsyncReact" in asset.text
    assert client.get("/api/v1/app").status_code == 200


def test_spa_fallback_supports_future_client_routes_but_not_unknown_apis(tmp_path):
    conn = db.connect(str(tmp_path / "app.db"))
    client = TestClient(create_app(conn, web_dir=_web_build(tmp_path)))

    client_route = client.get("/future/deep-link", follow_redirects=False)
    unknown_api = client.get("/api/v1/not-a-route", follow_redirects=False)

    assert client_route.status_code == 200
    assert "React cutover" in client_route.text
    assert unknown_api.status_code == 404
    assert "React cutover" not in unknown_api.text
