"""Sample data used only by the local development server."""

from __future__ import annotations

import json
import sqlite3
from datetime import datetime, timedelta, timezone

from activsync import config, db, dev_mock, hevy_db

SEED_VERSION = 8

_HEVY_TABLES = ("hevy_workouts", "hevy_operations", "exercise_templates",
                "exercise_mappings", "merge_backups", "hevy_events_seen")


def seed(conn: sqlite3.Connection, *, mark_onboarded: bool = False) -> None:
    """Insert repeatable, multi-month activity data into the dev database.

    ``mark_onboarded`` is applied on every call that clears the non-dev-database
    guard below, independent of the version-gate that follows it: the E2E dev
    DB persists across local runs, so if it was already seeded (matching
    ``SEED_VERSION``) before the caller started asking for onboarding to be
    marked complete, the early return must not skip that. Applying it there is
    safe — it is idempotent.
    """
    existing = db.list_activities(conn)
    if existing and not all(
        str(row["content_hash"]).startswith("dev-") for row in existing
    ):
        # Never touch a database containing non-dev activity IDs — including
        # via mark_onboarded, which must be refused on the same terms as
        # every other seed write.
        return
    if mark_onboarded:
        _mark_onboarding_complete(conn)
    if db.get_config_value(conn, "dev_seed_version") == SEED_VERSION:
        return
    if existing:
        # Rebuild an older version of this generated-only database.
        conn.execute("DELETE FROM activities")
        for table in _HEVY_TABLES:
            conn.execute(f"DELETE FROM {table}")
        conn.commit()

    now = datetime.now(timezone.utc).replace(microsecond=0)
    samples = [
        ("running", "Morning run", "Easy loop before work.", 42 * 60, 7_850, "pending"),
        ("cycling", "Saturday ride", "Longer ride with a few good climbs.", 2 * 3600 + 8 * 60, 52_400, "held"),
        ("strength_training", "Strength workout", "Upper body and core.", 54 * 60, None, "published"),
        ("walking", "Evening walk", "Recovery walk.", 38 * 60, 3_120, "excluded"),
        ("trail_running", "Forest trail run", "Technical trails and soft ground.", 68 * 60, 10_200, "pending"),
        ("hiking", "Mountain hike", "Weekend hike with a view.", 4 * 3600 + 12 * 60, 14_600, "held"),
        ("swimming", "Pool intervals", "Technique and steady intervals.", 47 * 60, None, "published"),
        ("indoor_cycling", "Indoor bike", "Tempo session indoors.", 51 * 60, 28_700, "pending"),
        ("yoga", "Yoga recovery", "Mobility and breathing session.", 36 * 60, None, "excluded"),
        ("rowing", "Rowing intervals", "Short, hard intervals.", 32 * 60, 6_400, "pending"),
        ("elliptical", "Cross trainer", "Low-impact aerobic session.", 44 * 60, None, "held"),
        ("cardio", "Cardio workout", "Mixed cardio session.", 39 * 60, None, "published"),
        # The taxonomy's longest type key. Every other sample here is short
        # enough to flatter the layout; the detail dialog only shows what it
        # does with a long category if something actually has one.
        # Published, so both service links render: the longest key and the
        # widest actions row are the same worst case the header has to survive.
        ("backcountry_skiing_snowboarding_ws", "Backcountry day", "Skinned up, rode down.",
         5 * 3600 + 20 * 60, 9_400, "published"),
    ]
    for offset in range(12):
        month_index = now.month - offset
        year = now.year + (month_index - 1) // 12
        month = (month_index - 1) % 12 + 1
        for variant in range(3):
            sample_index = (offset * 3 + variant) % len(samples)
            activity_type, title, description, duration, distance, status = samples[sample_index]
            activity_number = offset * 3 + variant
            start = now.replace(
                year=year, month=month, day=min(5 + variant * 8, 28),
                hour=7 + variant * 3,
            )
            activity_id = 900001 + activity_number
            title = f"{title} #{variant + 1}"
            data = {
                "duration": duration,
                "moving_duration": duration - 90,
                "elapsed_duration": duration + 180,
                "distance": distance,
                "elevation_gain": 120 + activity_number * 18,
                "elevation_loss": 110 + activity_number * 14,
                "calories": 420 + activity_number * 28,
                "avg_hr": 142 - activity_number % 8,
                "max_hr": 168 - activity_number % 6,
            }
            db.insert_activity(
                conn, activity_id, activity_type, title, description, start.strftime("%Y-%m-%d %H:%M:%S"),
                f"dev-{activity_id}", status, now, json.dumps(data),
            )

    _seed_hevy(conn, now)

    db.set_config_value(conn, "settings", config.DEFAULT_CONFIG)
    # One source of truth with the fake Garmin client, so a seeded dev DB
    # and a refreshed one show the same categories.
    db.set_config_value(conn, "garmin_activity_types", dev_mock.garmin_activity_types())
    db.set_config_value(conn, "dev_seed_version", SEED_VERSION)


