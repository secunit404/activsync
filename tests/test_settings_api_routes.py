from fastapi.testclient import TestClient

from activsync import config, db, dev_mock, hevy_db
from activsync.server import create_app


def _client(tmp_path, monkeypatch, *, mock=True):
    monkeypatch.setenv("ACTIVSYNC_DEV_MOCK_DATA", "1" if mock else "0")
    conn = db.connect(str(tmp_path / "settings-api.db"))
    return conn, TestClient(create_app(conn), follow_redirects=False)


def _complete_setup(conn):
    db.set_config_value(
        conn,
        "garmin_credentials",
        {"email": "athlete@example.com", "password": "working-password"},
    )
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(
        conn,
        "strava_credentials",
        {"client_id": "client-id", "client_secret": "client-secret"},
    )
    db.set_config_value(
        conn,
        "strava_tokens",
        {
            "access_token": "access",
            "refresh_token": "refresh",
            "expires_at": 4102444800,
        },
    )
    db.set_config_value(conn, "initial_sync_done", True)


def test_settings_state_is_typed_and_never_exposes_secrets(tmp_path, monkeypatch):
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    db.set_config_value(
        conn,
        "garmin_activity_types",
        [
            {"type_key": "running", "label": "Running"},
            {"type_key": "cycling", "label": "Cycling"},
        ],
    )
    cfg = config.load_config(conn)
    cfg["held_activity_types"] = ["cycling"]
    config.save_config(conn, cfg)

    response = client.get("/api/v1/settings")

    assert response.status_code == 200
    body = response.json()
    assert body["setup"] == {
        "complete": True,
        "step": None,
        "mfaRequired": False,
    }
    assert body["credentials"] == {
        "garminEmail": "athlete@example.com",
        "garminPasswordSaved": True,
        "stravaClientId": "client-id",
        "stravaClientSecretSaved": True,
    }
    assert "working-password" not in response.text
    assert "client-secret" not in response.text
    assert body["activityTypes"] == [
        {"typeKey": "running", "label": "Running", "autosync": True},
        {"typeKey": "cycling", "label": "Cycling", "autosync": False},
    ]


def test_failed_garmin_reconnect_keeps_last_verified_password(
    tmp_path, monkeypatch
):
    conn, client = _client(tmp_path, monkeypatch, mock=False)
    _complete_setup(conn)

    def reject_login(*_args, **_kwargs):
        raise RuntimeError("401 Unauthorized")

    monkeypatch.setattr("activsync.server.garmin_begin_login", reject_login)

    response = client.post(
        "/api/v1/settings/garmin/reconnect",
        json={"email": "athlete@example.com", "password": "mistyped"},
    )

    assert response.status_code == 502
    assert db.get_config_value(conn, "garmin_credentials") == {
        "email": "athlete@example.com",
        "password": "working-password",
    }
    assert db.get_config_value(conn, "garmin_credentials_verified") is True


def test_mfa_credentials_are_only_committed_after_a_valid_code(
    tmp_path, monkeypatch
):
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)

    challenge = client.post(
        "/api/v1/settings/garmin/reconnect",
        json={
            "email": "new@example.com",
            "password": dev_mock.MFA_TRIGGER_PASSWORD,
        },
    )
    assert challenge.status_code == 200
    assert challenge.json()["mfaRequired"] is True
    assert db.get_config_value(conn, "garmin_credentials")["password"] == (
        "working-password"
    )

    rejected = client.post(
        "/api/v1/garmin/mfa", json={"code": dev_mock.MFA_REJECT_CODE}
    )
    assert rejected.status_code == 401
    assert db.get_config_value(conn, "garmin_credentials")["password"] == (
        "working-password"
    )

    accepted = client.post("/api/v1/garmin/mfa", json={"code": "123456"})
    assert accepted.status_code == 200
    assert accepted.json()["mfaRequired"] is False
    assert db.get_config_value(conn, "garmin_credentials") == {
        "email": "new@example.com",
        "password": dev_mock.MFA_TRIGGER_PASSWORD,
    }


