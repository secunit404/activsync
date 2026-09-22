"""The poller's third leg: Hevy sync on its own interval with Garmin handoff."""

from datetime import datetime, timedelta, timezone
from unittest.mock import MagicMock

import pytest

from activsync import config, db
from activsync.fit_builder import Profile
from activsync.hevy_client import HevyAuthError
from activsync.poller import Poller

START = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def _hevy_ready(conn):
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "hevy_api_key", "key")
    db.set_config_value(conn, "hevy_auth_ok", True)
    cfg = config.load_config(conn)
    cfg["hevy_enabled"] = True
    config.save_config(conn, cfg)


def _make_poller(
    conn,
    monkeypatch,
    leg=None,
    garmin_interval=100000,
    garmin_polling_enabled=True,
):
    """Poller with stubbed sync legs; returns (poller, calls dict)."""
    calls = {"hevy": 0, "garmin": 0}

    def fake_leg(c, garmin, hevy, cfg, now, strava=None):
        calls["hevy"] += 1
        return leg(c, garmin, hevy, cfg, now) if leg else False

    monkeypatch.setattr("activsync.poller.hevy_sync.run_hevy_leg", fake_leg)
    monkeypatch.setattr(
        "activsync.poller.hevy_profile.get_profile",
        lambda c, garmin, now: (
            calls.__setitem__("profile", True)
            or Profile(weight_kg=80.0, birth_year=1990, vo2max=45.0, sex="male")
        ),
    )
    monkeypatch.setattr(
        "activsync.poller.sync.sync_garmin",
        lambda c, garmin, cfg, now: calls.update(garmin=calls["garmin"] + 1)
        or MagicMock(new=0, updated=0, removed=0),
    )
    poller = Poller(
        conn,
        garmin_factory=lambda: MagicMock(),
        strava_factory=lambda: MagicMock(),
        hevy_factory=lambda: MagicMock(),
        garmin_interval_seconds_override=garmin_interval,
        hevy_interval_seconds_override=0,
        garmin_polling_enabled=garmin_polling_enabled,
    )
    return poller, calls


def test_hevy_leg_runs_when_due_and_writes_success_timestamp(conn, monkeypatch):
    _hevy_ready(conn)
    poller, calls = _make_poller(conn, monkeypatch)

    poller._loop_once(START)

    assert calls["hevy"] == 1
    assert db.get_config_value(conn, "hevy_last_success_at") == START.isoformat()


def test_hevy_leg_refreshes_the_profile_cache(conn, monkeypatch):
    _hevy_ready(conn)
    poller, calls = _make_poller(conn, monkeypatch)

    poller._loop_once(START)

    assert calls.get("profile") is True


def test_hevy_change_triggers_garmin_leg_immediately(conn, monkeypatch):
    _hevy_ready(conn)
    poller, calls = _make_poller(conn, monkeypatch,
                                 leg=lambda *a, **k: True)
    # Garmin ran recently: without the handoff it would not be due this tick.
    poller._last_garmin_run = START - timedelta(seconds=30)

    poller._loop_once(START)

    assert calls["hevy"] == 1
    assert calls["garmin"] == 1, "hevy change must hand off to the garmin leg"


def test_hevy_only_poller_does_not_run_the_garmin_leg(conn, monkeypatch):
    _hevy_ready(conn)
    poller, calls = _make_poller(
        conn,
        monkeypatch,
        leg=lambda *a, **k: True,
        garmin_polling_enabled=False,
    )

    poller._loop_once(START)

    assert calls["hevy"] == 1
    assert calls["garmin"] == 0


def test_explicit_match_choice_refreshes_local_garmin_state(conn, monkeypatch):
    poller, calls = _make_poller(conn, monkeypatch)
    choices = []
    monkeypatch.setattr(
        "activsync.poller.hevy_sync.apply_match_choice",
        lambda _conn, _garmin, _hevy, hevy_id, strategy, _cfg, _now: (
            choices.append((hevy_id, strategy)) or {"status": "merged"}
        ),
    )

    status = poller.apply_hevy_match("hevy-1", "merge")

    assert status == "merged"
    assert choices == [("hevy-1", "merge")]
    assert calls["garmin"] == 1


def test_hevy_no_change_does_not_force_the_garmin_leg(conn, monkeypatch):
    _hevy_ready(conn)
    poller, calls = _make_poller(conn, monkeypatch)
    poller._last_garmin_run = START - timedelta(seconds=30)

    poller._loop_once(START)

    assert calls["garmin"] == 0


def test_hevy_auth_error_pauses_the_leg(conn, monkeypatch):
    _hevy_ready(conn)

    def raise_auth(*a, **k):
        raise HevyAuthError("bad key")

    poller, calls = _make_poller(conn, monkeypatch, leg=raise_auth)

    poller._loop_once(START)
    assert calls["hevy"] == 1
    assert db.get_config_value(conn, "hevy_auth_ok") is False
    assert db.get_config_value(conn, "hevy_last_success_at") is None

    poller._loop_once(START + timedelta(minutes=30))
    assert calls["hevy"] == 1, "paused leg must not run again until reconnected"


def test_hevy_generic_failure_does_not_write_success_timestamp(conn, monkeypatch):
    _hevy_ready(conn)

    def boom(*a, **k):
        raise RuntimeError("hevy down")

    poller, calls = _make_poller(conn, monkeypatch, leg=boom)

    poller._loop_once(START)

    assert db.get_config_value(conn, "hevy_last_success_at") is None
    assert db.get_config_value(conn, "hevy_auth_ok") is True, \
        "an outage is not an auth failure"


def test_hevy_leg_not_run_when_disabled(conn, monkeypatch):
    _hevy_ready(conn)
    cfg = config.load_config(conn)
    cfg["hevy_enabled"] = False
    config.save_config(conn, cfg)
    poller, calls = _make_poller(conn, monkeypatch)

    poller._loop_once(START)

    assert calls["hevy"] == 0


def test_hevy_leg_not_run_without_api_key(conn, monkeypatch):
    _hevy_ready(conn)
    db.set_config_value(conn, "hevy_api_key", None)
    poller, calls = _make_poller(conn, monkeypatch)

    poller._loop_once(START)

    assert calls["hevy"] == 0


def test_hevy_leg_not_run_without_factory(conn, monkeypatch):
    _hevy_ready(conn)
    calls = {"hevy": 0}
    monkeypatch.setattr(
        "activsync.poller.hevy_sync.run_hevy_leg",
        lambda *a, **k: calls.update(hevy=calls["hevy"] + 1) or False,
    )
    poller = Poller(conn, garmin_factory=lambda: MagicMock(),
                    strava_factory=lambda: MagicMock())

    poller._loop_once(START)

    assert calls["hevy"] == 0


def test_config_defaults_cover_the_hevy_settings():
    assert config.DEFAULT_CONFIG["hevy_enabled"] is False
    assert config.DEFAULT_CONFIG["hevy_watch_strategy"] == "replace"
    assert config.DEFAULT_CONFIG["hevy_poll_interval_minutes"] == 10
    assert config.DEFAULT_CONFIG["hevy_grace_minutes"] == 120
    assert config.DEFAULT_CONFIG["hevy_device_identity"] is None
