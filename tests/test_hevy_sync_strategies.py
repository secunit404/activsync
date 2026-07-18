"""Tests for hevy_sync part 2: strategies + the operation journal."""

import json
from datetime import datetime, timedelta, timezone

import pytest

from activsync import db, hevy_db, hevy_sync
from activsync.garmin_client import ActivityGone, GarminUploadRejected, SubcategoryRejected

NOW = datetime(2026, 7, 18, 12, 0, 0, tzinfo=timezone.utc)


def make_conn():
    return db.connect(":memory:")


def base_cfg(**overrides):
    cfg = {
        "hevy_watch_strategy": "merge",
        "hevy_grace_minutes": 120,
        "hevy_poll_interval_minutes": 10,
        "hevy_device_identity": {"manufacturer": 1, "product": 4534, "serial": 42},
    }
    cfg.update(overrides)
    return cfg


MAPPED_EX = {"title": "Bench Press (Barbell)", "exercise_template_id": "79D0BB3A",
             "sets": [{"type": "normal", "reps": 5, "weight_kg": 80.0},
                      {"type": "normal", "reps": 5, "weight_kg": 80.0}]}


def seed_row(conn, hevy_id="w1", start=None, end=None, exercises=None):
    start = start or "2026-07-18T10:00:00Z"
    end = end or "2026-07-18T11:00:00Z"
    payload = {"id": hevy_id, "title": "Push Day", "start_time": start,
               "end_time": end,
               "exercises": exercises if exercises is not None else [MAPPED_EX]}
    hevy_db.upsert_workout(conn, hevy_id, "Push Day", start, end,
                           "2026-07-18T11:05:00Z", payload)
    return hevy_db.get_workout(conn, hevy_id)


def seed_activity(conn, activity_id, start="2026-07-18 10:05:00",
                  duration=3300.0, activity_type="strength_training",
                  strava_id=None):
    db.insert_activity(
        conn, activity_id, activity_type, "Strength", "", start, "hash",
        "held", NOW, garmin_data=json.dumps({"duration": duration}))
    if strava_id is not None:
        conn.execute(
            "UPDATE activities SET strava_activity_id = ? WHERE garmin_activity_id = ?",
            (strava_id, activity_id))
        conn.commit()


class StubGarmin:
    """Records every call; behaviors configurable per test."""

    def __init__(self):
        self.calls = []
        self.exercise_sets = {"exerciseSets": [{"old": True}]}
        self.fit_bytes = b"not-really-fit"
        self.put_exc = None
        self.upload_results = [{"upload_id": "u1", "activity_id": 999}]
        self.upload_exc = None
        self.snapshot_results = [[]]
        self.delete_exc = None
        self.daily_hr = {"heartRateValues": []}

    def get_exercise_sets(self, activity_id):
        self.calls.append(("get_exercise_sets", activity_id))
        return self.exercise_sets

    def put_exercise_sets(self, activity_id, payload):
        self.calls.append(("put_exercise_sets", activity_id, payload))
        if self.put_exc:
            raise self.put_exc

    def set_title(self, activity_id, title):
        self.calls.append(("set_title", activity_id, title))

    def set_description(self, activity_id, description):
        self.calls.append(("set_description", activity_id, description))

    def delete_activity(self, activity_id):
        self.calls.append(("delete_activity", activity_id))
        if self.delete_exc:
            raise self.delete_exc

    def download_fit(self, activity_id):
        self.calls.append(("download_fit", activity_id))
        return self.fit_bytes

    def upload_fit(self, fit_path):
        self.calls.append(("upload_fit", fit_path))
        if self.upload_exc:
            raise self.upload_exc
        return self.upload_results.pop(0)

    def list_activity_ids_near(self, start_time):
        self.calls.append(("list_activity_ids_near", start_time))
        return self.snapshot_results.pop(0)

    def get_daily_heart_rates(self, date_str):
        self.calls.append(("get_daily_heart_rates", date_str))
        return self.daily_hr

    def called(self, name):
        return [c for c in self.calls if c[0] == name]


