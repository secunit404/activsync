"""Tests for hevy_sync part 2: strategies + the operation journal."""

import json
from datetime import datetime, timedelta, timezone

import pytest

from activsync import db, hevy_db, hevy_sync
from activsync.fit_builder import DeviceIdentity
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
                  strava_id=None, calories=None, avg_hr=None):
    garmin_data = {"duration": duration}
    if calories is not None:
        garmin_data["calories"] = calories
    if avg_hr is not None:
        garmin_data["avg_hr"] = avg_hr
    db.insert_activity(
        conn, activity_id, activity_type, "Strength", "", start, "hash",
        "held", NOW, garmin_data=json.dumps(garmin_data))
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
        self.activities_near = []
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

    def list_activities_near(self, start_time):
        self.calls.append(("list_activities_near", start_time))
        return list(self.activities_near)

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


def test_merge_can_leave_the_existing_description_untouched():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)

    result = process(
        conn,
        garmin,
        row,
        base_cfg(hevy_summary_on_structured=False),
    )

    assert result["status"] == "merged"
    assert garmin.called("set_title")
    assert garmin.called("set_description") == []


def test_review_mode_claims_a_single_match_without_touching_garmin():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)

    result = process(
        conn,
        garmin,
        row,
        base_cfg(hevy_match_mode="review"),
    )

    assert result["status"] == "awaiting_match"
    assert result["source_garmin_activity_id"] == 111
    assert result["garmin_activity_id"] is None
    assert garmin.calls == []


def test_reviewed_match_applies_the_selected_strategy():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)
    cfg = base_cfg(
        hevy_match_mode="review",
        hevy_summary_on_structured=False,
    )
    process(conn, garmin, row, cfg)

    result = hevy_sync.apply_match_choice(
        conn,
        garmin,
        NoHevy(),
        "w1",
        "merge",
        cfg,
        NOW,
    )

    assert result["status"] == "merged"
    assert result["garmin_activity_id"] == 111
    assert result["applied_strategy"] == "merge"


def test_published_match_requires_safe_choice_and_rejects_replace():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)
    db.set_published(conn, 111, 222, NOW)
    cfg = base_cfg(hevy_match_mode="automatic", hevy_watch_strategy="replace")

    awaiting = process(conn, garmin, row, cfg)

    assert awaiting["status"] == "awaiting_match"
    assert "Already published to Strava" in awaiting["error"]
    assert garmin.calls == []

    with pytest.raises(ValueError, match="already on Strava"):
        hevy_sync.apply_match_choice(
            conn,
            garmin,
            NoHevy(),
            "w1",
            "replace",
            cfg,
            NOW,
        )

    assert hevy_db.get_workout(conn, "w1")["status"] == "awaiting_match"
    assert hevy_db.get_open_operation(conn, "w1") is None


def test_published_match_can_merge_into_same_activity_without_duplicate():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)
    db.set_published(conn, 111, 222, NOW)
    cfg = base_cfg(hevy_match_mode="automatic", hevy_watch_strategy="replace")
    process(conn, garmin, row, cfg)

    result = hevy_sync.apply_match_choice(
        conn,
        garmin,
        NoHevy(),
        "w1",
        "merge",
        cfg,
        NOW,
    )

    assert result["status"] == "merged"
    assert result["garmin_activity_id"] == 111
    assert [activity["garmin_activity_id"] for activity in db.list_activities(conn)] == [111]


def test_switching_to_automatic_releases_an_existing_review_on_next_leg():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)
    process(conn, garmin, row, base_cfg(hevy_match_mode="review"))

    changed = hevy_sync.run_hevy_leg(
        conn,
        garmin,
        NoHevy(),
        base_cfg(hevy_match_mode="automatic", hevy_watch_strategy="merge"),
        NOW,
    )

    assert changed is True
    assert hevy_db.get_workout(conn, "w1")["status"] == "merged"


def test_review_mode_can_choose_description_for_an_unmapped_workout():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(
        conn,
        exercises=[
            {
                "title": "Unknown movement",
                "exercise_template_id": "unknown-template",
                "sets": [],
            }
        ],
    )
    seed_activity(conn, 111)
    cfg = base_cfg(hevy_match_mode="review")
    awaiting = process(conn, garmin, row, cfg)

    result = hevy_sync.apply_match_choice(
        conn,
        garmin,
        NoHevy(),
        "w1",
        "describe",
        cfg,
        NOW,
    )

    assert awaiting["status"] == "awaiting_match"
    assert result["status"] == "described"
    assert garmin.called("put_exercise_sets") == []
    assert garmin.called("set_description")


