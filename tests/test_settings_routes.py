import os
from datetime import datetime, timezone
from unittest.mock import MagicMock

from fastapi.testclient import TestClient

from activsync import config, db
from activsync import server as server_module
from activsync.garmin_client import MfaRequired
from activsync.server import create_app
from activsync.strava_client import StravaAuthError


def _logged_in_client(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))
    return conn, client


def _empty_catch_up_stats():
    """A zero-activity CatchUpStats, matching what the real catch_up_sync
    returns on a quiet reconnect. _run_catch_up reads .garmin/.status off the
    result, so a bare None (fine before Task 11 started consuming the return
    value) no longer stands in for "nothing happened"."""
    from activsync import sync
    return sync.CatchUpStats(
        garmin=sync.GarminSyncStats(), status=sync.StatusCheckStats(), lookback_days=0,
    )


def _mark_garmin_connected(conn):
    db.set_config_value(conn, "garmin_credentials", {
        "email": "me@example.com", "password": "hunter2",
    })
    db.set_config_value(conn, "garmin_credentials_verified", True)


def _mark_strava_connected(conn):
    db.set_config_value(conn, "strava_credentials", {
        "client_id": "cid", "client_secret": "csecret",
    })
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "access", "refresh_token": "refresh", "expires_at": 4102444800,
    })


def _setup_done(conn):
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)


def _pending_oauth_state(conn, state="test-oauth-state"):
    """Put the app in the state /strava/connect leaves behind: an in-flight
    OAuth handshake whose state the callback expects to see echoed back."""
    db.set_config_value(conn, "strava_oauth_state", state)
    return state


def test_settings_redirects_to_setup_when_incomplete(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    response = client.get("/settings", follow_redirects=False)
    assert response.status_code == 303
    assert response.headers["location"] == "/setup"


def test_settings_preferences_updates_log_timezone(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)

    calls = []
    monkeypatch.setattr(
        server_module.logging_setup, "set_log_timezone", lambda tz: calls.append(tz)
    )

    response = client.post("/settings/preferences", data={
        "display_timezone": "America/New_York",
        "garmin_poll_interval_minutes": "30",
        "strava_poll_interval_minutes": "8",
        "lookback_days": "14",
        "hevy2garmin_marker": "— via hevy",
        "hevy2garmin_marker_enabled": "true",
    }, follow_redirects=False)

    assert response.status_code == 303
    assert calls == ["America/New_York"]


def test_settings_preferences_saves_all_fields(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)

    response = client.post("/settings/preferences", data={
        "display_timezone": "Europe/Oslo",
        "garmin_poll_interval_minutes": "30",
        "strava_poll_interval_minutes": "8",
        "lookback_days": "14",
        "hevy2garmin_marker": "— via hevy",
        "hevy2garmin_marker_enabled": "true",
    }, follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/settings#preferences"
    cfg = config.load_config(conn)
    assert cfg["display_timezone"] == "Europe/Oslo"
    assert cfg["garmin_poll_interval_minutes"] == 30
    assert cfg["strava_poll_interval_minutes"] == 8
    assert cfg["lookback_days"] == 14
    assert cfg["hevy2garmin_marker"] == "— via hevy"
    assert cfg["hevy2garmin_marker_enabled"] is True


def test_settings_preferences_rejects_bad_timezone(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn); _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)

    before = config.load_config(conn)["display_timezone"]
    response = client.post("/settings/preferences", data={
        "display_timezone": "Mars/Phobos",
        "garmin_poll_interval_minutes": "30",
        "strava_poll_interval_minutes": "8",
        "lookback_days": "14",
        "hevy2garmin_marker": "x",
    })
    assert response.status_code == 400
    assert "Unknown timezone" in response.text
    assert config.load_config(conn)["display_timezone"] == before
    assert config.load_config(conn)["display_timezone"] != "Mars/Phobos"


def test_settings_preferences_disables_hevy_toggle_when_unchecked(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)

    # First, enable the hevy2garmin_marker_enabled feature
    cfg = config.load_config(conn)
    cfg["hevy2garmin_marker_enabled"] = True
    config.save_config(conn, cfg)

    # POST without including hevy2garmin_marker_enabled in form data (unchecked checkbox)
    response = client.post("/settings/preferences", data={
        "display_timezone": "Europe/Oslo",
        "garmin_poll_interval_minutes": "30",
        "strava_poll_interval_minutes": "8",
        "lookback_days": "14",
        "hevy2garmin_marker": "— via hevy",
    }, follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/settings#preferences"
    assert config.load_config(conn)["hevy2garmin_marker_enabled"] is False


def test_settings_page_shows_connection_rows_and_one_preferences_form(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn); _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)

    response = client.get("/settings")
    assert response.status_code == 200
    assert "/settings/preferences" in response.text
    assert "me@example.com" in response.text          # Garmin connection meta
    assert response.text.count('class="connection-manage-button"') == 2
    assert response.text.count('class="conn-identity"') == 2
    assert 'id="garmin-manage-dialog"' in response.text
    assert 'id="strava-manage-dialog"' in response.text
    assert 'class="conn-manage"' not in response.text
    assert "/settings/garmin-sync" not in response.text
    assert "/settings/strava-sync" not in response.text
    assert "/settings/general" not in response.text


def test_settings_page_loads(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)

    response = client.get("/settings")

    assert response.status_code == 200
    assert "Settings" in response.text


def test_settings_garmin_reconnect_renders_mfa_in_settings_not_the_wizard(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    monkeypatch.setattr(
        server_module, "garmin_begin_login",
        MagicMock(side_effect=MfaRequired(pending_auth=MagicMock())),
    )

    response = client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "hunter2",
    }, follow_redirects=False)

    assert response.status_code == 200
    assert "mfa_code" in response.text
    assert "Settings" in response.text          # settings.html, never setup.html
    # A <dialog> without showModal() is display:none — asserting mfa_code is
    # in the markup proves nothing about what the user actually sees. Assert
    # the dialog is really opened.
    assert "garmin-manage-dialog').showModal()" in response.text


