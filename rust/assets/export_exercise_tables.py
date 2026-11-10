"""Write exercise_tables.json from the Python app's tables.

Run from the repository root after a FIT profile or mapping-table change:
    .venv/bin/python -I rust/assets/export_exercise_tables.py
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "src"))

from activsync import fit_profile, hevy_name_map, hevy_strava_map, hevy_template_map, strava_exercises  # noqa: E402


def pairs(table):
    return {key: list(value) for key, value in sorted(table.items())}


tables = {
    "categories": [[value, name] for value, name in sorted(fit_profile.CATEGORY_NAMES.items())],
    "subcategories": [
        [category, [[value, name] for value, name in sorted(names.items())]]
        for category, names in sorted(fit_profile.SUBCATEGORY_NAMES.items())
    ],
    # Kept in source order: suggestions build a normalised-name table from it
    # where a later duplicate wins.
    "hevyToGarmin": [[name, list(pair)] for name, pair in hevy_name_map.HEVY_TO_GARMIN.items()],
    "templateOverrides": pairs(hevy_name_map.TEMPLATE_OVERRIDES),
    "templateToGarmin": pairs(hevy_template_map.TEMPLATE_TO_GARMIN),
    "templateToStrava": dict(sorted(hevy_strava_map.TEMPLATE_TO_STRAVA.items())),
    "stravaGroups": [[group, list(types)] for group, types in sorted(strava_exercises.EXERCISE_GROUPS.items())],
    "stravaTypes": sorted(strava_exercises.EXERCISE_TYPES),
}

out = Path(__file__).with_name("exercise_tables.json")
out.write_text(json.dumps(tables, ensure_ascii=False, separators=(",", ":")) + "\n")
print(f"wrote {out.relative_to(ROOT)}")