class NoHevy:
    def iter_events_since(self, since):
        return []

    def get_exercise_template(self, template_id):
        return None


def process(conn, garmin, row, cfg, now=NOW):
    hevy_sync.process_workout(conn, garmin, NoHevy(), row, cfg, now)
    return hevy_db.get_workout(conn, row["hevy_id"])


# -- merge ------------------------------------------------------------------

def test_merge_happy_path():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)
    result = process(conn, garmin, row, base_cfg())

    assert result["status"] == "merged"
    assert result["source_garmin_activity_id"] == 111
    assert result["garmin_activity_id"] == 111
    assert result["applied_strategy"] == "merge"
    assert result["garmin_applied_updated_at"] == "2026-07-18T11:05:00Z"
    # sets were backed up before the PUT
    assert hevy_db.get_backup(conn, 111) is not None
    put = garmin.called("put_exercise_sets")[0]
    assert put[1] == 111
    categories = {ex["category"] for s in put[2]["exerciseSets"]
                  for ex in s["exercises"]}
    assert categories == {"BENCH_PRESS"}
    assert garmin.called("set_title")[0][2] == "Push Day"
    assert "synced by activsync" in garmin.called("set_description")[0][2]


def test_merge_subcategory_rejection_lists_candidates_without_blame():
    conn = make_conn()
    garmin = StubGarmin()
    garmin.put_exc = SubcategoryRejected("400 invalid sub-category")
    row = seed_row(conn)
    seed_activity(conn, 111)
    result = process(conn, garmin, row, base_cfg())

    assert result["status"] == "needs_mapping"
    assert "Bench Press (Barbell)" in result["error"]  # subcategory 1 != 0
    # no single mapping was blamed
    assert hevy_db.get_mapping(conn, "79D0BB3A") is None or \
        hevy_db.get_mapping(conn, "79D0BB3A")["garmin_rejected"] == 0


def test_merge_activity_gone_parks_needs_review():
    conn = make_conn()
    garmin = StubGarmin()
    garmin.put_exc = ActivityGone("404")
    row = seed_row(conn)
    seed_activity(conn, 111)
    result = process(conn, garmin, row, base_cfg())
    assert result["status"] == "needs_review"


# -- describe ---------------------------------------------------------------

def test_describe_never_pushes_sets():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)
    result = process(conn, garmin, row, base_cfg(hevy_watch_strategy="describe"))

    assert result["status"] == "described"
    assert garmin.called("put_exercise_sets") == []
    assert garmin.called("set_title") and garmin.called("set_description")


def test_edit_after_describe_only_updates_metadata():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)
    process(conn, garmin, row, base_cfg(hevy_watch_strategy="describe"))

    # a newer Hevy revision arrives
    payload = json.loads(hevy_db.get_workout(conn, "w1")["payload"])
    hevy_db.upsert_workout(conn, "w1", "Push Day v2", row["start_time"],
                           row["end_time"], "2026-07-18T12:30:00Z", payload)
    garmin.calls.clear()
    changed = hevy_sync.run_hevy_leg(conn, garmin, NoHevy(), base_cfg(), NOW)
    assert changed
    updated = hevy_db.get_workout(conn, "w1")
    assert updated["garmin_applied_updated_at"] == "2026-07-18T12:30:00Z"
    assert garmin.called("set_title") and garmin.called("set_description")
    assert garmin.called("put_exercise_sets") == []  # describe's promise


# -- replace ----------------------------------------------------------------

def replace_setup(conn, garmin):
    row = seed_row(conn)
    seed_activity(conn, 111)
    garmin.snapshot_results = [[111, 500]]      # pre-upload snapshot
    garmin.upload_results = [{"upload_id": "u1", "activity_id": 999}]
    return row


def test_replace_happy_path():
    conn = make_conn()
    garmin = StubGarmin()
    row = replace_setup(conn, garmin)
    result = process(conn, garmin, row, base_cfg(hevy_watch_strategy="replace"))

    assert result["status"] == "replaced"
    assert result["garmin_activity_id"] == 999
    backup = hevy_db.get_backup(conn, 111)
    assert backup["original_fit"] == garmin.fit_bytes  # full FIT backed up
    assert garmin.called("delete_activity") == [("delete_activity", 111)]
    assert len(garmin.called("upload_fit")) == 1
    op = conn.execute("SELECT * FROM hevy_operations").fetchone()
    assert op["phase"] == "done"


