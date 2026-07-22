"""In-memory fakes for the local dev server (``ACTIVSYNC_DEV_MOCK_DATA``).

These let the entire first-run setup wizard — Garmin login, MFA, Strava OAuth,
and the initial sync — be walked end to end without any real account or network
call. They are wired in only when mock mode is on (see ``server._mock_mode`` and
the ``_build_*`` / login seams); production code paths never reach this module.

Dev conventions, chosen so both the happy path and the error paths of the wizard
can be exercised deliberately:

* Garmin login succeeds for any email/password...
* ...unless the password equals :data:`MFA_TRIGGER_PASSWORD` (``"mfa"``), which
  simulates an MFA challenge so the code-entry modal can be tested.
* During that challenge, :data:`MFA_REJECT_CODE` (``"000000"``) is rejected to
  exercise the error screen; any other code is accepted.
* Strava "OAuth" bounces straight back to the local callback and stores a fake
  token, so no external Strava app or browser round-trip is needed.

Activities are inserted by :mod:`activsync.dev_seed` at startup, so the fake
Garmin client reports no new recent activities — the initial sync simply
confirms the connection and the dashboard shows the seeded data.
"""

from __future__ import annotations

import sqlite3
import time
from datetime import datetime, timedelta, timezone

import json
from dataclasses import fields as dataclass_fields

from activsync import db
from activsync.garmin_client import ActivityRecord, MfaRequired, SubcategoryRejected

MFA_TRIGGER_PASSWORD = "mfa"
MFA_REJECT_CODE = "000000"

# -- Hevy demo fixtures -------------------------------------------------------
# Five canned workouts covering the demo scenarios the dev pass must show:
# a merge onto a seeded watch activity, an unmapped custom exercise (mapping
# queue), a graceless passive upload, a midnight-spanning session, and an
# in-flight sync holding the publish interlock on its watch activity.
HEVY_DEV_MERGED_ID = "hw-dev-merged"
HEVY_DEV_UNMAPPED_ID = "hw-dev-unmapped"
HEVY_DEV_PASSIVE_ID = "hw-dev-passive"
HEVY_DEV_MIDNIGHT_ID = "hw-dev-midnight"
HEVY_DEV_SYNCING_ID = "hw-dev-syncing"
HEVY_DEV_CUSTOM_TEMPLATE_ID = "tpl-dev-custom"

# Two more, added for Task 16 (backfill). The five above are all seeded into
# `hevy_workouts` by `dev_seed._seed_hevy` (looked up there by id, one at a
# time — see that function), so `hevy_backfill.preview_items` short-circuits
# every one of them to `"already tracked"` before it ever reaches the
# mapping-miss check. That leaves no reachable `needs_mapping` (locked) row
# for the backfill screen to demo or for `e2e/backfill.spec.ts` to click
# through — these two exist to fix that, and are deliberately left OUT of
# `dev_seed._seed_hevy` (which only looks up the five ids above by name) so
# they stay "unseen" and land in a backfill preview as fresh workouts.
# - UNMAPPED reuses the same custom exercise/template as HEVY_DEV_UNMAPPED_ID
#   above, so `missing_template_ids` comes back non-empty: `["tpl-dev-custom"]`.
# - UNMAPPABLE's exercise has no `exercise_template_id` at all (Hevy sends a
#   falsy one for some entries) *and* a title with no built-in name mapping,
#   so it also fails to resolve — but `missing_template_ids` stays empty,
#   since `hevy_backfill.preview_items` only appends a template id when one
#   exists. This is the "locked with nothing to link" edge case Task 4 added
#   (`missingTemplateIds` can disagree with `action`) — without this fixture
#   that path is untestable in a real browser, unit tests only.
HEVY_DEV_BACKFILL_UNMAPPED_ID = "hw-dev-backfill-unmapped"
HEVY_DEV_BACKFILL_UNMAPPABLE_ID = "hw-dev-backfill-unmappable"

