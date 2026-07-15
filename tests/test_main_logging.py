import importlib
import logging

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
