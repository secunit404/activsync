from unittest.mock import MagicMock, patch

import pytest
from garminconnect import GarminConnectTooManyRequestsError

from activsync.rate_limit import RateLimiter


@patch("activsync.rate_limit.time.sleep")
def test_call_returns_result_and_paces_the_next_call(mock_sleep):
    limiter = RateLimiter(delay=1.5)
    func = MagicMock(return_value="ok")

    assert limiter.call(func, 1, key="v") == "ok"

    func.assert_called_once_with(1, key="v")
    mock_sleep.assert_called_once_with(1.5)


@patch("activsync.rate_limit.time.sleep")
def test_call_retries_a_429_with_growing_backoff(mock_sleep):
    limiter = RateLimiter(delay=1.0, max_retries=3, base_wait=30)
    func = MagicMock(side_effect=[
        GarminConnectTooManyRequestsError("429"),
        GarminConnectTooManyRequestsError("429"),
        "ok",
    ])

    assert limiter.call(func) == "ok"

    assert func.call_count == 3
    assert [c.args[0] for c in mock_sleep.call_args_list] == [30, 60, 1.0]


@patch("activsync.rate_limit.time.sleep")
def test_call_reraises_the_original_429_once_retries_are_exhausted(mock_sleep):
    """The original carries the response other error handling reads."""
    limiter = RateLimiter(max_retries=2, base_wait=30)
    final = GarminConnectTooManyRequestsError("429")
    func = MagicMock(side_effect=[GarminConnectTooManyRequestsError("429"), final])

    with pytest.raises(GarminConnectTooManyRequestsError) as exc_info:
        limiter.call(func)

    assert exc_info.value is final
    assert [c.args[0] for c in mock_sleep.call_args_list] == [30]


@patch("activsync.rate_limit.time.sleep")
def test_call_does_not_retry_other_errors(mock_sleep):
    limiter = RateLimiter()
    func = MagicMock(side_effect=ValueError("boom"))

    with pytest.raises(ValueError):
        limiter.call(func)

    assert func.call_count == 1
    mock_sleep.assert_not_called()


def test_max_retries_must_allow_at_least_one_attempt():
    with pytest.raises(ValueError, match="at least 1"):
        RateLimiter(max_retries=0)
