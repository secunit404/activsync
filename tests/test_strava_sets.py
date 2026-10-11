from datetime import datetime, timezone

from garmin_fit_sdk import Decoder, Encoder, Stream

from activsync.fit_profile import mesg
from activsync.strava_sets import fit_sets_from_garmin, replace_sets, with_current_sets

START = datetime(2026, 7, 9, 9, 0, tzinfo=timezone.utc)
UNKNOWN = 65534


def _watch_fit() -> bytes:
    """A watch-style strength FIT whose only set the watch couldn't place."""
    encoder = Encoder()
    encoder.write_mesg(mesg("file_id", type="activity", manufacturer=1,
                            product=0, serial_number=1, time_created=START))
    encoder.write_mesg(mesg("record", timestamp=START, heart_rate=90))
    encoder.write_mesg(mesg(
        "set", timestamp=datetime(2026, 7, 9, 9, 0, 40, tzinfo=timezone.utc),
        start_time=START, duration=40.0, set_type="active",
        category=[UNKNOWN], category_subtype=[0], repetitions=7,
        message_index=0))
    encoder.write_mesg(mesg("record", timestamp=datetime(
        2026, 7, 9, 9, 1, tzinfo=timezone.utc), heart_rate=120))
    encoder.write_mesg(mesg("session", timestamp=datetime(
        2026, 7, 9, 9, 10, tzinfo=timezone.utc), start_time=START,
        sport="training", sub_sport="strength_training",
        total_elapsed_time=600.0, total_timer_time=600.0))
    return bytes(encoder.close())


def _decode(fit_bytes: bytes) -> dict:
    messages, errors = Decoder(Stream.from_byte_array(bytearray(fit_bytes))).read(
        convert_types_to_strings=False, enable_crc_check=True)
    assert errors == []
    return messages


GARMIN_SETS = {
    "activityId": 10,
    "exerciseSets": [
        {"exercises": [{"category": "BENCH_PRESS", "name": "BARBELL_BENCH_PRESS",
                        "probability": None}],
         "duration": 40.0, "repetitionCount": 8, "weight": 60000.0,
         "setType": "ACTIVE", "startTime": "2026-07-09T09:00:00.0",
         "wktStepIndex": 0, "messageIndex": 0},
        {"exercises": [], "duration": 75.0, "setType": "REST",
         "startTime": "2026-07-09T09:00:40.0", "wktStepIndex": 0,
         "messageIndex": 1},
        {"exercises": [{"category": "SQUAT", "name": None, "probability": None}],
         "duration": 40.0, "repetitionCount": 5, "weight": 100000.0,
         "setType": "ACTIVE", "startTime": "2026-07-09T09:01:55.0",
         "wktStepIndex": 1, "messageIndex": 2},
    ],
}


def test_garmin_sets_become_fit_sets_with_numeric_exercise_ids():
    sets = fit_sets_from_garmin(GARMIN_SETS)

    assert [s["set_type"] for s in sets] == ["active", "rest", "active"]
    bench, _, squat = sets
    assert bench["category"] == [0] and bench["category_subtype"] == [1]
    assert bench["repetitions"] == 8 and bench["weight"] == 60.0
    assert squat["category"] == [28]
    assert squat["category_subtype"] == [0xFFFF]  # no name: category only


def test_watch_guess_with_highest_probability_wins():
    sets = fit_sets_from_garmin({"exerciseSets": [{
        "exercises": [
            {"category": "CURL", "name": None, "probability": 20.0},
            {"category": "ROW", "name": None, "probability": 80.0}],
        "duration": 30.0, "repetitionCount": 10, "setType": "ACTIVE",
        "startTime": "2026-07-09T09:00:00.0"}]})

    assert sets[0]["category"] == [23]


def test_replace_sets_swaps_only_the_set_messages():
    rewritten = replace_sets(_watch_fit(), fit_sets_from_garmin(GARMIN_SETS))

    messages = _decode(rewritten)
    sets = messages["set_mesgs"]
    active = [s for s in sets if s["set_type"] == 1]  # 1 = active
    # The decoder unwraps one-element arrays to scalars.
    assert [s["category"] for s in active] == [0, 28]
    assert active[0]["category_subtype"] == 1
    assert "category_subtype" not in active[1]  # invalid → omitted
    assert active[0]["repetitions"] == 8 and active[0]["weight"] == 60.0
    assert len(messages["record_mesgs"]) == 2
    assert messages["session_mesgs"][0]["total_timer_time"] == 600.0
    assert len(messages["file_id_mesgs"]) == 1


def test_without_active_sets_the_original_bytes_are_kept():
    original = _watch_fit()

    assert with_current_sets(original, {"exerciseSets": []}) is original
    assert with_current_sets(original, None) is original


def test_unparseable_input_is_returned_unchanged():
    sets = fit_sets_from_garmin(GARMIN_SETS)

    assert replace_sets(b"FIT", sets) == b"FIT"
    truncated = _watch_fit()[:-10]
    assert replace_sets(truncated, sets) == truncated
