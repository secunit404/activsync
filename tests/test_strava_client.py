import time
from datetime import datetime, timedelta, timezone
from unittest.mock import MagicMock, patch

import pytest
import requests

from activsync import db
from activsync.strava_client import (
    StravaAuthError,
    StravaClient,
    StravaRateLimitError,
    StravaUploadError,
    StravaWindowIncompleteError,
)


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def test_is_connected_false_when_no_tokens(conn):
    client = StravaClient(conn, "cid", "csecret")
    assert client.is_connected() is False


def test_is_connected_true_after_tokens_stored(conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "a", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    client = StravaClient(conn, "cid", "csecret")
    assert client.is_connected() is True


def test_authorize_url_includes_client_id_and_scope(conn):
    client = StravaClient(conn, "cid123", "csecret")

    url = client.authorize_url("http://localhost:8000/strava/callback", "s3cr3t-state")

    assert "client_id=cid123" in url
    assert "scope=activity:write,activity:read_all" in url
    assert "redirect_uri=http://localhost:8000/strava/callback" in url


def test_authorize_url_includes_the_csrf_state(conn):
    client = StravaClient(conn, "cid123", "csecret")

    url = client.authorize_url("http://localhost:8000/strava/callback", "s3cr3t-state")

    assert "state=s3cr3t-state" in url


@patch("activsync.strava_client.requests.post")
def test_exchange_code_stores_tokens(mock_post, conn):
    mock_post.return_value = MagicMock(
        status_code=200,
        json=lambda: {"access_token": "a1", "refresh_token": "r1", "expires_at": 12345},
    )
    mock_post.return_value.raise_for_status = lambda: None

    client = StravaClient(conn, "cid", "csecret")
    client.exchange_code("authcode")

    tokens = db.get_config_value(conn, "strava_tokens")
    assert tokens == {"access_token": "a1", "refresh_token": "r1", "expires_at": 12345}


@patch("activsync.strava_client.requests.post")
def test_get_access_token_returns_cached_token_when_not_expired(mock_post, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "still-valid", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    client = StravaClient(conn, "cid", "csecret")

    token = client._get_access_token()

    assert token == "still-valid"
    mock_post.assert_not_called()


@patch("activsync.strava_client.requests.post")
def test_get_access_token_refreshes_when_expired(mock_post, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "old", "refresh_token": "r", "expires_at": time.time() - 10,
    })
    mock_post.return_value = MagicMock(
        status_code=200,
        json=lambda: {"access_token": "fresh", "refresh_token": "r2", "expires_at": time.time() + 3600},
    )
    mock_post.return_value.raise_for_status = lambda: None

    client = StravaClient(conn, "cid", "csecret")
    token = client._get_access_token()

    assert token == "fresh"
    assert db.get_config_value(conn, "strava_tokens")["refresh_token"] == "r2"


@patch("activsync.strava_client.requests.post")
def test_get_access_token_raises_auth_error_on_revoked_refresh_token(mock_post, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "old", "refresh_token": "r", "expires_at": time.time() - 10,
    })
    mock_post.return_value = MagicMock(status_code=401)

    client = StravaClient(conn, "cid", "csecret")

    with pytest.raises(StravaAuthError):
        client._get_access_token()


def test_get_access_token_raises_auth_error_when_never_connected(conn):
    client = StravaClient(conn, "cid", "csecret")

    with pytest.raises(StravaAuthError):
        client._get_access_token()


