"""Garmin Connect access: activity polling and FIT retrieval."""

from __future__ import annotations

import io
import logging
import zipfile
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone

from garmin_auth import GarminAuth, RateLimiter
from garminconnect import Garmin

from activsync.timeutil import parse_iso_utc

logger = logging.getLogger("activsync.garmin_client")
_limiter = RateLimiter(delay=1.0, max_retries=3, base_wait=30)
_PAGE_SIZE = 20


class GarminUploadRejected(RuntimeError):
    """Garmin definitively rejected a FIT upload (failures, no successes)."""


class SubcategoryRejected(Exception):
    """Garmin 400-rejected an exerciseSets payload over a (category,
    subcategory) pair. The PUT is atomic, so the whole payload failed; the
    response does not identify which pair was at fault."""


class ActivityGone(Exception):
    """The target activity no longer exists on Garmin (404)."""


def _sanitize_activity_id(raw: object) -> int | None:
    """Normalize an activity id from the upload API. Garmin occasionally
    returns internalId as a string wrapped in quote characters (upstream
    hevy2garmin #153); stored verbatim, every later call 404s."""
    if raw is None:
        return None
    if isinstance(raw, int):
        return raw
    cleaned = str(raw).strip().strip("'\"").strip()
    try:
        return int(cleaned)
    except (ValueError, TypeError):
        return None


def _is_subcategory_rejection(exc: Exception) -> bool:
    """A 400 whose body complains about the exercise sub-category (upstream
    finding: fit_tool-valid pairs can still be rejected by the API)."""
    msg = str(exc).lower()
    return "sub-category" in msg or "subcategory" in msg or "invalid sub" in msg


def _is_not_found(exc: Exception) -> bool:
    response = getattr(exc, "response", None)
    if response is not None and getattr(response, "status_code", None) == 404:
        return True
    return "404" in str(exc)


