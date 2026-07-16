import logging
import threading
import time
from datetime import datetime, timedelta, timezone
from types import SimpleNamespace
from unittest.mock import MagicMock

import pytest

from activsync import config, db
from activsync.poller import Poller
from activsync.strava_client import StravaRateLimitError


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def test_run_garmin_once_invokes_sync_garmin_and_returns_stats(conn, monkeypatch):
    fake_stats = object()
    called_with = {}

    def fake_sync_garmin(c, garmin, cfg, now):
        called_with["conn"] = c
        called_with["garmin"] = garmin
        called_with["cfg"] = cfg
        called_with["now"] = now
        return fake_stats

    monkeypatch.setattr("activsync.poller.sync.sync_garmin", fake_sync_garmin)
    garmin = MagicMock()
    poller = Poller(conn, garmin_factory=lambda: garmin, strava_factory=lambda: MagicMock())

    result = poller.run_garmin_once()

    assert result is fake_stats
    assert called_with["conn"] is conn
    assert called_with["garmin"] is garmin
    assert called_with["cfg"] == config.DEFAULT_CONFIG


def test_run_strava_once_invokes_publish_then_status_check(conn, monkeypatch):
    calls = []

    def fake_publish_pending(c, garmin, strava, now, garmin_activity_ids=None):
        calls.append("publish")
        return object()

    def fake_check_strava_status(c, strava, cfg, now):
        calls.append("status")
        return object()

    monkeypatch.setattr("activsync.poller.sync.publish_pending", fake_publish_pending)
    monkeypatch.setattr("activsync.poller.sync.check_strava_status", fake_check_strava_status)
    poller = Poller(conn, garmin_factory=lambda: MagicMock(), strava_factory=lambda: MagicMock())

    poller.run_strava_once()

    assert calls == ["publish", "status"]


def test_loop_runs_garmin_and_strava_independently_at_different_intervals(conn, monkeypatch):
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_tokens", {"refresh_token": "refresh"})
    garmin_calls = {"n": 0}
    strava_calls = {"n": 0}

    monkeypatch.setattr(
        "activsync.poller.sync.sync_garmin",
        lambda c, garmin, cfg, now: garmin_calls.update(n=garmin_calls["n"] + 1) or object(),
    )
    monkeypatch.setattr(
        "activsync.poller.sync.publish_pending",
        lambda c, garmin, strava, now, garmin_activity_ids=None: strava_calls.update(n=strava_calls["n"] + 1) or SimpleNamespace(published=0, failed=0),
    )
    monkeypatch.setattr(
        "activsync.poller.sync.check_strava_status",
        lambda c, strava, cfg, now: SimpleNamespace(flagged_missing=0, linked_existing=0),
    )

    poller = Poller(
        conn, garmin_factory=lambda: MagicMock(), strava_factory=lambda: MagicMock(),
        tick_seconds=0.05,
        garmin_interval_seconds_override=1000,
        strava_interval_seconds_override=0.05,
    )
    poller.start()
    time.sleep(0.3)
    poller.stop()

    assert garmin_calls["n"] == 1, "garmin should run once (long interval), not repeatedly"
    assert strava_calls["n"] >= 2, "strava should run repeatedly (short interval)"


def test_loop_checks_strava_immediately_after_garmin_changes(conn, monkeypatch):
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_tokens", {"refresh_token": "refresh"})
    status_calls = {"n": 0}

    monkeypatch.setattr(
        "activsync.poller.sync.sync_garmin",
        lambda *args: SimpleNamespace(new=1, updated=0, removed=0),
    )
    monkeypatch.setattr(
        "activsync.poller.sync.publish_pending",
        lambda *args, **kwargs: SimpleNamespace(published=0, failed=0),
    )
    monkeypatch.setattr(
        "activsync.poller.sync.check_strava_status",
        lambda *args: status_calls.update(n=status_calls["n"] + 1) or SimpleNamespace(
            flagged_missing=0, linked_existing=1,
        ),
    )

    poller = Poller(
        conn, garmin_factory=lambda: MagicMock(), strava_factory=lambda: MagicMock(),
        tick_seconds=0.05,
        garmin_interval_seconds_override=1000,
        strava_interval_seconds_override=1000,
    )
    poller.start()
    time.sleep(0.15)
    poller.stop()

    assert status_calls["n"] == 1


