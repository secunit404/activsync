import logging

import pytest
from argon2 import PasswordHasher

from activsync import auth_core, db

PASSWORD = "correct horse battery staple"
NOW = 1_800_000_000.0


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "auth.db"))


def test_password_policy_bounds_length_without_composition_rules():
    assert auth_core.password_problem("x" * 14) is not None
    assert auth_core.password_problem("x" * 15) is None
    assert auth_core.password_problem("x" * 256) is None
    assert auth_core.password_problem("x" * 257) is not None


def test_set_password_stores_an_argon2id_hash_not_the_password(conn):
    auth_core.set_password(conn, PASSWORD, NOW)

    owner = db.get_config_value(conn, "auth_owner")
    assert owner["password_hash"].startswith("$argon2id$")
    assert PASSWORD not in owner["password_hash"]
    assert owner["password_changed_at"] == int(NOW)
    assert auth_core.owner_claimed(conn) is True


def test_set_password_refuses_a_short_password(conn):
    with pytest.raises(ValueError):
        auth_core.set_password(conn, "too short", NOW)
    assert auth_core.owner_claimed(conn) is False


def test_verify_password_accepts_only_the_right_password(conn):
    auth_core.set_password(conn, PASSWORD, NOW)

    assert auth_core.verify_password(conn, PASSWORD) is True
    assert auth_core.verify_password(conn, PASSWORD + "!") is False


def test_verify_password_without_an_owner_is_false(conn):
    assert auth_core.verify_password(conn, PASSWORD) is False


def test_verify_password_upgrades_outdated_parameters(conn):
    old = PasswordHasher(time_cost=2, memory_cost=16, parallelism=1).hash(PASSWORD)
    db.set_config_value(conn, "auth_owner", {"password_hash": old, "password_changed_at": 1})

    assert auth_core.verify_password(conn, PASSWORD) is True

    upgraded = db.get_config_value(conn, "auth_owner")["password_hash"]
    assert upgraded != old
    assert auth_core.verify_password(conn, PASSWORD) is True
    assert db.get_config_value(conn, "auth_owner")["password_hash"] == upgraded


def test_throttle_allows_four_free_failures_then_doubles_up_to_fifteen_minutes(conn):
    delays = []
    for _ in range(16):
        auth_core.record_failure(conn, NOW)
        delays.append(auth_core.throttle_wait(conn, NOW))

    assert delays[:4] == [0, 0, 0, 0]
    assert delays[4:10] == [1, 2, 4, 8, 16, 32]
    assert delays[-1] == 900


def test_throttle_wait_expires_and_reset_clears_it(conn):
    for _ in range(6):
        auth_core.record_failure(conn, NOW)

    assert auth_core.throttle_wait(conn, NOW) == 2
    assert auth_core.throttle_wait(conn, NOW + 2) == 0
    auth_core.reset_throttle(conn)
    auth_core.record_failure(conn, NOW)
    assert auth_core.throttle_wait(conn, NOW) == 0


def test_attempt_limiter_allows_ten_per_minute_per_client():
    limiter = auth_core.AttemptLimiter()

    allowed = [limiter.retry_after("203.0.113.7", NOW + i) for i in range(10)]
    blocked = limiter.retry_after("203.0.113.7", NOW + 10)

    assert allowed == [0] * 10
    assert blocked == 50
    assert limiter.retry_after("198.51.100.4", NOW + 10) == 0
    assert limiter.retry_after("203.0.113.7", NOW + 61) == 0


def test_setup_code_is_logged_and_checked_case_and_dash_insensitively(conn, caplog):
    codes = auth_core.SetupCode(conn)
    with caplog.at_level(logging.WARNING, logger="activsync.auth"):
        codes.ensure()

    code = codes.current
    assert code and len(code) == 9 and code[4] == "-"
    assert set(code.replace("-", "")) <= set(auth_core.SETUP_CODE_ALPHABET)
    assert f"ActivSync is unclaimed. Open the app and enter setup code {code}" in caplog.text
    assert codes.check(code.replace("-", "").lower()) is True


def test_five_wrong_setup_codes_issue_a_new_one(conn, caplog):
    codes = auth_core.SetupCode(conn)
    codes.ensure()
    first = codes.current

    for _ in range(4):
        assert codes.check("WRONG-CODE") is False
    assert codes.current == first
    with caplog.at_level(logging.WARNING, logger="activsync.auth"):
        assert codes.check("WRONG-CODE") is False

    assert codes.current != first
    assert codes.check(first) is False
    assert codes.current in caplog.text


def test_claimed_install_issues_no_setup_code(conn, caplog):
    auth_core.set_password(conn, PASSWORD, NOW)
    codes = auth_core.SetupCode(conn)

    with caplog.at_level(logging.WARNING, logger="activsync.auth"):
        codes.ensure()

    assert codes.current is None
    assert "setup code" not in caplog.text


def test_disabled_auth_issues_no_setup_code(conn, monkeypatch):
    monkeypatch.setenv("ACTIVSYNC_AUTH_DISABLED", "1")
    codes = auth_core.SetupCode(conn)

    codes.ensure()

    assert codes.current is None