def test_preferences_and_categories_save_through_json_api(tmp_path, monkeypatch):
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    db.set_config_value(
        conn,
        "garmin_activity_types",
        [
            {"type_key": "running", "label": "Running"},
            {"type_key": "cycling", "label": "Cycling"},
        ],
    )
    timezone_updates = []
    monkeypatch.setattr(
        "activsync.settings_api_routes.logging_setup.set_log_timezone",
        timezone_updates.append,
    )

    preferences = client.put(
        "/api/v1/settings/preferences",
        json={
            "displayTimezone": "Europe/Oslo",
            "garminPollIntervalMinutes": 30,
            "stravaPollIntervalMinutes": 8,
            "lookbackDays": 14,
            "hevy2garminMarker": "— via hevy",
            "hevy2garminMarkerEnabled": True,
        },
    )
    categories = client.put(
        "/api/v1/settings/activity-types",
        json={"autosyncTypes": ["running"]},
    )

    assert preferences.status_code == 200
    assert categories.status_code == 200
    cfg = config.load_config(conn)
    assert cfg["display_timezone"] == "Europe/Oslo"
    assert cfg["lookback_days"] == 14
    assert cfg["hevy2garmin_marker_enabled"] is True
    assert cfg["held_activity_types"] == ["cycling"]
    assert timezone_updates == ["Europe/Oslo"]


def test_mock_setup_can_complete_through_json_contract(tmp_path, monkeypatch):
    conn, client = _client(tmp_path, monkeypatch)

    garmin = client.post(
        "/api/v1/setup/garmin",
        json={
            "email": "someone@example.com",
            "password": "safe-mock-password",
            "lookbackDays": 7,
            "detectedTimezone": "Europe/Oslo",
        },
    )
    assert garmin.status_code == 200
    assert garmin.json()["setupStep"] == "strava"

    credentials = client.put(
        "/api/v1/settings/strava-credentials",
        json={"clientId": "dev-id", "clientSecret": "dev-secret"},
    )
    assert credentials.status_code == 200

    connect = client.get("/strava/connect")
    assert connect.status_code == 307
    callback = client.get(connect.headers["location"])
    assert callback.status_code == 303

    skipped = client.post("/api/v1/setup/hevy/skip")
    assert skipped.status_code == 200
    assert skipped.json()["setupStep"] == "syncing"

    initial = client.post("/api/v1/setup/initial-sync")
    assert initial.status_code == 200
    assert initial.json()["setupStep"] is None
    assert db.get_config_value(conn, "initial_sync_done") is True


def test_dev_onboarding_state_reopens_and_restores_the_wizard_gate(
    tmp_path, monkeypatch
):
    """The shared E2E dev-mock server always seeds onboarded (see
    ``main.py``'s ``ACTIVSYNC_DEV_E2E_ONBOARDED``), so `/setup` alone shows
    the "you're all set" screen and `POST /setup/garmin` 409s. This
    mock-only endpoint is what `e2e/setup.spec.ts` uses to reach the wizard
    without disturbing every other spec that assumes onboarding is done.
    """
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    assert client.get("/api/v1/settings").json()["setup"]["complete"] is True

    reset = client.post("/api/v1/dev/onboarding-state", json={"complete": False})
    assert reset.status_code == 200
    assert db.get_config_value(conn, "initial_sync_done") is False

    restore = client.post("/api/v1/dev/onboarding-state", json={"complete": True})
    assert restore.status_code == 200
    assert db.get_config_value(conn, "initial_sync_done") is True


def test_dev_onboarding_state_also_clears_a_pending_mfa_session(
    tmp_path, monkeypatch
):
    conn, client = _client(tmp_path, monkeypatch)
    started = client.post(
        "/api/v1/setup/garmin",
        json={"email": "athlete@example.com", "password": "mfa"},
    )
    assert started.json()["mfaRequired"] is True

    client.post("/api/v1/dev/onboarding-state", json={"complete": True})

    settings = client.get("/api/v1/settings").json()
    assert settings["setup"]["mfaRequired"] is False


def test_dev_onboarding_state_is_404_outside_mock_mode(tmp_path, monkeypatch):
    _conn, client = _client(tmp_path, monkeypatch, mock=False)
    response = client.post("/api/v1/dev/onboarding-state", json={"complete": False})
    assert response.status_code == 404


