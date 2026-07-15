import logging

from activsync import logging_setup


def test_per_activity_debug_line_is_filtered_at_info_level(capsys):
    """At the default INFO level, run-level summaries appear but per-activity
    DEBUG lines are filtered out, so `docker logs` stays scannable."""
    logging_setup.configure_logging(level="INFO", tz_name="UTC")
    logger = logging.getLogger("activsync.sync")
    logger.info("strava status check: 2 activities linked to existing Strava entries")
    logger.debug("linked created activity 123 to existing Strava 456")
    out = capsys.readouterr().out
    assert "status check" in out
    assert "linked created activity" not in out


def test_per_activity_debug_line_shows_at_debug_level(capsys):
    """Setting DEBUG surfaces the per-activity detail for troubleshooting."""
    logging_setup.configure_logging(level="DEBUG", tz_name="UTC")
    logger = logging.getLogger("activsync.sync")
    logger.debug("linked created activity 123 to existing Strava 456")
    out = capsys.readouterr().out
    assert "linked created activity" in out