def test_merge_subcategory_rejection_lists_candidates_without_blame():
    conn = make_conn()
    garmin = StubGarmin()
    garmin.put_exc = SubcategoryRejected("400 invalid sub-category")
    row = seed_row(conn)
    seed_activity(conn, 111)
    result = process(conn, garmin, row, base_cfg())

    assert result["status"] == "needs_mapping"
    assert "Bench Press (Barbell)" in result["error"]  # subcategory 1 != 0
    # No single exercise mapping is blamed.
    assert hevy_db.get_mapping(conn, "79D0BB3A") is None


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


def test_backfill_claim_is_used_without_rematching_cached_duration():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(
        conn,
        111,
        start="2026-07-18 10:00:00",
        duration=0,
    )
    assert hevy_db.claim_source(conn, "w1", 111)
    garmin.snapshot_results = [[111, 500]]

    result = process(
        conn,
        garmin,
        hevy_db.get_workout(conn, "w1"),
        base_cfg(hevy_watch_strategy="replace"),
    )

    assert result["status"] == "replaced"
    assert garmin.called("download_fit") == [("download_fit", 111)]


def test_replace_can_leave_the_existing_description_untouched():
    conn = make_conn()
    garmin = StubGarmin()
    row = replace_setup(conn, garmin)

    result = process(
        conn,
        garmin,
        row,
        base_cfg(
            hevy_watch_strategy="replace",
            hevy_summary_on_structured=False,
        ),
    )

    assert result["status"] == "replaced"
    assert garmin.called("set_title")
    assert garmin.called("set_description") == []


def test_replace_description_uses_metrics_from_the_garmin_watch_activity():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111, calories=487.4, avg_hr=126.6)
    garmin.snapshot_results = [[111, 500]]

    result = process(
        conn,
        garmin,
        row,
        base_cfg(
            hevy_watch_strategy="replace",
            hevy_description_template="{calories}\n{avg_hr}",
        ),
    )

    assert result["status"] == "replaced"
    assert garmin.called("set_description") == [
        ("set_description", 999, "🔥 487 kcal\n❤️ avg 127 bpm")
    ]


def test_replace_upgrades_generic_fallback_to_detected_watch_identity(monkeypatch):
    conn = make_conn()
    generic = {"manufacturer": 1, "product": 0, "serial": 123}
    db.set_config_value(conn, "settings", {"hevy_device_identity": generic})
    garmin = StubGarmin()
    row = replace_setup(conn, garmin)
    detected = DeviceIdentity(manufacturer=1, product=4534, serial=987)
    monkeypatch.setattr("activsync.hevy_apply.identity_from_fit",
                        lambda _fit: detected)

    process(conn, garmin, row, base_cfg(
        hevy_watch_strategy="replace", hevy_device_identity=generic))

    stored = db.get_config_value(conn, "settings")["hevy_device_identity"]
    assert stored == {"manufacturer": 1, "product": 4534, "serial": 987}


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
    garmin.activities_near = [
        {"activityId": 111, "activityType": {"typeKey": "strength_training"},
         "startTimeGMT": "2026-07-18 10:05:00", "duration": 3300},
        {"activityId": 999, "activityType": {"typeKey": "strength_training"},
         "startTimeGMT": "2026-07-18 10:00:00", "duration": 3600},
    ]
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
    # two new ids, BOTH plausible strength uploads near the workout start
    garmin.activities_near = [
        {"activityId": 888, "activityType": {"typeKey": "strength_training"},
         "startTimeGMT": "2026-07-18 10:00:00", "duration": 3600},
        {"activityId": 999, "activityType": {"typeKey": "strength_training"},
         "startTimeGMT": "2026-07-18 10:01:00", "duration": 3600},
    ]
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


def test_description_surfaces_set_types_rpe_and_custom_metrics():
    workout = {
        "title": "Push Day",
        "start_time": "2026-07-18T10:00:00Z",
        "end_time": "2026-07-18T11:00:00Z",
        "exercises": [{
            "title": "Bench Press",
            "sets": [
                {"type": "warmup", "reps": 10, "weight_kg": 40},
                {"type": "normal", "reps": 5, "weight_kg": 80, "rpe": 8},
                {"type": "dropset", "reps": 8, "weight_kg": 60, "rpe": 9,
                 "custom_metric": 12.5},
                {"type": "failure", "reps": 10, "weight_kg": 50},
            ],
        }],
    }

    description = hevy_sync.generate_description(workout)

    assert "3 sets" in description
    assert "1 warmup" in description
    assert "1 dropset" in description
    assert "1 failure" in description
    assert "RPE 8, 9" in description
    assert "metric 12.5" in description


