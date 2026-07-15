from fastapi.testclient import TestClient

from activsync import db
from activsync.server import create_app


def _client(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    return TestClient(create_app(conn))


def test_health_returns_ok(tmp_path):
    client = _client(tmp_path)

    response = client.get("/health")

    assert response.status_code == 200
    assert response.json() == {"status": "ok"}