def test_replace_refuses_published_source():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111, strava_id=777)
    result = process(conn, garmin, row, base_cfg(hevy_watch_strategy="replace"))

    assert result["status"] == "needs_review"
    assert garmin.called("upload_fit") == []
    assert garmin.called("delete_activity") == []


def test_replace_crash_resume_no_second_upload():
    conn = make_conn()
    garmin = StubGarmin()
    row = replace_setup(conn, garmin)
    # upload call itself dies with an ambiguous error
    garmin.upload_exc = RuntimeError("connection reset mid-upload")
    result = process(conn, garmin, row, base_cfg(hevy_watch_strategy="replace"))
    assert result["status"] == "syncing"
    op = hevy_db.get_open_operation(conn, "w1")
    assert op["phase"] == "submission_unknown"

    # next tick: the upload actually landed — snapshot diff finds exactly 999
    garmin.upload_exc = None
    garmin.snapshot_results = [[111, 500, 999]]
    result = process(conn, garmin, hevy_db.get_workout(conn, "w1"),
                     base_cfg(hevy_watch_strategy="replace"))
    assert result["status"] == "replaced"
    assert len(garmin.called("upload_fit")) == 1  # NEVER auto-resubmitted


def test_submission_unknown_multiple_candidates_parks():
    conn = make_conn()
    garmin = StubGarmin()
    row = replace_setup(conn, garmin)
    garmin.upload_exc = RuntimeError("timeout")
    process(conn, garmin, row, base_cfg(hevy_watch_strategy="replace"))

    garmin.upload_exc = None
    garmin.snapshot_results = [[111, 500, 888, 999]]  # two new ids
    result = process(conn, garmin, hevy_db.get_workout(conn, "w1"),
                     base_cfg(hevy_watch_strategy="replace"))
    assert result["status"] == "needs_review"
    # the op stays open in needs_review — it keeps holding the source claim
    # and the publish interlock until the user resolves it
    assert hevy_db.get_open_operation(conn, "w1")["phase"] == "needs_review"
    assert len(garmin.called("upload_fit")) == 1


def test_upload_rejection_is_plain_failure():
    conn = make_conn()
    garmin = StubGarmin()
    row = replace_setup(conn, garmin)
    garmin.upload_exc = GarminUploadRejected("409 Duplicate Activity")
    result = process(conn, garmin, row, base_cfg(hevy_watch_strategy="replace"))
    assert result["status"] == "failed"
    assert hevy_db.get_open_operation(conn, "w1") is None


def test_finalize_delete_guard_when_target_equals_source():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)
    assert hevy_db.claim_source(conn, "w1", 111)
    op_id = hevy_db.open_operation(conn, "w1", "replace", 111, [500])
    hevy_db.update_operation(conn, op_id, phase="finalizing",
                             target_activity_id=111)  # pathological: same id
    hevy_db.set_workout_status(conn, "w1", "syncing")
    result = process(conn, garmin, hevy_db.get_workout(conn, "w1"),
                     base_cfg(hevy_watch_strategy="replace"))
    assert garmin.called("delete_activity") == []  # guard held
    assert result["status"] == "replaced"


def test_delete_failures_cap_to_needs_review():
    conn = make_conn()
    garmin = StubGarmin()
    garmin.delete_exc = RuntimeError("500")
    row = seed_row(conn)
    seed_activity(conn, 111)
    assert hevy_db.claim_source(conn, "w1", 111)
    op_id = hevy_db.open_operation(conn, "w1", "replace", 111, [500])
    hevy_db.update_operation(conn, op_id, phase="finalizing",
                             target_activity_id=999)
    hevy_db.set_workout_status(conn, "w1", "syncing")
    cfg = base_cfg(hevy_watch_strategy="replace")
    for _ in range(3):
        process(conn, garmin, hevy_db.get_workout(conn, "w1"), cfg)
    assert hevy_db.get_workout(conn, "w1")["status"] == "needs_review"


