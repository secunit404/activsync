"""Logging configuration: readable, timezone-aware logs for `docker logs`.

Timestamps follow the app's configured display_timezone (updated live via
`set_log_timezone`) so log lines match what the user sees in the UI. Safe to
call from any thread — the poller logs from a background thread.
"""

from __future__ import annotations

import logging
import sys
import threading
from datetime import datetime
from zoneinfo import ZoneInfo

from activsync import timeutil

_DEFAULT_TZ = "Europe/Stockholm"

_tz_lock = threading.Lock()
_log_tz = ZoneInfo(_DEFAULT_TZ)


def set_log_timezone(tz_name: str) -> None:
    """Update the timezone used for log timestamps. Invalid zones are ignored
    (the current zone is kept)."""
    if not timeutil.is_valid_timezone(tz_name):
        return
    global _log_tz
    with _tz_lock:
        _log_tz = ZoneInfo(tz_name)


def _current_tz() -> ZoneInfo:
    with _tz_lock:
        return _log_tz


# uvicorn's lifecycle logger is named "uvicorn.error" but carries ordinary
# startup and shutdown messages, so the last-segment rule below would label a
# clean boot "error". It can't be "server" — activsync.server owns that.
_COMPONENT_OVERRIDES = {"uvicorn.error": "uvicorn"}


def _component(logger_name: str) -> str:
    return _COMPONENT_OVERRIDES.get(logger_name, logger_name.split(".")[-1])


class LocalTimeFormatter(logging.Formatter):
    """Renders timestamps in the current display timezone and adds a short
    `component` field (the last dotted segment of the logger name)."""

    def format(self, record: logging.LogRecord) -> str:
        record.component = _component(record.name)
        return super().format(record)

    def formatTime(self, record: logging.LogRecord, datefmt: str | None = None) -> str:
        dt = datetime.fromtimestamp(record.created, tz=_current_tz())
        return dt.strftime(datefmt or "%Y-%m-%d %H:%M:%S %Z")


_LOG_FORMAT = "%(asctime)s  %(levelname)-7s %(component)-8s %(message)s"
_UVICORN_LOGGERS = ("uvicorn", "uvicorn.access", "uvicorn.error")

_HEALTH_PATH = "/health"
_STATIC_PREFIX = "/static/"

# uvicorn's access records carry the request as args rather than a formatted
# string: (client_addr, method, path_with_query, http_version, status_code).
_ACCESS_PATH_ARG = 2
_ACCESS_STATUS_ARG = 4


def _is_routine_path(path: str) -> bool:
    """Paths whose successful requests carry no information.

    The health probe fires every 30s regardless of what the app is doing, and
    static assets come a dozen at a time with each page load — which the page's
    own access line already reported.
    """
    return path == _HEALTH_PATH or path.startswith(_STATIC_PREFIX)


class AccessNoiseFilter(logging.Filter):
    """Drops access lines for routine, successful requests.

    Health probes and static assets together bury the handful of lines that
    say something. Only *successful* ones are dropped: a failing probe is the
    only time /health has anything to report, and a 404 on a stylesheet is a
    broken deploy. Pass verbose=True (ACTIVSYNC_LOG_LEVEL=DEBUG) to keep
    everything, so the noise is recoverable when it's what you're debugging.
    """

    def __init__(self, verbose: bool = False):
        super().__init__()
        self._verbose = verbose

    def filter(self, record: logging.LogRecord) -> bool:
        if self._verbose:
            return True

        args = record.args
        if not isinstance(args, tuple) or len(args) <= _ACCESS_STATUS_ARG:
            return True  # not an access record; never drop what we can't parse

        path, status = args[_ACCESS_PATH_ARG], args[_ACCESS_STATUS_ARG]
        if not isinstance(path, str) or not isinstance(status, int):
            return True
        return not (_is_routine_path(path.split("?")[0]) and status < 400)


def _resolve_level(level: str | int) -> int:
    if isinstance(level, int):
        return level
    resolved = logging.getLevelName(str(level).upper())
    return resolved if isinstance(resolved, int) else logging.INFO


def _reset_handlers(lg: logging.Logger) -> None:
    for handler in list(lg.handlers):
        lg.removeHandler(handler)


def configure_logging(*, level: str | int = "INFO", tz_name: str = _DEFAULT_TZ) -> None:
    """Attach a single readable, timezone-aware stdout handler to the app's
    loggers. Idempotent: safe to call more than once."""
    set_log_timezone(tz_name)
    resolved_level = _resolve_level(level)

    handler = logging.StreamHandler(sys.stdout)
    handler.setFormatter(LocalTimeFormatter(_LOG_FORMAT))
    handler.addFilter(AccessNoiseFilter(verbose=resolved_level <= logging.DEBUG))

    app_logger = logging.getLogger("activsync")
    _reset_handlers(app_logger)
    app_logger.addHandler(handler)
    app_logger.setLevel(resolved_level)
    app_logger.propagate = False

    for name in _UVICORN_LOGGERS:
        uv = logging.getLogger(name)
        _reset_handlers(uv)
        uv.addHandler(handler)
        uv.propagate = False