def test_stop_halts_the_loop(conn, monkeypatch):
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_tokens", {"refresh_token": "refresh"})
    call_count = {"n": 0}
    ready = threading.Event()

    def fake_sync_garmin(c, garmin, cfg, now):
        call_count["n"] += 1
        ready.set()
        return object()

    monkeypatch.setattr("activsync.poller.sync.sync_garmin", fake_sync_garmin)
    monkeypatch.setattr(
        "activsync.poller.sync.publish_pending",
        lambda *a, **k: SimpleNamespace(published=0, failed=0),
    )
    monkeypatch.setattr(
        "activsync.poller.sync.check_strava_status",
        lambda *a, **k: SimpleNamespace(flagged_missing=0, linked_existing=0),
    )

    poller = Poller(
        conn, garmin_factory=lambda: MagicMock(), strava_factory=lambda: MagicMock(),
        tick_seconds=0.05, garmin_interval_seconds_override=0.05, strava_interval_seconds_override=0.05,
    )
    poller.start()
    assert ready.wait(timeout=2), "poller did not run within timeout"
    poller.stop()

    count_after_stop = call_count["n"]
    time.sleep(0.15)
    assert call_count["n"] == count_after_stop, "poller kept running after stop()"


def test_poller_syncs_on_the_next_tick_after_a_reconnect(conn):
    db.set_config_value(conn, "initial_sync_done", True)
    garmin = MagicMock()
    garmin.fetch_recent_activities.return_value = []
    poller = Poller(
        conn, lambda: garmin, lambda: MagicMock(),
        garmin_interval_seconds_override=3600,   # an hour
    )

    # Tick once while disconnected: nothing runs, and no run is recorded.
    poller._loop_once(datetime(2026, 7, 14, 12, 0, tzinfo=timezone.utc))
    assert garmin.fetch_recent_activities.call_count == 0

    # Reconnect, then tick a minute later — it must not wait out the interval.
    db.set_config_value(conn, "garmin_credentials_verified", True)
    poller._loop_once(datetime(2026, 7, 14, 12, 1, tzinfo=timezone.utc))
    assert garmin.fetch_recent_activities.call_count == 1


def test_poller_does_nothing_before_the_initial_sync(conn):
    db.set_config_value(conn, "garmin_credentials_verified", True)
    garmin = MagicMock()
    poller = Poller(conn, lambda: garmin, lambda: MagicMock())

    poller._loop_once(datetime(2026, 7, 14, 12, 0, tzinfo=timezone.utc))

    assert garmin.fetch_recent_activities.call_count == 0


def _strava_ready(conn):
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_tokens", {"refresh_token": "refresh"})


def test_poller_stops_calling_strava_while_rate_limited(conn, monkeypatch):
    """A 429 means the quota is already gone; retrying every tick just keeps it
    gone. The poller must sit out the window Strava asked for."""
    _strava_ready(conn)
    calls = {"n": 0}

    def rate_limited(*args, **kwargs):
        calls["n"] += 1
        raise StravaRateLimitError("rate limited", retry_after_seconds=600)

    monkeypatch.setattr("activsync.poller.sync.sync_garmin", lambda *a, **k: SimpleNamespace(new=0, updated=0, removed=0))
    monkeypatch.setattr("activsync.poller.sync.publish_pending", rate_limited)
    monkeypatch.setattr("activsync.poller.sync.check_strava_status", lambda *a, **k: SimpleNamespace(flagged_missing=0, linked_existing=0))

    poller = Poller(
        conn, garmin_factory=lambda: MagicMock(), strava_factory=lambda: MagicMock(),
        garmin_interval_seconds_override=100000,
        strava_interval_seconds_override=0,
    )

    start = datetime(2026, 7, 14, 12, 0, tzinfo=timezone.utc)
    poller._loop_once(start)
    assert calls["n"] == 1

    # Ticks inside the backoff window must not touch Strava again.
    poller._loop_once(start + timedelta(minutes=1))
    poller._loop_once(start + timedelta(minutes=5))
    assert calls["n"] == 1, "poller kept hammering Strava while rate limited"


