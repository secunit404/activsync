import pytest

from activsync import config, db


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def test_load_config_returns_defaults_when_unset(conn):
    cfg = config.load_config(conn)

    assert cfg == config.DEFAULT_CONFIG


def test_save_and_load_config_roundtrip(conn):
    cfg = dict(config.DEFAULT_CONFIG)
    cfg["garmin_poll_interval_minutes"] = 30
    cfg["strava_poll_interval_minutes"] = 2

    config.save_config(conn, cfg)
    reloaded = config.load_config(conn)

    assert reloaded["garmin_poll_interval_minutes"] == 30
    assert reloaded["strava_poll_interval_minutes"] == 2


def test_load_config_merges_partial_saved_settings(conn):
    config.save_config(conn, {"garmin_poll_interval_minutes": 15})

    cfg = config.load_config(conn)

    assert cfg["garmin_poll_interval_minutes"] == 15
    assert cfg["lookback_days"] == config.DEFAULT_CONFIG["lookback_days"]


def test_default_lookback_days_is_seven(tmp_path):
    from activsync import config, db
    conn = db.connect(str(tmp_path / "test.db"))
    assert config.load_config(conn)["lookback_days"] == 7


@pytest.mark.parametrize("env_name", ["ACTIVSYNC_DEV_MOCK_DATA", "ACTIVSYNC_MANUAL_ONLY"])
def test_development_modes_default_hevy_matches_to_review(conn, monkeypatch, env_name):
    monkeypatch.setenv(env_name, "1")

    assert config.load_config(conn)["hevy_match_mode"] == "review"


def test_saved_automatic_match_mode_overrides_development_default(conn, monkeypatch):
    monkeypatch.setenv("ACTIVSYNC_MANUAL_ONLY", "1")
    config.save_config(conn, {"hevy_match_mode": "automatic"})

    assert config.load_config(conn)["hevy_match_mode"] == "automatic"
