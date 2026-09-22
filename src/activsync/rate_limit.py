"""Retry-with-backoff for Garmin Connect calls.

Ported from the deprecated ``garmin_auth.RateLimiter`` (retired 2026-10-31).
Garmin throttles aggressively and answers 429 without a Retry-After, so the
wait is a fixed multiple of the attempt number rather than header-driven.
"""

from __future__ import annotations

import logging
import time
from typing import Any, Callable, TypeVar

from garminconnect import GarminConnectTooManyRequestsError

logger = logging.getLogger("activsync.rate_limit")

T = TypeVar("T")

DEFAULT_CALL_DELAY = 1.0
DEFAULT_MAX_RETRIES = 3
DEFAULT_BASE_WAIT = 30


class RateLimiter:
    """Serializes Garmin calls behind a fixed delay, retrying only on 429."""

    def __init__(
        self,
        delay: float = DEFAULT_CALL_DELAY,
        max_retries: int = DEFAULT_MAX_RETRIES,
        base_wait: int = DEFAULT_BASE_WAIT,
    ) -> None:
        if max_retries < 1:
            raise ValueError("max_retries must be at least 1")
        self.delay = delay
        self.max_retries = max_retries
        self.base_wait = base_wait

    def call(self, func: Callable[..., T], *args: Any, **kwargs: Any) -> T:
        for attempt in range(self.max_retries):
            try:
                result = func(*args, **kwargs)
            except GarminConnectTooManyRequestsError:
                if attempt == self.max_retries - 1:
                    logger.error("rate limited (429): %d retries exhausted",
                                 self.max_retries)
                    raise
                wait = (attempt + 1) * self.base_wait
                logger.warning("rate limited (429); waiting %ds before retry %d/%d",
                               wait, attempt + 2, self.max_retries)
                time.sleep(wait)
                continue
            time.sleep(self.delay)
            return result
        raise AssertionError("unreachable")
