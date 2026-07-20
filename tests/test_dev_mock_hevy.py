"""Dev-mode Hevy fakes: MockHevyClient surface, FakeGarminClient scenario
behaviour, and seeded demo rows."""

from datetime import datetime, timezone

import pytest

from activsync import db, dev_mock, dev_seed, hevy_db, view
from activsync.garmin_client import SubcategoryRejected
from activsync.hevy_client import HevyClient

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


def test_fake_garmin_upload_and_delete_change_later_fetches(conn):
    garmin = dev_mock.FakeGarminClient(conn)
    db.insert_activity(
        conn, 42, "strength_training", "Watch source", "",
        "2026-07-19 10:00:00", "dev-42", "pending", NOW,
        garmin_data='{"duration": 3600}',
    )

    uploaded = garmin.upload_fit("/tmp/dev-generated.fit")["activity_id"]
    garmin.set_title(uploaded, "Uploaded from Hevy")
    garmin.delete_activity(42)

    fetched = {record.garmin_activity_id: record
               for record in garmin.fetch_recent_activities(7)}
    assert 42 not in fetched
    assert fetched[uploaded].title == "Uploaded from Hevy"


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
