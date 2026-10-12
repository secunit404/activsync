import math
import threading
import time

import pytest

from activsync import auth_core, auth_devices, db

PASSWORD = "correct horse battery staple"
NOW = 1_800_000_000.0


@pytest.fixture
def conn(tmp_path):
    connection = db.connect(str(tmp_path / "devices.db"))
    auth_core.set_password(connection, PASSWORD, NOW)
    return connection


def _devices(conn):
    return db.get_config_value(conn, "auth_trusted_devices")


def _exhaust_shared_bucket(conn, now=NOW):
    for _ in range(20):
        auth_core.record_failure(conn, now)


def test_trusting_a_device_stores_only_its_hash(conn):
    device_id = auth_devices.trust_device(conn, NOW)

    stored = _devices(conn)
    assert len(stored) == 1
    assert stored[0]["hash"] == auth_core.hash_token(device_id)
    assert device_id not in str(stored)
    assert stored[0]["failures"] == 0
    assert auth_devices.is_trusted(conn, device_id) is True
    assert auth_devices.is_trusted(conn, "unknown") is False
    assert auth_devices.is_trusted(conn, None) is False


def test_only_the_newest_twenty_devices_are_kept(conn):
    ids = [auth_devices.trust_device(conn, NOW + i) for i in range(21)]

    assert len(_devices(conn)) == 20
    assert auth_devices.is_trusted(conn, ids[0]) is False
    assert all(auth_devices.is_trusted(conn, device_id) for device_id in ids[1:])


def test_re_trusting_replaces_the_previous_cookie(conn):
    first = auth_devices.trust_device(conn, NOW)

    second = auth_devices.trust_device(conn, NOW + 1, replacing=first)

    assert auth_devices.is_trusted(conn, first) is False
    assert auth_devices.is_trusted(conn, second) is True
    assert len(_devices(conn)) == 1


def test_clearing_forgets_every_device(conn):
    ids = [auth_devices.trust_device(conn, NOW) for _ in range(3)]

    auth_devices.clear_trusted_devices(conn)

    assert not any(auth_devices.is_trusted(conn, device_id) for device_id in ids)
    assert _devices(conn) == []


def test_shared_attempts_are_counted_before_verifying(conn):
    waits = [auth_core.reserve_attempt(conn, NOW) for _ in range(7)]

    assert waits == [0, 0, 0, 0, 0, 1, 1]
    assert db.get_config_value(conn, "auth_login_throttle")["failures"] == 5


def test_device_counters_back_off_like_the_shared_bucket(conn):
    device_id = auth_devices.trust_device(conn, NOW)
    waits = []
    now = NOW
    for _ in range(9):
        assert auth_devices.reserve_device_attempt(conn, device_id, now) == 0
        next_allowed_at = _devices(conn)[0]["next_allowed_at"]
        waits.append(math.ceil(next_allowed_at - now) if next_allowed_at else 0)
        now += waits[-1]

    assert waits == [0, 0, 0, 0, 1, 2, 4, 8, 16]
    assert auth_devices.reserve_device_attempt(conn, device_id, now - 1) == 1
    assert _devices(conn)[0]["failures"] == 9


def test_a_trusted_device_ignores_an_exhausted_shared_bucket(conn):
    device_id = auth_devices.trust_device(conn, NOW)
    _exhaust_shared_bucket(conn)

    anonymous = auth_devices.check_password(conn, PASSWORD, NOW)
    trusted = auth_devices.check_password(conn, PASSWORD, NOW, device_id=device_id)

    assert anonymous.verified is False and anonymous.retry_after == 900
    assert trusted.verified is True and trusted.retry_after == 0
    assert db.get_config_value(conn, "auth_login_throttle")["failures"] == 20


def test_device_success_resets_only_the_device_counters(conn):
    device_id = auth_devices.trust_device(conn, NOW)
    for _ in range(3):
        auth_devices.check_password(conn, "wrong", NOW, device_id=device_id)
        auth_core.record_failure(conn, NOW)

    assert _devices(conn)[0]["failures"] == 3
    assert auth_devices.check_password(conn, PASSWORD, NOW, device_id=device_id).verified

    assert _devices(conn)[0]["failures"] == 0
    assert db.get_config_value(conn, "auth_login_throttle")["failures"] == 3


def test_an_unknown_device_falls_back_to_the_shared_bucket(conn):
    _exhaust_shared_bucket(conn)

    result = auth_devices.check_password(conn, PASSWORD, NOW, device_id="forged")

    assert result.verified is False
    assert result.retry_after == 900


def test_ten_device_failures_drop_trust(conn):
    device_id = auth_devices.trust_device(conn, NOW)
    now = NOW
    for attempt in range(10):
        assert auth_devices.is_trusted(conn, device_id), attempt
        result = auth_devices.check_password(conn, "wrong", now, device_id=device_id)
        assert result.retry_after == 0
        now += auth_core.MAX_DELAY_SECONDS

    assert auth_devices.is_trusted(conn, device_id) is False
    _exhaust_shared_bucket(conn, now)
    after = auth_devices.check_password(conn, PASSWORD, now, device_id=device_id)
    assert after.verified is False
    assert after.retry_after > 0


def _race(conn, monkeypatch, attempts, **kwargs):
    verified = []
    lock = threading.Lock()

    def slow_wrong(*_args):
        with lock:
            verified.append(time.monotonic())
        time.sleep(0.05)
        return False

    monkeypatch.setattr(auth_core, "verify_password", slow_wrong)
    barrier = threading.Barrier(attempts)
    results = []

    def guess():
        barrier.wait()
        result = auth_devices.check_password(conn, "wrong", NOW, **kwargs)
        with lock:
            results.append(result)

    threads = [threading.Thread(target=guess) for _ in range(attempts)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    return verified, results


def test_parallel_guesses_cannot_slip_past_the_shared_throttle(conn, monkeypatch):
    verified, results = _race(conn, monkeypatch, 20)

    failures = db.get_config_value(conn, "auth_login_throttle")["failures"]
    assert failures == len(verified) == auth_core.FREE_FAILURES
    assert sum(1 for result in results if result.retry_after) == 20 - failures
    assert not any(result.verified for result in results)


def test_parallel_guesses_cannot_slip_past_a_device_throttle(conn, monkeypatch):
    device_id = auth_devices.trust_device(conn, NOW)

    verified, results = _race(conn, monkeypatch, 20, device_id=device_id)

    assert _devices(conn)[0]["failures"] == len(verified) == auth_core.FREE_FAILURES
    assert sum(1 for result in results if result.retry_after) == 15
    assert db.get_config_value(conn, "auth_login_throttle") is None