# Watch/upload activity ids the seeded scenarios link against.
HEVY_DEV_MERGED_ACTIVITY_ID = 910001
HEVY_DEV_SYNCING_ACTIVITY_ID = 910002
HEVY_DEV_PASSIVE_ACTIVITY_ID = 910050
# Putting exercise sets onto this activity 400s, exercising the
# subcategory-rejection path end to end in dev.
HEVY_SUBCATEGORY_REJECT_ACTIVITY_ID = HEVY_DEV_SYNCING_ACTIVITY_ID

_DEV_GARMIN_ACTIVITIES_KEY = "dev_garmin_remote_activities"
_DEV_GARMIN_DELETED_KEY = "dev_garmin_deleted_activity_ids"


def _iso(dt: datetime) -> str:
    return dt.strftime("%Y-%m-%dT%H:%M:%S+00:00")


def _bench_sets() -> list[dict]:
    return [{"type": "normal", "weight_kg": 80.0, "reps": 5} for _ in range(3)]


def dev_hevy_workouts(now: datetime | None = None) -> list[dict]:
    """The canned Hevy workouts, timestamped relative to `now` so the demo
    stays fresh no matter when the dev DB was seeded."""
    now = (now or datetime.now(timezone.utc)).replace(microsecond=0)
    midnight_start = (now - timedelta(days=1)).replace(hour=23, minute=30, second=0)
    return [
        {
            "id": HEVY_DEV_MERGED_ID,
            "title": "Push day (Hevy)",
            "start_time": _iso(now - timedelta(hours=3)),
            "end_time": _iso(now - timedelta(hours=2)),
            "updated_at": _iso(now - timedelta(hours=2)),
            "exercises": [{
                "title": "Bench Press (Barbell)",
                "exercise_template_id": "tpl-dev-bench",
                "sets": _bench_sets(),
            }],
        },
        {
            "id": HEVY_DEV_UNMAPPED_ID,
            "title": "Ring circuit (Hevy)",
            "start_time": _iso(now - timedelta(hours=6)),
            "end_time": _iso(now - timedelta(hours=5)),
            "updated_at": _iso(now - timedelta(hours=5)),
            "exercises": [{
                "title": "Bulgarian Ring Row",
                "exercise_template_id": HEVY_DEV_CUSTOM_TEMPLATE_ID,
                "sets": _bench_sets(),
            }],
        },
        {
            "id": HEVY_DEV_PASSIVE_ID,
            "title": "Forgot the watch (Hevy)",
            "start_time": _iso(now - timedelta(hours=27)),
            "end_time": _iso(now - timedelta(hours=26)),
            "updated_at": _iso(now - timedelta(hours=26)),
            "exercises": [{
                "title": "Bench Press (Barbell)",
                "exercise_template_id": "tpl-dev-bench",
                "sets": _bench_sets(),
            }],
        },
        {
            "id": HEVY_DEV_MIDNIGHT_ID,
            "title": "Midnight session (Hevy)",
            "start_time": _iso(midnight_start),
            "end_time": _iso(midnight_start + timedelta(minutes=75)),
            "updated_at": _iso(midnight_start + timedelta(minutes=75)),
            "exercises": [{
                "title": "Bench Press (Barbell)",
                "exercise_template_id": "tpl-dev-bench",
                "sets": _bench_sets(),
            }],
        },
        {
            "id": HEVY_DEV_SYNCING_ID,
            "title": "Leg day (Hevy)",
            "start_time": _iso(now - timedelta(minutes=90)),
            "end_time": _iso(now - timedelta(minutes=30)),
            "updated_at": _iso(now - timedelta(minutes=30)),
            "exercises": [{
                "title": "Bench Press (Barbell)",
                "exercise_template_id": "tpl-dev-bench",
                "sets": _bench_sets(),
            }],
        },
        {
            "id": HEVY_DEV_BACKFILL_UNMAPPED_ID,
            "title": "Old ring circuit (Hevy)",
            "start_time": _iso(now - timedelta(days=5, minutes=17)),
            "end_time": _iso(now - timedelta(days=5, minutes=17) + timedelta(hours=1)),
            "updated_at": _iso(now - timedelta(days=5, minutes=17) + timedelta(hours=1)),
            "exercises": [{
                "title": "Bulgarian Ring Row",
                "exercise_template_id": HEVY_DEV_CUSTOM_TEMPLATE_ID,
                "sets": _bench_sets(),
            }],
        },
        {
            "id": HEVY_DEV_BACKFILL_UNMAPPABLE_ID,
            "title": "Old mystery session (Hevy)",
            "start_time": _iso(now - timedelta(days=6, minutes=23)),
            "end_time": _iso(now - timedelta(days=6, minutes=23) + timedelta(hours=1)),
            "updated_at": _iso(now - timedelta(days=6, minutes=23) + timedelta(hours=1)),
            "exercises": [{
                "title": "Mystery Movement",
                "exercise_template_id": None,
                "sets": _bench_sets(),
            }],
        },
    ]


