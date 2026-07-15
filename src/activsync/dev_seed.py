"""Sample data used only by the local development server."""

from __future__ import annotations

import json
import sqlite3
from datetime import datetime, timedelta, timezone

from activsync import config, db

SEED_VERSION = 4


def seed(conn: sqlite3.Connection) -> None:
    """Insert repeatable, multi-month activity data into the dev database."""
    if db.get_config_value(conn, "dev_seed_version") == SEED_VERSION:
        return
    existing = db.list_activities(conn)
    if existing:
        # Rebuild an older version of this generated-only database if needed;
        # never touch a database containing non-dev activity IDs.
        if not all(str(row["content_hash"]).startswith("dev-") for row in existing):
            return
        conn.execute("DELETE FROM activities")
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

    db.set_config_value(conn, "settings", config.DEFAULT_CONFIG)
    db.set_config_value(conn, "garmin_activity_types", [
        {"type_key": key, "label": label}
        for key, label in [
            ("running", "Running"), ("trail_running", "Trail Running"),
            ("cycling", "Cycling"), ("indoor_cycling", "Indoor Cycling"),
            ("walking", "Walking"), ("hiking", "Hiking"),
            ("swimming", "Swimming"), ("rowing", "Rowing"),
            ("strength_training", "Strength Training"), ("yoga", "Yoga"),
            ("elliptical", "Elliptical"), ("cardio", "Cardio"),
            ("tennis", "Tennis"), ("basketball", "Basketball"),
            ("golf", "Golf"), ("skiing", "Skiing"),
        ]
    ])
    db.set_config_value(conn, "dev_seed_version", SEED_VERSION)