def test_settings_garmin_reconnect_surfaces_failure_in_the_dialog(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    monkeypatch.setattr(
        server_module, "garmin_begin_login",
        MagicMock(side_effect=RuntimeError("401 Unauthorized")),
    )

    response = client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "wrong",
    }, follow_redirects=False)

    assert response.status_code == 502
    assert "401 Unauthorized" in response.text
    # A failed attempt with newly-typed credentials proves nothing about the
    # ones already stored: a healthy connection must survive a typo, not be
    # flipped to unverified.
    assert db.get_config_value(conn, "garmin_credentials_verified") is True


def test_settings_garmin_reconnect_failure_does_not_overwrite_saved_password(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)  # good creds saved: me@example.com / hunter2, verified True
    monkeypatch.setattr(
        server_module, "garmin_begin_login",
        MagicMock(side_effect=RuntimeError("401 Unauthorized")),
    )

    response = client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "typo-oops",
    }, follow_redirects=False)

    assert response.status_code == 502
    # The old, working password must still be the one on disk — not the typo
    # that was just rejected. Otherwise the background poller retries Garmin
    # with the bad password, and repeated bad logins are how Garmin locks
    # accounts.
    assert db.get_config_value(conn, "garmin_credentials") == {
        "email": "me@example.com", "password": "hunter2",
    }
    assert db.get_config_value(conn, "garmin_credentials_verified") is True


def test_settings_garmin_reconnect_success_verifies_and_catches_up(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    db.set_config_value(conn, "garmin_credentials_verified", False)
    monkeypatch.setattr(server_module, "garmin_begin_login", MagicMock())
    # A real return value here matters: _mark_garmin_verified -> _finalize_garmin_connect
    # -> _fetch_and_store_garmin_categories JSON-serializes fetch_activity_types()'s
    # result. A bare MagicMock() return value isn't serializable, raises, and gets
    # swallowed by _finalize_garmin_connect's bare except — which would let this
    # test pass by accident even if verification were broken.
    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.return_value = [{"type_key": "running", "label": "Running"}]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    catch_up = MagicMock(return_value=_empty_catch_up_stats())
    monkeypatch.setattr(server_module.sync, "catch_up_sync", catch_up)

    response = client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "hunter2",
    }, follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/settings"
    assert db.get_config_value(conn, "garmin_credentials_verified") is True
    assert db.get_config_value(conn, "garmin_activity_types") == [
        {"type_key": "running", "label": "Running"}
    ]
    catch_up.assert_called_once()


def test_reconnect_catch_up_survives_the_poller_stealing_garmin_last_sync_ok_at(tmp_path, monkeypatch):
    """Race: _mark_garmin_verified flips garmin_credentials_verified and then
    makes a network call before the catch-up reads garmin_last_sync_ok_at. The
    poller (same sqlite connection, no lock) sees the flag, finds itself due
    within the minute, and runs a NORMAL-window sync that rewrites
    garmin_last_sync_ok_at = now. If the catch-up re-read the timestamp then,
    it would measure a 0-day outage, never widen, and silently fetch none of
    the backlog. The outage must be measured at reconnect time instead.
    """
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    db.set_config_value(conn, "garmin_credentials_verified", False)
    outage_start = datetime(2026, 6, 18, 12, 0, tzinfo=timezone.utc)
    db.set_config_value(conn, "garmin_last_sync_ok_at", outage_start.isoformat())

    monkeypatch.setattr(server_module, "garmin_begin_login", MagicMock())
    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.return_value = [{"type_key": "running", "label": "Running"}]

    def _steal_the_timestamp(_conn):
        # Stand in for the poller's tick landing mid-reconnect: a normal-window
        # sync_garmin completing and stamping garmin_last_sync_ok_at = now.
        db.set_config_value(
            conn, "garmin_last_sync_ok_at", datetime.now(timezone.utc).isoformat(),
        )
        return fake_garmin

    monkeypatch.setattr(server_module, "_build_garmin_client", _steal_the_timestamp)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())
    catch_up = MagicMock(return_value=_empty_catch_up_stats())
    monkeypatch.setattr(server_module.sync, "catch_up_sync", catch_up)

    response = client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "hunter2",
    }, follow_redirects=False)

    assert response.status_code == 303
    # The catch-up must size its window from the timestamp observed at reconnect
    # time, not from whatever the poller left behind.
    assert catch_up.call_args.kwargs["last_sync_ok_at"] == outage_start.isoformat()


def test_reconnect_catch_up_clears_a_stale_report_when_nothing_is_found(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    db.set_config_value(conn, "garmin_credentials_verified", False)
    db.set_config_value(conn, "catch_up_report", {"new": 40, "held": 40, "linked": 0, "days": 22})

    monkeypatch.setattr(server_module, "garmin_begin_login", MagicMock())
    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.return_value = [{"type_key": "running", "label": "Running"}]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())
    monkeypatch.setattr(
        server_module.sync, "catch_up_sync", MagicMock(return_value=_empty_catch_up_stats()),
    )

    client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "hunter2",
    }, follow_redirects=False)

    # A quiet reconnect must not leave the previous reconnect's banner up.
    assert db.get_config_value(conn, "catch_up_report") is None


