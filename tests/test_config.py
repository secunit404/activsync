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
