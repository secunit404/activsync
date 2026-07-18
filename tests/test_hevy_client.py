"""Tests for the Hevy API client."""

import pytest

from activsync import hevy_client
from activsync.hevy_client import HevyAuthError, HevyClient


def test_requires_api_key():
    with pytest.raises(ValueError):
        HevyClient(api_key="")


def test_iter_events_since_paginates(monkeypatch):
    client = HevyClient(api_key="k")
    pages = {
        1: {"page": 1, "page_count": 2, "events": [
            {"type": "updated", "workout": {"id": "b", "updated_at": "2026-07-18T10:00:00Z"}}]},
        2: {"page": 2, "page_count": 2, "events": [
            {"type": "deleted", "id": "a", "deleted_at": "2026-07-18T09:00:00Z"}]},
    }
    monkeypatch.setattr(client, "_get", lambda path, params=None: pages[params["page"]])
    events = client.iter_events_since("2026-07-01T00:00:00Z")
    assert len(events) == 2 and events[1]["type"] == "deleted"


def test_iter_events_since_skips_malformed(monkeypatch):
    client = HevyClient(api_key="k")
    page = {"page": 1, "page_count": 1, "events": [
        {"type": "updated", "workout": {"id": "ok", "updated_at": "2026-07-18T10:00:00Z"}},
        {"type": "updated", "workout": {"updated_at": "2026-07-18T10:00:00Z"}},  # no id
        {"type": "updated"},                                # no workout at all
        {"type": "deleted", "deleted_at": "2026-07-18T09:00:00Z"},  # no id
        {"type": "renamed", "id": "x"},                     # unknown type
        "not-a-dict",
    ]}
    monkeypatch.setattr(client, "_get", lambda path, params=None: page)
    events = client.iter_events_since("2026-07-01T00:00:00Z")
    assert [e["workout"]["id"] for e in events] == ["ok"]


def test_iter_events_since_empty(monkeypatch):
    client = HevyClient(api_key="k")
    monkeypatch.setattr(
        client, "_get",
        lambda path, params=None: {"page": 1, "page_count": 1, "events": []})
    assert client.iter_events_since("2026-07-01T00:00:00Z") == []


def test_auth_error_on_401(monkeypatch):
    client = HevyClient(api_key="bad")

    class StubResponse:
        status_code = 401
        headers = {}

        def raise_for_status(self):
            raise AssertionError("should not be reached")

        def json(self):
            return {}

    monkeypatch.setattr(client.session, "get",
                        lambda url, params=None, timeout=None: StubResponse())
    with pytest.raises(HevyAuthError):
        client.get_user_info()


def test_get_workout_unwraps_and_handles_missing(monkeypatch):
    client = HevyClient(api_key="k")
    monkeypatch.setattr(client, "_get",
                        lambda path, params=None: {"workout": {"id": "w1"}})
    assert client.get_workout("w1")["id"] == "w1"

    def boom(path, params=None):
        raise RuntimeError("404")
    monkeypatch.setattr(client, "_get", boom)
    assert client.get_workout("gone") is None


def test_iter_all_exercise_templates_paginates(monkeypatch):
    client = HevyClient(api_key="k")
    pages = {
        1: {"page": 1, "page_count": 2,
            "exercise_templates": [{"id": "t1", "title": "Bench"}]},
        2: {"page": 2, "page_count": 2,
            "exercise_templates": [{"id": "t2", "title": "Squat"}]},
    }
    monkeypatch.setattr(client, "_get", lambda path, params=None: pages[params["page"]])
    templates = client.iter_all_exercise_templates()
    assert [t["id"] for t in templates] == ["t1", "t2"]


def test_no_sleep_in_module():
    import inspect
    source = inspect.getsource(hevy_client)
    assert "time.sleep" not in source


def test_retry_covers_429():
    client = HevyClient(api_key="k")
    adapter = client.session.get_adapter("https://api.hevyapp.com/v1/workouts")
    assert 429 in adapter.max_retries.status_forcelist