# -- Checkpoint C.1: review-driven safety tests ------------------------------

def test_submission_unknown_ignores_unrelated_activities():
    """A new run appearing in the window must NOT be adopted as the target."""
    conn = make_conn()
    garmin = StubGarmin()
    row = replace_setup(conn, garmin)
    garmin.upload_exc = RuntimeError("timeout")
    process(conn, garmin, row, base_cfg(hevy_watch_strategy="replace"))

    garmin.upload_exc = None
    # one new id, but it's a RUN far from the workout start
    garmin.activities_near = [
        {"activityId": 111, "activityType": {"typeKey": "strength_training"},
         "startTimeGMT": "2026-07-18 10:05:00", "duration": 3300},
        {"activityId": 500, "activityType": {"typeKey": "running"},
         "startTimeGMT": "2026-07-18 07:00:00", "duration": 1800},
        {"activityId": 888, "activityType": {"typeKey": "running"},
         "startTimeGMT": "2026-07-18 16:00:00", "duration": 1800},
    ]
    result = process(conn, garmin, hevy_db.get_workout(conn, "w1"),
                     base_cfg(hevy_watch_strategy="replace"))
    # unresolved: still syncing, no adoption, nothing renamed or deleted
    assert result["status"] == "syncing"
    assert hevy_db.get_open_operation(conn, "w1")["phase"] == "submission_unknown"
    assert garmin.called("set_title") == []
    assert garmin.called("delete_activity") == []


def test_submission_unknown_adopts_strict_match_only():
    conn = make_conn()
    garmin = StubGarmin()
    row = replace_setup(conn, garmin)
    garmin.upload_exc = RuntimeError("timeout")
    process(conn, garmin, row, base_cfg(hevy_watch_strategy="replace"))

    garmin.upload_exc = None
    garmin.activities_near = [
        {"activityId": 999, "activityType": {"typeKey": "strength_training"},
         "startTimeGMT": "2026-07-18 10:00:00", "duration": 3600},  # the upload
        {"activityId": 777, "activityType": {"typeKey": "running"},
         "startTimeGMT": "2026-07-18 10:01:00", "duration": 3500},  # wrong type
    ]
    result = process(conn, garmin, hevy_db.get_workout(conn, "w1"),
                     base_cfg(hevy_watch_strategy="replace"))
    assert result["status"] == "replaced"
    assert result["garmin_activity_id"] == 999


def test_finalize_resume_from_delete_step_treats_404_as_done():
    """Crash after the delete happened: retry sees 404 → success, not parking."""
    conn = make_conn()
    garmin = StubGarmin()
    garmin.delete_exc = RuntimeError("404 Not Found for url")
    row = seed_row(conn)
    seed_activity(conn, 111)
    assert hevy_db.claim_source(conn, "w1", 111)
    op_id = hevy_db.open_operation(conn, "w1", "replace", 111, [500])
    hevy_db.update_operation(conn, op_id, phase="finalizing",
                             target_activity_id=999, next_step="delete")
    hevy_db.set_workout_status(conn, "w1", "syncing")
    result = process(conn, garmin, hevy_db.get_workout(conn, "w1"),
                     base_cfg(hevy_watch_strategy="replace"))
    assert result["status"] == "replaced"
    assert garmin.called("set_title") == []  # metadata step already done


def test_finalize_resume_from_finalize_step_links_without_side_effects():
    """Crash after delete but before the terminal transition: resume only
    links — no metadata calls, no delete attempt."""
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)
    assert hevy_db.claim_source(conn, "w1", 111)
    op_id = hevy_db.open_operation(conn, "w1", "replace", 111, [500])
    hevy_db.update_operation(conn, op_id, phase="finalizing",
                             target_activity_id=999, next_step="finalize")
    hevy_db.set_workout_status(conn, "w1", "syncing")
    result = process(conn, garmin, hevy_db.get_workout(conn, "w1"),
                     base_cfg(hevy_watch_strategy="replace"))
    assert result["status"] == "replaced"
    assert result["garmin_activity_id"] == 999
    assert garmin.calls == []  # nothing left to do against Garmin
    op = conn.execute("SELECT * FROM hevy_operations").fetchone()
    assert op["phase"] == "done"


def test_terminal_transition_is_atomic():
    """Operation completion and workout linking land together, never split."""
    conn = make_conn()
    garmin = StubGarmin()
    row = replace_setup(conn, garmin)
    process(conn, garmin, row, base_cfg(hevy_watch_strategy="replace"))
    op = conn.execute("SELECT * FROM hevy_operations").fetchone()
    result = hevy_db.get_workout(conn, "w1")
    assert op["phase"] == "done"
    assert result["status"] == "replaced"
    assert result["garmin_activity_id"] == 999
    assert result["garmin_applied_updated_at"] == "2026-07-18T11:05:00Z"


