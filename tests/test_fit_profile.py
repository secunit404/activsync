"""Tests for the FIT profile tables and the encoder message helper."""

import pytest

from activsync import fit_profile
from activsync.fit_profile import (
    CATEGORY_NAMES,
    SUBCATEGORY_NAMES,
    mesg,
    subcategory_name,
)


def test_category_names_cover_fit_categories():
    assert CATEGORY_NAMES[0] == "BENCH_PRESS"
    assert CATEGORY_NAMES[28] == "SQUAT"
    assert CATEGORY_NAMES[65534] == "UNKNOWN"


def test_subcategory_names_resolve():
    assert SUBCATEGORY_NAMES[0][1] == "BARBELL_BENCH_PRESS"
    # unresolvable pair → absent (payload builder then sends null name)
    assert 999 not in SUBCATEGORY_NAMES.get(0, {})
    assert subcategory_name(0, 1) == "BARBELL_BENCH_PRESS"
    assert subcategory_name(0, 999) is None


def test_names_carry_no_python_identifier_artefacts():
    """The old binding prefixed digit-leading names with N (N45_DEGREE_PLANK)
    because they had to be Python identifiers. Garmin's own name is what goes
    into the exerciseSets payload, so the prefix must be gone."""
    assert SUBCATEGORY_NAMES[19][0] == "45_DEGREE_PLANK"


def test_machine_categories_use_garmin_names():
    """These were hand-transcribed while the old profile lacked them, and
    several were wrong (47 was "STAIR_MACHINE", 52 "TREADMILL")."""
    assert CATEGORY_NAMES[47] == "STAIR_STEPPER"
    assert CATEGORY_NAMES[52] == "RUN_INDOOR"


def test_mesg_resolves_message_number_and_keeps_fields():
    built = mesg("set", timestamp=1, repetitions=8)

    assert built == {"mesg_num": fit_profile.MESG_NUM["set"],
                     "timestamp": 1, "repetitions": 8}


def test_mesg_rejects_unknown_field():
    """The encoder silently drops keys it cannot match to a profile field, so
    a typo would cost a missing field in an already-uploaded activity."""
    with pytest.raises(ValueError, match="workout_step_index"):
        mesg("set", workout_step_index=0)


def test_mesg_rejects_unknown_message():
    with pytest.raises(ValueError, match="not_a_message"):
        mesg("not_a_message", timestamp=1)


def test_garmin_is_manufacturer_one():
    assert fit_profile.GARMIN_MANUFACTURER == 1
    assert fit_profile.GARMIN_PRODUCT_NAMES[2050] == "FENIX3"
