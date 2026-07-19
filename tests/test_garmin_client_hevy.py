"""Tests for the Hevy-integration extensions on GarminClient."""

import pytest

from activsync import garmin_client as gc_module
from activsync.garmin_client import (
    ActivityGone,
    GarminClient,
    GarminUploadRejected,
    SubcategoryRejected,
)


class PassThroughLimiter:
    def call(self, func, *args, **kwargs):
        return func(*args, **kwargs)


@pytest.fixture(autouse=True)
def no_rate_limit(monkeypatch):
    monkeypatch.setattr(gc_module, "_limiter", PassThroughLimiter())


class StubRaw:
    """Stand-in for garminconnect.Garmin, recording calls."""

    def __init__(self):
        self.calls = []
        self.upload_response = {}
        self.upload_exc = None
        self.activities_by_date = []
        self.user_profile = {}
        self.max_metrics = []
        self.heart_rates = {"heartRateValues": []}
        self.request_exc = None

        outer = self

        class _Inner:
            def request(self, method, subdomain, url, **kwargs):
                outer.calls.append(("request", method, url, kwargs))
                if outer.request_exc is not None:
                    raise outer.request_exc
                return None

        self.client = _Inner()

    def upload_activity(self, path):
        self.calls.append(("upload_activity", path))
        if self.upload_exc is not None:
            raise self.upload_exc
        return self.upload_response

    def get_activity_exercise_sets(self, activity_id):
        self.calls.append(("get_activity_exercise_sets", activity_id))
        return {"exerciseSets": []}

    def set_activity_name(self, activity_id, title):
        self.calls.append(("set_activity_name", activity_id, title))

    def set_activity_description(self, activity_id, description):
        self.calls.append(("set_activity_description", activity_id, description))

    def delete_activity(self, activity_id):
        self.calls.append(("delete_activity", activity_id))

    def get_heart_rates(self, date_str):
        self.calls.append(("get_heart_rates", date_str))
        return self.heart_rates

    def get_user_profile(self):
        return self.user_profile

    def get_max_metrics(self, date_str):
        return self.max_metrics

    def get_activities_by_date(self, date_from, date_to):
        self.calls.append(("get_activities_by_date", date_from, date_to))
        return self.activities_by_date


def make_fit(tmp_path):
    path = tmp_path / "up.fit"
    path.write_bytes(b"fake fit")
    return str(path)


# -- upload_fit -------------------------------------------------------------

def test_upload_happy_path(tmp_path):
    raw = StubRaw()
    raw.upload_response = {"detailedImportResult": {
        "uploadId": "u1", "successes": [{"internalId": 12345}], "failures": []}}
    client = GarminClient(raw)
    result = client.upload_fit(make_fit(tmp_path))
    assert result == {"upload_id": "u1", "activity_id": 12345}


def test_upload_quoted_internal_id(tmp_path):
    raw = StubRaw()
    raw.upload_response = {"detailedImportResult": {
        "uploadId": "u2", "successes": [{"internalId": "'23126363872'"}],
        "failures": []}}
    client = GarminClient(raw)
    assert client.upload_fit(make_fit(tmp_path))["activity_id"] == 23126363872


def test_upload_rejected_raises(tmp_path):
    raw = StubRaw()
    raw.upload_response = {"detailedImportResult": {
        "uploadId": "u3", "successes": [],
        "failures": [{"messages": [{"content": "Duplicate Activity"}]}]}}
    client = GarminClient(raw)
    with pytest.raises(GarminUploadRejected):
        client.upload_fit(make_fit(tmp_path))


def test_upload_http_409_duplicate_is_a_definite_rejection(tmp_path):
    raw = StubRaw()
    response = type("Response", (), {
        "status_code": 409,
        "text": '{"code":202,"message":"Duplicate Activity"}',
    })()
    error = RuntimeError("409 Client Error: Duplicate Activity")
    error.response = response
    raw.upload_exc = error

    with pytest.raises(GarminUploadRejected, match="Duplicate Activity"):
        GarminClient(raw).upload_fit(make_fit(tmp_path))


def test_upload_pending_processing_returns_none_id(tmp_path):
    raw = StubRaw()
    raw.upload_response = {"detailedImportResult": {
        "uploadId": "u4", "successes": [], "failures": []}}
    client = GarminClient(raw)
    result = client.upload_fit(make_fit(tmp_path))
    assert result == {"upload_id": "u4", "activity_id": None}


def test_upload_missing_file_raises(tmp_path):
    client = GarminClient(StubRaw())
    with pytest.raises(FileNotFoundError):
        client.upload_fit(str(tmp_path / "nope.fit"))


# -- exercise sets ----------------------------------------------------------

def test_put_exercise_sets_sends_payload():
    raw = StubRaw()
    client = GarminClient(raw)
    client.put_exercise_sets(99, {"exerciseSets": [1]})
    kind, method, url, kwargs = raw.calls[-1]
    assert (kind, method) == ("request", "PUT")
    assert url == "/activity-service/activity/99/exerciseSets"
    assert kwargs["json"] == {"exerciseSets": [1]}


def test_put_exercise_sets_subcategory_rejection():
    raw = StubRaw()
    raw.request_exc = RuntimeError('400 Bad Request: "Invalid Sub-Category provided"')
    client = GarminClient(raw)
    with pytest.raises(SubcategoryRejected):
        client.put_exercise_sets(99, {"exerciseSets": []})


