import logging
from datetime import datetime, timezone

from activsync import logging_setup


def _record(ts: float) -> logging.LogRecord:
    r = logging.LogRecord("activsync.sync", logging.INFO, "path", 1, "hi", None, None)
    r.created = ts
    return r


def test_formatter_uses_stockholm_summer_time():
    logging_setup.set_log_timezone("Europe/Stockholm")
    fmt = logging_setup.LocalTimeFormatter("%(asctime)s | %(component)s | %(message)s")
    ts = datetime(2026, 7, 9, 9, 0, 3, tzinfo=timezone.utc).timestamp()
    assert fmt.format(_record(ts)) == "2026-07-09 11:00:03 CEST | sync | hi"


def test_formatter_uses_stockholm_winter_time():
    logging_setup.set_log_timezone("Europe/Stockholm")
    fmt = logging_setup.LocalTimeFormatter("%(asctime)s")
    ts = datetime(2026, 1, 9, 9, 0, 0, tzinfo=timezone.utc).timestamp()
    assert fmt.format(_record(ts)) == "2026-01-09 10:00:00 CET"


def test_set_log_timezone_changes_output_live():
    fmt = logging_setup.LocalTimeFormatter("%(asctime)s")
    ts = datetime(2026, 7, 9, 20, 0, 0, tzinfo=timezone.utc).timestamp()
    logging_setup.set_log_timezone("America/New_York")
    assert fmt.format(_record(ts)) == "2026-07-09 16:00:00 EDT"
    logging_setup.set_log_timezone("Europe/Stockholm")
    assert fmt.format(_record(ts)) == "2026-07-09 22:00:00 CEST"


def test_set_log_timezone_ignores_invalid_zone():
    logging_setup.set_log_timezone("Europe/Stockholm")
    logging_setup.set_log_timezone("Not/AZone")
    fmt = logging_setup.LocalTimeFormatter("%(asctime)s")
    ts = datetime(2026, 1, 9, 9, 0, 0, tzinfo=timezone.utc).timestamp()
    assert fmt.format(_record(ts)) == "2026-01-09 10:00:00 CET"


def test_configure_logging_sets_level_from_string():
    logging_setup.configure_logging(level="DEBUG", tz_name="UTC")
    assert logging.getLogger("activsync").level == logging.DEBUG


def test_configure_logging_defaults_invalid_level_to_info():
    logging_setup.configure_logging(level="NONSENSE", tz_name="UTC")
    assert logging.getLogger("activsync").level == logging.INFO


def test_configure_logging_is_idempotent():
    logging_setup.configure_logging(level="INFO", tz_name="UTC")
    logging_setup.configure_logging(level="INFO", tz_name="UTC")
    assert len(logging.getLogger("activsync").handlers) == 1


def test_configure_logging_emits_info_to_stdout(capsys):
    logging_setup.configure_logging(level="INFO", tz_name="UTC")
    logging.getLogger("activsync.sync").info("publish complete")
    out = capsys.readouterr().out
    assert "publish complete" in out
    assert "INFO" in out
    assert "sync" in out


def test_format_log_time_matches_the_timestamp_prefix_timezone():
    """Times embedded in a message must read in the same zone as the line's own
    timestamp — a UTC instant next to a CEST prefix looks like the past."""
    logging_setup.set_log_timezone("Europe/Stockholm")
    instant = datetime(2026, 7, 16, 20, 15, tzinfo=timezone.utc)

    assert logging_setup.format_log_time(instant) == "2026-07-16 22:15:00 CEST"


def test_format_log_time_follows_the_configured_timezone():
    instant = datetime(2026, 7, 16, 20, 15, tzinfo=timezone.utc)

    logging_setup.set_log_timezone("UTC")
    assert logging_setup.format_log_time(instant).startswith("2026-07-16 20:15:00")

    logging_setup.set_log_timezone("Asia/Tokyo")
    assert logging_setup.format_log_time(instant).startswith("2026-07-17 05:15:00")

    logging_setup.set_log_timezone("Europe/Stockholm")
