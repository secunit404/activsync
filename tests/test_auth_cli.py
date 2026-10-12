import time

import pytest

from activsync import auth_cli, auth_core, auth_devices, db

PASSWORD = "correct horse battery staple"


@pytest.fixture
def conn(tmp_path, monkeypatch):
    path = tmp_path / "cli.db"
    monkeypatch.setenv("ACTIVSYNC_DB_PATH", str(path))
    return db.connect(str(path))


def _answers(monkeypatch, *values):
    replies = iter(values)
    monkeypatch.setattr(auth_cli.getpass, "getpass", lambda prompt="": next(replies))


def test_reset_password_sets_it_and_signs_everyone_out(conn, monkeypatch, capsys):
    auth_core.set_password(conn, PASSWORD, time.time())
    auth_core.create_session(conn, method="password", ip="", user_agent="", now=time.time())
    for _ in range(7):
        auth_core.record_failure(conn, time.time())
    _answers(monkeypatch, "a brand new passphrase", "a brand new passphrase")

    assert auth_cli.main(["reset-password"]) == 0

    assert auth_core.verify_password(conn, "a brand new passphrase") is True
    assert auth_core.list_sessions(conn) == []
    assert auth_core.throttle_wait(conn, time.time()) == 0
    assert "signed out" in capsys.readouterr().out


def test_reset_password_forgets_trusted_devices(conn, monkeypatch):
    auth_core.set_password(conn, PASSWORD, time.time())
    device_id = auth_devices.trust_device(conn, time.time())
    _answers(monkeypatch, "a brand new passphrase", "a brand new passphrase")

    assert auth_cli.main(["reset-password"]) == 0

    assert auth_devices.is_trusted(conn, device_id) is False


def test_reset_password_refuses_a_mismatch_or_a_short_password(conn, monkeypatch):
    auth_core.set_password(conn, PASSWORD, time.time())
    device_id = auth_devices.trust_device(conn, time.time())

    _answers(monkeypatch, "a brand new passphrase", "something else entirely")
    assert auth_cli.main(["reset-password"]) == 1
    _answers(monkeypatch, "short")
    assert auth_cli.main(["reset-password"]) == 1
    assert auth_core.verify_password(conn, PASSWORD) is True
    assert auth_devices.is_trusted(conn, device_id) is True


def test_revoke_sessions(conn, capsys):
    for _ in range(2):
        auth_core.create_session(conn, method="password", ip="", user_agent="", now=time.time())

    assert auth_cli.main(["revoke-sessions"]) == 0

    assert auth_core.list_sessions(conn) == []
    assert "Signed out 2 session(s)." in capsys.readouterr().out


def test_revoke_sessions_forgets_trusted_devices_like_sign_out_everywhere(conn):
    device_id = auth_devices.trust_device(conn, time.time())

    assert auth_cli.main(["revoke-sessions"]) == 0

    assert auth_devices.is_trusted(conn, device_id) is False


def test_status_never_prints_the_hash(conn, capsys):
    auth_core.set_password(conn, PASSWORD, time.time())

    assert auth_cli.main(["status"]) == 0

    out = capsys.readouterr().out
    assert "Owner claimed:        yes" in out
    assert "Active sessions:      0" in out
    assert db.get_config_value(conn, "auth_owner")["password_hash"] not in out


def test_db_path_follows_the_server(monkeypatch, tmp_path):
    monkeypatch.delenv("ACTIVSYNC_DB_PATH", raising=False)
    monkeypatch.delenv("G2S_DB_PATH", raising=False)
    assert db.resolve_path(mock_mode=False) == "/config/activsync.db"
    monkeypatch.setenv("G2S_DB_PATH", str(tmp_path / "legacy.db"))
    assert db.resolve_path(mock_mode=False) == str(tmp_path / "legacy.db")


def test_disable_strava_login(conn, capsys):
    auth_core.save_settings(conn, strava_login=True)

    assert auth_cli.main(["disable-strava-login"]) == 0

    assert auth_core.load_settings(conn)["strava_login"] is False
    auth_cli.main(["status"])
    assert "Sign in with Strava:  off" in capsys.readouterr().out