def test_failed_rows_are_not_auto_retried():
    conn = make_conn()
    garmin = StubGarmin()
    row = replace_setup(conn, garmin)
    garmin.upload_exc = GarminUploadRejected("409 Duplicate Activity")
    process(conn, garmin, row, base_cfg(hevy_watch_strategy="replace"))
    assert hevy_db.get_workout(conn, "w1")["status"] == "failed"

    garmin.upload_exc = None
    garmin.snapshot_results = [[111, 500]]
    garmin.upload_results = [{"upload_id": "u2", "activity_id": 998}]
    changed = hevy_sync.run_hevy_leg(conn, garmin, NoHevy(),
                                     base_cfg(hevy_watch_strategy="replace"), NOW)
    assert len(garmin.called("upload_fit")) == 1  # no automatic re-upload
    assert hevy_db.get_workout(conn, "w1")["status"] == "failed"


def test_needs_mapping_rows_wait_for_explicit_wake():
    conn = make_conn()
    garmin = StubGarmin()
    garmin.put_exc = SubcategoryRejected("400 invalid sub-category")
    row = seed_row(conn)
    seed_activity(conn, 111)
    process(conn, garmin, row, base_cfg())
    assert hevy_db.get_workout(conn, "w1")["status"] == "needs_mapping"

    # next tick: the built-in mapping still resolves, but the row stays
    # parked — the identical rejected PUT must not repeat automatically
    garmin.put_exc = None
    garmin.calls.clear()
    hevy_sync.run_hevy_leg(conn, garmin, NoHevy(), base_cfg(), NOW)
    assert garmin.called("put_exercise_sets") == []
    assert hevy_db.get_workout(conn, "w1")["status"] == "needs_mapping"

    # An unrelated parked row must not wake when w1's template is mapped.
    seed_row(conn, hevy_id="w2", exercises=[{
        "title": "Other Custom", "exercise_template_id": "OTHER02",
        "sets": [{"type": "normal", "reps": 3}],
    }])
    hevy_db.set_workout_status(conn, "w2", "needs_mapping",
                               error="unmapped exercises: Other Custom")

    # Explicit targeted wake (mapping saved) re-enters only the affected flow.
    assert hevy_db.wake_needs_mapping(conn, template_id="79D0BB3A") == 1
    hevy_sync.run_hevy_leg(conn, garmin, NoHevy(), base_cfg(), NOW)
    assert hevy_db.get_workout(conn, "w1")["status"] == "merged"
    assert hevy_db.get_workout(conn, "w2")["status"] == "needs_mapping"


def test_describe_strategy_passive_path_still_gated():
    """describe bypasses the gate only while a watch match is possible; the
    passive fallback pushes structured sets and must be gated."""
    conn = make_conn()
    garmin = StubGarmin()
    end = NOW - timedelta(minutes=121)
    row = seed_row(conn, start=(end - timedelta(hours=1)).isoformat(),
                   end=end.isoformat(),
                   exercises=[{"title": "Custom Blaster",
                               "exercise_template_id": "CUSTOM01",
                               "sets": [{"type": "normal", "reps": 5}]}])
    result = process(conn, garmin, row, base_cfg(hevy_watch_strategy="describe"))
    assert result["status"] == "needs_mapping"
    assert garmin.called("upload_fit") == []
    assert hevy_db.get_open_operation(conn, "w1") is None  # no op left open


def test_pre_submission_failure_closes_operation():
    """An unmapped exercise surfacing in the uploading phase must not leave
    the op wedged in `uploading` with the row `syncing` forever."""
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn, exercises=[{"title": "Custom Blaster",
                                     "exercise_template_id": "CUSTOM01",
                                     "sets": [{"type": "normal", "reps": 5}]}])
    op_id = hevy_db.open_operation(conn, "w1", "upload_passive", None, [])
    hevy_db.update_operation(conn, op_id, phase="uploading")
    hevy_db.set_workout_status(conn, "w1", "syncing")
    result = process(conn, garmin, hevy_db.get_workout(conn, "w1"), base_cfg())
    assert result["status"] == "needs_mapping"
    assert hevy_db.get_open_operation(conn, "w1") is None
    assert garmin.called("upload_fit") == []