# -- passive ----------------------------------------------------------------

def test_grace_boundary_waiting_then_passive():
    cfg = base_cfg()
    # ended 119 minutes ago → still waiting for the watch
    conn = make_conn()
    garmin = StubGarmin()
    end = NOW - timedelta(minutes=119)
    row = seed_row(conn, start=(end - timedelta(hours=1)).isoformat(),
                   end=end.isoformat())
    result = process(conn, garmin, row, cfg)
    assert result["status"] == "waiting_watch"
    assert garmin.called("upload_fit") == []

    # ended 121 minutes ago → passive upload
    conn = make_conn()
    garmin = StubGarmin()
    garmin.snapshot_results = [[500]]
    garmin.upload_results = [{"upload_id": "u1", "activity_id": 999}]
    end = NOW - timedelta(minutes=121)
    row = seed_row(conn, start=(end - timedelta(hours=1)).isoformat(),
                   end=end.isoformat())
    result = process(conn, garmin, row, cfg)
    assert result["status"] == "uploaded_passive"
    assert result["garmin_activity_id"] == 999
    assert garmin.called("delete_activity") == []  # passive never deletes


def test_passive_wrong_type_guard():
    conn = make_conn()
    garmin = StubGarmin()
    end = NOW - timedelta(minutes=121)
    start = end - timedelta(hours=1)
    row = seed_row(conn, start=start.isoformat(), end=end.isoformat())
    # an overlapping activity of the WRONG type exists — blind upload would dupe
    seed_activity(conn, 333, start=start.strftime("%Y-%m-%d %H:%M:%S"),
                  duration=3600, activity_type="indoor_cardio")
    result = process(conn, garmin, row, base_cfg())
    assert result["status"] == "needs_review"
    assert "indoor_cardio" in (result["error"] or "")
    assert garmin.called("upload_fit") == []


# -- matching outcomes ------------------------------------------------------

def test_multiple_matches_park_needs_review():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111, start="2026-07-18 10:02:00")
    seed_activity(conn, 222, start="2026-07-18 10:04:00")
    result = process(conn, garmin, row, base_cfg())
    assert result["status"] == "needs_review"


# -- lease ------------------------------------------------------------------

def test_lease_blocks_concurrent_processing():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)
    assert hevy_db.acquire_lease(conn, "w1", NOW)  # someone else holds it
    result = process(conn, garmin, row, base_cfg())
    assert result["status"] == "waiting_watch"  # untouched
    assert garmin.calls == []


# -- payload builder --------------------------------------------------------

def test_build_exercise_sets_payload_shape():
    from activsync.fit_builder import ResolvedExercise

    resolved = [ResolvedExercise("Bench Press (Barbell)", 0, 1,
                                 [{"type": "normal", "reps": 5, "weight_kg": 80.0},
                                  {"type": "normal", "reps": 3, "weight_kg": 90.0}])]
    payload = hevy_sync.build_exercise_sets_payload(
        resolved, "2026-07-18 10:05:00", 3300.0)
    sets = payload["exerciseSets"]
    active = [s for s in sets if s["setType"] == "ACTIVE"]
    rest = [s for s in sets if s["setType"] == "REST"]
    assert len(active) == 2 and len(rest) == 1
    assert active[0]["exercises"][0]["category"] == "BENCH_PRESS"
    assert active[0]["exercises"][0]["name"] == "BARBELL_BENCH_PRESS"
    assert active[0]["weight"] == 80000.0  # grams
    assert active[0]["repetitionCount"] == 5
    assert active[0]["startTime"].startswith("2026-07-18T10:05:00")


def test_build_exercise_sets_payload_rejects_unknown():
    from activsync.fit_builder import ResolvedExercise

    resolved = [ResolvedExercise("Mystery", 65534, 0, [{"reps": 1}])]
    with pytest.raises(ValueError):
        hevy_sync.build_exercise_sets_payload(resolved, "2026-07-18 10:05:00", 600.0)
