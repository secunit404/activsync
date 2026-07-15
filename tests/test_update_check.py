import threading
from datetime import datetime, timezone

import pytest

from activsync import update_check
from activsync.update_check import UpdateChecker, UpdateStatus


@pytest.fixture(autouse=True)
def _clear_cache():
    update_check.reset_cache()
    yield
    update_check.reset_cache()


def test_parse_version_plain():
    assert update_check._parse_version("1.2.3") == (1, 2, 3)


def test_parse_version_strips_v_prefix():
    assert update_check._parse_version("v1.2.3") == (1, 2, 3)


def test_parse_version_malformed_is_empty():
    assert update_check._parse_version("not-a-version") == ()


def test_is_newer_true_when_latest_greater():
    assert update_check._is_newer("v1.1.0", "1.0.0") is True


def test_is_newer_false_when_equal():
    assert update_check._is_newer("1.0.0", "1.0.0") is False


def test_is_newer_false_when_older():
    assert update_check._is_newer("0.9.0", "1.0.0") is False


def test_is_newer_false_when_unparseable():
    assert update_check._is_newer("garbage", "1.0.0") is False


def test_get_status_default_has_no_latest_and_repo_urls():
    status = update_check.get_status()
    assert status.latest is None
    assert status.update_available is False
    assert status.repo_url == update_check.REPO_URL
    assert status.release_url == update_check.RELEASE_PAGE


def test_refresh_flags_update_when_newer():
    status = update_check.refresh(fetcher=lambda: "v2.0.0", current="1.0.0")
    assert status.latest == "v2.0.0"
    assert status.update_available is True
    assert update_check.get_status().update_available is True


def test_refresh_no_update_when_same():
    status = update_check.refresh(fetcher=lambda: "1.0.0", current="1.0.0")
    assert status.update_available is False


def test_refresh_is_silent_on_error():
    def boom():
        raise RuntimeError("network down")

    status = update_check.refresh(fetcher=boom, current="1.0.0")
    assert status.update_available is False
    assert status.latest is None  # no prior cache -> initial status


def test_refresh_keeps_last_known_status_on_later_error():
    update_check.refresh(fetcher=lambda: "v3.0.0", current="1.0.0")

    def boom():
        raise RuntimeError("network down")

    status = update_check.refresh(fetcher=boom, current="1.0.0")
    assert status.latest == "v3.0.0"
    assert status.update_available is True


def test_refresh_records_checked_at():
    now = datetime(2026, 7, 15, tzinfo=timezone.utc)
    status = update_check.refresh(fetcher=lambda: "1.0.0", current="1.0.0", now=now)
    assert status.checked_at == now


def test_update_checker_refreshes_on_start_then_stops():
    fetched = threading.Event()

    def fetcher():
        fetched.set()
        return "v5.0.0"

    checker = UpdateChecker(interval_seconds=3600, fetcher=fetcher)
    checker.start()
    try:
        assert fetched.wait(timeout=2.0), "checker did not refresh on start"
    finally:
        checker.stop()

    assert update_check.get_status().latest == "v5.0.0"
