"""Paging behaviour of the Hevy backfill scan."""

from datetime import datetime, timezone

from activsync import hevy_backfill


class StubClient:
    """Pages of newest-first workouts, with controllable paging metadata."""

    def __init__(self, pages, count=None, count_exc=None):
        self.pages = pages
        self.count = count
        self.count_exc = count_exc
        self.requested_pages = []

    def get_workout_count(self):
        if self.count_exc is not None:
            raise self.count_exc
        return self.count

    def get_workouts_page(self, page, page_size=10):
        self.requested_pages.append(page)
        return self.pages.get(page, {"workouts": []})


def _workout(hevy_id, start):
    return {"id": hevy_id, "start_time": start, "exercises": []}


SINCE = datetime(2026, 1, 1, tzinfo=timezone.utc)


def test_scan_follows_the_page_count_the_api_reports():
    pages = {
        1: {"page_count": 2, "workouts": [_workout("a", "2026-03-01T10:00:00Z")]},
        2: {"page_count": 2, "workouts": [_workout("b", "2026-02-01T10:00:00Z")]},
    }
    client = StubClient(pages, count=20)

    found = hevy_backfill.workouts_since(client, SINCE)

    assert [w["id"] for w in found] == ["a", "b"]
    assert client.requested_pages == [1, 2]


def test_scan_stops_at_the_since_boundary_without_reading_older_pages():
    pages = {
        1: {"page_count": 9, "workouts": [_workout("a", "2026-03-01T10:00:00Z"),
                                          _workout("old", "2025-12-01T10:00:00Z")]},
    }
    client = StubClient(pages, count=90)

    found = hevy_backfill.workouts_since(client, SINCE)

    assert [w["id"] for w in found] == ["a"]
    assert client.requested_pages == [1]


def test_scan_uses_the_account_total_when_the_api_omits_page_count():
    """Without a total, a missing page_count silently truncated history at
    page 1 — the scan stopped long before the since boundary."""
    pages = {
        1: {"workouts": [_workout("a", "2026-03-01T10:00:00Z")]},
        2: {"workouts": [_workout("b", "2026-02-01T10:00:00Z")]},
        3: {"workouts": [_workout("c", "2026-01-15T10:00:00Z")]},
    }
    client = StubClient(pages, count=25)

    found = hevy_backfill.workouts_since(client, SINCE)

    assert [w["id"] for w in found] == ["a", "b", "c"]
    assert client.requested_pages == [1, 2, 3]


def test_scan_stops_at_page_one_when_neither_total_nor_page_count_is_known():
    pages = {1: {"workouts": [_workout("a", "2026-03-01T10:00:00Z")]},
             2: {"workouts": [_workout("b", "2026-02-01T10:00:00Z")]}}
    client = StubClient(pages, count=None)

    found = hevy_backfill.workouts_since(client, SINCE)

    assert [w["id"] for w in found] == ["a"]
    assert client.requested_pages == [1]


def test_scan_degrades_to_api_paging_when_the_count_call_fails():
    """An outage on /workouts/count must not abort the backfill."""
    pages = {
        1: {"page_count": 2, "workouts": [_workout("a", "2026-03-01T10:00:00Z")]},
        2: {"page_count": 2, "workouts": [_workout("b", "2026-02-01T10:00:00Z")]},
    }
    client = StubClient(pages, count_exc=RuntimeError("503"))

    found = hevy_backfill.workouts_since(client, SINCE)

    assert [w["id"] for w in found] == ["a", "b"]


def test_scan_accepts_a_total_supplied_by_the_caller_without_refetching():
    pages = {1: {"workouts": [_workout("a", "2026-03-01T10:00:00Z")]},
             2: {"workouts": [_workout("b", "2026-02-01T10:00:00Z")]}}
    client = StubClient(pages, count_exc=AssertionError("should not be called"))

    found = hevy_backfill.workouts_since(client, SINCE, total_count=15)

    assert [w["id"] for w in found] == ["a", "b"]