@patch("activsync.strava_client.requests.get")
@patch("activsync.strava_client.requests.post")
def test_publish_uploads_and_returns_strava_activity_id(mock_post, mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_post.return_value = MagicMock(status_code=200, json=lambda: {"id": 555})
    mock_post.return_value.raise_for_status = lambda: None
    mock_get.return_value = MagicMock(
        status_code=200, json=lambda: {"activity_id": 9001, "error": None},
    )
    mock_get.return_value.raise_for_status = lambda: None

    client = StravaClient(conn, "cid", "csecret")
    strava_id = client.publish(garmin_activity_id=42, fit_bytes=b"FIT-DATA")

    assert strava_id == 9001
    upload_kwargs = mock_post.call_args
    assert upload_kwargs.kwargs["data"]["external_id"] == "garmin-42"
    assert upload_kwargs.kwargs["data"]["data_type"] == "fit"
    assert "name" not in upload_kwargs.kwargs["data"]


@patch("activsync.strava_client.requests.get")
@patch("activsync.strava_client.requests.post")
def test_publish_sends_activity_title_as_strava_name(mock_post, mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_post.return_value = MagicMock(status_code=200, json=lambda: {"id": 555})
    mock_post.return_value.raise_for_status = lambda: None
    mock_get.return_value = MagicMock(
        status_code=200, json=lambda: {"activity_id": 9001, "error": None},
    )
    mock_get.return_value.raise_for_status = lambda: None

    client = StravaClient(conn, "cid", "csecret")
    client.publish(garmin_activity_id=42, fit_bytes=b"FIT-DATA", name="🏃 Morning Run")

    upload_kwargs = mock_post.call_args
    assert upload_kwargs.kwargs["data"]["name"] == "🏃 Morning Run"


@patch("activsync.strava_client.requests.get")
@patch("activsync.strava_client.requests.post")
def test_publish_polls_until_activity_id_present(mock_post, mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_post.return_value = MagicMock(status_code=200, json=lambda: {"id": 555})
    mock_post.return_value.raise_for_status = lambda: None
    mock_get.side_effect = [
        MagicMock(status_code=200, json=lambda: {"activity_id": None, "error": None},
                   raise_for_status=lambda: None),
        MagicMock(status_code=200, json=lambda: {"activity_id": 9002, "error": None},
                   raise_for_status=lambda: None),
    ]

    client = StravaClient(conn, "cid", "csecret")
    strava_id = client.publish(garmin_activity_id=43, fit_bytes=b"FIT-DATA")

    assert strava_id == 9002
    assert mock_get.call_count == 2


@patch("activsync.strava_client.requests.get")
@patch("activsync.strava_client.requests.post")
def test_publish_raises_upload_error_when_strava_reports_error(mock_post, mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_post.return_value = MagicMock(status_code=200, json=lambda: {"id": 555})
    mock_post.return_value.raise_for_status = lambda: None
    mock_get.return_value = MagicMock(
        status_code=200, json=lambda: {"activity_id": None, "error": "duplicate of activity 123"},
        raise_for_status=lambda: None,
    )

    client = StravaClient(conn, "cid", "csecret")

    with pytest.raises(StravaUploadError):
        client.publish(garmin_activity_id=44, fit_bytes=b"FIT-DATA")


@patch("activsync.strava_client.requests.get")
def test_find_existing_activity_returns_id_of_matching_activity(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = MagicMock(status_code=200, json=lambda: [
        {"id": 7777, "start_date": "2026-07-09T09:00:30Z"},
    ])
    mock_get.return_value.raise_for_status = lambda: None

    client = StravaClient(conn, "cid", "csecret")
    start_time = datetime(2026, 7, 9, 9, 0, 0, tzinfo=timezone.utc)

    result = client.find_existing_activity(start_time)

    assert result == 7777
    call_kwargs = mock_get.call_args
    assert "athlete/activities" in call_kwargs.args[0]
    assert call_kwargs.kwargs["params"]["after"] < int(start_time.timestamp())
    assert call_kwargs.kwargs["params"]["before"] > int(start_time.timestamp())


@patch("activsync.strava_client.requests.get")
def test_find_existing_activity_returns_none_when_no_activities(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = MagicMock(status_code=200, json=lambda: [])
    mock_get.return_value.raise_for_status = lambda: None

    client = StravaClient(conn, "cid", "csecret")

    result = client.find_existing_activity(datetime(2026, 7, 9, 9, 0, 0, tzinfo=timezone.utc))

    assert result is None


@patch("activsync.strava_client.requests.get")
def test_find_existing_activity_returns_none_when_outside_tolerance(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    # Simulate the API returning something outside the requested window
    # (defensive — shouldn't normally happen given after/before params).
    mock_get.return_value = MagicMock(status_code=200, json=lambda: [
        {"id": 8888, "start_date": "2026-07-09T09:30:00Z"},
    ])
    mock_get.return_value.raise_for_status = lambda: None

    client = StravaClient(conn, "cid", "csecret")

    result = client.find_existing_activity(
        datetime(2026, 7, 9, 9, 0, 0, tzinfo=timezone.utc), tolerance_minutes=5,
    )

    assert result is None


@patch("activsync.strava_client.requests.get")
def test_find_existing_activity_picks_closest_match(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = MagicMock(status_code=200, json=lambda: [
        {"id": 1111, "start_date": "2026-07-09T09:04:00Z"},
        {"id": 2222, "start_date": "2026-07-09T09:00:10Z"},
    ])
    mock_get.return_value.raise_for_status = lambda: None

    client = StravaClient(conn, "cid", "csecret")

    result = client.find_existing_activity(datetime(2026, 7, 9, 9, 0, 0, tzinfo=timezone.utc))

    assert result == 2222


@patch("activsync.strava_client.requests.get")
def test_find_existing_activity_raises_auth_error_on_401(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = MagicMock(status_code=401)

    client = StravaClient(conn, "cid", "csecret")

    with pytest.raises(StravaAuthError):
        client.find_existing_activity(datetime(2026, 7, 9, 9, 0, 0, tzinfo=timezone.utc))


@patch("activsync.strava_client.requests.post")
def test_disconnect_revokes_token_and_clears_local_state(mock_post, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_post.return_value = MagicMock(status_code=200)

    client = StravaClient(conn, "cid", "csecret")
    client.disconnect()

    assert db.get_config_value(conn, "strava_tokens") is None
    assert client.is_connected() is False
    mock_post.assert_called_once()
    call_kwargs = mock_post.call_args
    assert call_kwargs.kwargs["data"]["token"] == "tok"


def test_disconnect_is_a_no_op_when_not_connected(conn):
    client = StravaClient(conn, "cid", "csecret")

    client.disconnect()

    assert client.is_connected() is False


@patch("activsync.strava_client.requests.post")
def test_disconnect_still_clears_local_state_when_revoke_request_fails(mock_post, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_post.side_effect = requests.RequestException("network error")

    client = StravaClient(conn, "cid", "csecret")
    client.disconnect()

    assert db.get_config_value(conn, "strava_tokens") is None








@patch("activsync.strava_client.requests.put")
def test_update_activity_metadata_puts_name_and_description(mock_put, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_put.return_value = MagicMock(status_code=200)
    mock_put.return_value.raise_for_status = lambda: None

    client = StravaClient(conn, "cid", "csecret")
    client.update_activity_metadata(9001, "Morning Run", "Easy Z2")

    mock_put.assert_called_once()
    call_args = mock_put.call_args
    assert call_args.args[0].endswith("/activities/9001")
    assert call_args.kwargs["headers"]["Authorization"] == "Bearer tok"
    assert call_args.kwargs["json"] == {"name": "Morning Run", "description": "Easy Z2"}


@patch("activsync.strava_client.requests.put")
def test_update_activity_metadata_raises_auth_error_on_401(mock_put, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_put.return_value = MagicMock(status_code=401)

    client = StravaClient(conn, "cid", "csecret")

    with pytest.raises(StravaAuthError):
        client.update_activity_metadata(9001, "Morning Run", "Easy Z2")


def _page(activities, status_code=200):
    response = MagicMock(status_code=status_code, json=lambda: activities, headers={})
    response.raise_for_status = lambda: None
    return response


@patch("activsync.strava_client.requests.get")
def test_list_activities_between_returns_normalized_activities(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = _page([
        {"id": 7777, "start_date": "2026-07-09T09:00:30Z"},
    ])

    client = StravaClient(conn, "cid", "csecret")
    result = client.list_activities_between(
        datetime(2026, 7, 2, tzinfo=timezone.utc), datetime(2026, 7, 9, 12, tzinfo=timezone.utc),
    )

    assert result == [
        {"id": 7777, "start_date": datetime(2026, 7, 9, 9, 0, 30, tzinfo=timezone.utc)},
    ]
    params = mock_get.call_args.kwargs["params"]
    assert params["after"] == int(datetime(2026, 7, 2, tzinfo=timezone.utc).timestamp())
    assert params["before"] == int(datetime(2026, 7, 9, 12, tzinfo=timezone.utc).timestamp())


@patch("activsync.strava_client.requests.get")
def test_list_activities_between_uses_one_request_for_a_single_short_page(mock_get, conn):
    """The whole point of the batch fetch: N activities cost ONE request, not N."""
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = _page([
        {"id": i, "start_date": "2026-07-09T09:00:30Z"} for i in range(1, 26)
    ])

    client = StravaClient(conn, "cid", "csecret")
    result = client.list_activities_between(
        datetime(2026, 7, 2, tzinfo=timezone.utc), datetime(2026, 7, 9, 12, tzinfo=timezone.utc),
    )

    assert len(result) == 25
    assert mock_get.call_count == 1


@patch("activsync.strava_client.requests.get")
def test_list_activities_between_paginates_until_a_short_page(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    full = [{"id": i, "start_date": "2026-07-09T09:00:30Z"} for i in range(200)]
    tail = [{"id": 999, "start_date": "2026-07-09T10:00:00Z"}]
    mock_get.side_effect = [_page(full), _page(tail)]

    client = StravaClient(conn, "cid", "csecret")
    result = client.list_activities_between(
        datetime(2026, 7, 2, tzinfo=timezone.utc), datetime(2026, 7, 9, 12, tzinfo=timezone.utc),
    )

    assert len(result) == 201
    assert mock_get.call_count == 2
    assert mock_get.call_args_list[0].kwargs["params"]["page"] == 1
    assert mock_get.call_args_list[1].kwargs["params"]["page"] == 2


@patch("activsync.strava_client.requests.get")
def test_list_activities_between_stops_on_an_empty_page(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    full = [{"id": i, "start_date": "2026-07-09T09:00:30Z"} for i in range(200)]
    mock_get.side_effect = [_page(full), _page([])]

    client = StravaClient(conn, "cid", "csecret")
    result = client.list_activities_between(
        datetime(2026, 7, 2, tzinfo=timezone.utc), datetime(2026, 7, 9, 12, tzinfo=timezone.utc),
    )

    assert len(result) == 200
    assert mock_get.call_count == 2


@patch("activsync.strava_client.requests.get")
def test_list_activities_between_raises_auth_error_on_401(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = MagicMock(status_code=401, headers={})

    client = StravaClient(conn, "cid", "csecret")

    with pytest.raises(StravaAuthError):
        client.list_activities_between(
            datetime(2026, 7, 2, tzinfo=timezone.utc), datetime(2026, 7, 9, tzinfo=timezone.utc),
        )


@patch("activsync.strava_client.requests.get")
def test_list_activities_between_raises_rate_limit_error_on_429(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = MagicMock(status_code=429, headers={"Retry-After": "300"})

    client = StravaClient(conn, "cid", "csecret")

    now = datetime(2026, 7, 9, 12, 7, tzinfo=timezone.utc)
    with pytest.raises(StravaRateLimitError) as excinfo:
        client.list_activities_between(
            datetime(2026, 7, 2, tzinfo=timezone.utc), datetime(2026, 7, 9, tzinfo=timezone.utc),
            now=now,
        )

    assert excinfo.value.retry_at == now + timedelta(seconds=300)


@patch("activsync.strava_client.requests.get")
def test_find_existing_activity_raises_rate_limit_error_on_429(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = MagicMock(status_code=429, headers={"Retry-After": "42"})

    client = StravaClient(conn, "cid", "csecret")

    now = datetime(2026, 7, 9, 12, 7, tzinfo=timezone.utc)
    with pytest.raises(StravaRateLimitError) as excinfo:
        client.find_existing_activity(datetime(2026, 7, 9, 9, 0, tzinfo=timezone.utc), now=now)

    assert excinfo.value.retry_at == now + timedelta(seconds=42)


@patch("activsync.strava_client.requests.post")
def test_publish_raises_rate_limit_error_on_429(mock_post, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_post.return_value = MagicMock(status_code=429, headers={"Retry-After": "60"})

    client = StravaClient(conn, "cid", "csecret")

    with pytest.raises(StravaRateLimitError):
        client.publish(garmin_activity_id=44, fit_bytes=b"FIT-DATA")


@patch("activsync.strava_client.requests.get")
def test_rate_limit_error_falls_back_to_the_next_quarter_hour_when_no_retry_after(mock_get, conn):
    """Strava's short-term quota resets on 15-minute boundaries, and 429s often
    arrive without a Retry-After header — so the wait must be derived, not zero."""
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = MagicMock(status_code=429, headers={})

    client = StravaClient(conn, "cid", "csecret")
    now = datetime(2026, 7, 9, 12, 7, 30, tzinfo=timezone.utc)

    with pytest.raises(StravaRateLimitError) as excinfo:
        client.find_existing_activity(datetime(2026, 7, 9, 9, 0, tzinfo=timezone.utc), now=now)

    # 12:07:30 -> the next reset boundary is 12:15:00.
    assert excinfo.value.retry_at == datetime(2026, 7, 9, 12, 15, tzinfo=timezone.utc)


@patch("activsync.strava_client.requests.get")
def test_rate_limit_error_ignores_an_unparseable_retry_after(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = MagicMock(status_code=429, headers={"Retry-After": "Wed, 21 Oct"})

    client = StravaClient(conn, "cid", "csecret")
    now = datetime(2026, 7, 9, 12, 7, 30, tzinfo=timezone.utc)

    with pytest.raises(StravaRateLimitError) as excinfo:
        client.find_existing_activity(datetime(2026, 7, 9, 9, 0, tzinfo=timezone.utc), now=now)

    assert excinfo.value.retry_at == datetime(2026, 7, 9, 12, 15, tzinfo=timezone.utc)


@patch("activsync.strava_client.requests.get")
def test_list_activities_between_raises_rather_than_returning_a_partial_window(mock_get, conn):
    """Callers treat absence from the window as 'deleted on Strava'. A truncated
    window would therefore flag live activities as missing, so an unfinishable
    fetch must fail loudly instead of returning what it managed to get."""
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    full = [{"id": i, "start_date": "2026-07-09T09:00:30Z"} for i in range(200)]
    mock_get.return_value = _page(full)  # every page is full: never terminates

    client = StravaClient(conn, "cid", "csecret")

    with pytest.raises(StravaWindowIncompleteError):
        client.list_activities_between(
            datetime(2026, 7, 2, tzinfo=timezone.utc), datetime(2026, 7, 9, 12, tzinfo=timezone.utc),
        )


@patch("activsync.strava_client.requests.get")
def test_rate_limit_deadline_is_absolute_and_lands_on_the_quota_reset(mock_get, conn):
    """The deadline must be an instant, not a duration.

    A duration is only meaningful against the clock that produced it: the
    fallback is computed when the 429 lands, but the poller applies it against
    its (older) tick clock, so the pause used to end early — right before the
    quota reset, guaranteeing another 429.
    """
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = MagicMock(status_code=429, headers={})

    client = StravaClient(conn, "cid", "csecret")
    reset = datetime(2026, 7, 9, 12, 15, tzinfo=timezone.utc)

    # However stale the clock reading, the deadline is the same instant.
    for seconds_late in (0, 3, 59):
        observed_at = datetime(2026, 7, 9, 12, 7, 0, tzinfo=timezone.utc) + timedelta(seconds=seconds_late)
        with pytest.raises(StravaRateLimitError) as excinfo:
            client.find_existing_activity(
                datetime(2026, 7, 9, 9, 0, tzinfo=timezone.utc), now=observed_at,
            )
        assert excinfo.value.retry_at == reset


@patch("activsync.strava_client.requests.get")
def test_rate_limit_deadline_honours_the_retry_after_header(mock_get, conn):
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "tok", "refresh_token": "r", "expires_at": time.time() + 3600,
    })
    mock_get.return_value = MagicMock(status_code=429, headers={"Retry-After": "300"})

    client = StravaClient(conn, "cid", "csecret")
    now = datetime(2026, 7, 9, 12, 7, 0, tzinfo=timezone.utc)

    with pytest.raises(StravaRateLimitError) as excinfo:
        client.find_existing_activity(datetime(2026, 7, 9, 9, 0, tzinfo=timezone.utc), now=now)

    assert excinfo.value.retry_at == now + timedelta(seconds=300)