def _exception_response(exc: Exception):
    """Find an HTTP response on an exception or its immediate wrapper chain."""
    for candidate in (exc, exc.__cause__, exc.__context__):
        if candidate is not None:
            response = getattr(candidate, "response", None)
            if response is not None:
                return response
    return None


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

    def _activity_call(self, activity_id: int, func, *args):
        """Call an activity endpoint and normalize a real 404."""
        try:
            return _limiter.call(func, activity_id, *args)
        except Exception as exc:
            if _is_not_found(exc):
                raise ActivityGone(f"activity {activity_id} not found") from exc
            raise

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

    # -- Hevy integration additions -------------------------------------

    def upload_fit(self, fit_path: str) -> dict:
        """Upload a FIT file; returns {"upload_id", "activity_id"}.

        activity_id is None when Garmin's response omits it (import still
        processing) — resolution is the operation journal's job, never a
        start-time retry loop here. Definite rejection raises
        GarminUploadRejected."""
        from pathlib import Path

        if not Path(fit_path).exists():
            raise FileNotFoundError(f"FIT file not found: {fit_path}")
        try:
            resp = _limiter.call(self._client.upload_activity, str(fit_path))
        except Exception as exc:
            response = _exception_response(exc)
            status = getattr(response, "status_code", None)
            body = getattr(response, "text", "") if response is not None else ""
            # The phase-0 spike proved this is a definite non-import, not an
            # ambiguous outcome that belongs in submission_unknown.
            if status == 409 or "duplicate activity" in str(exc).lower():
                raise GarminUploadRejected(
                    f"Garmin rejected upload: {body or exc}") from exc
            raise

        upload_id = None
        activity_id = None
        if isinstance(resp, dict):
            detail = resp.get("detailedImportResult", {})
            upload_id = detail.get("uploadId")
            successes = detail.get("successes", [])
            if successes and isinstance(successes, list):
                activity_id = _sanitize_activity_id(successes[0].get("internalId"))
            failures = detail.get("failures", [])
            if failures and not activity_id and not successes:
                raise GarminUploadRejected(f"Garmin rejected upload: {failures}")
        logger.info("fit upload: upload_id=%s activity_id=%s", upload_id, activity_id)
        return {"upload_id": upload_id, "activity_id": activity_id}

    def get_exercise_sets(self, activity_id: int) -> dict:
        return self._activity_call(
            activity_id, self._client.get_activity_exercise_sets)

    def put_exercise_sets(self, activity_id: int, payload: dict) -> None:
        """PUT the full exercise-set list (atomic replace of ALL sets)."""
        url = f"/activity-service/activity/{activity_id}/exerciseSets"
        try:
            _limiter.call(self._client.client.request, "PUT", "connectapi", url,
                          json=payload)
        except Exception as exc:
            if _is_subcategory_rejection(exc):
                raise SubcategoryRejected(str(exc)) from exc
            if _is_not_found(exc):
                raise ActivityGone(f"activity {activity_id} not found") from exc
            raise

    def set_title(self, activity_id: int, title: str) -> None:
        self._activity_call(activity_id, self._client.set_activity_name, title)

    def set_description(self, activity_id: int, description: str) -> None:
        self._activity_call(
            activity_id, self._client.set_activity_description, description)

    def delete_activity(self, activity_id: int) -> None:
        _limiter.call(self._client.delete_activity, activity_id)
        logger.info("deleted garmin activity %s", activity_id)

    def get_daily_heart_rates(self, date_str: str) -> dict:
        return _limiter.call(self._client.get_heart_rates, date_str)

    def fetch_user_profile(self) -> dict:
        """User physiology for calorie estimation: weight (grams → kg), birth
        year, sex, and VO2max from the max-metrics endpoint. Missing pieces
        come back as None — hevy_profile fills defaults."""
        user_data = {}
        try:
            user_data = _limiter.call(self._client.get_user_profile).get("userData") or {}
        except Exception as exc:
            logger.warning("user profile fetch failed: %s", exc)

        weight = user_data.get("weight")
        weight_kg = round(float(weight) / 1000.0, 1) if weight else None
        birth_date = user_data.get("birthDate") or ""
        try:
            birth_year = int(str(birth_date)[:4])
        except (ValueError, TypeError):
            birth_year = None
        gender = user_data.get("gender")
        sex = str(gender).lower() if gender else None

        vo2max = None
        try:
            today = datetime.now(timezone.utc).date().isoformat()
            metrics = _limiter.call(self._client.get_max_metrics, today)
            entries = metrics if isinstance(metrics, list) else [metrics]
            for entry in entries:
                generic = (entry or {}).get("generic") or {}
                value = generic.get("vo2MaxPreciseValue") or generic.get("vo2MaxValue")
                if value:
                    vo2max = float(value)
                    break
        except Exception as exc:
            logger.debug("vo2max fetch failed: %s", exc)

        return {"weight_kg": weight_kg, "birth_year": birth_year,
                "sex": sex, "vo2max": vo2max}

    def list_activities_near(self, start_time: str) -> list[dict]:
        """ALL activities (raw dicts) in start_time's date ±1 day — the
        journal's resolution primitive needs metadata (type, start, duration)
        for strict candidate matching, not bare ids. Failures PROPAGATE."""
        target = _parse_garmin_time(start_time) or parse_iso_utc(start_time)
        if target is None:
            raise ValueError(f"unparseable start_time: {start_time!r}")
        date_from = (target - timedelta(days=1)).date().isoformat()
        date_to = (target + timedelta(days=1)).date().isoformat()
        activities = _limiter.call(
            self._client.get_activities_by_date, date_from, date_to)
        return list(activities or [])

    def list_activity_ids_near(self, start_time: str) -> list[int]:
        """ALL activity ids in start_time's date ±1 day, every type — the
        operation journal's pre/post-upload snapshot primitive. Failures
        PROPAGATE: an outage must never read as an empty snapshot, or a later
        submission_unknown diff would adopt the wrong activity."""
        target = _parse_garmin_time(start_time) or parse_iso_utc(start_time)
        if target is None:
            raise ValueError(f"unparseable start_time: {start_time!r}")
        date_from = (target - timedelta(days=1)).date().isoformat()
        date_to = (target + timedelta(days=1)).date().isoformat()
        activities = _limiter.call(
            self._client.get_activities_by_date, date_from, date_to)
        return [int(act["activityId"]) for act in (activities or [])
                if act.get("activityId") is not None]

    def find_activity_near(
        self,
        start_time: str,
        exclude_ids: set,
        window_minutes: int = 10,
    ) -> int | None:
        """A strength_training/other activity starting within window_minutes
        of start_time, searching the date ±1 day (timezone edges). Excluded
        ids are skipped. Ported from upstream find_activity_by_start_time."""
        target = _parse_garmin_time(start_time) or parse_iso_utc(start_time)
        if target is None:
            return None
        date_from = (target - timedelta(days=1)).date().isoformat()
        date_to = (target + timedelta(days=1)).date().isoformat()
        try:
            activities = _limiter.call(
                self._client.get_activities_by_date, date_from, date_to)
        except Exception as exc:
            logger.warning("activity search failed: %s", exc)
            return None

        excluded = {str(x) for x in (exclude_ids or set())}
        for act in activities or []:
            activity_id = act.get("activityId")
            if str(activity_id) in excluded:
                continue
            act_type = act.get("activityType", {}).get("typeKey", "")
            if act_type and act_type not in ("strength_training", "other"):
                continue
            act_start = (_parse_garmin_time(act.get("startTimeGMT", ""))
                         or parse_iso_utc(act.get("startTimeGMT", "")))
            if act_start is None:
                continue
            if abs((act_start - target).total_seconds()) < window_minutes * 60:
                return activity_id
        return None

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
