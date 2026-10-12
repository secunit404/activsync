import pytest

from activsync import db, hevy_db
from activsync.hevy_strava_map import TEMPLATE_TO_STRAVA
from activsync.strava_exercise_mapper import (
    SOURCE_GARMIN,
    SOURCE_STANDARD,
    SOURCE_USER,
    StravaMappingMiss,
    delete_strava_mapping,
    get_strava_mapping,
    lookup_strava_exercise,
    save_strava_mapping,
    default_strava_exercise,
)
from activsync.strava_exercises import EXERCISE_TYPES

CHEST_PRESS_MACHINE = "7EB3F7C3"
GARMIN_DUMBBELL_ROW = (23, 2)
GARMIN_SWISS_BALL_CHEST_PRESS = (0, 0)


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def test_every_standard_entry_is_a_strava_exercise_or_none():
    invalid = {
        template_id: exercise_type
        for template_id, exercise_type in TEMPLATE_TO_STRAVA.items()
        if exercise_type is not None and exercise_type not in EXERCISE_TYPES
    }
    assert not invalid


def test_standard_table_maps_machine_exercises_to_machine_variants(conn):
    assert lookup_strava_exercise(conn, "Chest Press (Machine)", CHEST_PRESS_MACHINE) == (
        "MACHINE_CHEST_PRESS", SOURCE_STANDARD)


def test_user_choice_overrides_the_standard_table(conn):
    save_strava_mapping(conn, CHEST_PRESS_MACHINE, "MACHINE_ISOLATERAL_CHEST_PRESS")

    assert lookup_strava_exercise(conn, "Chest Press (Machine)", CHEST_PRESS_MACHINE) == (
        "MACHINE_ISOLATERAL_CHEST_PRESS", SOURCE_USER)
    assert default_strava_exercise(conn, "Chest Press (Machine)", CHEST_PRESS_MACHINE) == (
        "MACHINE_CHEST_PRESS", SOURCE_STANDARD)


def test_deleting_the_user_choice_restores_the_standard(conn):
    save_strava_mapping(conn, CHEST_PRESS_MACHINE, "MACHINE_ISOLATERAL_CHEST_PRESS")
    delete_strava_mapping(conn, CHEST_PRESS_MACHINE)

    assert get_strava_mapping(conn, CHEST_PRESS_MACHINE) is None
    assert lookup_strava_exercise(conn, "Chest Press (Machine)", CHEST_PRESS_MACHINE)[0] == (
        "MACHINE_CHEST_PRESS")


def test_saving_an_id_strava_does_not_have_is_rejected(conn):
    with pytest.raises(ValueError):
        save_strava_mapping(conn, CHEST_PRESS_MACHINE, "MACHINE_CHEST_PRES")


def test_custom_exercise_uses_its_garmin_exercise_when_strava_has_it(conn):
    hevy_db.save_mapping(conn, "CUSTOM01", *GARMIN_DUMBBELL_ROW)

    assert lookup_strava_exercise(conn, "My Row", "CUSTOM01") == ("DUMBBELL_ROW", SOURCE_GARMIN)


def test_custom_exercise_falls_back_to_the_garmin_category_generic(conn):
    hevy_db.save_mapping(conn, "CUSTOM02", *GARMIN_SWISS_BALL_CHEST_PRESS)

    assert lookup_strava_exercise(conn, "My Press", "CUSTOM02") == (
        "BENCH_PRESS_GENERIC", SOURCE_GARMIN)


def test_unmapped_custom_exercise_is_a_miss(conn):
    with pytest.raises(StravaMappingMiss):
        lookup_strava_exercise(conn, "Something Unheard Of", "CUSTOM03")
