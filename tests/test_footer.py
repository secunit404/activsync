from datetime import datetime, timezone

from fastapi.testclient import TestClient

from activsync import db, update_check
from activsync.update_check import UpdateStatus
from activsync.server import create_app


def _client(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))
    return conn, client


def test_footer_shows_version_and_github_link(tmp_path):
    update_check.reset_cache()
    _conn, client = _client(tmp_path)
    body = client.get("/").text
    assert f"ActivSync v{update_check._CURRENT_VERSION}" in body
    assert 'href="https://github.com/secunit404/activsync"' in body


def test_footer_hides_update_icon_when_up_to_date(tmp_path):
    update_check.reset_cache()
    _conn, client = _client(tmp_path)
    body = client.get("/").text
    assert "Update available" not in body


def test_footer_shows_update_icon_when_newer_release(tmp_path):
    update_check._cache = UpdateStatus(
        current="1.0.0",
        latest="9.9.9",
        update_available=True,
        checked_at=datetime(2026, 7, 15, tzinfo=timezone.utc),
        repo_url=update_check.REPO_URL,
        release_url=update_check.RELEASE_PAGE,
    )
    try:
        _conn, client = _client(tmp_path)
        body = client.get("/").text
        assert "Update available: v9.9.9" in body
        assert 'href="https://github.com/secunit404/activsync/releases/latest"' in body
    finally:
        update_check.reset_cache()
