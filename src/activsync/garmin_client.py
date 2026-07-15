"""Garmin Connect access: activity polling and FIT retrieval."""

from __future__ import annotations

import io
import logging
import zipfile
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone

from garminconnect import Garmin
from garmin_auth import GarminAuth, RateLimiter

logger = logging.getLogger("activsync.garmin_client")
_limiter = RateLimiter(delay=1.0, max_retries=3, base_wait=30)
_PAGE_SIZE = 20


@dataclass
class ActivityRecord:
    garmin_activity_id: int
    activity_type: str
    title: str
    description: str
    start_time: str
    # --- optional metrics from the list endpoint ---
    distance: float | None = None          # metres
    duration: float | None = None          # seconds
    moving_duration: float | None = None   # seconds
    elapsed_duration: float | None = None  # seconds
    elevation_gain: float | None = None    # metres
    elevation_loss: float | None = None    # metres
    calories: float | None = None
    avg_hr: float | None = None            # bpm
    max_hr: float | None = None            # bpm
    avg_power: float | None = None         # watts
    max_power: float | None = None         # watts
    norm_power: float | None = None        # watts
    aerobic_te: float | None = None        # aerobic training effect
    anaerobic_te: float | None = None      # anaerobic training effect
    training_load: float | None = None
    avg_cadence: float | None = None       # steps/min (running)
    max_cadence: float | None = None       # steps/min (running)
    total_sets: int | None = None          # strength
    total_reps: int | None = None          # strength
    total_volume: float | None = None      # strength (kg)


def get_client(email: str, password: str, token_dir: str) -> Garmin:
    """Get an authenticated Garmin client using garmin-auth's cached-token login."""
    auth = GarminAuth(email=email, password=password, token_dir=token_dir)
    return auth.login()


class MfaRequired(Exception):
    """Raised by begin_login when Garmin challenges the login with a one-time code.

    Carries the in-flight GarminAuth so the caller can complete the challenge
    later via complete_login(), without restarting the login from scratch.
    """

    def __init__(self, pending_auth: GarminAuth):
        super().__init__("Garmin requires a one-time code to complete login")
        self.pending_auth = pending_auth


def begin_login(email: str, password: str, token_dir: str) -> Garmin:
    """Start a Garmin login, raising MfaRequired if a one-time code is needed.

    Unlike get_client(), this never blocks on a synchronous input() prompt —
    callers that catch MfaRequired hold its .pending_auth and call
    complete_login() once the user has supplied a code (e.g. from a web form).
    """
    auth = GarminAuth(email=email, password=password, token_dir=token_dir, return_on_mfa=True)
    result = auth.login()
    if result == "needs_mfa":
        logger.info("garmin login requires MFA for %s", email)
        raise MfaRequired(auth)
    logger.info("garmin login succeeded for %s", email)
    return result


def complete_login(pending_auth: GarminAuth, mfa_code: str) -> Garmin:
    """Finish a login that raised MfaRequired, using the code the user supplied."""
    result = pending_auth.resume_login(mfa_code)
    logger.info("garmin MFA login completed")
    return result


def _parse_garmin_time(value: str) -> datetime | None:
    if not value:
        return None
    try:
        return datetime.strptime(value, "%Y-%m-%d %H:%M:%S").replace(tzinfo=timezone.utc)
    except ValueError:
        return None


class GarminClient:
    """Thin wrapper around garminconnect.Garmin for what ActivSync needs."""

    def __init__(self, raw_client: Garmin):
        self._client = raw_client

    def fetch_recent_activities(self, lookback_days: int) -> list[ActivityRecord]:
        cutoff = datetime.now(timezone.utc) - timedelta(days=lookback_days)
        records: list[ActivityRecord] = []
        start = 0

        while True:
            batch = _limiter.call(self._client.get_activities, start, _PAGE_SIZE)
            if not batch:
                break

            reached_cutoff = False
            for act in batch:
                start_time = act.get("startTimeGMT", "")
                start_dt = _parse_garmin_time(start_time)
                if start_dt is not None and start_dt < cutoff:
                    reached_cutoff = True
                    break
                records.append(ActivityRecord(
                    garmin_activity_id=int(act["activityId"]),
                    activity_type=act.get("activityType", {}).get("typeKey", "unknown"),
                    title=act.get("activityName") or "",
                    description=act.get("description") or "",
                    start_time=start_time,
                    distance=act.get("distance"),
                    duration=act.get("duration"),
                    moving_duration=act.get("movingDuration"),
                    elapsed_duration=act.get("elapsedDuration"),
                    elevation_gain=act.get("elevationGain"),
                    elevation_loss=act.get("elevationLoss"),
                    calories=act.get("calories"),
                    avg_hr=act.get("averageHR"),
                    max_hr=act.get("maxHR"),
                    avg_power=act.get("avgPower"),
                    max_power=act.get("maxPower"),
                    norm_power=act.get("normPower"),
                    aerobic_te=act.get("aerobicTrainingEffect"),
                    anaerobic_te=act.get("anaerobicTrainingEffect"),
                    training_load=act.get("activityTrainingLoad"),
                    avg_cadence=act.get("averageRunningCadenceInStepsPerMinute"),
                    max_cadence=act.get("maxRunningCadenceInStepsPerMinute"),
                    total_sets=act.get("totalSets"),
                    total_reps=act.get("totalReps"),
                    total_volume=act.get("totalVolume"),
                ))

            if reached_cutoff or len(batch) < _PAGE_SIZE:
                break
            start += _PAGE_SIZE

        return records

    def download_fit(self, garmin_activity_id: int) -> bytes:
        zip_bytes = _limiter.call(
            self._client.download_activity,
            garmin_activity_id,
            dl_fmt=Garmin.ActivityDownloadFormat.ORIGINAL,
        )
        with zipfile.ZipFile(io.BytesIO(zip_bytes)) as zf:
            fit_names = [name for name in zf.namelist() if name.lower().endswith(".fit")]
            if not fit_names:
                raise ValueError(f"no .fit file in original download for activity {garmin_activity_id}")
            return zf.read(fit_names[0])

    def update_activity_metadata(self, garmin_activity_id: int, title: str, description: str) -> None:
        _limiter.call(self._client.set_activity_name, garmin_activity_id, title)
        _limiter.call(self._client.set_activity_description, garmin_activity_id, description)

    def fetch_activity_types(self) -> list[dict]:
        """Garmin's canonical activity type taxonomy, as
        [{"type_key": "running", "label": "Running"}, ...], de-duplicated and
        sorted by label. Used to populate the autosync-category checklist."""
        raw_types = self._client.get_activity_types()
        seen: set[str] = set()
        result: list[dict] = []
        for entry in raw_types:
            type_key = entry.get("typeKey")
            # Skip "all" — it's Garmin's own "All activities" pseudo-category,
            # not a real activity type anyone would want to autosync/hold.
            if not type_key or type_key in seen or type_key == "all":
                continue
            seen.add(type_key)
            result.append({"type_key": type_key, "label": type_key.replace("_", " ").title()})
        result.sort(key=lambda t: t["label"])
        return result
