"""Dev-mode Hevy fakes: MockHevyClient surface, FakeGarminClient scenario
behaviour, seeded demo rows, and the wizard's optional Hevy step."""

from datetime import datetime, timezone

import pytest
from fastapi.testclient import TestClient

from activsync import db, dev_mock, dev_seed, hevy_db, view
from activsync.garmin_client import SubcategoryRejected
from activsync.hevy_client import HevyClient
from activsync.server import create_app

NOW = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "test.db"))


def _public_methods(cls) -> set[str]:
    return {
        name for name, value in vars(cls).items()
        if not name.startswith("_") and callable(value)
    }


# -- MockHevyClient ----------------------------------------------------------


def test_mock_hevy_client_implements_the_real_hevy_surface(conn):
    missing = _public_methods(HevyClient) - _public_methods(dev_mock.MockHevyClient)

    assert not missing, f"MockHevyClient is missing {sorted(missing)} — mock mode will crash"


def test_mock_hevy_workouts_are_schema_valid(conn):
    workouts = dev_mock.MockHevyClient(conn).get_workouts_page(1)["workouts"]

    assert workouts, "dev mock must ship canned workouts"
    for workout in workouts:
        assert workout["id"]
        assert workout["start_time"] and workout["end_time"]
        assert workout["updated_at"]
        assert isinstance(workout["exercises"], list)
        for exercise in workout["exercises"]:
            assert exercise["title"]
            assert "exercise_template_id" in exercise
            assert isinstance(exercise.get("sets", []), list)


def test_mock_hevy_ships_the_four_demo_scenarios(conn):
    client = dev_mock.MockHevyClient(conn)
    workouts = {w["id"]: w for w in client.get_workouts_page(1)["workouts"]}

    assert dev_mock.HEVY_DEV_MERGED_ID in workouts
    assert dev_mock.HEVY_DEV_UNMAPPED_ID in workouts
    assert dev_mock.HEVY_DEV_PASSIVE_ID in workouts
    midnight = workouts[dev_mock.HEVY_DEV_MIDNIGHT_ID]
    start = midnight["start_time"]
    end = midnight["end_time"]
    assert start[:10] != end[:10], "midnight workout must span two dates"

    unmapped = workouts[dev_mock.HEVY_DEV_UNMAPPED_ID]
    template_ids = {e["exercise_template_id"] for e in unmapped["exercises"]}
    assert dev_mock.HEVY_DEV_CUSTOM_TEMPLATE_ID in template_ids


def test_mock_hevy_templates_include_the_custom_unmapped_one(conn):
    templates = dev_mock.MockHevyClient(conn).iter_all_exercise_templates()

    custom = [t for t in templates if t["id"] == dev_mock.HEVY_DEV_CUSTOM_TEMPLATE_ID]
    assert custom and custom[0]["is_custom"] is True


def test_mock_hevy_get_workout_and_user_info(conn):
    client = dev_mock.MockHevyClient(conn)

    assert client.get_user_info()
    assert client.get_workout(dev_mock.HEVY_DEV_MERGED_ID)["id"] == dev_mock.HEVY_DEV_MERGED_ID
    assert client.get_workout("nope") is None


# -- FakeGarminClient scenario behaviour -------------------------------------


def test_fake_garmin_upload_returns_fresh_ids(conn):
    garmin = dev_mock.FakeGarminClient(conn)

    first = garmin.upload_fit("/tmp/a.fit")
    second = garmin.upload_fit("/tmp/b.fit")

    assert first["activity_id"] and second["activity_id"]
    assert first["activity_id"] != second["activity_id"]


def test_fake_garmin_records_exercise_set_payloads(conn):
    garmin = dev_mock.FakeGarminClient(conn)

    garmin.put_exercise_sets(910001, {"exerciseSets": [1]})

    assert garmin.recorded_exercise_sets == [(910001, {"exerciseSets": [1]})]


def test_fake_garmin_subcategory_rejection_seed(conn):
    garmin = dev_mock.FakeGarminClient(conn)

    with pytest.raises(SubcategoryRejected):
        garmin.put_exercise_sets(
            dev_mock.HEVY_SUBCATEGORY_REJECT_ACTIVITY_ID, {"exerciseSets": []})


# -- seeded demo state -------------------------------------------------------


def test_dev_seed_populates_the_hevy_demo_scenarios(conn):
    dev_seed.seed(conn)

    merged = hevy_db.get_workout(conn, dev_mock.HEVY_DEV_MERGED_ID)
    assert merged["status"] == "merged"
    assert merged["garmin_activity_id"] is not None
    badge_activity = merged["garmin_activity_id"]
    badges = {a["garmin_activity_id"]: a["hevy_badge"]
              for a in view.activities_view(conn)}
    assert badges[badge_activity] == "merge"

    unmapped = hevy_db.get_workout(conn, dev_mock.HEVY_DEV_UNMAPPED_ID)
    assert unmapped["status"] == "needs_mapping"
    assert hevy_db.get_template(conn, dev_mock.HEVY_DEV_CUSTOM_TEMPLATE_ID)

    passive = hevy_db.get_workout(conn, dev_mock.HEVY_DEV_PASSIVE_ID)
    assert passive["status"] == "uploaded_passive"
    assert badges[passive["garmin_activity_id"]] == "passive"

    # Interlock demo: the claimed watch activity is publish-blocked.
    syncing = hevy_db.get_workout(conn, dev_mock.HEVY_DEV_SYNCING_ID)
    blocked = hevy_db.blocking_workout_for_activity(
        conn, syncing["source_garmin_activity_id"])
    assert blocked is not None and blocked["hevy_id"] == dev_mock.HEVY_DEV_SYNCING_ID


def test_dev_seed_is_idempotent_for_hevy_rows(conn):
    dev_seed.seed(conn)
    dev_seed.seed(conn)

    assert len([w for w in hevy_db.list_workouts(conn)
                if w["hevy_id"] == dev_mock.HEVY_DEV_MERGED_ID]) == 1


# -- wizard step -------------------------------------------------------------


def _wizard_at_hevy_step(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "a", "refresh_token": "r", "expires_at": 4102444800,
    })
    return conn, TestClient(create_app(conn))


def test_wizard_shows_optional_hevy_step_after_strava(tmp_path):
    conn, client = _wizard_at_hevy_step(tmp_path)

    page = client.get("/setup")

    assert "Hevy" in page.text
    assert 'action="/setup/hevy/skip"' in page.text


def test_wizard_hevy_step_is_skippable(tmp_path):
    conn, client = _wizard_at_hevy_step(tmp_path)

    resp = client.post("/setup/hevy/skip", follow_redirects=False)
    assert resp.status_code == 303
    assert resp.headers["location"] == "/setup"

    page = client.get("/setup")
    assert "Syncing activities" in page.text


def test_wizard_hevy_connect_marks_step_done_and_returns_to_setup(tmp_path, monkeypatch):
    conn, client = _wizard_at_hevy_step(tmp_path)
    from activsync import hevy_routes

    class _Stub:
        def __init__(self, api_key, base_url=None):
            pass

        def get_user_info(self):
            return {"username": "dev"}

        def iter_all_exercise_templates(self):
            return []

    monkeypatch.setattr(hevy_routes, "HevyClient", _Stub)

    resp = client.post("/settings/hevy-credentials", data={"api_key": "k"},
                       follow_redirects=False)

    assert resp.status_code == 303
    assert resp.headers["location"] == "/setup"
    assert db.get_config_value(conn, "hevy_api_key") == "k"
    page = client.get("/setup")
    assert "Syncing activities" in page.text