# Titles here must match `hevy_mapper.HEVY_TO_GARMIN` keys exactly, or they
# resolve to nothing and the mapping screen shows a wall of needs-mapping rows
# instead of the realistic mix. Verified against that table.
_DEV_RESOLVING_TEMPLATES: list[tuple[str, str, str, str]] = [
    ("tpl-dev-bench", "Bench Press (Barbell)", "chest", "barbell"),
    ("tpl-dev-squat", "Squat (Barbell)", "quadriceps", "barbell"),
    ("tpl-dev-deadlift", "Deadlift (Barbell)", "hamstrings", "barbell"),
    ("tpl-dev-curl", "Bicep Curl (Dumbbell)", "biceps", "dumbbell"),
    ("tpl-dev-pulldown", "Lat Pulldown (Cable)", "lats", "cable"),
    ("tpl-dev-ohp", "Overhead Press (Barbell)", "shoulders", "barbell"),
    ("tpl-dev-plank", "Plank", "abdominals", "none"),
    ("tpl-dev-pullup", "Pull Up", "lats", "none"),
    ("tpl-dev-rdl", "Romanian Deadlift (Barbell)", "hamstrings", "barbell"),
]

# Seeded with a saved user mapping (see dev_seed) so the mapping screen has a
# row whose pair someone actually chose, not just table-resolved ones.
HEVY_DEV_OVERRIDDEN_TEMPLATE_ID = "tpl-dev-pushdown"
# Seeded with a mapping Garmin rejected, so the "needs action" path has a
# second shape besides "never mapped".
HEVY_DEV_REJECTED_TEMPLATE_ID = "tpl-dev-rejected"


def dev_hevy_templates() -> list[dict]:
    """A realistic spread for the mapping screen: mostly built-ins the ported
    tables resolve on their own, one the user overrode, one Garmin rejected,
    and one custom exercise nothing can place."""
    templates = [
        {
            "id": template_id,
            "title": title,
            "primary_muscle_group": muscle,
            "secondary_muscle_groups": [],
            "equipment_category": equipment,
            "is_custom": False,
        }
        for template_id, title, muscle, equipment in _DEV_RESOLVING_TEMPLATES
    ]
    templates.append({
        "id": HEVY_DEV_OVERRIDDEN_TEMPLATE_ID,
        "title": "Triceps Pushdown (Cable)",
        "primary_muscle_group": "triceps",
        "secondary_muscle_groups": [],
        "equipment_category": "cable",
        "is_custom": False,
    })
    templates.append({
        "id": HEVY_DEV_REJECTED_TEMPLATE_ID,
        "title": "Copenhagen Plank",
        "primary_muscle_group": "abductors",
        "secondary_muscle_groups": ["abdominals"],
        "equipment_category": "none",
        "is_custom": True,
    })
    templates.append({
        "id": HEVY_DEV_CUSTOM_TEMPLATE_ID,
        "title": "Bulgarian Ring Row",
        "primary_muscle_group": "upper_back",
        "secondary_muscle_groups": ["biceps"],
        "equipment_category": "other",
        "is_custom": True,
    })
    return templates

