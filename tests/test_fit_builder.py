"""Tests for strength FIT generation."""

from pathlib import Path

import pytest

from activsync import fit_builder
from activsync.fit_builder import (
    DEVELOPMENT_IDENTITY,
    GENERIC_GARMIN_IDENTITY,
    DeviceIdentity,
    Profile,
    ResolvedExercise,
    build_fit,
    identity_from_config,
    identity_from_fit,
    keytel_kcal_per_min,
)

MALE = Profile(weight_kg=80.0, birth_year=1990, vo2max=45.0, sex="male")
FEMALE = Profile(weight_kg=80.0, birth_year=1990, vo2max=45.0, sex="female")

WORKOUT = {
    "id": "w1",
    "title": "Push Day",
    "start_time": "2026-07-17T06:00:00Z",
    "end_time": "2026-07-17T07:00:00Z",
}

RESOLVED = [
    ResolvedExercise(title="Bench Press (Barbell)", category=0, subcategory=1, sets=[
        {"type": "warmup", "reps": 10, "weight_kg": 40.0},
        {"type": "normal", "reps": 8, "weight_kg": 80.0},
        {"type": "normal", "reps": 8, "weight_kg": 80.0},
    ]),
    ResolvedExercise(title="Squat (Barbell)", category=28, subcategory=6, sets=[
        {"type": "normal", "reps": 5, "weight_kg": 100.0},
        {"type": "normal", "reps": 5, "weight_kg": 100.0},
    ]),
]

HR = [{"time": float(i * 60), "hr": 110 + i} for i in range(30)]

IDENTITY = DeviceIdentity(manufacturer=1, product=4534, serial=987654321)


def build(tmp_path, workout=WORKOUT, resolved=RESOLVED, hr=HR,
          profile=MALE, identity=IDENTITY):
    out = str(tmp_path / "out.fit")
    return build_fit(workout, resolved, hr, profile, identity, out)


def test_builds_nonempty_file_and_summary(tmp_path):
    result = build(tmp_path)
    path = Path(result["output_path"])
    assert path.exists() and path.stat().st_size > 0
    assert result["exercises"] == 2
    assert result["total_sets"] == 5
    assert result["duration_s"] == 3600.0
    assert result["calories"] > 0


def test_calories_differ_by_sex(tmp_path):
    male = build(tmp_path, profile=MALE)
    female = build(tmp_path, profile=FEMALE)
    assert male["calories"] != female["calories"]
    assert female["calories"] < male["calories"]  # female constants are lower


def test_keytel_female_equation():
    # Direct check of the female Keytel constants
    kcal = keytel_kcal_per_min(140, FEMALE, age=36)
    expected = (-59.3954 + 0.45 * 140 + 0.380 * 45.0 + 0.103 * 80.0 + 0.274 * 36) / 4.184
    assert abs(kcal - expected) < 1e-9


def test_timestamped_hr_lands_in_avg(tmp_path):
    result = build(tmp_path)
    hr_values = [s["hr"] for s in HR]
    assert result["avg_hr"] == round(sum(hr_values) / len(hr_values))


def test_no_hr_yields_none_avg_and_default_calories(tmp_path):
    result = build(tmp_path, hr=None)
    assert result["avg_hr"] is None
    assert result["calories"] > 0  # default-HR calories


def test_missing_start_raises(tmp_path):
    workout = dict(WORKOUT, start_time=None)
    with pytest.raises(ValueError):
        build(tmp_path, workout=workout)


def test_nonpositive_duration_raises(tmp_path):
    workout = dict(WORKOUT, end_time=WORKOUT["start_time"])
    with pytest.raises(ValueError):
        build(tmp_path, workout=workout)
    workout = dict(WORKOUT, end_time="2026-07-17T05:00:00Z")  # ends before start
    with pytest.raises(ValueError):
        build(tmp_path, workout=workout)


def test_unknown_category_rejected(tmp_path):
    bad = [ResolvedExercise("Mystery", 65534, 0, [{"reps": 5}])]
    with pytest.raises(ValueError):
        build(tmp_path, resolved=bad)


def test_all_timestamps_stay_within_activity_window(tmp_path):
    # Short workout (5 min) with many sets → the 0.3 scale floor would push
    # synthetic sets past the end; HR beyond the end must be dropped too.
    workout = dict(WORKOUT, end_time="2026-07-17T06:05:00Z")
    hr = [{"time": 0.0, "hr": 100}, {"time": 290.0, "hr": 120},
          {"time": 350.0, "hr": 130}]  # 350 s > 300 s duration
    result = build(tmp_path, workout=workout, hr=hr)

    from fit_tool.fit_file import FitFile
    fit = FitFile.from_file(result["output_path"])
    end_ms = None
    timestamps = []
    for record in fit.records:
        message = record.message
        name = type(message).__name__
        if name == "SessionMessage":
            end_ms = message.timestamp
        if name in ("RecordMessage", "SetMessage"):
            timestamps.append(message.timestamp)
    assert end_ms is not None
    assert timestamps and all(ts <= end_ms for ts in timestamps)


def test_identity_fields_land_in_file(tmp_path):
    from fit_tool.fit_file import FitFile
    from fit_tool.profile.messages.file_id_message import FileIdMessage

    result = build(tmp_path)
    fit = FitFile.from_file(result["output_path"])
    file_id = next(r.message for r in fit.records
                   if isinstance(r.message, FileIdMessage))
    assert file_id.manufacturer == 1
    assert file_id.serial_number == 987654321


def test_identity_round_trips_through_fit_bytes(tmp_path):
    result = build(tmp_path)
    fit_bytes = Path(result["output_path"]).read_bytes()
    detected = identity_from_fit(fit_bytes)
    assert detected == IDENTITY


def test_identity_from_fit_garbage_returns_none():
    assert identity_from_fit(b"not a fit file") is None


def test_identity_from_config():
    assert identity_from_config({}) == GENERIC_GARMIN_IDENTITY
    assert identity_from_config({"hevy_device_identity": None}) == GENERIC_GARMIN_IDENTITY
    cfg = {"hevy_device_identity": {"manufacturer": 1, "product": 4534, "serial": 42}}
    assert identity_from_config(cfg) == DeviceIdentity(1, 4534, 42)
    # malformed stored value degrades to the generic fallback
    assert identity_from_config({"hevy_device_identity": {"manufacturer": 1}}) \
        == GENERIC_GARMIN_IDENTITY


def test_identity_constants():
    assert GENERIC_GARMIN_IDENTITY.manufacturer == 1
    assert GENERIC_GARMIN_IDENTITY.product == 0
    assert DEVELOPMENT_IDENTITY.manufacturer == 255