def _mark_onboarding_complete(conn: sqlite3.Connection) -> None:
    """Skip the first-run wizard for a seeded dev DB.

    ``onboarding.setup_step`` short-circuits to "onboarding complete" the
    moment ``initial_sync_done`` is truthy, before it looks at Garmin/Strava
    connection state or the Hevy step — see
    ``src/activsync/onboarding.py::setup_step``. That single key is also
    exactly what ``/api/v1/app`` reads into ``setup.complete``
    (``src/activsync/api_routes.py``), so it is the only config key that
    needs to be set here.
    """
    db.set_config_value(conn, "initial_sync_done", True)


def _garmin_time(iso_time: str) -> str:
    """Hevy ISO timestamp → the space-separated format activities rows use."""
    return iso_time.replace("T", " ").split("+")[0]


def _seed_hevy(conn: sqlite3.Connection, now: datetime) -> None:
    """The four demo scenarios the manual dev pass must show: merge badge,
    mapping queue, passive flag, and the publish interlock on a claimed watch
    activity. Rows are seeded in their end states because the poller (and so
    the Hevy leg) does not run in mock mode."""
    workouts = {w["id"]: w for w in dev_mock.dev_hevy_workouts(now)}

    for template in dev_mock.dev_hevy_templates():
        hevy_db.upsert_template(conn, {
            "exercise_template_id": template["id"],
            "title": template["title"],
            "primary_muscle_group": template["primary_muscle_group"],
            "secondary_muscle_groups": template["secondary_muscle_groups"],
            "equipment_category": template["equipment_category"],
            "is_custom": template["is_custom"],
        })

    def seed_workout(workout: dict) -> None:
        hevy_db.upsert_workout(
            conn, workout["id"], workout["title"], workout["start_time"],
            workout["end_time"], workout["updated_at"], workout,
        )

    def seed_activity(activity_id: int, title: str, start_iso: str,
                      duration_s: int, publish_status: str) -> None:
        data = {"duration": duration_s, "avg_hr": 128, "calories": 380,
                "total_sets": 15, "total_reps": 75}
        db.insert_activity(
            conn, activity_id, "strength_training", title, "",
            _garmin_time(start_iso), f"dev-{activity_id}", publish_status, now,
            json.dumps(data),
        )

    # 1. Merged: watch activity enriched by Hevy — shows the merge badge.
    merged = workouts[dev_mock.HEVY_DEV_MERGED_ID]
    seed_activity(dev_mock.HEVY_DEV_MERGED_ACTIVITY_ID, "Gym session (watch)",
                  merged["start_time"], 3600, "pending")
    seed_workout(merged)
    hevy_db.claim_source(conn, merged["id"], dev_mock.HEVY_DEV_MERGED_ACTIVITY_ID)
    hevy_db.link_target(conn, merged["id"], dev_mock.HEVY_DEV_MERGED_ACTIVITY_ID,
                        "merge")
    hevy_db.set_workout_status(conn, merged["id"], "merged")
    hevy_db.set_applied(conn, merged["id"], "garmin", merged["updated_at"])

    # 2. Unmapped custom exercise — populates the mapping queue.
    unmapped = workouts[dev_mock.HEVY_DEV_UNMAPPED_ID]
    seed_workout(unmapped)
    hevy_db.set_workout_status(
        conn, unmapped["id"], "needs_mapping",
        error="unmapped exercises: Bulgarian Ring Row")

    # 3. Graceless passive upload — visibly flagged on its fresh activity.
    passive = workouts[dev_mock.HEVY_DEV_PASSIVE_ID]
    seed_activity(dev_mock.HEVY_DEV_PASSIVE_ACTIVITY_ID, "Forgot the watch (Hevy)",
                  passive["start_time"], 3600, "pending")
    seed_workout(passive)
    hevy_db.link_target(conn, passive["id"], dev_mock.HEVY_DEV_PASSIVE_ACTIVITY_ID,
                        "passive")
    hevy_db.set_workout_status(conn, passive["id"], "uploaded_passive")
    hevy_db.set_applied(conn, passive["id"], "garmin", passive["updated_at"])

    # 4. Midnight-spanning workout still waiting for its watch activity.
    seed_workout(workouts[dev_mock.HEVY_DEV_MIDNIGHT_ID])

    # 5. In-flight sync holding the interlock: the claimed watch activity is
    #    unpublishable until the workout resolves.
    syncing = workouts[dev_mock.HEVY_DEV_SYNCING_ID]
    seed_activity(dev_mock.HEVY_DEV_SYNCING_ACTIVITY_ID, "Strength (watch)",
                  syncing["start_time"], 3600, "pending")
    seed_workout(syncing)
    hevy_db.claim_source(conn, syncing["id"], dev_mock.HEVY_DEV_SYNCING_ACTIVITY_ID)
    hevy_db.set_workout_status(conn, syncing["id"], "syncing")
