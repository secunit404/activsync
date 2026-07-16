"""Background thread that runs Garmin fetch and Strava publish/status-check
on independent intervals, checked on a short shared tick."""

from __future__ import annotations

import logging
import sqlite3
import threading
from datetime import datetime, timedelta, timezone
from typing import Callable

from activsync import config, db, events, sync
from activsync.garmin_client import GarminClient
from activsync.strava_client import StravaClient, StravaRateLimitError

logger = logging.getLogger("activsync.poller")

_DEFAULT_TICK_SECONDS = 60


class Poller:
    def __init__(
        self,
        conn: sqlite3.Connection,
        garmin_factory: Callable[[], GarminClient],
        strava_factory: Callable[[], StravaClient],
        tick_seconds: float = _DEFAULT_TICK_SECONDS,
        garmin_interval_seconds_override: float | None = None,
        strava_interval_seconds_override: float | None = None,
    ):
        self._conn = conn
        self._garmin_factory = garmin_factory
        self._strava_factory = strava_factory
        self._tick_seconds = tick_seconds
        self._garmin_interval_seconds_override = garmin_interval_seconds_override
        self._strava_interval_seconds_override = strava_interval_seconds_override
        self._stop_event = threading.Event()
        self._thread: threading.Thread | None = None
        self._last_garmin_run: datetime | None = None
        self._last_strava_run: datetime | None = None
        self._strava_backoff_until: datetime | None = None

    def run_garmin_once(self, now: datetime | None = None) -> sync.GarminSyncStats:
        cfg = config.load_config(self._conn)
        garmin = self._garmin_factory()
        now = now or datetime.now(timezone.utc)
        return sync.sync_garmin(self._conn, garmin, cfg, now)

    def run_strava_once(
        self, now: datetime | None = None
    ) -> tuple[sync.PublishStats, sync.StatusCheckStats]:
        cfg = config.load_config(self._conn)
        garmin = self._garmin_factory()
        strava = self._strava_factory()
        now = now or datetime.now(timezone.utc)
        publish_stats = sync.publish_pending(self._conn, garmin, strava, now)
        status_stats = sync.check_strava_status(self._conn, strava, cfg, now)
        return publish_stats, status_stats

    def _garmin_interval_seconds(self) -> float:
        if self._garmin_interval_seconds_override is not None:
            return self._garmin_interval_seconds_override
        return config.load_config(self._conn)["garmin_poll_interval_minutes"] * 60

    def _strava_interval_seconds(self) -> float:
        if self._strava_interval_seconds_override is not None:
            return self._strava_interval_seconds_override
        return config.load_config(self._conn)["strava_poll_interval_minutes"] * 60

    def _due(self, last_run: datetime | None, now: datetime, interval_seconds: float) -> bool:
        return last_run is None or (now - last_run).total_seconds() >= interval_seconds

    def _garmin_ready(self) -> bool:
        return bool(db.get_config_value(self._conn, "garmin_credentials_verified", default=False))

    def _strava_ready(self) -> bool:
        tokens = db.get_config_value(self._conn, "strava_tokens") or {}
        return bool(tokens.get("refresh_token"))

    def _strava_rate_limited(self, now: datetime) -> bool:
        return self._strava_backoff_until is not None and now < self._strava_backoff_until

    def _run_strava_guarded(self, now: datetime, failure_message: str) -> bool:
        """Run one Strava pass, absorbing failures. Returns whether anything changed.

        A 429 is not a crash — it means the quota is already spent, so the only
        useful response is to stop calling until Strava says the window has
        reset. Retrying every tick is what keeps the quota exhausted.
        """
        try:
            pub, status = self.run_strava_once(now)
        except StravaRateLimitError as exc:
            self._strava_backoff_until = now + timedelta(seconds=exc.retry_after_seconds)
            logger.warning(
                "strava rate limit reached; pausing strava sync for %ds (until %s)",
                int(exc.retry_after_seconds),
                self._strava_backoff_until.isoformat(timespec="seconds"),
            )
            return False
        except Exception:
            logger.exception(failure_message)
            return False

        self._strava_backoff_until = None
        return bool(
            pub.published or pub.failed or status.flagged_missing or status.linked_existing
        )

    def _ready_for_polling(self) -> bool:
        """The poller stays out of the way until first-run setup has finished —
        the wizard's initial sync runs a blocking sync on this same sqlite
        connection, and a concurrent poll would interleave with it."""
        return bool(db.get_config_value(self._conn, "initial_sync_done", default=False))

    def _loop_once(self, now: datetime) -> None:
        if not self._ready_for_polling():
            return

        changed = False
        strava_ran_this_tick = False

        # NOTE: while a side is not ready we deliberately do NOT advance its
        # _last_run. Advancing it would make a reconnect wait out a full
        # interval (up to an hour) before the first sync; leaving it stale means
        # the very next tick is due, so the list catches up within the minute.
        if self._garmin_ready() and self._due(
            self._last_garmin_run, now, self._garmin_interval_seconds()
        ):
            try:
                stats = self.run_garmin_once(now)
                garmin_changed = any(
                    getattr(stats, field, 0) for field in ("new", "updated", "removed")
                )
                if garmin_changed:
                    changed = True
                    if self._strava_ready() and not self._strava_rate_limited(now):
                        if self._run_strava_guarded(
                            now, "strava status check after garmin sync failed"
                        ):
                            changed = True
                        self._last_strava_run = now
                        strava_ran_this_tick = True
            except Exception:
                logger.exception("garmin sync failed")
            self._last_garmin_run = now

        if (
            self._garmin_ready() and self._strava_ready()
            and not strava_ran_this_tick
            and not self._strava_rate_limited(now)
            and self._due(self._last_strava_run, now, self._strava_interval_seconds())
        ):
            if self._run_strava_guarded(now, "strava sync failed"):
                changed = True
            self._last_strava_run = now

        if changed:
            events.bus.publish("refresh")

    def _loop(self) -> None:
        while not self._stop_event.is_set():
            self._loop_once(datetime.now(timezone.utc))
            self._stop_event.wait(self._tick_seconds)

    def start(self) -> None:
        self._stop_event.clear()
        self._thread = threading.Thread(target=self._loop, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop_event.set()
        if self._thread is not None:
            self._thread.join(timeout=2)
