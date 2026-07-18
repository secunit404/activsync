"""Tests for HR sourcing (watch FIT extraction, passive slice, merge)."""

from datetime import datetime, timezone
from pathlib import Path

from activsync import hr_sources
from activsync.hr_sources import extract_fit_hr, merge_hr_sources, passive_hr_for_window


class StubGarmin:
    """Records requested dates; serves canned daily-HR payloads per date."""

    def __init__(self, payloads):
        self.payloads = payloads
        self.requested = []

    def get_daily_heart_rates(self, date_str):
        self.requested.append(date_str)
        return self.payloads.get(date_str, {})


def ms(dt: datetime) -> int:
    return int(dt.timestamp() * 1000)


# -- passive_hr_for_window --------------------------------------------------

def test_midnight_window_fetches_both_dates():
    start = datetime(2026, 7, 17, 23, 30, tzinfo=timezone.utc)
    end = datetime(2026, 7, 18, 0, 30, tzinfo=timezone.utc)
    garmin = StubGarmin({
        "2026-07-17": {"heartRateValues": [[ms(start) + 60_000, 100]]},
        "2026-07-18": {"heartRateValues": [[ms(end) - 60_000, 110]]},
    })
    samples = passive_hr_for_window(garmin, start, end)
    assert garmin.requested == ["2026-07-17", "2026-07-18"]
    assert [s["hr"] for s in samples] == [100, 110]


def test_slice_keeps_only_in_window_samples():
    start = datetime(2026, 7, 17, 10, 0, tzinfo=timezone.utc)
    end = datetime(2026, 7, 17, 11, 0, tzinfo=timezone.utc)
    garmin = StubGarmin({
        "2026-07-17": {"heartRateValues": [
            [ms(start) - 3_600_000, 70],   # an hour early — dropped
            [ms(start) - 30_000, 95],      # inside the -60 s buffer — kept
            [ms(start) + 600_000, 120],    # in window — kept
            [ms(end) + 30_000, 105],       # inside the +60 s buffer — kept
            [ms(end) + 3_600_000, 80],     # an hour late — dropped
        ]},
    })
    samples = passive_hr_for_window(garmin, start, end)
    assert [s["hr"] for s in samples] == [95, 120, 105]
    assert samples[0]["time"] == 0.0  # pre-start clamps to zero


def test_malformed_daily_payload_returns_empty():
    start = datetime(2026, 7, 17, 10, 0, tzinfo=timezone.utc)
    end = datetime(2026, 7, 17, 11, 0, tzinfo=timezone.utc)
    for payload in ({}, {"heartRateValues": None},
                    {"heartRateValues": "bogus"}, "not-a-dict"):
        garmin = StubGarmin({"2026-07-17": payload})
        assert passive_hr_for_window(garmin, start, end) == []


def test_fetch_failure_returns_empty():
    class Exploding:
        def get_daily_heart_rates(self, date_str):
            raise RuntimeError("garmin down")

    start = datetime(2026, 7, 17, 10, 0, tzinfo=timezone.utc)
    end = datetime(2026, 7, 17, 11, 0, tzinfo=timezone.utc)
    assert passive_hr_for_window(Exploding(), start, end) == []


# -- merge_hr_sources -------------------------------------------------------

def test_merge_primary_wins_secondary_fills():
    primary = [{"time": 10.0, "hr": 150}]
    secondary = [{"time": 10.0, "hr": 90}, {"time": 200.0, "hr": 95}]
    merged = merge_hr_sources(primary, secondary)
    assert {(s["time"], s["hr"]) for s in merged} == {(10.0, 150), (200.0, 95)}


def test_merge_handles_empty_sides():
    only = [{"time": 5.0, "hr": 100}]
    assert merge_hr_sources([], only) == only
    assert merge_hr_sources(only, []) == only
    assert merge_hr_sources(None, None) == []


# -- extract_fit_hr ---------------------------------------------------------

def test_extract_fit_hr_from_generated_fit(tmp_path):
    from activsync.fit_builder import DeviceIdentity, Profile, ResolvedExercise, build_fit

    workout = {"title": "T", "start_time": "2026-07-17T06:00:00Z",
               "end_time": "2026-07-17T06:30:00Z"}
    hr = [{"time": 0.0, "hr": 100}, {"time": 60.0, "hr": 130}]
    resolved = [ResolvedExercise("Bench", 0, 1, [{"reps": 5}])]
    out = str(tmp_path / "t.fit")
    build_fit(workout, resolved, hr, Profile(80.0, 1990, 45.0, "male"),
              DeviceIdentity(1, 0, 1), out)

    start_ms = int(datetime(2026, 7, 17, 6, 0, tzinfo=timezone.utc).timestamp() * 1000)
    samples = extract_fit_hr(Path(out).read_bytes(), start_ms)
    assert [(s["time"], s["hr"]) for s in samples] == [(0.0, 100), (60.0, 130)]


def test_extract_fit_hr_garbage_returns_empty():
    assert extract_fit_hr(b"garbage", 0) == []