def test_settings_garmin_reconnect_succeeds_even_when_catch_up_sync_raises(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    db.set_config_value(conn, "garmin_credentials_verified", False)
    monkeypatch.setattr(server_module, "garmin_begin_login", MagicMock())
    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.return_value = [{"type_key": "running", "label": "Running"}]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    # catch_up_sync is best-effort: a reconnect that SUCCEEDED must not be
    # reported as a failure because the follow-up sync hiccupped.
    monkeypatch.setattr(
        server_module.sync, "catch_up_sync", MagicMock(side_effect=RuntimeError("boom"))
    )

    response = client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "hunter2",
    }, follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/settings"
    assert db.get_config_value(conn, "garmin_credentials_verified") is True


def test_settings_garmin_mfa_success_verifies_and_catches_up(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    db.set_config_value(conn, "garmin_credentials_verified", False)
    monkeypatch.setattr(
        server_module, "garmin_begin_login",
        MagicMock(side_effect=MfaRequired(pending_auth=MagicMock())),
    )
    client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "hunter2",
    })

    monkeypatch.setattr(server_module, "_complete_garmin_login", lambda pending, code: MagicMock())
    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.return_value = [{"type_key": "running", "label": "Running"}]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    catch_up = MagicMock(return_value=_empty_catch_up_stats())
    monkeypatch.setattr(server_module.sync, "catch_up_sync", catch_up)

    response = client.post("/settings/garmin/mfa", data={"mfa_code": "123456"}, follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/settings"
    assert db.get_config_value(conn, "garmin_credentials_verified") is True
    catch_up.assert_called_once()


def test_settings_garmin_mfa_wrong_code_reopens_dialog_with_error(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    monkeypatch.setattr(
        server_module, "garmin_begin_login",
        MagicMock(side_effect=MfaRequired(pending_auth=MagicMock())),
    )
    client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "hunter2",
    })

    monkeypatch.setattr(
        server_module, "_complete_garmin_login",
        MagicMock(side_effect=RuntimeError("Invalid code")),
    )

    response = client.post("/settings/garmin/mfa", data={"mfa_code": "000000"}, follow_redirects=False)

    assert response.status_code == 401
    assert "Invalid code" in response.text
    assert "garmin-manage-dialog').showModal()" in response.text
    assert "Settings" in response.text  # settings.html, never setup.html


def test_settings_garmin_mfa_with_no_pending_challenge_redirects(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)

    response = client.post("/settings/garmin/mfa", data={"mfa_code": "123456"}, follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/settings"


def test_settings_garmin_reconnect_success_clears_a_stale_prior_mfa_challenge(tmp_path, monkeypatch):
    """Cross-contamination: an earlier reconnect raised MFA and was abandoned;
    a LATER reconnect succeeds without a challenge. The dead pending_auth from
    the abandoned attempt must not linger and be used by a later MFA submit."""
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    monkeypatch.setattr(
        server_module, "garmin_begin_login",
        MagicMock(side_effect=MfaRequired(pending_auth=MagicMock())),
    )
    client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "hunter2",
    })

    monkeypatch.setattr(server_module, "garmin_begin_login", MagicMock())
    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.return_value = [{"type_key": "running", "label": "Running"}]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module.sync, "catch_up_sync", MagicMock(return_value=_empty_catch_up_stats()))

    response = client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "hunter2",
    }, follow_redirects=False)
    assert response.status_code == 303

    # The abandoned challenge must be gone: completing it now finds nothing
    # pending rather than resurrecting a dead pending_auth.
    stale_complete = client.post(
        "/settings/garmin/mfa", data={"mfa_code": "000000"}, follow_redirects=False
    )
    assert stale_complete.status_code == 303
    assert stale_complete.headers["location"] == "/settings"


def test_settings_garmin_reconnect_mfa_does_not_overwrite_saved_password(tmp_path, monkeypatch):
    """The user's saved Garmin connection is healthy. They mistype the
    password in the Manage Garmin dialog and hit Reconnect. Garmin issues an
    MFA challenge BEFORE validating the password (verified against the real
    API), so MfaRequired is not proof the typed password is correct. If the
    typo were persisted here, closing the dialog without entering a code
    would silently destroy the working password."""
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)  # good creds saved: me@example.com / hunter2, verified True
    monkeypatch.setattr(
        server_module, "garmin_begin_login",
        MagicMock(side_effect=MfaRequired(pending_auth=MagicMock())),
    )

    response = client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "typo-oops",
    }, follow_redirects=False)

    assert response.status_code == 200
    assert "mfa_code" in response.text
    # The typo must NOT have overwritten the working saved password while
    # the challenge is still pending.
    assert db.get_config_value(conn, "garmin_credentials") == {
        "email": "me@example.com", "password": "hunter2",
    }
    assert db.get_config_value(conn, "garmin_credentials_verified") is True


def test_settings_garmin_mfa_success_commits_newly_typed_credentials(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)  # old creds: me@example.com / hunter2
    monkeypatch.setattr(
        server_module, "garmin_begin_login",
        MagicMock(side_effect=MfaRequired(pending_auth=MagicMock())),
    )
    client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "new-correct-pw",
    })

    monkeypatch.setattr(server_module, "_complete_garmin_login", lambda pending, code: MagicMock())
    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.return_value = [{"type_key": "running", "label": "Running"}]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module.sync, "catch_up_sync", MagicMock(return_value=_empty_catch_up_stats()))

    response = client.post("/settings/garmin/mfa", data={"mfa_code": "123456"}, follow_redirects=False)

    assert response.status_code == 303
    # Only once the code is verified does the newly-typed password become
    # the one that's actually persisted.
    assert db.get_config_value(conn, "garmin_credentials") == {
        "email": "me@example.com", "password": "new-correct-pw",
    }


