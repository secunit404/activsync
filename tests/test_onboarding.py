import time

import pytest

from activsync import auth_core, db, onboarding

FULL_SCOPE = "read,activity:write,activity:read_all"


@pytest.fixture
def conn(tmp_path):
    conn = db.connect(str(tmp_path / "onboarding.db"))
    auth_core.set_password(conn, "correct horse battery staple", time.time())
    db.set_config_value(conn, "garmin_credentials_verified", True)
    return conn


def _store_tokens(conn, **extra):
    db.set_config_value(
        conn,
        "strava_tokens",
        {"access_token": "a", "refresh_token": "r", "expires_at": 0, **extra},
    )


def test_a_strava_grant_without_upload_access_keeps_setup_on_the_strava_step(conn):
    _store_tokens(conn, scope="read")

    assert onboarding.setup_step(conn) == "strava"


def test_a_grant_missing_only_read_all_keeps_setup_on_the_strava_step(conn):
    _store_tokens(conn, scope="read,activity:write")

    assert onboarding.setup_step(conn) == "strava"


def test_the_full_strava_grant_moves_setup_on_to_hevy(conn):
    _store_tokens(conn, scope=FULL_SCOPE)

    assert onboarding.setup_step(conn) == "hevy"


def test_tokens_stored_before_scopes_were_recorded_move_setup_on(conn):
    _store_tokens(conn)

    assert onboarding.setup_step(conn) == "hevy"


def test_no_strava_tokens_stay_on_the_strava_step(conn):
    assert onboarding.setup_step(conn) == "strava"
