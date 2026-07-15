from fastapi.testclient import TestClient

from activsync import db
from activsync.server import create_app


def _client(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    return conn, TestClient(create_app(conn))


def test_setup_route_serves_wizard_not_legacy_redirect(tmp_path):
    conn, client = _client(tmp_path)

    response = client.get("/setup")

    assert response.status_code == 200
    assert "Connect Garmin" in response.text


def test_legacy_login_and_logout_routes_redirect_without_authentication(tmp_path):
    _, client = _client(tmp_path)

    for method, path in [(client.get, "/login"), (client.post, "/login"), (client.post, "/logout")]:
        response = method(path, follow_redirects=False)
        assert response.status_code == 303
        assert response.headers["location"] == "/"