def test_settings_garmin_mfa_wrong_code_leaves_saved_credentials_untouched(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)  # old creds: me@example.com / hunter2
    monkeypatch.setattr(
        server_module, "garmin_begin_login",
        MagicMock(side_effect=MfaRequired(pending_auth=MagicMock())),
    )
    client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "typo-oops",
    })

    monkeypatch.setattr(
        server_module, "_complete_garmin_login",
        MagicMock(side_effect=RuntimeError("Invalid code")),
    )

    response = client.post("/settings/garmin/mfa", data={"mfa_code": "000000"}, follow_redirects=False)

    assert response.status_code == 401
    assert db.get_config_value(conn, "garmin_credentials") == {
        "email": "me@example.com", "password": "hunter2",
    }


def test_setup_garmin_mfa_cancel_leaves_saved_credentials_untouched(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)  # old creds: me@example.com / hunter2
    monkeypatch.setattr(
        server_module, "garmin_begin_login",
        MagicMock(side_effect=MfaRequired(pending_auth=MagicMock())),
    )
    client.post("/settings/garmin/reconnect", data={
        "garmin_email": "me@example.com", "garmin_password": "typo-oops",
    })

    response = client.post("/setup/garmin/mfa/cancel", follow_redirects=False)

    assert response.status_code == 303
    assert db.get_config_value(conn, "garmin_credentials") == {
        "email": "me@example.com", "password": "hunter2",
    }


def test_setup_garmin_mfa_cancel_clears_pending_and_redirects(tmp_path, monkeypatch):
    """First-run scenario: the user abandons a challenged login and Garmin's
    pending_auth expires. Without an escape, setup_wizard's show_mfa keeps
    auto-reopening the modal (no close button, no cancel route) and every
    code 401s forever."""
    conn, client = _logged_in_client(tmp_path)
    monkeypatch.setattr(
        server_module, "_begin_garmin_login",
        lambda e, p: (_ for _ in ()).throw(MfaRequired(MagicMock())),
    )
    client.post("/setup/garmin/connect",
                data={"garmin_email": "me@example.com", "garmin_password": "hunter2", "lookback_days": "7"})

    page = client.get("/setup")
    assert "Enter your code" in page.text
    assert "/setup/garmin/mfa/cancel" in page.text

    response = client.post("/setup/garmin/mfa/cancel", follow_redirects=False)
    assert response.status_code == 303
    assert response.headers["location"] == "/setup"

    page_after = client.get("/setup")
    assert "Enter your code" not in page_after.text  # modal no longer stuck open

    # A fresh MFA submit now finds nothing pending rather than 401ing on a
    # dead challenge.
    stale = client.post("/setup/garmin-mfa", data={"mfa_code": "123456"}, follow_redirects=False)
    assert stale.status_code == 303
    assert stale.headers["location"] == "/setup"