# Keep the local publish request on screen long enough to exercise the button's
# busy state. This fake is only wired in when mock mode is enabled.
DEV_PUBLISH_DELAY_SECONDS = 1.0

# Garmin's activity-type taxonomy, taken from a real account. Dev used to show
# a hand-picked 16, which is a size at which the picker lies to you: the search
# box, the "Show all" collapse and the bulk select/clear actions all only earn
# their keep against the ~150 categories production actually renders.
#
# Keys only — GarminClient.fetch_activity_types derives every label from the key
# and sorts by it, and garmin_activity_types below derives them the same way. A dev
# list of pre-written labels could drift from what production renders; a list of
# keys run through the same derivation cannot.
GARMIN_ACTIVITY_TYPE_KEYS = (
    "american_football",
    "apnea_diving",
    "apnea_hunting",
    "archery",
    "assistance",
    "atv_v2",
    "auto_racing",
    "backcountry_skiing",
    "backcountry_skiing_snowboarding_ws",
    "backcountry_snowboarding",
    "badminton",
    "baseball",
    "basketball",
    "biketoruntransition_v2",
    "bmx",
    "boating_v2",
    "bouldering",
    "boxing",
    "breathwork",
    "casual_walking",
    "ccr_diving",
    "cricket",
    "cross_country_indoor_skiing",
    "cross_country_skiing_ws",
    "cycling",
    "cyclocross",
    "dance",
    "disc_golf",
    "diving",
    "downhill_biking",
    "driving_general",
    "e_bike_fitness",
    "e_bike_mountain",
    "e_enduro_mtb",
    "e_sport",
    "elliptical",
    "enduro_mtb",
    "field_hockey",
    "fishing_v2",
    "fitness_equipment",
    "floor_climbing",
    "flying",
    "gauge_diving",
    "golf",
    "gravel_cycling",
    "hand_cycling",
    "hang_gliding",
    "hiit",
    "hiking",
    "horseback_riding",
    "hunting",
    "hunting_fishing",
    "ice_hockey",
    "incident_detected",
    "indoor_cardio",
    "indoor_climbing",
    "indoor_cycling",
    "indoor_hand_cycling",
    "indoor_rowing",
    "indoor_running",
    "inline_skating",
    "jump_rope",
    "kayaking_v2",
    "kiteboarding_v2",
    "lacrosse",
    "lap_swimming",
    "meditation",
    "mixed_martial_arts",
    "mobility",
    "motocross_v2",
    "motorcycling_v2",
    "mountain_biking",
    "mountaineering",
    "multi_gas_diving",
    "multi_sport",
    "obstacle_run",
    "offshore_grinding_v2",
    "onshore_grinding_v2",
    "open_water_swimming",
    "other",
    "overland",
    "paddelball",
    "paddling_v2",
    "para_sports",
    "pickleball",
    "pilates",
    "platform_tennis",
    "pool_apnea",
    "racket_sports",
    "racquetball",
    "rc_drone",
    "recumbent_cycling",
    "resort_skiing",
    "resort_skiing_snowboarding_ws",
    "resort_snowboarding",
    "road_biking",
    "rock_climbing",
    "rowing_v2",
    "rucking",
    "rugby",
    "running",
    "runtobiketransition_v2",
    "safety",
    "sailing_v2",
    "single_gas_diving",
    "skate_skiing_ws",
    "skating_ws",
    "sky_diving",
    "snorkeling",
    "snow_shoe_ws",
    "snowmobiling_ws",
    "soccer",
    "softball",
    "speed_walking",
    "squash",
    "stair_climbing",
    "stand_up_paddleboarding_v2",
    "steps",
    "stop_watch",
    "street_running",
    "strength_training",
    "surfing_v2",
    "swimming",
    "swimtobiketransition_v2",
    "table_tennis",
    "team_sports",
    "tennis_v2",
    "track_cycling",
    "track_running",
    "trail_running",
    "transition_v2",
    "treadmill_running",
    "ultimate_disc",
    "ultra_run",
    "virtual_ride",
    "virtual_run",
    "volleyball",
    "wakeboarding_v2",
    "wakesurfing",
    "walking",
    "water_sports",
    "water_tubing",
    "waterskiing",
    "wheelchair_push_run",
    "wheelchair_push_walk",
    "whitewater_rafting_kayaking",
    "whitewater_rafting_v2",
    "wind_kite_surfing",
    "windsurfing_v2",
    "wingsuit_flying",
    "winter_sports",
    "yoga",
)