def test_put_exercise_sets_404_raises_activity_gone():
    raw = StubRaw()
    raw.request_exc = RuntimeError("404 Not Found for url")
    client = GarminClient(raw)
    with pytest.raises(ActivityGone):
        client.put_exercise_sets(99, {"exerciseSets": []})


def test_put_exercise_sets_other_errors_pass_through():
    raw = StubRaw()
    raw.request_exc = RuntimeError("500 Internal Server Error")
    client = GarminClient(raw)
    with pytest.raises(RuntimeError):
        client.put_exercise_sets(99, {"exerciseSets": []})


# -- metadata wrappers ------------------------------------------------------

def test_set_title_and_description_and_delete():
    raw = StubRaw()
    client = GarminClient(raw)
    client.set_title(5, "Push Day")
    client.set_description(5, "desc")
    client.delete_activity(5)
    kinds = [c[0] for c in raw.calls]
    assert kinds == ["set_activity_name", "set_activity_description",
                     "delete_activity"]


@pytest.mark.parametrize("method,args", [
    ("get_exercise_sets", (5,)),
    ("set_title", (5, "Push Day")),
    ("set_description", (5, "desc")),
])
def test_activity_specific_wrappers_translate_404(method, args):
    raw = StubRaw()

    def gone(*_args, **_kwargs):
        raise RuntimeError("404 Not Found")

    raw.get_activity_exercise_sets = gone
    raw.set_activity_name = gone
    raw.set_activity_description = gone

    with pytest.raises(ActivityGone):
        getattr(GarminClient(raw), method)(*args)


# -- profile ----------------------------------------------------------------

def test_fetch_user_profile_maps_fields():
    raw = StubRaw()
    raw.user_profile = {"userData": {
        "weight": 81500.0, "birthDate": "1990-05-04", "gender": "MALE"}}
    raw.max_metrics = [{"generic": {"vo2MaxPreciseValue": 47.3}}]
    client = GarminClient(raw)
    profile = client.fetch_user_profile()
    assert profile == {"weight_kg": 81.5, "birth_year": 1990,
                       "sex": "male", "vo2max": 47.3}


def test_fetch_user_profile_missing_pieces_are_none():
    raw = StubRaw()
    raw.user_profile = {"userData": {}}
    raw.max_metrics = []
    client = GarminClient(raw)
    profile = client.fetch_user_profile()
    assert profile == {"weight_kg": None, "birth_year": None,
                       "sex": None, "vo2max": None}


# -- daily HR ---------------------------------------------------------------

def test_get_daily_heart_rates_wraps():
    raw = StubRaw()
    client = GarminClient(raw)
    assert client.get_daily_heart_rates("2026-07-17") == {"heartRateValues": []}
    assert raw.calls[-1] == ("get_heart_rates", "2026-07-17")


# -- find_activity_near -----------------------------------------------------

ACTIVITIES = [
    {"activityId": 1, "startTimeGMT": "2026-07-17 06:02:00",
     "activityType": {"typeKey": "running"}},
    {"activityId": 2, "startTimeGMT": "2026-07-17 06:03:00",
     "activityType": {"typeKey": "strength_training"}},
    {"activityId": 3, "startTimeGMT": "2026-07-17 09:00:00",
     "activityType": {"typeKey": "strength_training"}},
]


def test_find_activity_near_matches_strength_within_window():
    raw = StubRaw()
    raw.activities_by_date = ACTIVITIES
    client = GarminClient(raw)
    found = client.find_activity_near("2026-07-17 06:00:00", exclude_ids=set())
    assert found == 2  # running excluded by type, 09:00 out of window


def test_find_activity_near_respects_exclusions():
    raw = StubRaw()
    raw.activities_by_date = ACTIVITIES
    client = GarminClient(raw)
    found = client.find_activity_near("2026-07-17 06:00:00", exclude_ids={"2"})
    assert found is None


def test_find_activity_near_none_when_no_match():
    raw = StubRaw()
    raw.activities_by_date = []
    client = GarminClient(raw)
    assert client.find_activity_near("2026-07-17 06:00:00", exclude_ids=set()) is None


# -- list_activity_ids_near (journal snapshots) -----------------------------

def test_list_activity_ids_near_returns_all_ids_any_type():
    raw = StubRaw()
    raw.activities_by_date = ACTIVITIES
    client = GarminClient(raw)
    ids = client.list_activity_ids_near("2026-07-17 06:00:00")
    assert ids == [1, 2, 3]  # snapshot includes every type, no window filter


def test_list_activity_ids_near_propagates_outage():
    class ExplodingRaw(StubRaw):
        def get_activities_by_date(self, date_from, date_to):
            raise RuntimeError("garmin down")

    client = GarminClient(ExplodingRaw())
    # An outage must NOT read as "no activities" — an empty snapshot would
    # poison later submission_unknown diffs.
    with pytest.raises(RuntimeError):
        client.list_activity_ids_near("2026-07-17 06:00:00")


def test_put_exercise_sets_goes_through_limiter(monkeypatch):
    calls = []

    class RecordingLimiter:
        def call(self, func, *args, **kwargs):
            calls.append(func)
            return func(*args, **kwargs)

    monkeypatch.setattr(gc_module, "_limiter", RecordingLimiter())
    raw = StubRaw()
    client = GarminClient(raw)
    client.put_exercise_sets(99, {"exerciseSets": []})
    assert calls, "put_exercise_sets bypassed the shared limiter"