def test_the_save_without_verify_route_is_gone(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    response = client.post("/settings/garmin-credentials", data={
        "garmin_email": "me@example.com", "garmin_password": "hunter2",
    }, follow_redirects=False)
    # No route registers this path for any method any more (the /settings
    # legacy alias was deleted too), so Starlette answers 404 rather than 405.
    assert response.status_code == 404


def test_setup_garmin_connect_one_step_success(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    monkeypatch.setattr(server_module, "_begin_garmin_login", lambda e, p: MagicMock())
    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.return_value = [
        {"type_key": "running", "label": "Running"},
        {"type_key": "cycling", "label": "Cycling"},
    ]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)

    response = client.post(
        "/setup/garmin/connect",
        data={"garmin_email": "me@example.com", "garmin_password": "hunter2",
              "lookback_days": "14", "detected_timezone": "Europe/Oslo"},
        follow_redirects=False,
    )

    assert response.status_code == 303
    assert response.headers["location"] == "/setup"
    assert db.get_config_value(conn, "garmin_credentials_verified") is True
    cfg = config.load_config(conn)
    assert cfg["lookback_days"] == 14
    assert cfg["display_timezone"] == "Europe/Oslo"
    # manual-by-default: every fetched type is held
    assert sorted(cfg["held_activity_types"]) == ["cycling", "running"]


def test_setup_garmin_connect_mfa_shows_modal(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)

    def fake_begin(email, password):
        raise MfaRequired(MagicMock())
    monkeypatch.setattr(server_module, "_begin_garmin_login", fake_begin)

    redirect = client.post(
        "/setup/garmin/connect",
        data={"garmin_email": "me@example.com", "garmin_password": "hunter2", "lookback_days": "7"},
        follow_redirects=False,
    )
    assert redirect.status_code == 303
    assert redirect.headers["location"] == "/setup"

    page = client.get("/setup")
    assert "Enter your code" in page.text
    assert "/setup/garmin-mfa" in page.text
    assert 'onsubmit="return activsyncSetLoading(this)"' in page.text
    assert "Verifying…" in page.text


def test_setup_garmin_mfa_submit_success(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    monkeypatch.setattr(server_module, "_begin_garmin_login",
                        lambda e, p: (_ for _ in ()).throw(MfaRequired(MagicMock())))
    monkeypatch.setattr(server_module, "_complete_garmin_login", lambda pending, code: MagicMock())
    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.return_value = [{"type_key": "running", "label": "Running"}]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)

    client.post("/setup/garmin/connect",
                data={"garmin_email": "me@example.com", "garmin_password": "hunter2", "lookback_days": "7"})
    response = client.post("/setup/garmin-mfa", data={"mfa_code": "123456"}, follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/setup"
    assert db.get_config_value(conn, "garmin_credentials_verified") is True


def test_setup_garmin_mfa_first_run_stores_credentials_after_verification(tmp_path, monkeypatch):
    """First run, no saved credentials yet. If the typed credentials were
    only held in memory and never committed on MFA success, the user would
    end up connected with garmin_credentials empty — and the background
    poller (which rebuilds its Garmin client from garmin_credentials on
    every tick) would have nothing to authenticate with."""
    conn, client = _logged_in_client(tmp_path)
    monkeypatch.setattr(
        server_module, "_begin_garmin_login",
        lambda e, p: (_ for _ in ()).throw(MfaRequired(MagicMock())),
    )
    client.post("/setup/garmin/connect",
                data={"garmin_email": "me@example.com", "garmin_password": "hunter2", "lookback_days": "7"})

    # Mid-challenge: nothing should be persisted yet.
    assert db.get_config_value(conn, "garmin_credentials") is None

    monkeypatch.setattr(server_module, "_complete_garmin_login", lambda pending, code: MagicMock())
    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.return_value = [{"type_key": "running", "label": "Running"}]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)

    response = client.post("/setup/garmin-mfa", data={"mfa_code": "123456"}, follow_redirects=False)

    assert response.status_code == 303
    assert db.get_config_value(conn, "garmin_credentials") == {
        "email": "me@example.com", "password": "hunter2",
    }
    assert db.get_config_value(conn, "garmin_credentials_verified") is True


def test_setup_garmin_connect_redirects_to_settings_after_setup(tmp_path, monkeypatch):
    """A stale open wizard tab or a bookmark must not be able to POST the
    wizard's connect route once setup is done: it would silently reset
    lookback_days to the form default and re-run the connect flow, which on
    error renders setup.html — forbidden for a post-setup user."""
    conn, client = _logged_in_client(tmp_path)
    db.set_config_value(conn, "initial_sync_done", True)
    cfg = config.load_config(conn)
    cfg["held_activity_types"] = ["cycling"]
    cfg["lookback_days"] = 30
    config.save_config(conn, cfg)
    begin_login = MagicMock()
    monkeypatch.setattr(server_module, "_begin_garmin_login", begin_login)

    response = client.post(
        "/setup/garmin/connect",
        data={"garmin_email": "me@example.com", "garmin_password": "hunter2", "lookback_days": "7"},
        follow_redirects=False,
    )

    assert response.status_code == 303
    assert response.headers["location"] == "/settings"
    begin_login.assert_not_called()
    after = config.load_config(conn)
    assert after["lookback_days"] == 30              # not reset to the form default
    assert after["held_activity_types"] == ["cycling"]  # category holds untouched


def test_settings_shows_category_checklist_from_cached_garmin_types(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(conn, "garmin_activity_types", [
        {"type_key": "running", "label": "Running"},
        {"type_key": "strength_training", "label": "Strength Training"},
    ])

    response = client.get("/settings")

    assert "Running" in response.text
    assert "Strength Training" in response.text


def test_settings_previews_garmin_categories_and_can_show_all(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(conn, "garmin_activity_types", [
        {"type_key": f"category_{index}", "label": f"Category {index}"}
        for index in range(1, 21)
    ])

    response = client.get("/settings")

    assert response.text.count('name="autosync_types"') == 20
    assert response.text.count('category-option-extra"') == 2
    assert "Show all 20 categories" in response.text
    assert "Select visible" in response.text
    assert "Clear visible" in response.text
    assert "Select all" in response.text
    assert "Clear all" in response.text
    assert 'aria-expanded="false"' in response.text
    assert "category-action-label-short" not in response.text


def test_saving_autosync_preferences_uses_separate_settings_action(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    db.set_config_value(conn, "garmin_activity_types", [
        {"type_key": "running", "label": "Running"},
        {"type_key": "strength_training", "label": "Strength Training"},
    ])

    response = client.post("/settings/autosync", data={"autosync_types": ["running"]}, follow_redirects=False)

    assert response.status_code == 303
    assert config.load_config(conn)["held_activity_types"] == ["strength_training"]


def test_saving_autosync_categories_does_not_unhold_the_outage_backlog(tmp_path):
    """The catch-up banner points the user at Settings. Re-saving the autosync
    categories there must not promote the outage backlog to pending — the next
    poller tick would blast-publish weeks of activities to the real Strava."""
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    db.set_config_value(conn, "garmin_activity_types", [
        {"type_key": "running", "label": "Running"},
        {"type_key": "strength_training", "label": "Strength Training"},
    ])
    now = datetime(2026, 7, 9, 12, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn, 900, "running", "Backlog Run", "", "2026-06-20 09:00:00", "h",
        "held", now, hold_reason="backlog",
    )
    db.insert_activity(
        conn, 901, "running", "Recent Run", "", "2026-07-08 09:00:00", "h2",
        "held", now, hold_reason="category",
    )

    response = client.post("/settings/autosync", data={"autosync_types": ["running"]}, follow_redirects=False)

    assert response.status_code == 303
    assert db.get_activity(conn, 900)["publish_status"] == "held"
    assert db.get_activity(conn, 901)["publish_status"] == "pending"


def test_settings_shows_no_categories_message_when_cache_empty(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)

    response = client.get("/settings")

    assert "Categories aren't loaded yet" in response.text


def test_refresh_garmin_activity_types_route_caches_types(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.return_value = [{"type_key": "running", "label": "Running"}]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)

    response = client.post("/settings/garmin-activity-types/refresh", follow_redirects=False)

    assert response.status_code == 303
    assert db.get_config_value(conn, "garmin_activity_types") == [{"type_key": "running", "label": "Running"}]


def test_refresh_garmin_activity_types_requires_garmin_setup(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))

    response = client.post("/settings/garmin-activity-types/refresh", follow_redirects=False)

    assert response.status_code == 400
    assert "Connect to Garmin" in response.text


def test_strava_connect_redirects_to_authorize_url(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    db.set_config_value(conn, "strava_credentials", {"client_id": "cid", "client_secret": "csecret"})

    response = client.get("/strava/connect", follow_redirects=False)

    assert response.status_code == 307
    assert "strava.com/oauth/authorize" in response.headers["location"]
    assert "client_id=cid" in response.headers["location"]


def test_strava_connect_with_no_saved_credentials_opens_the_dialog_with_error(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)
    # No strava_credentials saved at all.

    response = client.get("/strava/connect", follow_redirects=False)

    assert response.status_code == 400
    assert "Save your Strava client ID and client secret" in response.text
    # strava_error only renders inside the <dialog>, which is closed by
    # default — without strava_dialog_open the user sees zero feedback.
    assert "strava-manage-dialog').showModal()" in response.text


def test_setup_strava_connect_saves_credentials_then_starts_oauth(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    fake_strava = MagicMock()
    fake_strava.authorize_url.return_value = "https://strava.com/oauth/authorize?client_id=cid"
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.post(
        "/setup/strava/connect",
        data={"strava_client_id": "cid", "strava_client_secret": "csecret"},
        follow_redirects=False,
    )

    assert response.status_code == 303
    assert response.headers["location"] == "/strava/connect"
    assert db.get_config_value(conn, "strava_credentials") == {
        "client_id": "cid", "client_secret": "csecret",
    }
    oauth = client.get("/strava/connect", follow_redirects=False)
    assert oauth.status_code == 307
    assert "client_id=cid" in oauth.headers["location"]


def test_saving_strava_credentials_persists_them(tmp_path):
    conn, client = _logged_in_client(tmp_path)

    response = client.post(
        "/settings/strava-credentials",
        data={"strava_client_id": "cid", "strava_client_secret": "csecret"},
        follow_redirects=False,
    )

    assert response.status_code == 303
    assert db.get_config_value(conn, "strava_credentials") == {
        "client_id": "cid", "client_secret": "csecret",
    }
    settings = client.get("/settings")
    assert "csecret" not in settings.text


def test_updating_strava_credentials_disconnects_old_oauth_session(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_strava_connected(conn)

    response = client.post(
        "/settings/strava-credentials",
        data={"strava_client_id": "new-cid", "strava_client_secret": "new-secret"},
        follow_redirects=False,
    )

    assert response.status_code == 303
    assert db.get_config_value(conn, "strava_credentials") == {
        "client_id": "new-cid", "client_secret": "new-secret",
    }
    assert db.get_config_value(conn, "strava_tokens") is None


def test_saving_changed_strava_credentials_goes_straight_to_oauth(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)

    response = client.post("/settings/strava-credentials", data={
        "strava_client_id": "new-id", "strava_client_secret": "new-secret",
    }, follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/strava/connect"
    assert db.get_config_value(conn, "strava_tokens") is None


def test_strava_callback_exchanges_code_and_redirects_to_settings(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    db.set_config_value(conn, "initial_sync_done", True)
    state = _pending_oauth_state(conn)

    fake_strava = MagicMock()
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.get(
        f"/strava/callback?code=authcode123&state={state}", follow_redirects=False
    )

    assert response.status_code == 303
    assert response.headers["location"] == "/settings"
    fake_strava.exchange_code.assert_called_once_with("authcode123")


def test_strava_callback_returns_to_setup_during_first_run(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    db.set_config_value(conn, "strava_credentials", {"client_id": "cid", "client_secret": "csecret"})
    state = _pending_oauth_state(conn)
    fake_strava = MagicMock()
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.get(f"/strava/callback?code=abc&state={state}", follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/setup"
    fake_strava.exchange_code.assert_called_once_with("abc")


def test_strava_callback_returns_to_settings_after_setup(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    db.set_config_value(conn, "initial_sync_done", True)
    state = _pending_oauth_state(conn)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())

    response = client.get(f"/strava/callback?code=abc&state={state}", follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/settings"


def test_strava_callback_catches_up_after_reconnect(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    state = _pending_oauth_state(conn)
    monkeypatch.setattr(server_module.StravaClient, "exchange_code", MagicMock())
    monkeypatch.setattr(server_module, "_build_garmin_client", MagicMock())
    catch_up = MagicMock(return_value=_empty_catch_up_stats())
    monkeypatch.setattr(server_module.sync, "catch_up_sync", catch_up)

    response = client.get(f"/strava/callback?code=abc&state={state}", follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/settings"
    catch_up.assert_called_once()


def test_strava_connect_stores_state_and_sends_it_to_strava(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    db.set_config_value(conn, "strava_credentials", {"client_id": "cid", "client_secret": "csecret"})

    response = client.get("/strava/connect", follow_redirects=False)

    state = db.get_config_value(conn, "strava_oauth_state")
    assert state
    assert f"state={state}" in response.headers["location"]


def test_strava_callback_reports_access_denied_instead_of_a_raw_422(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    _pending_oauth_state(conn)

    # Exactly what Strava sends when the athlete declines — no `code` at all,
    # which used to fail FastAPI validation and dump a raw JSON 422 in the
    # user's face.
    response = client.get("/strava/callback?state=&error=access_denied", follow_redirects=False)

    assert response.status_code == 400
    # Strava's raw error code is for the log, not the page.
    assert "access_denied" not in response.text
    assert "authorization was declined" in response.text
    assert "strava-manage-dialog').showModal()" in response.text


def test_strava_callback_without_code_reports_an_error(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    _pending_oauth_state(conn)

    response = client.get("/strava/callback", follow_redirects=False)

    assert response.status_code == 400
    assert "did not send an authorization back" in response.text


def test_strava_callback_during_setup_reports_errors_on_the_setup_page(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    db.set_config_value(conn, "strava_credentials", {"client_id": "cid", "client_secret": "csecret"})
    _pending_oauth_state(conn)

    response = client.get("/strava/callback?error=access_denied", follow_redirects=False)

    assert response.status_code == 400
    assert "authorization was declined" in response.text
    # Mid-setup there is no settings page to fall back to: the user lands back
    # on wizard step 2 with what they already typed still there, so fixing the
    # callback domain on Strava's side doesn't cost them a re-entry.
    assert 'action="/setup/strava/connect"' in response.text
    assert 'value="cid"' in response.text
    assert "Saved — leave blank to keep it" in response.text


def test_strava_callback_rejects_a_state_it_did_not_issue(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    _pending_oauth_state(conn, "the-real-state")
    fake_strava = MagicMock()
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.get(
        "/strava/callback?code=abc&state=forged-state", follow_redirects=False
    )

    assert response.status_code == 400
    fake_strava.exchange_code.assert_not_called()


def test_strava_callback_rejects_a_code_with_no_handshake_in_flight(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    # No strava_oauth_state stored: nobody started an authorization here.
    fake_strava = MagicMock()
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.get("/strava/callback?code=abc&state=whatever", follow_redirects=False)

    assert response.status_code == 400
    fake_strava.exchange_code.assert_not_called()


def test_strava_callback_state_cannot_be_replayed(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    state = _pending_oauth_state(conn)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())
    monkeypatch.setattr(server_module, "_build_garmin_client", MagicMock())
    monkeypatch.setattr(
        server_module.sync, "catch_up_sync", MagicMock(return_value=_empty_catch_up_stats())
    )

    first = client.get(f"/strava/callback?code=abc&state={state}", follow_redirects=False)
    replayed = client.get(f"/strava/callback?code=abc&state={state}", follow_redirects=False)

    assert first.status_code == 303
    assert replayed.status_code == 400
    assert db.get_config_value(conn, "strava_oauth_state") is None


def test_strava_callback_reports_a_rejected_code_exchange(tmp_path, monkeypatch):
    import requests

    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    state = _pending_oauth_state(conn)
    fake_strava = MagicMock()
    fake_strava.exchange_code.side_effect = requests.HTTPError("400 Bad Request")
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.get(f"/strava/callback?code=abc&state={state}", follow_redirects=False)

    assert response.status_code == 400
    assert "Could not complete the Strava connection" in response.text
    # A failed exchange must not leave the used state around for a retry.
    assert db.get_config_value(conn, "strava_oauth_state") is None


def test_strava_disconnect_returns_to_setup(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())

    response = client.post("/strava/disconnect", follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/setup"


def test_strava_disconnect_route_calls_disconnect_and_redirects(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    fake_strava = MagicMock()
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.post("/strava/disconnect", follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/setup"
    fake_strava.disconnect.assert_called_once()


def test_strava_disconnect_is_available_without_password_auth(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))

    response = client.post("/strava/disconnect", follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/setup"


def test_strava_disconnect_returns_to_settings_not_the_wizard(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    monkeypatch.setattr(server_module.StravaClient, "disconnect", MagicMock())

    response = client.post("/strava/disconnect", follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/settings"


def test_settings_is_accessible_without_password_auth(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)

    response = client.get("/settings")

    assert response.status_code == 200
    assert "Settings" in response.text


def test_setup_redirects_to_dashboard_once_setup_is_done(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(conn, "garmin_credentials_verified", False)   # broken

    response = client.get("/setup", follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/"


def test_settings_renders_while_a_connection_is_broken(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(conn, "garmin_credentials_verified", False)

    response = client.get("/settings", follow_redirects=False)

    assert response.status_code == 200
    assert "Settings" in response.text


def test_saving_preferences_over_htmx_saves_without_navigating(tmp_path):
    """The in-place save is the whole point: a redirect would reload the page
    and throw the reader back to the top, which is what this replaced."""
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)

    response = client.post("/settings/preferences", data={
        "display_timezone": "Europe/Oslo",
        "garmin_poll_interval_minutes": "30",
        "strava_poll_interval_minutes": "8",
        "lookback_days": "14",
        "hevy2garmin_marker": "— via hevy",
        "hevy2garmin_marker_enabled": "true",
    }, headers={"HX-Request": "true"}, follow_redirects=False)

    assert response.status_code == 204
    assert response.text == ""
    assert config.load_config(conn)["display_timezone"] == "Europe/Oslo"


def test_saving_preferences_over_htmx_reports_a_bad_timezone(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)
    before = config.load_config(conn)["display_timezone"]

    response = client.post("/settings/preferences", data={
        "display_timezone": "Mars/Phobos",
        "garmin_poll_interval_minutes": "30",
        "strava_poll_interval_minutes": "8",
        "lookback_days": "14",
        "hevy2garmin_marker": "x",
    }, headers={"HX-Request": "true"}, follow_redirects=False)

    assert response.status_code == 400
    assert response.text == "Unknown timezone: Mars/Phobos"
    assert config.load_config(conn)["display_timezone"] == before


def test_saving_autosync_over_htmx_saves_without_navigating(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    db.set_config_value(conn, "garmin_activity_types", [
        {"type_key": "running", "label": "Running"},
        {"type_key": "yoga", "label": "Yoga"},
    ])

    response = client.post("/settings/autosync", data={"autosync_types": ["running"]},
                           headers={"HX-Request": "true"}, follow_redirects=False)

    assert response.status_code == 204
    assert config.load_config(conn)["held_activity_types"] == ["yoga"]


def test_saving_autosync_over_htmx_reports_a_missing_garmin_connection(tmp_path):
    conn, client = _logged_in_client(tmp_path)

    response = client.post("/settings/autosync", data={"autosync_types": ["running"]},
                           headers={"HX-Request": "true"}, follow_redirects=False)

    assert response.status_code == 400
    assert "Connect to Garmin" in response.text


def test_refreshing_categories_over_htmx_returns_only_the_category_list(tmp_path, monkeypatch):
    """Refresh swaps the list in place rather than reloading, so it answers with
    the picker fragment alone — a whole page here would nest a second document
    inside the form."""
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)
    garmin = MagicMock()
    garmin.fetch_activity_types.return_value = [
        {"type_key": "running", "label": "Running"},
        {"type_key": "padel", "label": "Padel"},
    ]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda conn: garmin)

    response = client.post("/settings/garmin-activity-types/refresh",
                           headers={"HX-Request": "true"}, follow_redirects=False)

    assert response.status_code == 200
    assert "Padel" in response.text
    assert 'name="autosync_types"' in response.text
    # The fragment must not drag the rest of the page in with it.
    assert "<html" not in response.text
    assert "<h2>" not in response.text
    assert "/settings/preferences" not in response.text
    assert db.get_config_value(conn, "garmin_activity_types") == [
        {"type_key": "running", "label": "Running"},
        {"type_key": "padel", "label": "Padel"},
    ]


def test_refreshing_categories_over_htmx_reports_a_missing_garmin_connection(tmp_path):
    conn, client = _logged_in_client(tmp_path)

    response = client.post("/settings/garmin-activity-types/refresh",
                           headers={"HX-Request": "true"}, follow_redirects=False)

    assert response.status_code == 400
    assert "Connect to Garmin" in response.text


def test_refreshing_categories_without_htmx_returns_to_the_categories_section(tmp_path, monkeypatch):
    """The no-htmx fallback still reloads, so the anchor is what keeps the
    reader on the categories instead of at the top of the page."""
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn)
    _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)
    garmin = MagicMock()
    garmin.fetch_activity_types.return_value = [{"type_key": "running", "label": "Running"}]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda conn: garmin)

    response = client.post("/settings/garmin-activity-types/refresh", follow_redirects=False)

    assert response.status_code == 303
    assert response.headers["location"] == "/settings#autosync"


def test_settings_offers_manual_sync_for_both_services(tmp_path):
    """The poller does both on a loop, so these are manual overrides and belong
    next to the connections they act on rather than atop the activities list."""
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn); _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)

    page = client.get("/settings")

    assert page.status_code == 200
    assert 'hx-post="/api/sync/garmin"' in page.text
    assert 'hx-post="/api/sync/strava/status"' in page.text
    assert ">Sync Garmin<" in page.text
    assert ">Sync Strava<" in page.text
    assert ">Check Strava<" not in page.text


def test_manual_sync_answers_204_like_every_other_settings_action(tmp_path, monkeypatch):
    """Same contract as the save buttons: 204, nothing swaps, and the button
    reports the outcome itself. Anything rendered here would have to be a
    second, parallel way of saying what the button already says."""
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn); _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)
    fake_garmin = MagicMock()
    fake_garmin.fetch_recent_activities.return_value = []
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())
    monkeypatch.setattr(server_module.sync, "check_strava_status", MagicMock())

    response = client.post("/api/sync/garmin", headers={"HX-Request": "true"},
                           follow_redirects=False)

    assert response.status_code == 204
    assert response.text == ""
    fake_garmin.fetch_recent_activities.assert_called_once()


def test_manual_sync_reports_a_broken_connection_as_plain_text(tmp_path):
    """The error goes to the button's error slot, which reads xhr.responseText
    verbatim — so the body has to be the message, not a page or a fragment."""
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn); _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(conn, "garmin_credentials_verified", False)

    response = client.post("/api/sync/garmin", headers={"HX-Request": "true"},
                           follow_redirects=False)

    assert response.status_code == 409
    assert response.text == "Garmin is disconnected — reconnect it to sync."
    assert "<" not in response.text


def test_manual_strava_sync_reports_a_rejected_token_as_plain_text(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn); _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())
    monkeypatch.setattr(
        server_module.sync, "check_strava_status",
        MagicMock(side_effect=StravaAuthError("Strava access token rejected")),
    )

    response = client.post("/api/sync/strava/status", headers={"HX-Request": "true"},
                           follow_redirects=False)

    assert response.status_code == 409
    assert response.text == "Strava access token rejected"
    assert "<" not in response.text


def test_manual_sync_buttons_wear_the_same_states_as_the_rest_of_settings(tmp_path):
    """Idle, working, done — all three in the button, as on the save and setup
    buttons. .button-swap holds every state in one grid cell, so swapping
    between them does not resize the button."""
    conn, client = _logged_in_client(tmp_path)
    _mark_garmin_connected(conn); _mark_strava_connected(conn)
    db.set_config_value(conn, "initial_sync_done", True)

    page = client.get("/settings")

    tools = page.text.split('class="manual-sync-actions"', 1)[1].split("</div>", 1)[0]
    for button in tools.split("<button")[1:]:
        assert "button-swap" in button
        assert "button-label" in button
        assert "button-loading" in button and "button-spinner" in button
        assert "button-saved" in button
        assert "data-save-feedback" in button and "data-saved-message" in button
    # The section needs both slots the shared feedback script writes into.
    section = page.text.split('id="connections"', 1)[1].split("</section>", 1)[0]
    assert "settings-save-error" in section
    assert "settings-save-announcement" in section