def garmin_activity_types() -> list[dict]:
    """The taxonomy as GarminClient.fetch_activity_types would report it."""
    types = [
        {"type_key": key, "label": key.replace("_", " ").title()}
        for key in GARMIN_ACTIVITY_TYPE_KEYS
    ]
    types.sort(key=lambda t: t["label"])
    return types


class _FakePendingAuth:
    """Stand-in for ``garmin_auth.GarminAuth`` during a simulated MFA challenge."""

    def resume_login(self, mfa_code: str):
        if mfa_code == MFA_REJECT_CODE:
            raise ValueError("Invalid MFA code (dev mock rejects 000000)")
        return None


def begin_login(email: str, password: str):
    """Fake Garmin login. Raises :class:`MfaRequired` when the password is
    :data:`MFA_TRIGGER_PASSWORD`, otherwise succeeds immediately."""
    if password == MFA_TRIGGER_PASSWORD:
        raise MfaRequired(_FakePendingAuth())
    return None


def complete_login(pending_auth, mfa_code: str):
    """Finish a simulated MFA challenge; delegates to the pending auth."""
    return pending_auth.resume_login(mfa_code)


class FakeGarminClient:
    """No-network Garmin client backed by seeded and DB-persisted remote state.

    Uploads, metadata edits, and deletions survive the fresh client instance
    built for the next request, so multi-tick workflows behave like Garmin.
    """

    def __init__(self, conn: sqlite3.Connection):
        self._conn = conn
        # Per-instance recording (a fresh client is built per request):
        # lets tests and the dev pass inspect what a merge would have PUT.
        self.recorded_exercise_sets: list[tuple[int, dict]] = []

    def _remote_activities(self) -> dict[str, dict]:
        return db.get_config_value(
            self._conn, _DEV_GARMIN_ACTIVITIES_KEY, default={}
        ) or {}

    def _save_remote_activities(self, activities: dict[str, dict]) -> None:
        db.set_config_value(self._conn, _DEV_GARMIN_ACTIVITIES_KEY, activities)

    def _deleted_ids(self) -> set[int]:
        return {
            int(activity_id)
            for activity_id in db.get_config_value(
                self._conn, _DEV_GARMIN_DELETED_KEY, default=[]
            ) or []
        }

    def _ensure_remote_activity(self, activity_id: int) -> tuple[dict, dict]:
        activities = self._remote_activities()
        key = str(activity_id)
        activity = activities.get(key)
        if activity is None:
            row = db.get_activity(self._conn, activity_id)
            if row is None:
                activity = {
                    "garmin_activity_id": activity_id,
                    "activity_type": "strength_training",
                    "title": "Strength Training",
                    "description": "",
                    "start_time": datetime.now(timezone.utc).strftime(
                        "%Y-%m-%d %H:%M:%S"
                    ),
                    "duration": 3600.0,
                }
            else:
                try:
                    data = json.loads(row.get("garmin_data") or "{}")
                except (ValueError, TypeError):
                    data = {}
                activity = {
                    "garmin_activity_id": activity_id,
                    "activity_type": row["activity_type"],
                    "title": row["title"],
                    "description": row.get("description") or "",
                    "start_time": row["start_time"],
                    **{key: data.get(key) for key in {
                        field.name for field in dataclass_fields(ActivityRecord)
                    } if key not in {
                        "garmin_activity_id", "activity_type", "title",
                        "description", "start_time",
                    }},
                }
        return activities, dict(activity)

    def fetch_activity_types(self) -> list[dict]:
        # Report the taxonomy, the way the real client reports what Garmin
        # holds. Echoing the stored list back (as this used to) made Refresh
        # categories a no-op in dev — it could never disagree with the DB, so
        # the one thing the button exists to do went untested.
        return garmin_activity_types()

    def fetch_recent_activities(self, lookback_days: int) -> list[ActivityRecord]:
        # Echo the local table back as what "Garmin" holds. Returning [] (as
        # this used to) made sync_garmin's removal pass mark every seeded
        # activity inside the lookback window as deleted from Garmin — which
        # wiped the recent Hevy demo activities on the wizard's initial sync.
        cutoff = (
            datetime.now(timezone.utc) - timedelta(days=lookback_days)
        ).strftime("%Y-%m-%d %H:%M:%S")
        optional_keys = {
            field.name for field in dataclass_fields(ActivityRecord)
        } - {"garmin_activity_id", "activity_type", "title", "description",
             "start_time"}
        deleted = self._deleted_ids()
        records: dict[int, ActivityRecord] = {}
        for row in db.list_activities(self._conn):
            if row["garmin_activity_id"] in deleted or row["start_time"] < cutoff:
                continue
            try:
                data = json.loads(row.get("garmin_data") or "{}")
            except (ValueError, TypeError):
                data = {}
            record = ActivityRecord(
                garmin_activity_id=row["garmin_activity_id"],
                activity_type=row["activity_type"],
                title=row["title"],
                description=row.get("description") or "",
                start_time=row["start_time"],
                **{key: data.get(key) for key in optional_keys},
            )
            records[record.garmin_activity_id] = record
        # Remote overlays represent uploads and metadata mutations. They win
        # over the local echo until sync_garmin reconciles the same values.
        for raw in self._remote_activities().values():
            activity_id = int(raw["garmin_activity_id"])
            if activity_id in deleted or raw["start_time"] < cutoff:
                continue
            records[activity_id] = ActivityRecord(
                **{field.name: raw.get(field.name)
                   for field in dataclass_fields(ActivityRecord)}
            )
        return list(records.values())

    def download_fit(self, garmin_activity_id: int) -> bytes:
        return b""

    def update_activity_metadata(
        self, garmin_activity_id: int, title: str, description: str
    ) -> None:
        activities, activity = self._ensure_remote_activity(garmin_activity_id)
        activity.update({"title": title, "description": description})
        activities[str(garmin_activity_id)] = activity
        self._save_remote_activities(activities)

    # -- Hevy-integration surface ---------------------------------------------

    def upload_fit(self, fit_path: str) -> dict:
        # DB-backed sequence so ids stay fresh across requests (each request
        # builds a new client instance).
        sequence = int(
            db.get_config_value(self._conn, "dev_upload_seq", default=0)
        ) + 1
        db.set_config_value(self._conn, "dev_upload_seq", sequence)
        activity_id = 950000 + sequence
        start_time = datetime.now(timezone.utc)
        duration = 3600.0
        try:
            from fit_tool.fit_file import FitFile
            from fit_tool.profile.messages.session_message import SessionMessage

            fit = FitFile.from_file(fit_path)
            session = next(
                record.message for record in fit.records
                if isinstance(record.message, SessionMessage)
            )
            start_time = datetime.fromtimestamp(
                float(session.start_time) / 1000.0, tz=timezone.utc
            )
            duration = float(session.total_elapsed_time)
        except Exception:
            # Surface tests may pass a placeholder path; the fake still needs
            # a coherent activity record for later reconciliation.
            pass
        activities = self._remote_activities()
        activities[str(activity_id)] = {
            "garmin_activity_id": activity_id,
            "activity_type": "strength_training",
            "title": "Strength Training",
            "description": "",
            "start_time": start_time.strftime("%Y-%m-%d %H:%M:%S"),
            "duration": duration,
        }
        self._save_remote_activities(activities)
        return {"upload_id": f"dev-upload-{sequence}",
                "activity_id": activity_id}

    def get_exercise_sets(self, activity_id: int) -> dict:
        return {"exerciseSets": []}

    def put_exercise_sets(self, activity_id: int, payload: dict) -> None:
        if activity_id == HEVY_SUBCATEGORY_REJECT_ACTIVITY_ID:
            raise SubcategoryRejected(
                "dev mock: Garmin rejects this category/subcategory pair")
        self.recorded_exercise_sets.append((activity_id, payload))
        return None

    def set_title(self, activity_id: int, title: str) -> None:
        activities, activity = self._ensure_remote_activity(activity_id)
        activity["title"] = title
        activities[str(activity_id)] = activity
        self._save_remote_activities(activities)

    def set_description(self, activity_id: int, description: str) -> None:
        activities, activity = self._ensure_remote_activity(activity_id)
        activity["description"] = description
        activities[str(activity_id)] = activity
        self._save_remote_activities(activities)

    def delete_activity(self, activity_id: int) -> None:
        deleted = self._deleted_ids()
        deleted.add(int(activity_id))
        db.set_config_value(self._conn, _DEV_GARMIN_DELETED_KEY, sorted(deleted))

    def get_daily_heart_rates(self, date_str: str) -> dict:
        return {"heartRateValues": []}

    def fetch_user_profile(self) -> dict:
        return {"weight_kg": 80.0, "birth_year": 1990, "sex": "male", "vo2max": 45.0}

    def find_activity_near(
        self, start_time: str, exclude_ids: set, window_minutes: int = 10
    ) -> int | None:
        target = datetime.fromisoformat(start_time.replace("Z", "+00:00"))
        if target.tzinfo is None:
            target = target.replace(tzinfo=timezone.utc)
        for activity in self.fetch_recent_activities(3):
            if activity.garmin_activity_id in exclude_ids:
                continue
            actual = datetime.strptime(
                activity.start_time, "%Y-%m-%d %H:%M:%S"
            ).replace(tzinfo=timezone.utc)
            if abs((actual - target).total_seconds()) <= window_minutes * 60:
                return activity.garmin_activity_id
        return None

    def list_activity_ids_near(self, start_time: str) -> list[int]:
        return [
            activity["activityId"] for activity in self.list_activities_near(start_time)
        ]

    def list_activities_near(self, start_time: str) -> list[dict]:
        target = datetime.fromisoformat(start_time.replace("Z", "+00:00"))
        if target.tzinfo is None:
            target = target.replace(tzinfo=timezone.utc)
        activities = []
        for record in self.fetch_recent_activities(3):
            actual = datetime.strptime(
                record.start_time, "%Y-%m-%d %H:%M:%S"
            ).replace(tzinfo=timezone.utc)
            if abs((actual.date() - target.date()).days) > 1:
                continue
            activities.append({
                "activityId": record.garmin_activity_id,
                "activityType": {"typeKey": record.activity_type},
                "startTimeGMT": record.start_time,
                "duration": record.duration,
            })
        return activities


