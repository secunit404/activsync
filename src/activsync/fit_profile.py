"""The FIT profile, read from Garmin's official SDK.

Every profile lookup in the app goes through here: message/field definitions
for the encoder, the exercise category and name tables that back the mapping
UI and the Garmin ``exerciseSets`` payload, and the device pickers. One module
owns the SDK import so a profile bump is a dependency bump and nothing else.

Names are upper-cased because that is the form Garmin's Connect API expects in
``exerciseSets`` and the form the UI labels are derived from; the SDK itself
spells them lower-case.
"""

from __future__ import annotations

from garmin_fit_sdk.profile import Profile as _Profile

_MESSAGES = _Profile["messages"]
_TYPES = _Profile["types"]

# FIT global message number by name ("set" -> 225).
MESG_NUM: dict[str, int] = {
    definition["name"]: number for number, definition in _MESSAGES.items()
}

PROFILE_VERSION = "{major}.{minor}".format(**_Profile["version"])


def _names(type_name: str) -> dict[int, str]:
    """A FIT enum as {value: UPPER_NAME}."""
    return {value: name.upper() for value, name in _TYPES[type_name].items()}


def mesg(name: str, **fields) -> dict:
    """Build one encoder message, rejecting field names the profile lacks.

    The Encoder takes plain dicts and *silently drops* keys it cannot match to
    a profile field, so a typo would otherwise cost a missing field in an
    already-uploaded activity. Fields left as None are passed through and
    dropped by the encoder, which is how optional values stay optional.
    """
    number = MESG_NUM.get(name)
    if number is None:
        raise ValueError(f"unknown FIT message {name!r}")
    known = {field["name"] for field in _MESSAGES[number]["fields"].values()}
    unknown = set(fields) - known
    if unknown:
        raise ValueError(
            f"FIT message {name!r} has no field(s) {sorted(unknown)}"
        )
    return {"mesg_num": number, **fields}


# -- exercise tables --------------------------------------------------------

CATEGORY_NAMES: dict[int, str] = _names("exercise_category")

# Every category has its own name enum, keyed <category>_exercise_name.
SUBCATEGORY_NAMES: dict[int, dict[int, str]] = {
    value: _names(f"{name}_exercise_name")
    for value, name in _TYPES["exercise_category"].items()
    if f"{name}_exercise_name" in _TYPES
}


def subcategory_name(category: int, subcategory: int) -> str | None:
    """Garmin's enum name for the pair, or None when unresolvable. Callers
    must send a null exercise name in that case — Garmin renders an
    unrecognised *name* as "Unknown", but accepts null under a valid
    category."""
    return SUBCATEGORY_NAMES.get(category, {}).get(subcategory)


# -- device identity tables -------------------------------------------------

MANUFACTURER_NAMES: dict[int, str] = _names("manufacturer")
GARMIN_PRODUCT_NAMES: dict[int, str] = _names("garmin_product")
GARMIN_MANUFACTURER = next(
    value for value, name in MANUFACTURER_NAMES.items() if name == "GARMIN"
)
