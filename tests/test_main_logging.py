import importlib
import logging

from fastapi.testclient import TestClient

from activsync import timeutil


def test_resolve_log_timezone_prefers_display_setting(tmp_path, monkeypatch):
    monkeypatch.setenv("ACTIVSYNC_DB_PATH", str(tmp_path / "main.db"))
    import activsync.main as main
    importlib.reload(main)
    from activsync import config, db
    conn = db.connect(str(tmp_path / "main.db"))
    cfg = config.load_config(conn)
    cfg["display_timezone"] = "America/New_York"
    config.save_config(conn, cfg)
    monkeypatch.delenv("TZ", raising=False)
    assert timeutil.is_valid_timezone(main._resolve_log_timezone())


def test_resolve_log_timezone_falls_back_to_tz_env(tmp_path, monkeypatch):
    monkeypatch.setenv("ACTIVSYNC_DB_PATH", str(tmp_path / "main2.db"))
    monkeypatch.setenv("TZ", "Asia/Tokyo")
    import activsync.main as main
    importlib.reload(main)
    assert main._resolve_log_timezone() in ("Asia/Tokyo", "Europe/Stockholm")


def test_import_main_configures_activsync_logger(tmp_path, monkeypatch):
    monkeypatch.setenv("ACTIVSYNC_DB_PATH", str(tmp_path / "main3.db"))
    import activsync.main as main
    importlib.reload(main)
    assert logging.getLogger("activsync").handlers  # a handler was attached


def test_manual_only_mode_starts_only_the_hevy_polling_leg(tmp_path, monkeypatch):
    monkeypatch.setenv("ACTIVSYNC_DB_PATH", str(tmp_path / "manual-only.db"))
    monkeypatch.setenv("ACTIVSYNC_DEV_MOCK_DATA", "0")
    monkeypatch.setenv("ACTIVSYNC_MANUAL_ONLY", "1")
    import activsync.main as main
    importlib.reload(main)

    lifecycle = []
    monkeypatch.setattr(main._poller, "start", lambda: lifecycle.append("poller-start"))
    monkeypatch.setattr(main._poller, "stop", lambda: lifecycle.append("poller-stop"))
    monkeypatch.setattr(main._update_checker, "start", lambda: lifecycle.append("updates"))

    with TestClient(main.app) as client:
        assert client.get("/health").status_code == 200

    assert lifecycle == ["poller-start", "poller-stop"]
    assert main._poller._garmin_polling_enabled is False
    assert main._poller._strava_polling_enabled is False
    assert main._poller._hevy_polling_enabled is True


def test_real_mode_starts_background_services_by_default(tmp_path, monkeypatch):
    monkeypatch.setenv("ACTIVSYNC_DB_PATH", str(tmp_path / "normal.db"))
    monkeypatch.setenv("ACTIVSYNC_DEV_MOCK_DATA", "0")
    monkeypatch.delenv("ACTIVSYNC_MANUAL_ONLY", raising=False)
    import activsync.main as main
    importlib.reload(main)

    lifecycle = []
    monkeypatch.setattr(main._poller, "start", lambda: lifecycle.append("poller-start"))
    monkeypatch.setattr(main._update_checker, "start", lambda: lifecycle.append("updates-start"))
    monkeypatch.setattr(main._poller, "stop", lambda: lifecycle.append("poller-stop"))
    monkeypatch.setattr(main._update_checker, "stop", lambda: lifecycle.append("updates-stop"))

    with TestClient(main.app) as client:
        assert client.get("/health").status_code == 200

    assert lifecycle == [
        "poller-start",
        "updates-start",
        "poller-stop",
        "updates-stop",
    ]