class FakeStravaClient:
    """No-network Strava client. The "OAuth" handshake loops straight back to
    the local callback and stores a fake token so the connection reads as live,
    and duplicate/existence checks are answered without any HTTP call."""

    def __init__(self, conn: sqlite3.Connection):
        self._conn = conn

    def is_connected(self) -> bool:
        tokens = db.get_config_value(self._conn, "strava_tokens") or {}
        return bool(tokens.get("refresh_token"))

    def authorize_url(self, redirect_uri: str, state: str) -> str:
        # Skip Strava entirely: bounce straight to our own callback with a
        # placeholder code that exchange_code accepts unconditionally. The
        # state is echoed back exactly as the real Strava would, so the
        # callback's state check exercises the same path in mock mode.
        separator = "&" if "?" in redirect_uri else "?"
        return f"{redirect_uri}{separator}code=dev-mock-code&state={state}"

    def exchange_code(self, code: str) -> None:
        db.set_config_value(self._conn, "strava_tokens", {
            "access_token": "dev-mock-access-token",
            "refresh_token": "dev-mock-refresh-token",
            "expires_at": int(time.time()) + 6 * 3600,
        })

    def disconnect(self) -> None:
        db.set_config_value(self._conn, "strava_tokens", None)

    def list_activities_between(
        self, after: datetime, before: datetime, now: datetime | None = None
    ) -> list[dict]:
        # Report back exactly what the mock itself "published" and nothing more.
        # The caller reads absence from this window as "deleted on Strava", so
        # returning an empty list would flag every published row as missing;
        # returning anything extra would link a pending row to a duplicate that
        # doesn't exist. Neither is true in dev.
        window = []
        for row in db.list_activities(self._conn, status="published"):
            strava_activity_id = row.get("strava_activity_id")
            if strava_activity_id is None:
                continue
            start = datetime.strptime(
                row["start_time"], "%Y-%m-%d %H:%M:%S"
            ).replace(tzinfo=timezone.utc)
            if after <= start <= before:
                window.append({"id": int(strava_activity_id), "start_date": start})
        return window

    def find_existing_activity(
        self, start_time: datetime, tolerance_minutes: int = 5, now: datetime | None = None
    ) -> int | None:
        # No pre-existing Strava duplicates to link against in dev.
        return None

    def publish(
        self,
        garmin_activity_id: int,
        fit_bytes: bytes,
        name: str | None = None,
        description: str | None = None,
    ) -> int:
        time.sleep(DEV_PUBLISH_DELAY_SECONDS)
        return 9_000_000 + int(garmin_activity_id)

    def update_activity_metadata(
        self, strava_activity_id: int, name: str, description: str
    ) -> None:
        return None


