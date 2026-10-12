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
