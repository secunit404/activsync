"""Hevy API v1 client.

Ported from hevy2garmin's hevy.py (MIT) with ActivSync's rules: no
unconditional sleeps (retry/backoff only when the API signals 429/5xx),
HevyAuthError on bad keys, and the events-cursor pagination that powers
incremental sync. Responses are validated defensively — Hevy's docs warn the
event shapes may change.
"""

from __future__ import annotations

import logging

import requests
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry

logger = logging.getLogger("activsync.hevy_client")

DEFAULT_BASE_URL = "https://api.hevyapp.com/v1"
EVENT_TYPES = ("updated", "deleted")


class HevyAuthError(Exception):
    """Raised when the Hevy API rejects the API key (401/403)."""


def _valid_event(event: object) -> bool:
    """An event we know how to process: updated carries workout.id +
    workout.updated_at; deleted carries id + deleted_at."""
    if not isinstance(event, dict):
        return False
    event_type = event.get("type")
    if event_type == "updated":
        workout = event.get("workout")
        return (isinstance(workout, dict)
                and bool(workout.get("id"))
                and bool(workout.get("updated_at")))
    if event_type == "deleted":
        return bool(event.get("id")) and bool(event.get("deleted_at"))
    return False


class HevyClient:
    """HTTP client for the Hevy API v1."""

    def __init__(self, api_key: str, base_url: str = DEFAULT_BASE_URL) -> None:
        if not api_key:
            raise ValueError("Hevy API key required")
        self.base_url = base_url.rstrip("/")
        self.session = requests.Session()
        self.session.headers.update({
            "api-key": api_key,
            "Accept": "application/json",
        })
        retry = Retry(
            total=5,
            backoff_factor=2,
            status_forcelist=[429, 500, 502, 503, 504],
            allowed_methods=["GET"],
            raise_on_status=False,
        )
        self.session.mount("https://", HTTPAdapter(max_retries=retry))

    def _get(self, path: str, params: dict | None = None) -> dict:
        url = f"{self.base_url}{path}"
        resp = self.session.get(url, params=params, timeout=30)
        if resp.status_code in (401, 403):
            raise HevyAuthError(
                "Hevy API key is invalid or expired. "
                "Check your Hevy Pro subscription and regenerate the key at hevy.com/settings."
            )
        resp.raise_for_status()
        remaining = (resp.headers.get("X-RateLimit-Remaining")
                     or resp.headers.get("x-ratelimit-remaining"))
        if remaining is not None:
            try:
                if int(remaining) < 10:
                    logger.warning("Hevy API rate limit low: %s requests remaining",
                                   remaining)
            except ValueError:
                pass
        return resp.json()

    # -- account ------------------------------------------------------------

    def get_user_info(self) -> dict:
        """GET /user/info — used to validate an API key at connect time."""
        return self._get("/user/info")

    # -- workouts -----------------------------------------------------------

    def get_workout(self, workout_id: str) -> dict | None:
        """Fetch a single workout by id. None means Garmin-side truth: the
        workout does not exist (404) or the response was malformed. Outages
        (timeouts, 5xx, 429 past retries) PROPAGATE — they must never read
        as "workout deleted"."""
        try:
            data = self._get(f"/workouts/{workout_id}")
        except requests.HTTPError as exc:
            if getattr(exc.response, "status_code", None) == 404:
                return None
            raise
        if isinstance(data, dict):
            if "id" in data:
                return data
            if isinstance(data.get("workout"), dict):
                return data["workout"]
        logger.warning("malformed workout response for %s: %.200s", workout_id, data)
        return None

    def get_workouts_page(self, page: int = 1, page_size: int = 10) -> dict:
        return self._get("/workouts", {"page": page, "pageSize": page_size})

    # -- events (incremental sync) -------------------------------------------

    def get_events(self, since: str, page: int = 1, page_size: int = 10) -> dict:
        return self._get("/workouts/events",
                         {"since": since, "page": page, "pageSize": page_size})

    def iter_events_since(self, since: str) -> list[dict]:
        """All events newer than `since`, across every page (newest first as
        delivered). Malformed entries are skipped and logged, never raised —
        one bad event must not stall the cursor."""
        events: list[dict] = []
        page = 1
        while True:
            data = self.get_events(since, page=page)
            for event in data.get("events", []):
                if _valid_event(event):
                    events.append(event)
                else:
                    logger.warning("skipping malformed hevy event: %.200s", event)
            page_count = data.get("page_count", page)
            if page >= page_count:
                break
            page += 1
        return events

    # -- exercise templates --------------------------------------------------

    def get_exercise_templates_page(self, page: int = 1, page_size: int = 10) -> dict:
        return self._get("/exercise_templates",
                         {"page": page, "pageSize": page_size})

    def iter_all_exercise_templates(self) -> list[dict]:
        templates: list[dict] = []
        page = 1
        while True:
            data = self.get_exercise_templates_page(page, page_size=100)
            templates.extend(data.get("exercise_templates", []))
            page_count = data.get("page_count", page)
            if page >= page_count:
                break
            page += 1
        return templates
