import io
import zipfile
from datetime import datetime, timedelta, timezone
from unittest.mock import MagicMock, patch

import pytest

from activsync.garmin_client import (
    ActivityRecord,
    GarminClient,
    MfaRequired,
    begin_login,
    complete_login,
)


def _garmin_activity(activity_id, type_key, name, description, start_time_gmt):
    return {
        "activityId": activity_id,
        "activityName": name,
        "description": description,
        "activityType": {"typeKey": type_key},
        "startTimeGMT": start_time_gmt,
    }


def test_fetch_recent_activities_maps_fields():
    recent = (datetime.now(timezone.utc) - timedelta(days=1)).strftime("%Y-%m-%d %H:%M:%S")
    raw_client = MagicMock()
    raw_client.get_activities.return_value = [
        _garmin_activity(1, "strength_training", "Strength Training", "", recent),
    ]

    client = GarminClient(raw_client)
    records = client.fetch_recent_activities(lookback_days=3)

    assert records == [
        ActivityRecord(
            garmin_activity_id=1,
            activity_type="strength_training",
            title="Strength Training",
            description="",
            start_time=recent,
        )
    ]


def test_fetch_recent_activities_stops_at_lookback_cutoff():
    now = datetime.now(timezone.utc)
    recent = (now - timedelta(days=1)).strftime("%Y-%m-%d %H:%M:%S")
    old = (now - timedelta(days=30)).strftime("%Y-%m-%d %H:%M:%S")

    raw_client = MagicMock()
    raw_client.get_activities.side_effect = [
        [_garmin_activity(1, "running", "Run", "", recent),
         _garmin_activity(2, "running", "Old Run", "", old)],
    ]

    client = GarminClient(raw_client)
    records = client.fetch_recent_activities(lookback_days=3)

    assert [r.garmin_activity_id for r in records] == [1]


def test_fetch_recent_activities_handles_missing_description():
    recent = (datetime.now(timezone.utc) - timedelta(days=1)).strftime("%Y-%m-%d %H:%M:%S")
    raw_client = MagicMock()
    raw_client.get_activities.return_value = [
        _garmin_activity(1, "running", "Run", None, recent),
    ]

    client = GarminClient(raw_client)
    records = client.fetch_recent_activities(lookback_days=3)

    assert records[0].description == ""


def test_download_fit_extracts_fit_from_zip():
    fit_bytes = b"FIT-FILE-CONTENTS"
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        zf.writestr("12345.fit", fit_bytes)
    zip_bytes = buf.getvalue()

    raw_client = MagicMock()
    raw_client.download_activity.return_value = zip_bytes

    client = GarminClient(raw_client)
    result = client.download_fit(12345)

    assert result == fit_bytes


def test_download_fit_raises_when_no_fit_in_zip():
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        zf.writestr("readme.txt", b"nothing here")

    raw_client = MagicMock()
    raw_client.download_activity.return_value = buf.getvalue()

    client = GarminClient(raw_client)

    try:
        client.download_fit(12345)
        assert False, "expected ValueError"
    except ValueError:
        pass


@patch("activsync.garmin_client._limiter.call")
def test_update_activity_metadata_sets_name_and_description(mock_limiter_call):
    mock_limiter_call.side_effect = lambda fn, *args, **kwargs: fn(*args, **kwargs)
    raw_client = MagicMock()

    client = GarminClient(raw_client)
    client.update_activity_metadata(12345, "Morning Run", "Easy Z2")

    raw_client.set_activity_name.assert_called_once_with(12345, "Morning Run")
    raw_client.set_activity_description.assert_called_once_with(12345, "Easy Z2")


@patch("activsync.garmin_client.GarminAuth")
def test_begin_login_returns_client_on_success(mock_garmin_auth_cls):
    mock_auth = MagicMock()
    mock_client = MagicMock()
    mock_auth.login.return_value = mock_client
    mock_garmin_auth_cls.return_value = mock_auth

    result = begin_login("me@example.com", "pw", "/tmp/tokens")

    assert result is mock_client
    mock_garmin_auth_cls.assert_called_once_with(
        email="me@example.com", password="pw", token_dir="/tmp/tokens", return_on_mfa=True,
    )


@patch("activsync.garmin_client.GarminAuth")
def test_begin_login_raises_mfa_required_with_pending_auth(mock_garmin_auth_cls):
    mock_auth = MagicMock()
    mock_auth.login.return_value = "needs_mfa"
    mock_garmin_auth_cls.return_value = mock_auth

    with pytest.raises(MfaRequired) as exc_info:
        begin_login("me@example.com", "pw", "/tmp/tokens")

    assert exc_info.value.pending_auth is mock_auth


def test_complete_login_calls_resume_login_on_pending_auth():
    mock_auth = MagicMock()
    mock_client = MagicMock()
    mock_auth.resume_login.return_value = mock_client

    result = complete_login(mock_auth, "123456")

    assert result is mock_client
    mock_auth.resume_login.assert_called_once_with("123456")


def test_fetch_activity_types_maps_dedupes_and_sorts_by_label():
    raw_client = MagicMock()
    raw_client.get_activity_types.return_value = [
        {"typeKey": "all", "typeId": 0},
        {"typeKey": "running", "typeId": 1},
        {"typeKey": "cycling", "typeId": 2},
        {"typeKey": "running", "typeId": 1},
    ]

    client = GarminClient(raw_client)
    types = client.fetch_activity_types()

    assert types == [
        {"type_key": "cycling", "label": "Cycling"},
        {"type_key": "running", "label": "Running"},
    ]
