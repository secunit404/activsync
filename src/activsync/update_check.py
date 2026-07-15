"""Checks GitHub Releases for a newer ActivSync version.

Fully network-optional: `get_status()` only ever reads an in-memory cache and
never blocks a request. `refresh()` performs the single network call and never
raises — a failed check leaves the last-known status in place.
"""

from __future__ import annotations

import logging
import threading
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Callable

import requests

from activsync import __version__

logger = logging.getLogger("activsync.update_check")

GITHUB_REPO = "secunit404/activsync"
REPO_URL = f"https://github.com/{GITHUB_REPO}"
RELEASE_PAGE = f"{REPO_URL}/releases/latest"
RELEASES_API = f"https://api.github.com/repos/{GITHUB_REPO}/releases/latest"

CACHE_TTL_SECONDS = 86400  # 24h
REQUEST_TIMEOUT_SECONDS = 5

_CURRENT_VERSION = __version__


@dataclass(frozen=True)
class UpdateStatus:
    current: str
    latest: str | None
    update_available: bool
    checked_at: datetime | None
    repo_url: str
    release_url: str


_lock = threading.Lock()
_cache: UpdateStatus | None = None


def _parse_version(text: str) -> tuple[int, ...]:
    """Turn a tag like 'v1.2.3' into (1, 2, 3). Stops at the first
    non-numeric component; unparseable input yields an empty tuple."""
    cleaned = text.strip().lstrip("vV")
    parts: list[int] = []
    for chunk in cleaned.split("."):
        digits = ""
        for ch in chunk:
            if ch.isdigit():
                digits += ch
            else:
                break
        if not digits:
            break
        parts.append(int(digits))
    return tuple(parts)


def _is_newer(latest: str, current: str) -> bool:
    latest_v = _parse_version(latest)
    current_v = _parse_version(current)
    if not latest_v or not current_v:
        return False
    return latest_v > current_v


def _initial_status(current: str) -> UpdateStatus:
    return UpdateStatus(
        current=current,
        latest=None,
        update_available=False,
        checked_at=None,
        repo_url=REPO_URL,
        release_url=RELEASE_PAGE,
    )


def _default_fetcher() -> str:
    resp = requests.get(
        RELEASES_API,
        timeout=REQUEST_TIMEOUT_SECONDS,
        headers={
            "User-Agent": "ActivSync-update-check",
            "Accept": "application/vnd.github+json",
        },
    )
    resp.raise_for_status()
    return resp.json()["tag_name"]


def get_status() -> UpdateStatus:
    with _lock:
        if _cache is not None:
            return _cache
    return _initial_status(_CURRENT_VERSION)


def refresh(
    *,
    fetcher: Callable[[], str] = _default_fetcher,
    now: datetime | None = None,
    current: str | None = None,
) -> UpdateStatus:
    """Fetch the latest release and replace the cache. Never raises."""
    global _cache
    current = current or _CURRENT_VERSION
    now = now or datetime.now(timezone.utc)
    try:
        latest = fetcher().strip()
        status = UpdateStatus(
            current=current,
            latest=latest,
            update_available=_is_newer(latest, current),
            checked_at=now,
            repo_url=REPO_URL,
            release_url=RELEASE_PAGE,
        )
    except Exception:
        logger.debug("update check failed", exc_info=True)
        with _lock:
            if _cache is not None:
                return _cache
        return _initial_status(current)
    with _lock:
        _cache = status
    return status


def reset_cache() -> None:
    """Test-only: drop the in-memory cache."""
    global _cache
    with _lock:
        _cache = None


class UpdateChecker:
    """Background daemon: refresh once on start, then every `interval_seconds`.

    Mirrors Poller's start/stop shape. Not started in mock mode, so no network
    call ever happens in dev/mock sessions.
    """

    def __init__(
        self,
        interval_seconds: float = CACHE_TTL_SECONDS,
        fetcher: Callable[[], str] = _default_fetcher,
    ):
        self._interval = interval_seconds
        self._fetcher = fetcher
        self._stop_event = threading.Event()
        self._thread: threading.Thread | None = None

    def start(self) -> None:
        if self._thread is not None and self._thread.is_alive():
            return
        self._stop_event.clear()
        self._thread = threading.Thread(
            target=self._loop, name="update-checker", daemon=True
        )
        self._thread.start()

    def stop(self) -> None:
        self._stop_event.set()
        if self._thread is not None:
            self._thread.join(timeout=5)

    def _loop(self) -> None:
        while not self._stop_event.is_set():
            refresh(fetcher=self._fetcher)
            self._stop_event.wait(self._interval)
