from datetime import datetime, timedelta, timezone

import pytest
from garmin_fit_sdk import Encoder

from activsync import db
from activsync.fit_profile import mesg
from activsync.strava_exercise_mapper import StravaMappingMiss
from activsync.strava_strength import build_strength_upload

START = datetime(2026, 10, 9, 15, 42, 38, tzinfo=timezone.utc)
CEST = timedelta(hours=2)

WORKOUT = {
    "start_time": "2026-10-09T15:42:38+00:00",
    "end_time": "2026-10-09T16:55:01+00:00",
    "exercises": [
        {"title": "Chest Press (Machine)", "exercise_template_id": "7EB3F7C3",
         "sets": [{"reps": 7, "weight_kg": 60}, {"reps": 8, "weight_kg": 60.5}]},
        {"title": "Hanging Knee Raise", "exercise_template_id": "08590920",
         "sets": [{"reps": 10, "weight_kg": None}]},
    ],
}


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def _fit(set_count=3, with_hr=True):
    encoder = Encoder()
    encoder.write_mesg(mesg("file_id", type="activity", manufacturer="garmin",
                            product=4534, time_created=START))
    if with_hr:
        for second in (0, 1, 1, 5):
            encoder.write_mesg(mesg("record", timestamp=START + timedelta(seconds=second),
                                    heart_rate=90 + second))
    for index in range(set_count):
        encoder.write_mesg(mesg(
            "set", timestamp=START, start_time=START + timedelta(minutes=3 * index),
            duration=66.5, set_type="active", message_index=index))
    encoder.write_mesg(mesg("session", timestamp=START, start_time=START,
                            total_elapsed_time=4343.0, total_timer_time=4306.0,
                            total_calories=421, sport="training",
                            sub_sport="strength_training"))
    end = START + timedelta(seconds=4343)
    encoder.write_mesg(mesg("activity", timestamp=end, local_timestamp=end + CEST,
                            num_sessions=1))
    return bytes(encoder.close())


def test_names_each_set_with_strava_exercise_and_keeps_hevy_reps_and_weight(conn):
    document = build_strength_upload(conn, WORKOUT, _fit())

    assert [
        (s["exercise_type"], s.get("repetitions"), s.get("weight"))
        for s in document["sets"]
    ] == [
        ("MACHINE_CHEST_PRESS", 7, 60.0),
        ("MACHINE_CHEST_PRESS", 8, 60.5),
        ("HANGING_KNEE_RAISE", 10, None),
    ]


def test_takes_time_heart_rate_and_calories_from_the_recording(conn):
    document = build_strength_upload(conn, WORKOUT, _fit())

    assert document["version"] == "1.0"
    assert document["start_time"] == "2026-10-09T15:42:38Z"
    assert document["utc_offset"] == 7200
    assert (document["elapsed_time"], document["active_time"]) == (4343, 4306)
    assert document["total_calories"] == 421
    assert document["creator"] == {"name": "Garmin fenix8 small"}
    assert document["streams"] == {"time": [0, 1, 5], "heartrate": [90, 91, 95]}


def test_uses_recorded_set_timing_when_the_set_counts_match(conn):
    document = build_strength_upload(conn, WORKOUT, _fit(set_count=3))

    assert [(s["start_time"], s["duration"]) for s in document["sets"]] == [
        ("2026-10-09T15:42:38Z", 66),
        ("2026-10-09T15:45:38Z", 66),
        ("2026-10-09T15:48:38Z", 66),
    ]


def test_leaves_set_timing_out_when_the_recording_has_different_sets(conn):
    document = build_strength_upload(conn, WORKOUT, _fit(set_count=5))

    assert all("start_time" not in s and "duration" not in s for s in document["sets"])


def test_falls_back_to_the_workout_window_when_the_fit_is_unreadable(conn):
    document = build_strength_upload(conn, WORKOUT, b"not a fit", tz_name="Europe/Stockholm")

    assert document["start_time"] == "2026-10-09T15:42:38Z"
    assert document["elapsed_time"] == 4343
    assert document["utc_offset"] == 7200
    assert "streams" not in document and "total_calories" not in document


def test_an_exercise_without_a_strava_exercise_is_a_miss(conn):
    workout = {**WORKOUT, "exercises": [
        {"title": "Something Unheard Of", "exercise_template_id": "CUSTOM99",
         "sets": [{"reps": 5}]},
    ]}

    with pytest.raises(StravaMappingMiss):
        build_strength_upload(conn, workout, _fit())


def test_a_workout_without_sets_is_rejected(conn):
    with pytest.raises(ValueError):
        build_strength_upload(conn, {**WORKOUT, "exercises": []}, _fit())