def test_hevy_mapping_tools_save_remove_and_wake_waiting_workouts(
    tmp_path, monkeypatch
):
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    hevy_db.upsert_template(
        conn,
        {
            "exercise_template_id": "custom-1",
            "title": "Landmine Press",
            "primary_muscle_group": "chest",
            "secondary_muscle_groups": [],
            "equipment_category": "barbell",
            "is_custom": True,
        },
    )
    hevy_db.upsert_workout(
        conn,
        "workout-1",
        "Push",
        "2026-07-18T10:00:00Z",
        "2026-07-18T11:00:00Z",
        "2026-07-18T12:00:00Z",
        {
            "exercises": [
                {
                    "exercise_template_id": "custom-1",
                    "title": "Landmine Press",
                }
            ]
        },
    )
    hevy_db.set_workout_status(
        conn, "workout-1", "needs_mapping", error="unmapped"
    )

    tools = client.get("/api/v1/settings/hevy/tools")
    saved = client.put(
        "/api/v1/settings/hevy/mappings/custom-1",
        json={"category": 0, "subcategory": 1},
    )

    assert tools.status_code == 200
    assert tools.json()["mappings"][0]["templateId"] == "custom-1"
    assert saved.status_code == 200
    assert "1 waiting workout resumed" in saved.json()["message"]
    assert hevy_db.get_mapping(conn, "custom-1")["subcategory"] == 1
    assert hevy_db.get_workout(conn, "workout-1")["status"] == "waiting_watch"

    removed = client.delete("/api/v1/settings/hevy/mappings/custom-1")
    assert removed.status_code == 200
    assert hevy_db.get_mapping(conn, "custom-1") is None


def test_hevy_backfill_json_preview_is_read_only_and_run_ingests(
    tmp_path, monkeypatch
):
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    db.set_config_value(conn, "hevy_api_key", "safe-mock-key")

    preview = client.post(
        "/api/v1/settings/hevy/backfill/preview",
        json={"since": "2020-01-01"},
    )

    # `dev_mock.dev_hevy_workouts()` ships 7 canned workouts: the original 5
    # demo scenarios, plus 2 added for Task 16 (`HEVY_DEV_BACKFILL_UNMAPPED_ID`
    # / `HEVY_DEV_BACKFILL_UNMAPPABLE_ID`) so the backfill screen has a
    # reachable `needs_mapping` row to preview and link out of — this test's
    # `conn` has none of them pre-seeded into `hevy_workouts` (unlike
    # `dev_seed._seed_hevy`), so all 7 come back fresh.
    assert preview.status_code == 200
    assert preview.json()["ran"] is False
    assert len(preview.json()["items"]) == 7
    assert hevy_db.list_workouts(conn) == []

    run = client.post(
        "/api/v1/settings/hevy/backfill/run",
        json={"since": "2020-01-01"},
    )
    assert run.status_code == 200
    assert run.json()["ran"] is True
    assert len(hevy_db.list_workouts(conn)) == 7
    assert "7 workouts ingested" in run.json()["message"]


def test_hevy_backfill_run_with_explicit_ids_imports_only_those(
    tmp_path, monkeypatch
):
    """The backfill screen lets a user tick a subset of the preview and
    press "Import N selected" — `hevyIds` on the run request is how that
    selection reaches the server. Of the 7 fresh demo workouts, only the two
    named here (both non-locked, see the preview-actions comment above) may
    land in `hevy_workouts`; the other 5 must be left untouched even though
    they were part of the same `since` window."""
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    db.set_config_value(conn, "hevy_api_key", "safe-mock-key")

    run = client.post(
        "/api/v1/settings/hevy/backfill/run",
        json={
            "since": "2020-01-01",
            "hevyIds": [dev_mock.HEVY_DEV_MERGED_ID, dev_mock.HEVY_DEV_PASSIVE_ID],
        },
    )

    assert run.status_code == 200
    assert run.json()["ran"] is True
    assert "2 workouts ingested" in run.json()["message"]
    ingested_ids = {w["hevy_id"] for w in hevy_db.list_workouts(conn)}
    assert ingested_ids == {dev_mock.HEVY_DEV_MERGED_ID, dev_mock.HEVY_DEV_PASSIVE_ID}


def test_hevy_backfill_run_rejects_needs_mapping_ids_even_when_requested(
    tmp_path, monkeypatch
):
    """Locked rows are disabled in the UI, but the server — not the client —
    is the actual enforcement point: an id whose preview action is
    `needs_mapping` must never be ingested, even if a request explicitly
    names it alongside a valid one."""
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    db.set_config_value(conn, "hevy_api_key", "safe-mock-key")

    run = client.post(
        "/api/v1/settings/hevy/backfill/run",
        json={
            "since": "2020-01-01",
            "hevyIds": [dev_mock.HEVY_DEV_UNMAPPED_ID, dev_mock.HEVY_DEV_MERGED_ID],
        },
    )

    assert run.status_code == 200
    assert "1 workout ingested" in run.json()["message"]
    assert hevy_db.get_workout(conn, dev_mock.HEVY_DEV_UNMAPPED_ID) is None
    assert hevy_db.get_workout(conn, dev_mock.HEVY_DEV_MERGED_ID) is not None