def test_resolution_candidates_require_complete_metadata():
    """Ambiguous-upload recovery fails closed on partial Garmin rows."""
    from activsync.hevy_apply import _resolution_candidates

    row = {
        "start_time": "2026-07-18T10:00:00Z",
        "end_time": "2026-07-18T11:00:00Z",
    }
    incomplete = [
        # no type
        {"activityId": 901, "startTimeGMT": "2026-07-18 10:00:00",
         "duration": 3600},
        # no duration
        {"activityId": 902, "activityType": {"typeKey": "strength_training"},
         "startTimeGMT": "2026-07-18 10:00:00"},
        # non-positive duration
        {"activityId": 903, "activityType": {"typeKey": "other"},
         "startTimeGMT": "2026-07-18 10:00:00", "duration": 0},
        # malformed id
        {"activityId": "not-an-id",
         "activityType": {"typeKey": "strength_training"},
         "startTimeGMT": "2026-07-18 10:00:00", "duration": 3600},
    ]
    assert _resolution_candidates(incomplete, row, set(), None) == set()


def test_resolution_candidates_normalize_ids_before_exclusion():
    from activsync.hevy_apply import _resolution_candidates

    row = {
        "start_time": "2026-07-18T10:00:00Z",
        "end_time": "2026-07-18T11:00:00Z",
    }
    activity = {
        "activityId": "999",
        "activityType": {"typeKey": "strength_training"},
        "startTimeGMT": "2026-07-18 10:00:00",
        "duration": 3600,
    }
    assert _resolution_candidates([activity], row, {999}, None) == set()


class StubStrava:
    def __init__(self):
        self.calls = []

    def update_activity_metadata(self, strava_activity_id, name, description):
        self.calls.append((strava_activity_id, name, description))


def test_post_sync_edit_reaches_strava():
    conn = make_conn()
    garmin = StubGarmin()
    strava = StubStrava()
    row = seed_row(conn)
    seed_activity(conn, 111)
    process(conn, garmin, row, base_cfg())
    assert hevy_db.get_workout(conn, "w1")["status"] == "merged"
    # publish the linked activity on Strava
    conn.execute("UPDATE activities SET strava_activity_id = 555 "
                 "WHERE garmin_activity_id = 111")
    conn.commit()

    hevy_sync.run_hevy_leg(conn, garmin, NoHevy(), base_cfg(), NOW,
                           strava=strava)
    assert strava.calls and strava.calls[0][0] == 555
    updated = hevy_db.get_workout(conn, "w1")
    assert updated["strava_applied_updated_at"] == updated["source_updated_at"]

    # already caught up → no second push
    strava.calls.clear()
    hevy_sync.run_hevy_leg(conn, garmin, NoHevy(), base_cfg(), NOW,
                           strava=strava)
    assert strava.calls == []


def test_post_sync_mapping_wake_keeps_original_strategy():
    conn = make_conn()
    garmin = StubGarmin()
    row = seed_row(conn)
    seed_activity(conn, 111)
    process(conn, garmin, row, base_cfg(hevy_watch_strategy="merge"))

    custom = {
        "title": "Custom Press", "exercise_template_id": "CUSTOM01",
        "sets": [{"type": "normal", "reps": 8, "weight_kg": 20}],
    }
    payload = {"id": "w1", "title": "Push Day",
               "start_time": "2026-07-18T10:00:00Z",
               "end_time": "2026-07-18T11:00:00Z", "exercises": [custom]}
    hevy_db.upsert_workout(
        conn, "w1", "Push Day", payload["start_time"], payload["end_time"],
        "2026-07-18T12:30:00Z", payload)
    garmin.calls.clear()

    hevy_sync.run_hevy_leg(
        conn, garmin, NoHevy(), base_cfg(hevy_watch_strategy="replace"), NOW)
    parked = hevy_db.get_workout(conn, "w1")
    assert parked["status"] == "needs_mapping"
    # Even when the template API has no row, the workout payload supplies
    # enough cache data for the mapping UI to remain actionable.
    assert hevy_db.get_template(conn, "CUSTOM01")["title"] == "Custom Press"

    hevy_db.save_mapping(conn, "CUSTOM01", 0, 1)
    assert hevy_db.wake_needs_mapping(conn, template_id="CUSTOM01") == 1
    assert hevy_db.get_workout(conn, "w1")["status"] == "merged"

    hevy_sync.run_hevy_leg(
        conn, garmin, NoHevy(), base_cfg(hevy_watch_strategy="replace"), NOW)
    reapplied = hevy_db.get_workout(conn, "w1")
    assert reapplied["status"] == "merged"
    assert reapplied["applied_strategy"] == "merge"
    assert reapplied["garmin_applied_updated_at"] == "2026-07-18T12:30:00Z"
    assert garmin.called("put_exercise_sets")
    assert garmin.called("upload_fit") == []
    assert garmin.called("delete_activity") == []
