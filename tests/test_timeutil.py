from activsync import timeutil


def test_format_local_time_converts_to_stockholm_summer_time():
    result = timeutil.format_local_time("2026-07-09 09:00:00", "Europe/Stockholm")
    assert result == "2026-07-09 11:00"


def test_format_local_time_converts_to_stockholm_winter_time():
    result = timeutil.format_local_time("2026-01-09 09:00:00", "Europe/Stockholm")
    assert result == "2026-01-09 10:00"


def test_format_local_time_converts_to_other_timezone():
    result = timeutil.format_local_time("2026-07-09 20:00:00", "America/New_York")
    assert result == "2026-07-09 16:00"


def test_is_valid_timezone_accepts_known_zone():
    assert timeutil.is_valid_timezone("Europe/Stockholm") is True


def test_is_valid_timezone_rejects_unknown_zone():
    assert timeutil.is_valid_timezone("Not/AZone") is False
