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


def _access_record(path: str, status: int) -> logging.LogRecord:
    """An access line exactly as uvicorn emits it."""
    return logging.LogRecord(
        "uvicorn.access", logging.INFO, "path", 1,
        '%s - "%s %s HTTP/%s" %d', ("127.0.0.1:49924", "GET", path, "1.1", status), None,
    )


def test_uvicorn_lifecycle_logs_are_not_labelled_error():
    """uvicorn's lifecycle logger is named 'uvicorn.error' but carries ordinary
    startup messages, so the last-dotted-segment rule made a clean boot read as
    four errors: 'INFO    error    Started server process'."""
    fmt = logging_setup.LocalTimeFormatter("%(levelname)s %(component)s %(message)s")
    record = logging.LogRecord(
        "uvicorn.error", logging.INFO, "path", 1, "Started server process [1]", None, None,
    )

    assert fmt.format(record) == "INFO uvicorn Started server process [1]"


def test_app_component_names_are_unchanged():
    fmt = logging_setup.LocalTimeFormatter("%(component)s")
    for name, expected in [
        ("activsync", "activsync"),
        ("activsync.poller", "poller"),
        ("activsync.server", "server"),
        ("uvicorn.access", "access"),
    ]:
        record = logging.LogRecord(name, logging.INFO, "path", 1, "m", None, None)
        assert fmt.format(record) == expected


def test_successful_health_probes_are_not_logged():
    """Docker probes /health every 30s — 2880 lines a day that say nothing."""
    log_filter = logging_setup.AccessNoiseFilter()

    assert log_filter.filter(_access_record("/health", 200)) is False


def test_failing_health_probes_are_still_logged():
    """A failing probe is the whole point of having one."""
    log_filter = logging_setup.AccessNoiseFilter()

    assert log_filter.filter(_access_record("/health", 500)) is True
    assert log_filter.filter(_access_record("/health", 404)) is True


def test_other_requests_are_still_logged():
    log_filter = logging_setup.AccessNoiseFilter()

    assert log_filter.filter(_access_record("/", 200)) is True
    assert log_filter.filter(_access_record("/api/sync/strava/publish", 200)) is True


def test_health_filter_tolerates_records_it_does_not_understand():
    """The filter must never drop a line it failed to parse."""
    log_filter = logging_setup.AccessNoiseFilter()
    plain = logging.LogRecord("uvicorn.error", logging.INFO, "p", 1, "Startup complete", None, None)

    assert log_filter.filter(plain) is True


def test_configure_logging_silences_health_probes_end_to_end(capsys):
    logging_setup.configure_logging(level="INFO", tz_name="UTC")
    access = logging.getLogger("uvicorn.access")

    access.handle(_access_record("/health", 200))
    access.handle(_access_record("/", 200))

    out = capsys.readouterr().out
    assert "/health" not in out
    assert '"GET / HTTP/1.1" 200' in out


def test_successful_static_asset_requests_are_not_logged():
    """A single page load emits a dozen of these; they say nothing that the
    page request itself didn't already say."""
    log_filter = logging_setup.AccessNoiseFilter()

    assert log_filter.filter(_access_record("/static/css/base.css?v=1.2.1", 200)) is False
    assert log_filter.filter(_access_record("/static/favicon.png", 200)) is False
    # A cache revalidation is just as routine as a fresh 200.
    assert log_filter.filter(_access_record("/static/favicon.png", 304)) is False


def test_failed_static_asset_requests_are_still_logged():
    """A 404 on a stylesheet is a broken deploy — the one case where these
    lines carry real information."""
    log_filter = logging_setup.AccessNoiseFilter()

    assert log_filter.filter(_access_record("/static/css/base.css?v=1.2.1", 404)) is True
    assert log_filter.filter(_access_record("/static/css/base.css", 500)) is True


def test_noise_filter_does_not_swallow_lookalike_paths():
    """Matching on the parsed path, not the message text, so these survive."""
    log_filter = logging_setup.AccessNoiseFilter()

    assert log_filter.filter(_access_record("/staticky", 200)) is True
    assert log_filter.filter(_access_record("/healthz", 200)) is True
    assert log_filter.filter(_access_record("/api/static/thing", 200)) is True


def test_verbose_filter_keeps_everything():
    """ACTIVSYNC_LOG_LEVEL=DEBUG must be able to get the noise back — otherwise
    'why is my CSS 200-ing but not applying' is undebuggable from logs."""
    log_filter = logging_setup.AccessNoiseFilter(verbose=True)

    assert log_filter.filter(_access_record("/health", 200)) is True
    assert log_filter.filter(_access_record("/static/css/base.css", 200)) is True


def test_configure_logging_at_debug_keeps_routine_access_lines(capsys):
    logging_setup.configure_logging(level="DEBUG", tz_name="UTC")
    access = logging.getLogger("uvicorn.access")

    access.handle(_access_record("/health", 200))
    access.handle(_access_record("/static/css/base.css", 200))

    out = capsys.readouterr().out
    assert "/health" in out
    assert "/static/css/base.css" in out
    logging_setup.configure_logging(level="INFO", tz_name="UTC")


def test_configure_logging_silences_static_assets_end_to_end(capsys):
    logging_setup.configure_logging(level="INFO", tz_name="UTC")
    access = logging.getLogger("uvicorn.access")

    access.handle(_access_record("/static/css/base.css?v=1.2.1", 200))
    access.handle(_access_record("/", 200))

    out = capsys.readouterr().out
    assert "/static/css/base.css" not in out
    assert '"GET / HTTP/1.1" 200' in out


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
