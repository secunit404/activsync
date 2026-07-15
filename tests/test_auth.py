from datetime import datetime, timedelta, timezone
from unittest.mock import MagicMock

import pytest

from activsync import auth, db


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def test_is_configured_false_before_password_set(conn):
    assert auth.is_configured(conn) is False


def test_is_configured_true_after_password_set(conn):
    auth.set_password(conn, "correct horse")
    assert auth.is_configured(conn) is True


def test_verify_password_accepts_correct_password(conn):
    auth.set_password(conn, "correct horse")
    assert auth.verify_password(conn, "correct horse") is True


def test_verify_password_rejects_wrong_password(conn):
    auth.set_password(conn, "correct horse")
    assert auth.verify_password(conn, "wrong password") is False


def test_verify_password_false_when_not_configured(conn):
    assert auth.verify_password(conn, "anything") is False


def test_password_hash_uses_pbkdf2_with_stored_params(conn):
    auth.set_password(conn, "correct horse")

    record = db.get_config_value(conn, "auth")
    assert record["algo"] == "pbkdf2_sha256"
    assert record["iterations"] >= 1000
    # The plaintext must never be recoverable from what we store.
    assert "correct horse" not in str(record)


def _request_with_cookie(value):
    request = MagicMock()
    request.cookies = {auth.SESSION_COOKIE: value} if value is not None else {}
    return request


def test_is_logged_in_true_with_session_token(conn):
    auth.set_password(conn, "correct horse")
    token = auth.create_session(conn)

    assert auth.is_logged_in(conn, _request_with_cookie(token)) is True


def test_is_logged_in_false_with_wrong_cookie(conn):
    auth.set_password(conn, "correct horse")
    auth.create_session(conn)

    assert auth.is_logged_in(conn, _request_with_cookie("not-a-real-token")) is False


def test_is_logged_in_false_with_no_cookie(conn):
    auth.set_password(conn, "correct horse")
    auth.create_session(conn)

    assert auth.is_logged_in(conn, _request_with_cookie(None)) is False


def test_is_logged_in_false_when_not_configured(conn):
    assert auth.is_logged_in(conn, _request_with_cookie("anything")) is False


def test_expired_session_is_rejected(conn):
    auth.set_password(conn, "correct horse")
    past = datetime.now(timezone.utc) - auth.SESSION_TTL - timedelta(minutes=1)
    token = auth.create_session(conn, now=past)

    assert auth.is_logged_in(conn, _request_with_cookie(token)) is False


def test_set_password_revokes_existing_sessions(conn):
    auth.set_password(conn, "correct horse")
    token = auth.create_session(conn)
    assert auth.is_logged_in(conn, _request_with_cookie(token)) is True

    auth.set_password(conn, "new password")

    assert auth.is_logged_in(conn, _request_with_cookie(token)) is False


def test_destroy_session_logs_user_out(conn):
    auth.set_password(conn, "correct horse")
    token = auth.create_session(conn)

    auth.destroy_session(conn, _request_with_cookie(token))

    assert auth.is_logged_in(conn, _request_with_cookie(token)) is False


def test_tokens_are_unique_per_session(conn):
    auth.set_password(conn, "correct horse")

    assert auth.create_session(conn) != auth.create_session(conn)
