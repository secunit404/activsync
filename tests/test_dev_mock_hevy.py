"""Dev-mode Hevy fakes: MockHevyClient surface, FakeGarminClient scenario
behaviour, and seeded demo rows."""

from datetime import datetime, timedelta, timezone

import pytest

from activsync import config, db, dev_mock, dev_seed, hevy_backfill, hevy_db, view
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


# -- preview_items missing template ids --------------------------------------


def test_preview_reports_missing_template_ids_for_unmapped_exercise(conn):
    client = dev_mock.MockHevyClient(conn)
    since_dt = datetime.now(timezone.utc) - timedelta(days=7)

    items = hevy_backfill.preview_items(conn, client, since_dt)

    locked = next(
        item for item in items
        if item["workout"]["id"] == dev_mock.HEVY_DEV_UNMAPPED_ID
    )
    assert locked["action"] == "needs_mapping"
    assert locked["missing_template_ids"] == [dev_mock.HEVY_DEV_CUSTOM_TEMPLATE_ID]

    # HEVY_DEV_BACKFILL_UNMAPPED_ID (Task 16's dev fixture, so the backfill
    # screen has a reachable locked row) shares the same unmapped custom
    # template and so reports the same non-empty list — it joins `locked` in
    # being excluded from the "everything else stays empty" check below.
    still_locked_ids = {dev_mock.HEVY_DEV_UNMAPPED_ID, dev_mock.HEVY_DEV_BACKFILL_UNMAPPED_ID}
    assert next(
        item for item in items
        if item["workout"]["id"] == dev_mock.HEVY_DEV_BACKFILL_UNMAPPED_ID
    )["missing_template_ids"] == [dev_mock.HEVY_DEV_CUSTOM_TEMPLATE_ID]

    # Every other workout never reaches the mapping-miss branch and must
    # still get a bound (empty) list, not a value leaked from a prior
    # iteration of the loop.
    others = [
        item for item in items
        if item["workout"]["id"] not in still_locked_ids
    ]
    assert others, "expected other demo workouts alongside the unmapped ones"
    assert all(item["missing_template_ids"] == [] for item in others)


class _StubHevyClient:
    """Serves a fixed, single-page set of workouts through the same shape as
    MockHevyClient, for scenarios the shared dev-mode fixtures don't cover."""

    def __init__(self, workouts: list[dict]):
        self._workouts = workouts

    def get_workouts_page(self, page: int, page_size: int = 10) -> dict:
        if page > 1:
            return {"workouts": [], "page_count": 1}
        return {"workouts": self._workouts, "page_count": 1}


def _iso(dt: datetime) -> str:
    return dt.strftime("%Y-%m-%dT%H:%M:%S+00:00")


def test_preview_deduplicates_repeated_missing_template_id(conn):
    """Two *exercises* (not sets) in the same workout that share one unmapped
    template id must collapse to a single missing_template_ids entry, not one
    per exercise — the collector iterates workout["exercises"], never sets."""
    now = datetime.now(timezone.utc)
    duplicate_exercise = {
        "title": "Bulgarian Ring Row",
        "exercise_template_id": dev_mock.HEVY_DEV_CUSTOM_TEMPLATE_ID,
        "sets": [],
    }
    workout = {
        "id": "hw-dup-unmapped",
        "title": "Duplicate unmapped circuit",
        "start_time": _iso(now - timedelta(hours=1)),
        "end_time": _iso(now - timedelta(minutes=30)),
        "updated_at": _iso(now - timedelta(minutes=30)),
        "exercises": [duplicate_exercise, dict(duplicate_exercise)],
    }
    client = _StubHevyClient([workout])
    since_dt = now - timedelta(days=7)

    items = hevy_backfill.preview_items(conn, client, since_dt)

    item = next(i for i in items if i["workout"]["id"] == "hw-dup-unmapped")
    assert item["action"] == "needs_mapping"
    assert item["missing_template_ids"] == [dev_mock.HEVY_DEV_CUSTOM_TEMPLATE_ID]


def test_preview_needs_mapping_when_template_id_is_falsy(conn):
    """A miss can occur with no template id to report (falsy id, unmappable
    title): action must still be needs_mapping even though the reporting
    list stays empty — the two are tracked independently."""
    now = datetime.now(timezone.utc)
    workout = {
        "id": "hw-falsy-template",
        "title": "Mystery move",
        "start_time": _iso(now - timedelta(hours=1)),
        "end_time": _iso(now - timedelta(minutes=30)),
        "updated_at": _iso(now - timedelta(minutes=30)),
        "exercises": [{
            "title": "Not A Real Exercise",
            "exercise_template_id": None,
            "sets": [],
        }],
    }
    client = _StubHevyClient([workout])
    since_dt = now - timedelta(days=7)

    items = hevy_backfill.preview_items(conn, client, since_dt)

    item = next(i for i in items if i["workout"]["id"] == "hw-falsy-template")
    assert item["action"] == "needs_mapping"
    assert item["missing_template_ids"] == []


def test_preview_reports_no_missing_templates_once_exercise_is_mapped(conn):
    # Map the previously-unmapped custom template to a valid Garmin
    # category/subcategory pair, mirroring what saving a mapping in the UI
    # does.
    hevy_db.save_mapping(conn, dev_mock.HEVY_DEV_CUSTOM_TEMPLATE_ID, 0, 1)
    client = dev_mock.MockHevyClient(conn)
    since_dt = datetime.now(timezone.utc) - timedelta(days=7)

    items = hevy_backfill.preview_items(conn, client, since_dt)

    # HEVY_DEV_BACKFILL_UNMAPPABLE_ID (Task 16's dev fixture for the "locked
    # but nothing to map" edge case) has no template id at all, so no saved
    # mapping can ever resolve it — it stays needs_mapping regardless.
    # Every other workout uses the now-mapped custom template or a built-in
    # exercise and must clear.
    resolvable = [
        item for item in items
        if item["workout"]["id"] != dev_mock.HEVY_DEV_BACKFILL_UNMAPPABLE_ID
    ]
    assert resolvable, "expected other demo workouts alongside the unmappable one"
    assert all(item["missing_template_ids"] == [] for item in resolvable)
    assert all(item["action"] != "needs_mapping" for item in resolvable)

    unmappable = next(
        item for item in items
        if item["workout"]["id"] == dev_mock.HEVY_DEV_BACKFILL_UNMAPPABLE_ID
    )
    assert unmappable["action"] == "needs_mapping"
    assert unmappable["missing_template_ids"] == []


def test_preview_describe_strategy_reports_missing_template_ids(conn):
    # The `describe` strategy short-circuits the first mapping check and
    # only calls the collector a second time, further down the function —
    # this exercises that second call site.
    cfg = config.load_config(conn)
    cfg["hevy_watch_strategy"] = "describe"
    config.save_config(conn, cfg)
    client = dev_mock.MockHevyClient(conn)
    since_dt = datetime.now(timezone.utc) - timedelta(days=7)

    items = hevy_backfill.preview_items(conn, client, since_dt)

    locked = next(
        item for item in items
        if item["workout"]["id"] == dev_mock.HEVY_DEV_UNMAPPED_ID
    )
    assert locked["action"] == "needs_mapping"
    assert locked["missing_template_ids"] == [dev_mock.HEVY_DEV_CUSTOM_TEMPLATE_ID]
