from datetime import datetime, timezone
from unittest.mock import MagicMock
from zoneinfo import ZoneInfo

from fastapi.testclient import TestClient

from activsync import api_routes, config, db, hevy_db
from activsync import server as server_module
from activsync.hevy_client import HevyAuthError
from activsync.server import create_app


def _connect_publish_services(conn):
    db.set_config_value(
        conn,
        "garmin_credentials",
        {"email": "athlete@example.com", "password": "secret"},
    )
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_tokens", {"refresh_token": "refresh"})


def test_app_state_starts_at_garmin_setup(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    response = TestClient(create_app(conn)).get("/api/v1/app")

    assert response.status_code == 200
    payload = response.json()
    assert payload["name"] == "ActivSync"
    assert payload["setup"] == {"complete": False, "step": "garmin"}
    assert payload["connections"]["broken"] == ["garmin", "strava"]
    assert payload["connections"]["garmin"]["connected"] is False
    assert payload["connections"]["strava"]["connected"] is False


def test_app_state_reports_connected_services_and_hevy(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(
        conn,
        "garmin_credentials",
        {"email": "athlete@example.com", "password": "secret"},
    )
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_tokens", {"refresh_token": "refresh"})
    db.set_config_value(conn, "hevy_api_key", "hevy-key")
    db.set_config_value(conn, "hevy_auth_ok", True)
    db.set_config_value(conn, "initial_sync_done", True)
    db.set_config_value(conn, "settings", {"hevy_enabled": True})
    monkeypatch.setenv("ACTIVSYNC_DEV_MOCK_DATA", "1")

    response = TestClient(create_app(conn)).get("/api/v1/app")

    assert response.status_code == 200
    payload = response.json()
    assert payload["development"] is True
    assert payload["setup"] == {"complete": True, "step": None}
    assert payload["connections"]["broken"] == []
    assert payload["connections"]["garmin"]["email"] == "athlete@example.com"
    assert payload["hevy"] == {
        "enabled": True,
        "connected": True,
        "status": "Connected",
    }


def test_app_state_exposes_and_dismisses_reconnect_report(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(
        conn,
        "catch_up_report",
        {"new": 4, "held": 3, "linked": 1, "days": 12},
    )
    client = TestClient(create_app(conn))

    before = client.get("/api/v1/app")
    dismissed = client.delete("/api/v1/catch-up-report")
    after = client.get("/api/v1/app")

    assert before.json()["catchUpReport"] == {
        "new": 4,
        "held": 3,
        "linked": 1,
        "days": 12,
    }
    assert before.json()["update"]["repoUrl"].endswith("/activsync")
    assert dismissed.status_code == 204
    assert after.json()["catchUpReport"] is None


def _seed_awaiting_hevy_match(conn):
    hevy_db.upsert_workout(
        conn,
        "hevy-match",
        "Leg Day",
        "2026-07-22T10:00:00Z",
        "2026-07-22T11:00:00Z",
        "2026-07-22T11:05:00Z",
        {"exercises": []},
    )
    hevy_db.set_workout_status(conn, "hevy-match", "awaiting_match")


def test_hevy_match_choice_runs_one_injected_strategy(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _seed_awaiting_hevy_match(conn)
    calls = []
    client = TestClient(
        create_app(
            conn,
            apply_hevy_match=lambda hevy_id, strategy: (
                calls.append((hevy_id, strategy)) or "merged"
            ),
        )
    )

    response = client.post(
        "/api/v1/hevy/hevy-match/match", json={"strategy": "merge"}
    )

    assert response.status_code == 200
    assert response.json() == {"message": "Merged Leg Day."}
    assert calls == [("hevy-match", "merge")]


def test_hevy_workout_detail_exposes_stored_sets_and_description_preview(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    hevy_db.upsert_workout(
        conn,
        "hevy-detail",
        "Upper body",
        "2026-07-22T10:00:00Z",
        "2026-07-22T11:05:00Z",
        "2026-07-22T11:06:00Z",
        {
            "title": "Upper body",
            "start_time": "2026-07-22T10:00:00Z",
            "end_time": "2026-07-22T11:05:00Z",
            "description": "Felt strong\nNo shoulder pain",
            "exercises": [
                {
                    "title": "Bench Press (Barbell)",
                    "exercise_template_id": "79D0BB3A",
                    "notes": "Pause on chest",
                    "sets": [
                        {
                            "type": "warmup",
                            "reps": 10,
                            "weight_kg": 40,
                        },
                        {
                            "type": "normal",
                            "reps": 8,
                            "weight_kg": 80,
                            "rpe": 8.5,
                        },
                    ],
                }
            ],
        },
    )

    response = TestClient(create_app(conn)).get("/api/v1/hevy/hevy-detail")

    assert response.status_code == 200
    payload = response.json()
    assert payload["notes"] == "Felt strong\nNo shoulder pain"
    assert payload["exercises"] == [
        {
            "title": "Bench Press (Barbell)",
            "notes": "Pause on chest",
            "templateId": "79D0BB3A",
            "sets": [
                {
                    "number": 1,
                    "setType": "warmup",
                    "reps": 10.0,
                    "weightKg": 40.0,
                    "distanceMeters": None,
                    "durationSeconds": None,
                    "rpe": None,
                    "customMetric": None,
                },
                {
                    "number": 2,
                    "setType": "normal",
                    "reps": 8.0,
                    "weightKg": 80.0,
                    "distanceMeters": None,
                    "durationSeconds": None,
                    "rpe": 8.5,
                    "customMetric": None,
                },
            ],
        }
    ]
    assert "Bench Press (Barbell)" in payload["descriptionPreview"]
    assert "— synced by activsync" in payload["descriptionPreview"]


def test_hevy_workout_detail_rejects_unknown_workout(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))

    response = TestClient(create_app(conn)).get("/api/v1/hevy/missing")

    assert response.status_code == 404


def test_hevy_match_choice_rejects_unknown_strategy(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _seed_awaiting_hevy_match(conn)
    client = TestClient(create_app(conn, apply_hevy_match=lambda *_: "merged"))

    response = client.post(
        "/api/v1/hevy/hevy-match/match", json={"strategy": "erase"}
    )

    assert response.status_code == 400


def test_hevy_match_choice_requires_awaiting_state(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _seed_awaiting_hevy_match(conn)
    hevy_db.set_workout_status(conn, "hevy-match", "merged")
    client = TestClient(create_app(conn, apply_hevy_match=lambda *_: "merged"))

    response = client.post(
        "/api/v1/hevy/hevy-match/match", json={"strategy": "merge"}
    )

    assert response.status_code == 409


def test_hevy_match_choice_rejects_replace_for_published_activity(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _seed_awaiting_hevy_match(conn)
    db.insert_activity(
        conn,
        123,
        "strength_training",
        "Published strength",
        "",
        "2026-07-22 10:00:00",
        "hash",
        "held",
        datetime.now(timezone.utc),
        garmin_data='{"duration": 3600}',
    )
    db.set_published(conn, 123, 456, datetime.now(timezone.utc))
    assert hevy_db.claim_source(conn, "hevy-match", 123)
    apply_match = MagicMock(return_value="replaced")
    client = TestClient(create_app(conn, apply_hevy_match=apply_match))

    response = client.post(
        "/api/v1/hevy/hevy-match/match", json={"strategy": "replace"}
    )

    assert response.status_code == 409
    assert "already on Strava" in response.json()["detail"]
    apply_match.assert_not_called()


def test_hevy_match_choice_marks_rejected_key_for_reconnect(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _seed_awaiting_hevy_match(conn)

    def reject_key(_hevy_id, _strategy):
        raise HevyAuthError("bad key")

    client = TestClient(create_app(conn, apply_hevy_match=reject_key))

    response = client.post(
        "/api/v1/hevy/hevy-match/match", json={"strategy": "describe"}
    )

    assert response.status_code == 401
    assert "reconnect Hevy" in response.json()["detail"]
    assert db.get_config_value(conn, "hevy_auth_ok") is False


def test_skipping_an_awaiting_match_releases_its_garmin_claim(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    _seed_awaiting_hevy_match(conn)
    db.insert_activity(
        conn,
        123,
        "strength_training",
        "Morning strength",
        "",
        "2026-07-22 10:00:00",
        "hash",
        "held",
        datetime.now(timezone.utc),
        garmin_data='{"duration": 3600}',
    )
    db.set_published(conn, 123, 456, datetime.now(timezone.utc))
    assert hevy_db.claim_source(conn, "hevy-match", 123)
    client = TestClient(create_app(conn))

    queue = client.get("/api/v1/hevy/queue")
    response = client.post("/api/v1/hevy/hevy-match/skip")

    awaiting = queue.json()["inFlight"][0]
    assert awaiting["awaitingMatch"] is True
    assert awaiting["matchedGarminActivityId"] == 123
    assert awaiting["matchedGarminTitle"] == "Morning strength"
    assert awaiting["matchedStravaActivityId"] == 456
    assert awaiting["matchedStravaUrl"] == "https://www.strava.com/activities/456"
    assert response.status_code == 200
    row = hevy_db.get_workout(conn, "hevy-match")
    assert row["status"] == "skipped"
    assert row["source_garmin_activity_id"] is None


def test_hevy_queue_json_actions_preserve_workout_state_machine(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    for hevy_id, status, error in (
        ("needs-map", "needs_mapping", "Map an exercise"),
        ("skip-me", "failed", "Temporary failure"),
        ("restore-me", "skipped", None),
        ("fresh-me", "needs_review", "linked activity deleted on Garmin"),
    ):
        hevy_db.upsert_workout(
            conn,
            hevy_id,
            hevy_id.replace("-", " ").title(),
            "2026-07-19T08:00:00Z",
            "2026-07-19T09:00:00Z",
            "2026-07-19T10:00:00Z",
            {"exercises": []},
        )
        hevy_db.set_workout_status(conn, hevy_id, status, error=error)
    client = TestClient(create_app(conn))

    queue = client.get("/api/v1/hevy/queue")
    retried = client.post("/api/v1/hevy/needs-map/retry")
    skipped = client.post("/api/v1/hevy/skip-me/skip")
    restored = client.post("/api/v1/hevy/restore-me/unskip")
    fresh = client.post("/api/v1/hevy/fresh-me/resync-fresh")

    assert queue.status_code == 200
    assert queue.json()["counts"] == {
        "inFlight": 0,
        "problems": 3,
        "skipped": 1,
    }
    assert queue.json()["problems"][0]["hevyId"]
    assert retried.status_code == 200
    assert skipped.status_code == 200
    assert restored.status_code == 200
    assert fresh.status_code == 200
    assert hevy_db.get_workout(conn, "needs-map")["status"] == "waiting_watch"
    assert hevy_db.get_workout(conn, "skip-me")["status"] == "skipped"
    assert hevy_db.get_workout(conn, "restore-me")["status"] == "waiting_watch"
    assert hevy_db.get_workout(conn, "fresh-me")["status"] == "waiting_watch"


def test_activities_page_filters_sorts_and_uses_camel_case(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    now = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn,
        1,
        "running",
        "Older run",
        "Easy pace",
        "2026-07-18 08:00:00",
        "hash-1",
        "held",
        now,
        garmin_data='{"distance": 5000, "duration": 1500}',
    )
    db.insert_activity(
        conn,
        2,
        "cycling",
        "Newer ride",
        "",
        "2026-07-19 09:00:00",
        "hash-2",
        "published",
        now,
    )
    db.set_published(conn, 2, 987654, now)

    response = TestClient(create_app(conn)).get(
        "/api/v1/activities?sort=oldest&status=held&pageSize=10"
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["sort"] == "oldest"
    assert payload["status"] == "held"
    assert payload["counts"] == {
        "pending": 0,
        "held": 1,
        "published": 1,
        "missing": 0,
        "excluded": 0,
    }
    assert payload["pagination"] == {
        "page": 1,
        "pageSize": 10,
        "pageCount": 1,
        "totalCount": 1,
        "firstItem": 1,
        "lastItem": 1,
    }
    assert payload["items"][0]["garminActivityId"] == 1
    assert payload["items"][0]["startDateDisplay"] == "18 Jul"
    assert payload["items"][0]["detail"]["distance"] == "5.00 km"
    assert payload["items"][0]["detail"]["duration"] == "25m 00s"


def test_activities_page_clamps_to_last_page_and_builds_strava_link(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    now = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)
    for activity_id in range(1, 12):
        db.insert_activity(
            conn,
            activity_id,
            "running",
            f"Run {activity_id}",
            "",
            f"2026-07-{activity_id:02d} 09:00:00",
            f"hash-{activity_id}",
            "published",
            now,
        )
        db.set_published(conn, activity_id, 9000 + activity_id, now)

    response = TestClient(create_app(conn)).get(
        "/api/v1/activities?page=99&pageSize=10"
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["pagination"]["page"] == 2
    assert payload["pagination"]["pageCount"] == 2
    assert payload["pagination"]["firstItem"] == 11
    assert payload["pagination"]["lastItem"] == 11
    assert payload["items"][0]["stravaUrl"].startswith(
        "https://www.strava.com/activities/"
    )


def test_activities_week_total_sums_current_week_excluding_excluded(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    cfg = config.load_config(conn)
    cfg["display_timezone"] = "UTC"
    config.save_config(conn, cfg)

    # Wednesday, so the current week's Monday (2026-07-20) is unambiguous.
    fixed_now = datetime(2026, 7, 22, 12, 0, tzinfo=timezone.utc)
    monkeypatch.setattr(
        api_routes.timeutil,
        "to_local_now",
        lambda tz_name: fixed_now.astimezone(ZoneInfo(tz_name)),
    )

    monday = datetime(2026, 7, 20, 9, 0, tzinfo=timezone.utc)
    last_week = datetime(2026, 7, 17, 9, 0, tzinfo=timezone.utc)  # prior Friday

    def add(activity_id, start, seconds, status):
        db.insert_activity(
            conn,
            activity_id,
            "running",
            f"Run {activity_id}",
            "",
            start.strftime("%Y-%m-%d %H:%M:%S"),
            f"hash-{activity_id}",
            status,
            start,
            garmin_data=f'{{"duration": {seconds}}}',
        )

    add(1, monday, 42 * 60, "pending")
    add(2, monday, 20 * 60, "published")
    add(3, monday, 99 * 60, "excluded")    # must not count
    add(4, last_week, 60 * 60, "pending")  # must not count

    response = TestClient(create_app(conn)).get("/api/v1/activities")

    assert response.status_code == 200
    assert response.json()["weekTotal"] == {"seconds": 3720, "display": "1h 02m"}


def test_activities_week_total_uses_configured_timezone_for_week_boundary(
    tmp_path, monkeypatch
):
    # 2026-07-20 05:00 UTC is Monday morning in UTC but still Sunday night
    # (prior week) in Honolulu (UTC-10) -- proves the week boundary is
    # computed in the configured display_timezone, not defaulted to UTC.
    boundary_start = datetime(2026, 7, 20, 5, 0, tzinfo=timezone.utc)
    fixed_now = datetime(2026, 7, 22, 22, 0, tzinfo=timezone.utc)  # Wednesday

    monkeypatch.setattr(
        api_routes.timeutil,
        "to_local_now",
        lambda tz_name: fixed_now.astimezone(ZoneInfo(tz_name)),
    )

    def week_total_seconds_for(tz_name):
        conn = db.connect(str(tmp_path / f"test-{tz_name.replace('/', '-')}.db"))
        cfg = config.load_config(conn)
        cfg["display_timezone"] = tz_name
        config.save_config(conn, cfg)
        db.insert_activity(
            conn,
            1,
            "running",
            "Run",
            "",
            boundary_start.strftime("%Y-%m-%d %H:%M:%S"),
            "hash-1",
            "pending",
            boundary_start,
            garmin_data='{"duration": 1800}',
        )
        response = TestClient(create_app(conn)).get("/api/v1/activities")
        return response.json()["weekTotal"]["seconds"]

    assert week_total_seconds_for("UTC") == 1800
    assert week_total_seconds_for("Pacific/Honolulu") == 0


def test_activities_week_total_sums_float_durations_without_per_row_truncation(
    tmp_path, monkeypatch
):
    """Each duration must be summed as a float and rounded once at the end,
    not truncated per-row -- three rows each losing 0.9s to int() truncation
    would previously drift the total low by 2s (2.7s rounds to 3s lost)."""
    conn = db.connect(str(tmp_path / "test.db"))
    cfg = config.load_config(conn)
    cfg["display_timezone"] = "UTC"
    config.save_config(conn, cfg)

    fixed_now = datetime(2026, 7, 22, 12, 0, tzinfo=timezone.utc)
    monkeypatch.setattr(
        api_routes.timeutil,
        "to_local_now",
        lambda tz_name: fixed_now.astimezone(ZoneInfo(tz_name)),
    )

    monday = datetime(2026, 7, 20, 9, 0, tzinfo=timezone.utc)

    def add(activity_id, seconds):
        db.insert_activity(
            conn,
            activity_id,
            "running",
            f"Run {activity_id}",
            "",
            monday.strftime("%Y-%m-%d %H:%M:%S"),
            f"hash-{activity_id}",
            "pending",
            monday,
            garmin_data=f'{{"duration": {seconds}}}',
        )

    add(1, 100.9)
    add(2, 100.9)
    add(3, 100.9)

    response = TestClient(create_app(conn)).get("/api/v1/activities")

    assert response.status_code == 200
    # 100.9 * 3 == 302.7 -> rounds to 303, not 300 (3 x int(100.9)).
    assert response.json()["weekTotal"]["seconds"] == 303


def test_activities_week_total_excludes_future_dated_activity(tmp_path, monkeypatch):
    """dev_seed.py routinely seeds rows dated later in the current month, so
    a bare "forward with no end" fold would pull next week's activities into
    this week's tile. The window must be exclusive at week_start + 7 days."""
    conn = db.connect(str(tmp_path / "test.db"))
    cfg = config.load_config(conn)
    cfg["display_timezone"] = "UTC"
    config.save_config(conn, cfg)

    # Wednesday, so the current week's Monday (2026-07-20) is unambiguous.
    fixed_now = datetime(2026, 7, 22, 12, 0, tzinfo=timezone.utc)
    monkeypatch.setattr(
        api_routes.timeutil,
        "to_local_now",
        lambda tz_name: fixed_now.astimezone(ZoneInfo(tz_name)),
    )

    this_week = datetime(2026, 7, 20, 9, 0, tzinfo=timezone.utc)
    next_week = datetime(2026, 7, 27, 9, 0, tzinfo=timezone.utc)  # beyond week_end

    def add(activity_id, start, seconds):
        db.insert_activity(
            conn,
            activity_id,
            "running",
            f"Run {activity_id}",
            "",
            start.strftime("%Y-%m-%d %H:%M:%S"),
            f"hash-{activity_id}",
            "pending",
            start,
            garmin_data=f'{{"duration": {seconds}}}',
        )

    add(1, this_week, 30 * 60)
    add(2, next_week, 45 * 60)  # future-dated, must not count

    response = TestClient(create_app(conn)).get("/api/v1/activities")

    assert response.status_code == 200
    assert response.json()["weekTotal"] == {"seconds": 1800, "display": "30m"}


def test_activities_week_total_spans_dst_spring_forward_transition(
    tmp_path, monkeypatch
):
    """America/New_York springs forward on 2026-03-08. The current week
    (Mon 2026-03-09, already in EDT) must exclude an activity from the prior
    week (Fri 2026-03-06, still in EST) even though the wall-clock week
    boundary itself crosses the UTC-offset change."""
    conn = db.connect(str(tmp_path / "test.db"))
    cfg = config.load_config(conn)
    cfg["display_timezone"] = "America/New_York"
    config.save_config(conn, cfg)

    tz = ZoneInfo("America/New_York")
    fixed_now = datetime(2026, 3, 11, 12, 0, tzinfo=tz)  # Wednesday, post-transition
    monkeypatch.setattr(
        api_routes.timeutil,
        "to_local_now",
        lambda tz_name: fixed_now.astimezone(ZoneInfo(tz_name)),
    )

    this_monday = datetime(2026, 3, 9, 9, 0, tzinfo=tz)  # EDT
    last_friday = datetime(2026, 3, 6, 9, 0, tzinfo=tz)  # EST, prior week

    def add(activity_id, start_local, seconds):
        start_utc = start_local.astimezone(timezone.utc)
        db.insert_activity(
            conn,
            activity_id,
            "running",
            f"Run {activity_id}",
            "",
            start_utc.strftime("%Y-%m-%d %H:%M:%S"),
            f"hash-{activity_id}",
            "pending",
            start_utc,
            garmin_data=f'{{"duration": {seconds}}}',
        )

    add(1, this_monday, 40 * 60)
    add(2, last_friday, 60 * 60)  # prior week, must not count

    response = TestClient(create_app(conn)).get("/api/v1/activities")

    assert response.status_code == 200
    assert response.json()["weekTotal"] == {"seconds": 2400, "display": "40m"}


def test_activities_page_rejects_unknown_status_and_page_size(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))

    assert client.get("/api/v1/activities?status=queued").status_code == 422
    assert client.get("/api/v1/activities?pageSize=25").status_code == 422


def test_publish_activity_returns_json_and_updates_status(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    _connect_publish_services(conn)
    now = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn,
        21,
        "running",
        "Lunch run",
        "",
        "2026-07-19 12:00:00",
        "hash-21",
        "pending",
        now,
    )
    fake_garmin = MagicMock()
    fake_garmin.download_fit.return_value = b"FIT"
    fake_strava = MagicMock()
    fake_strava.find_existing_activity.return_value = None
    fake_strava.publish.return_value = 9021
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda conn: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda conn: fake_strava)

    response = TestClient(create_app(conn)).post("/api/v1/activities/21/publish")

    assert response.status_code == 200
    assert response.json() == {
        "message": "Published Lunch run to Strava",
        "severity": "success",
        "publishedCount": 0,
        "failedCount": 0,
        "blockedCount": 0,
    }
    assert db.get_activity(conn, 21)["publish_status"] == "published"


def test_bulk_publish_returns_counts(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    _connect_publish_services(conn)
    now = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)
    for activity_id in (31, 32):
        db.insert_activity(
            conn,
            activity_id,
            "running",
            f"Run {activity_id}",
            "",
            f"2026-07-{activity_id - 20:02d} 08:00:00",
            f"hash-{activity_id}",
            "held",
            now,
        )
    fake_garmin = MagicMock()
    fake_garmin.download_fit.return_value = b"FIT"
    fake_strava = MagicMock()
    fake_strava.find_existing_activity.return_value = None
    fake_strava.publish.side_effect = [9031, 9032]
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda conn: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda conn: fake_strava)

    response = TestClient(create_app(conn)).post(
        "/api/v1/activities/publish", json={"activityIds": [31, 32]}
    )

    assert response.status_code == 200
    assert response.json()["message"] == "Published 2 activities to Strava"
    assert response.json()["publishedCount"] == 2


def test_edit_exclude_and_restore_activity_json_actions(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    now = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn,
        41,
        "running",
        "Old title",
        "Old description",
        "2026-07-19 12:00:00",
        "hash-41",
        "pending",
        now,
    )
    fake_garmin = MagicMock()
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda conn: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda conn: MagicMock())
    client = TestClient(create_app(conn))

    edit_response = client.put(
        "/api/v1/activities/41",
        json={"title": "New title", "description": "New description"},
    )
    exclude_response = client.post("/api/v1/activities/41/exclude")
    restore_response = client.post("/api/v1/activities/41/restore")

    assert edit_response.status_code == 200
    assert edit_response.json()["message"] == "Saved changes to Old title"
    assert exclude_response.json()["message"] == "Excluded New title"
    assert restore_response.json()["message"] == "Restored New title"
    assert db.get_activity(conn, 41)["publish_status"] == "pending"
    fake_garmin.update_activity_metadata.assert_called_once_with(
        41, "New title", "New description"
    )


def test_json_actions_return_actionable_errors(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    now = datetime(2026, 7, 19, 12, 0, tzinfo=timezone.utc)
    db.insert_activity(
        conn,
        51,
        "running",
        "Disconnected run",
        "",
        "2026-07-19 12:00:00",
        "hash-51",
        "pending",
        now,
    )
    client = TestClient(create_app(conn))

    publish_response = client.post("/api/v1/activities/51/publish")
    blank_edit_response = client.put(
        "/api/v1/activities/51", json={"title": "   ", "description": ""}
    )
    missing_response = client.post("/api/v1/activities/999/exclude")

    assert publish_response.status_code == 409
    assert "Both Garmin and Strava" in publish_response.json()["detail"]
    assert blank_edit_response.status_code == 422
    assert blank_edit_response.json()["detail"] == "Activity title cannot be blank"
    assert missing_response.status_code == 404