def test_poller_resumes_strava_after_the_backoff_expires(conn, monkeypatch):
    _strava_ready(conn)
    calls = {"n": 0}

    def rate_limited_once(*args, **kwargs):
        calls["n"] += 1
        if calls["n"] == 1:
            raise StravaRateLimitError("rate limited", retry_after_seconds=600)
        return SimpleNamespace(published=0, failed=0)

    monkeypatch.setattr("activsync.poller.sync.sync_garmin", lambda *a, **k: SimpleNamespace(new=0, updated=0, removed=0))
    monkeypatch.setattr("activsync.poller.sync.publish_pending", rate_limited_once)
    monkeypatch.setattr("activsync.poller.sync.check_strava_status", lambda *a, **k: SimpleNamespace(flagged_missing=0, linked_existing=0))

    poller = Poller(
        conn, garmin_factory=lambda: MagicMock(), strava_factory=lambda: MagicMock(),
        garmin_interval_seconds_override=100000,
        strava_interval_seconds_override=0,
    )

    start = datetime(2026, 7, 14, 12, 0, tzinfo=timezone.utc)
    poller._loop_once(start)
    poller._loop_once(start + timedelta(minutes=5))
    assert calls["n"] == 1

    poller._loop_once(start + timedelta(minutes=11))
    assert calls["n"] == 2, "poller never resumed after the backoff window passed"


def test_poller_rate_limit_does_not_block_garmin_sync(conn, monkeypatch):
    """Strava's quota is Strava's problem — Garmin polling must keep running."""
    _strava_ready(conn)
    garmin_calls = {"n": 0}

    monkeypatch.setattr(
        "activsync.poller.sync.sync_garmin",
        lambda *a, **k: garmin_calls.update(n=garmin_calls["n"] + 1) or SimpleNamespace(new=0, updated=0, removed=0),
    )
    monkeypatch.setattr(
        "activsync.poller.sync.publish_pending",
        MagicMock(side_effect=StravaRateLimitError("rate limited", retry_after_seconds=600)),
    )
    monkeypatch.setattr("activsync.poller.sync.check_strava_status", lambda *a, **k: SimpleNamespace(flagged_missing=0, linked_existing=0))

    poller = Poller(
        conn, garmin_factory=lambda: MagicMock(), strava_factory=lambda: MagicMock(),
        garmin_interval_seconds_override=0,
        strava_interval_seconds_override=0,
    )

    start = datetime(2026, 7, 14, 12, 0, tzinfo=timezone.utc)
    poller._loop_once(start)
    poller._loop_once(start + timedelta(minutes=1))

    assert garmin_calls["n"] == 2


def test_poller_logs_rate_limit_as_a_warning_without_a_traceback(conn, monkeypatch, caplog):
    """A hit quota is an expected condition, not a crash — it should not spam
    ERROR tracebacks into the log every tick."""
    _strava_ready(conn)

    monkeypatch.setattr("activsync.poller.sync.sync_garmin", lambda *a, **k: SimpleNamespace(new=0, updated=0, removed=0))
    monkeypatch.setattr(
        "activsync.poller.sync.publish_pending",
        MagicMock(side_effect=StravaRateLimitError("rate limited", retry_after_seconds=600)),
    )
    monkeypatch.setattr("activsync.poller.sync.check_strava_status", lambda *a, **k: SimpleNamespace(flagged_missing=0, linked_existing=0))

    poller = Poller(
        conn, garmin_factory=lambda: MagicMock(), strava_factory=lambda: MagicMock(),
        garmin_interval_seconds_override=100000,
        strava_interval_seconds_override=0,
    )

    with caplog.at_level(logging.DEBUG, logger="activsync.poller"):
        poller._loop_once(datetime(2026, 7, 14, 12, 0, tzinfo=timezone.utc))

    records = [r for r in caplog.records if r.name == "activsync.poller"]
    assert records, "rate limiting was not logged at all"
    assert all(r.levelno <= logging.WARNING for r in records)
    assert all(r.exc_info is None for r in records)
