"""Map Hevy exercises to Garmin FIT (category, subcategory) ids.

The tables themselves live in `hevy_name_map`; this module is the lookup over
them. ActivSync's lookup differs from upstream hevy2garmin: user mappings live
in the exercise_mappings table (not a JSON file), a miss raises MappingMiss
instead of returning the UNKNOWN sentinel (the mapping gate parks the workout;
UNKNOWN never reaches Garmin), and suggest_mapping powers the mapping queue's
pre-selected suggestions.
"""

from __future__ import annotations

import difflib
import re
import sqlite3

from activsync import hevy_db
from activsync.hevy_name_map import HEVY_TO_GARMIN, TEMPLATE_OVERRIDES
from activsync.hevy_template_map import TEMPLATE_TO_GARMIN

UNKNOWN_CATEGORY = 65534


class MappingMiss(Exception):
    """An exercise with no known Garmin mapping — routes to the mapping gate."""

    def __init__(self, title: str, template_id: str | None):
        super().__init__(f"no Garmin mapping for exercise {title!r} (template {template_id})")
        self.title = title
        self.template_id = template_id


# --------------------------------------------------------------------------- #


# --------------------------------------------------------------------------- #
# Lookup
# --------------------------------------------------------------------------- #

def lookup_exercise(
    conn: sqlite3.Connection, title: str, template_id: str | None
) -> tuple[int, int, str]:
    """Resolve an exercise to (category, subcategory, display_name).

    Order: user mapping -> template-id table (corrections first) -> built-in
    English-name table. A miss raises MappingMiss; this function never returns
    the UNKNOWN sentinel.
    """
    # Both ported tables contain UNKNOWN (65534) entries — exercises even
    # upstream could not place. Every resolution source filters them: the
    # sentinel behaves as a miss, so it can never reach Garmin.
    if template_id:
        user = hevy_db.get_mapping(conn, template_id)
        if user is not None and user["category"] != UNKNOWN_CATEGORY:
            return (user["category"], user["subcategory"], title)
    pair = lookup_standard_mapping(title, template_id)
    if pair is not None:
        return (pair[0], pair[1], title)
    raise MappingMiss(title, template_id)


def lookup_standard_mapping(
    title: str, template_id: str | None
) -> tuple[int, int] | None:
    """Return the ported standard-table pair, excluding the UNKNOWN sentinel.

    This deliberately ignores saved user mappings. The mapping UI uses it to
    decide whether an override can truthfully offer "Reset to standard".
    """
    if template_id:
        pair = TEMPLATE_OVERRIDES.get(template_id) or TEMPLATE_TO_GARMIN.get(template_id)
        if pair is not None and pair[0] != UNKNOWN_CATEGORY:
            return pair
    pair = HEVY_TO_GARMIN.get(title)
    if pair is not None and pair[0] != UNKNOWN_CATEGORY:
        return pair
    return None


# --------------------------------------------------------------------------- #
# Suggestions for the mapping queue
# --------------------------------------------------------------------------- #

_MUSCLE_GROUP_FALLBACK: dict[str, int] = {
    "chest": 0,          # BENCH_PRESS
    "lats": 23,          # ROW
    "upper_back": 23,    # ROW
    "biceps": 7,         # CURL
    "triceps": 30,       # TRICEPS_EXTENSION
    "quadriceps": 28,    # SQUAT
    "hamstrings": 15,    # LEG_CURL
    "glutes": 10,        # HIP_RAISE
    "shoulders": 24,     # SHOULDER_PRESS
    "abdominals": 5,     # CORE
    "calves": 1,         # CALF_RAISE
    "cardio": 2,         # CARDIO
}


def _normalize(title: str) -> str:
    return re.sub(r"[^a-z0-9 ]", "", title.lower()).strip()


_NORMALIZED_NAME_TABLE = {
    _normalize(name): pair
    for name, pair in HEVY_TO_GARMIN.items()
    if pair[0] != UNKNOWN_CATEGORY  # never suggest the sentinel
}


def suggest_mapping(title: str, template: dict | None) -> tuple[int, int] | None:
    """Best-guess (category, subcategory) for an unmapped exercise: fuzzy
    match against the name table first, then the template's muscle group.
    None when there is no signal — the picker starts blank."""
    normalized = _normalize(title)
    if normalized:
        close = difflib.get_close_matches(
            normalized, _NORMALIZED_NAME_TABLE.keys(), n=1, cutoff=0.6)
        if close:
            return _NORMALIZED_NAME_TABLE[close[0]]
    if template:
        muscle = template.get("primary_muscle_group")
        category = _MUSCLE_GROUP_FALLBACK.get(muscle or "")
        if category is not None:
            return (category, 0)
    return None
