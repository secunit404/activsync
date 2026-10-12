"""Map Hevy exercises to Strava's own strength ``exercise_type`` ids.

Strava's JSON upload names each set's exercise directly from its library
(`strava_exercises`), so a Hevy workout reaches Strava without going through
Garmin's FIT exercise ids — which have no machine variants and which Strava
only partly translates. Resolution order: the user's choice, the built-in
template table, then the exercise's Garmin mapping when Strava has a matching
or generic id.
"""

from __future__ import annotations

import sqlite3
from datetime import datetime, timezone

from activsync.fit_profile import CATEGORY_NAMES, SUBCATEGORY_NAMES
from activsync.hevy_mapper import MappingMiss, lookup_exercise
from activsync.hevy_strava_map import TEMPLATE_TO_STRAVA
from activsync.strava_exercises import EXERCISE_TYPES

SOURCE_USER = "user"
SOURCE_STANDARD = "standard"
SOURCE_GARMIN = "garmin"


class StravaMappingMiss(Exception):
    """An exercise with no Strava exercise_type to upload it as."""

    def __init__(self, title: str, template_id: str | None):
        super().__init__(
            f"no Strava exercise for {title!r} (template {template_id})")
        self.title = title
        self.template_id = template_id


def lookup_strava_exercise(
    conn: sqlite3.Connection, title: str, template_id: str | None
) -> tuple[str, str]:
    """Resolve an exercise to (exercise_type, source) or raise StravaMappingMiss."""
    if template_id:
        user = get_strava_mapping(conn, template_id)
        if user in EXERCISE_TYPES:
            return user, SOURCE_USER
    resolved = default_strava_exercise(conn, title, template_id)
    if resolved is None:
        raise StravaMappingMiss(title, template_id)
    return resolved


def default_strava_exercise(
    conn: sqlite3.Connection, title: str, template_id: str | None
) -> tuple[str, str] | None:
    """The resolution without the user's choice — what "Reset" restores."""
    standard = TEMPLATE_TO_STRAVA.get(template_id) if template_id else None
    if standard in EXERCISE_TYPES:
        return standard, SOURCE_STANDARD
    derived = _from_garmin(conn, title, template_id)
    return (derived, SOURCE_GARMIN) if derived is not None else None


def _from_garmin(
    conn: sqlite3.Connection, title: str, template_id: str | None
) -> str | None:
    """Strava's id for the exercise's Garmin mapping: the same exercise name
    when Strava has it, else the Garmin category's generic id."""
    try:
        category, subcategory, _ = lookup_exercise(conn, title, template_id)
    except MappingMiss:
        return None
    name = SUBCATEGORY_NAMES.get(category, {}).get(subcategory)
    if name in EXERCISE_TYPES:
        return name
    generic = f"{CATEGORY_NAMES.get(category)}_GENERIC"
    return generic if generic in EXERCISE_TYPES else None


def save_strava_mapping(
    conn: sqlite3.Connection, template_id: str, exercise_type: str
) -> None:
    """Create or replace a user's Strava exercise for a template."""
    if exercise_type not in EXERCISE_TYPES:
        raise ValueError(f"unknown Strava exercise type {exercise_type!r}")
    now = datetime.now(timezone.utc).isoformat()
    conn.execute(
        """INSERT INTO strava_exercise_mappings
           (exercise_template_id, exercise_type, created_at, updated_at)
           VALUES (?, ?, ?, ?)
           ON CONFLICT(exercise_template_id) DO UPDATE SET
               exercise_type = excluded.exercise_type,
               updated_at = excluded.updated_at""",
        (template_id, exercise_type, now, now),
    )
    conn.commit()


def get_strava_mapping(conn: sqlite3.Connection, template_id: str) -> str | None:
    row = conn.execute(
        "SELECT exercise_type FROM strava_exercise_mappings"
        " WHERE exercise_template_id = ?",
        (template_id,),
    ).fetchone()
    return row["exercise_type"] if row else None


def delete_strava_mapping(conn: sqlite3.Connection, template_id: str) -> None:
    conn.execute(
        "DELETE FROM strava_exercise_mappings WHERE exercise_template_id = ?",
        (template_id,),
    )
    conn.commit()
