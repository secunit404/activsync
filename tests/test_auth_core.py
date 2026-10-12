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
    groups = code.split("-")
    assert len(groups) == 4 and all(len(group) == 4 for group in groups)
    assert set(code.replace("-", "")) <= set(auth_core.SETUP_CODE_ALPHABET)
    assert f"ActivSync is unclaimed. Open the app and enter setup code {code}" in caplog.text
    assert codes.check(code.replace("-", "").lower()) is True


def test_wrong_setup_codes_never_rotate_the_logged_code(conn):
    codes = auth_core.SetupCode(conn)
    codes.ensure()
    first = codes.current

    for _ in range(50):
        assert codes.check("WRONG-CODE") is False

    assert codes.current == first
    assert codes.check(first) is True


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


def _session(conn, now=NOW, method="password"):
    return auth_core.create_session(
        conn, method=method, ip="203.0.113.7", user_agent="ua", now=now
    )


def test_sessions_store_only_the_token_hash(conn):
    token = _session(conn)

    rows = conn.execute("SELECT * FROM auth_sessions").fetchall()
    assert len(rows) == 1
    assert rows[0]["token_hash"] == auth_core.hash_token(token)
    assert token not in [str(value) for value in dict(rows[0]).values()]
    assert rows[0]["public_id"] != rows[0]["token_hash"]


def test_session_idle_timeout(conn):
    token = _session(conn)
    idle = auth_core.SESSION_IDLE_SECONDS

    assert auth_core.resolve_session(conn, token, NOW + idle - 1) is not None
    token = _session(conn)
    assert auth_core.resolve_session(conn, token, NOW + idle) is None
    assert auth_core.resolve_session(conn, token, NOW) is None


def test_session_absolute_timeout_despite_activity(conn):
    token = _session(conn)
    day = 24 * 3600
    for elapsed in range(day, auth_core.SESSION_ABSOLUTE_SECONDS, day):
        assert auth_core.resolve_session(conn, token, NOW + elapsed) is not None

    assert (
        auth_core.resolve_session(conn, token, NOW + auth_core.SESSION_ABSOLUTE_SECONDS)
        is None
    )


def test_last_seen_is_written_at_most_every_five_minutes(conn):
    token = _session(conn)

    soon = auth_core.resolve_session(conn, token, NOW + 299)
    later = auth_core.resolve_session(conn, token, NOW + 300)

    assert soon.last_seen_at == int(NOW)
    assert later.last_seen_at == int(NOW + 300)
    stored = conn.execute("SELECT last_seen_at FROM auth_sessions").fetchone()[0]
    assert stored == int(NOW + 300)


def test_revoke_one_and_revoke_all_except_current(conn):
    first, second, third = _session(conn), _session(conn), _session(conn)
    second_public = auth_core.resolve_session(conn, second, NOW).public_id

    assert auth_core.revoke_public_id(conn, second_public) is True
    assert auth_core.revoke_public_id(conn, second_public) is False
    assert auth_core.revoke_all(conn, keep_token_hash=auth_core.hash_token(first)) == 1
    assert auth_core.resolve_session(conn, first, NOW) is not None
    assert auth_core.resolve_session(conn, third, NOW) is None
    assert auth_core.revoke_all(conn) == 1


def test_prune_drops_idle_and_expired_sessions(conn):
    _session(conn, now=NOW - auth_core.SESSION_IDLE_SECONDS)
    keep = _session(conn)

    assert auth_core.prune_expired_sessions(conn, NOW) == 1
    assert auth_core.resolve_session(conn, keep, NOW) is not None


def test_settings_default_and_merge(conn):
    assert auth_core.load_settings(conn) == {"password_form": "shown", "strava_login": False}

    saved = auth_core.save_settings(conn, password_form="hidden")

    assert saved == {"password_form": "hidden", "strava_login": False}
    assert auth_core.load_settings(conn) == saved


def test_the_legacy_purge_leaves_auth_sessions_alone(tmp_path):
    path = str(tmp_path / "reopen.db")
    conn = db.connect(path)
    token = _session(conn)

    reopened = db.connect(path)

    assert auth_core.resolve_session(reopened, token, NOW) is not None


def test_disabled_auth_on_a_network_bind_logs_an_error(monkeypatch, caplog):
    monkeypatch.setenv("ACTIVSYNC_AUTH_DISABLED", "1")
    with caplog.at_level(logging.ERROR, logger="activsync.auth"):
        auth_core.warn_if_disabled_on_network("127.0.0.1")
        assert caplog.text == ""
        auth_core.warn_if_disabled_on_network("0.0.0.0")

    assert "Authentication is disabled" in caplog.text
