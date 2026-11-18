"""Write fit_profile.json: the slice of Garmin's FIT profile the strength
FIT builder encodes, taken from the SDK the Python app encodes with.

Run from the repository root after a garmin-fit-sdk bump:
    .venv/bin/python -I rust/assets/export_fit_profile.py
"""

import json
from pathlib import Path

from garmin_fit_sdk import fit as FIT
from garmin_fit_sdk.profile import Profile

MESSAGES = ["file_id", "sport", "exercise_title", "event", "record", "set", "lap", "session", "activity"]
# Enum types the builder passes by name; everything else goes in as a number.
NAMED_TYPES = ["file", "sport", "sub_sport", "event", "event_type", "set_type", "activity"]


def number(value, default):
    if isinstance(value, list):
        return value[0] if value else default
    return default if value is None else value


by_name = {definition["name"]: (num, definition) for num, definition in Profile["messages"].items()}
messages = {}
for name in MESSAGES:
    num, definition = by_name[name]
    fields = {}
    for field in definition["fields"].values():
        many = len(field["components"]) > 1
        fields[field["name"]] = {
            "num": field["num"],
            "baseType": FIT.FIELD_TYPE_TO_BASE_TYPE[field["base_type"]],
            "type": field["type"],
            "scale": 1 if many else number(field["scale"], 1),
            "offset": 0 if many else number(field["offset"], 0),
        }
    messages[name] = {"num": num, "fields": fields}

profile = {
    "version": Profile["version"]["major"] * 1000 + Profile["version"]["minor"],
    "baseTypes": {
        str(base): {key: d[key] for key in ("size", "invalid", "endian_flag", "signed")} | {"float": d["type_code"] in ("f", "d"), "string": d["type_code"] == "s"}
        for base, d in FIT.BASE_TYPE_DEFINITIONS.items()
    },
    "numericTypes": FIT.NUMERIC_FIELD_TYPES,
    "messages": messages,
    "types": {name: {value: str(key) for key, value in Profile["types"][name].items()} for name in NAMED_TYPES},
}
out = Path(__file__).with_name("fit_profile.json")
out.write_text(json.dumps(profile, indent=1, sort_keys=True) + "\n")
print(f"wrote {out}")