def test_hevy_backfill_run_with_empty_id_list_imports_nothing(tmp_path, monkeypatch):
    """`hevyIds: []` is an explicit selection of zero rows (e.g. every row
    the user could pick was locked), distinct from omitting the field
    entirely — it must not fall back to "import everything since the date"."""
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    db.set_config_value(conn, "hevy_api_key", "safe-mock-key")

    run = client.post(
        "/api/v1/settings/hevy/backfill/run",
        json={"since": "2020-01-01", "hevyIds": []},
    )

    assert run.status_code == 200
    assert "0 workouts ingested" in run.json()["message"]
    assert hevy_db.list_workouts(conn) == []


def test_hevy_backfill_run_ignores_unknown_ids_without_importing_anything_else(
    tmp_path, monkeypatch
):
    """An id that doesn't match any item in the since-window preview (stale
    client state, a typo, a workout that fell out of range) is dropped
    silently rather than falling back to a wider import — the selection is
    a ceiling, never a suggestion to import something else instead."""
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    db.set_config_value(conn, "hevy_api_key", "safe-mock-key")

    run = client.post(
        "/api/v1/settings/hevy/backfill/run",
        json={"since": "2020-01-01", "hevyIds": ["hw-not-in-preview"]},
    )

    assert run.status_code == 200
    assert "0 workouts ingested" in run.json()["message"]
    assert hevy_db.list_workouts(conn) == []


def test_hevy_backfill_run_without_ids_still_imports_everything(tmp_path, monkeypatch):
    """Back-compat: omitting `hevyIds` (or sending it as null) must preserve
    the original full since-derived import — existing callers, and the
    preview/run pair, keep working unmodified."""
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    db.set_config_value(conn, "hevy_api_key", "safe-mock-key")

    run = client.post(
        "/api/v1/settings/hevy/backfill/run",
        json={"since": "2020-01-01", "hevyIds": None},
    )

    assert run.status_code == 200
    assert "7 workouts ingested" in run.json()["message"]
    assert len(hevy_db.list_workouts(conn)) == 7


def test_device_options_lists_manufacturers_and_garmin_products(tmp_path, monkeypatch):
    """The Settings device-identity picker is fed from fit_tool's own FIT
    profile enums rather than a hardcoded list, so it cannot drift from what
    fit_builder can actually write."""
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)

    response = client.get("/api/v1/settings/hevy/device-options")

    assert response.status_code == 200
    body = response.json()
    manufacturers = body["manufacturers"]
    products = body["products"]

    assert len(manufacturers) > 100
    assert len(products) > 100

    # The two identities fit_builder actually writes (GENERIC_GARMIN_IDENTITY
    # and DEVELOPMENT_IDENTITY) must both be offerable.
    by_value = {option["value"]: option["label"] for option in manufacturers}
    assert by_value[1] == "Garmin"
    assert by_value[255] == "Development"

    # Labels are humanised, not raw SCREAMING_SNAKE enum names.
    assert all("_" not in option["label"] for option in manufacturers)
    assert {"value": 2050, "label": "Fenix3"} in products

    # Sorted by label so the select is scannable.
    assert products == sorted(products, key=lambda option: option["label"])


def test_device_options_values_are_unique(tmp_path, monkeypatch):
    """fit_tool's profile enums contain aliases — several names sharing one
    integer value. Emitting each would look like duplicate devices in the
    picker and would make the select's value ambiguous."""
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)

    body = client.get("/api/v1/settings/hevy/device-options").json()

    for key in ("manufacturers", "products"):
        values = [option["value"] for option in body[key]]
        assert len(values) == len(set(values)), f"duplicate values in {key}"


def test_device_options_needs_no_hevy_key(tmp_path, monkeypatch):
    """Reference data, not account data — it must load before Hevy is
    connected, since the identity fields render in Settings regardless."""
    conn, client = _client(tmp_path, monkeypatch)
    _complete_setup(conn)
    db.set_config_value(conn, "hevy_api_key", "")

    assert client.get("/api/v1/settings/hevy/device-options").status_code == 200