class MockHevyClient:
    """No-network Hevy client serving the canned demo workouts/templates.
    Any API key validates, so the wizard step and the settings card can be
    walked without a Hevy Pro account."""

    def __init__(self, conn: sqlite3.Connection):
        self._conn = conn

    def get_user_info(self) -> dict:
        return {"id": "dev-user", "username": "dev"}

    def get_workout(self, workout_id: str) -> dict | None:
        for workout in dev_hevy_workouts():
            if workout["id"] == workout_id:
                return workout
        return None

    def get_workouts_page(self, page: int = 1, page_size: int = 10) -> dict:
        workouts = dev_hevy_workouts() if page == 1 else []
        return {"page": page, "page_count": 1, "workouts": workouts}

    def get_events(self, since: str, page: int = 1, page_size: int = 10) -> dict:
        events = [
            {"type": "updated", "workout": workout}
            for workout in dev_hevy_workouts()
            if page == 1 and workout["updated_at"] > since
        ]
        return {"page": page, "page_count": 1, "events": events}

    def iter_events_since(self, since: str) -> list[dict]:
        return self.get_events(since)["events"]

    def get_exercise_template(self, template_id: str) -> dict | None:
        for template in dev_hevy_templates():
            if template["id"] == template_id:
                return template
        return None

    def get_exercise_templates_page(self, page: int = 1, page_size: int = 10) -> dict:
        templates = dev_hevy_templates() if page == 1 else []
        return {"page": page, "page_count": 1, "exercise_templates": templates}

    def iter_all_exercise_templates(self) -> list[dict]:
        return dev_hevy_templates()
