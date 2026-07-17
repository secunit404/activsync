import re
from datetime import datetime, timezone
from unittest.mock import MagicMock

from fastapi.testclient import TestClient

from activsync import db
from activsync import server as server_module
from activsync.server import create_app
from activsync.strava_client import StravaAuthError, StravaUploadError


def _logged_in_client(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(conn, "garmin_credentials", {
        "email": "me@example.com", "password": "hunter2",
    })
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_credentials", {
        "client_id": "cid", "client_secret": "csecret",
    })
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "access", "refresh_token": "refresh", "expires_at": 4102444800,
    })
    db.set_config_value(conn, "initial_sync_done", True)
    client = TestClient(create_app(conn))
    return conn, client


def _setup_done(conn):
    """No-op given _logged_in_client already leaves setup fully done; kept for
    parity with test_settings_routes.py so a broken-connection scenario reads
    the same way in both files: connect fully, then break one side."""
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_credentials", {
        "client_id": "cid", "client_secret": "csecret",
    })
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "access", "refresh_token": "refresh", "expires_at": 4102444800,
    })
    db.set_config_value(conn, "initial_sync_done", True)


def test_dashboard_redirects_to_setup_before_setup(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))

    response = client.get("/", follow_redirects=False)
    assert response.status_code == 303
    assert response.headers["location"] == "/setup"

    setup = client.get("/setup")
    assert setup.status_code == 200
    assert "ActivSync" in setup.text
    assert 'class="brand-activ">Activ</span><span class="brand-sync">Sync</span>' in setup.text
    assert "garmin2strava" not in setup.text
    assert "Connect Garmin" in setup.text


def test_pages_link_stylesheets_that_are_actually_served(tmp_path):
    """The CSS lives in static/, so a page can look fine in tests while the
    real deploy ships no stylesheet at all: fetch each one, don't just link it."""
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))

    page = client.get("/setup")
    hrefs = re.findall(r'<link rel="stylesheet" href="([^"]+)"', page.text)
    assert hrefs, "no stylesheets linked"
    for href in hrefs:
        assert "?v=" in href, f"{href} is not cache-busted"
        served = client.get(href.split("?")[0])
        assert served.status_code == 200, f"{href} is linked but not served"

    # Order is load-bearing: tokens define the vars every later file consumes.
    assert hrefs[0].startswith("/static/css/tokens.css")

    layout = client.get("/static/css/layout.css")
    assert ".brand-activ { color: var(--garmin); }" in layout.text
    assert "garmin2strava" not in layout.text


def test_setup_strava_step_shows_the_callback_domain_to_register(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(conn, "garmin_credentials_verified", True)
    client = TestClient(create_app(conn), base_url="http://192.168.1.16:8381")

    step2 = client.get("/setup")

    assert "Authorization Callback Domain" in step2.text
    # Bare host: Strava's field takes a domain, not a URL — no scheme, no port.
    assert "<code>192.168.1.16</code>" in step2.text
    assert "192.168.1.16:8381" not in step2.text


def test_setup_callback_domain_follows_the_host_the_user_arrived_on(tmp_path):
    """Behind a reverse proxy the registered domain is the public one, so the
    hint has to reflect the request rather than a hardcoded address — it must
    match the redirect_uri /strava/connect builds from the same request."""
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(conn, "garmin_credentials_verified", True)
    app = create_app(conn)

    lan = TestClient(app, base_url="http://192.168.1.16:8381").get("/setup")
    proxied = TestClient(app, base_url="https://activsync.example.com").get("/setup")

    assert "<code>192.168.1.16</code>" in lan.text
    assert "<code>activsync.example.com</code>" in proxied.text


def test_setup_advances_through_states(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))

    # Garmin only -> step 2 (Strava)
    db.set_config_value(conn, "garmin_credentials_verified", True)
    step2 = client.get("/setup")
    assert "Connect Strava" in step2.text
    assert 'action="/setup/strava/connect"' in step2.text
    assert "Save credentials" not in step2.text

    # Both connected, not yet synced -> step 3 (syncing)
    db.set_config_value(conn, "strava_credentials", {"client_id": "cid", "client_secret": "csecret"})
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "access", "refresh_token": "refresh", "expires_at": 4102444800,
    })
    step3 = client.get("/setup")
    # Syncing screen: Garmin/Strava already read as done, activity sync in progress.
    assert "Syncing activities" in step3.text
    assert "Garmin connected" in step3.text
    assert "Strava connected" in step3.text

    # Fully done -> /setup redirects to /
    db.set_config_value(conn, "initial_sync_done", True)
    done = client.get("/setup", follow_redirects=False)
    assert done.status_code == 303
    assert done.headers["location"] == "/"


def test_dashboard_lists_activities_grouped_by_status(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 1, "strength_training", "Leg Day", "", "2026-07-09 09:00:00", "h1", "held", now)
    db.insert_activity(conn, 2, "running", "Morning Run", "", "2026-07-09 08:00:00", "h2", "published", now)

    response = client.get("/")

    assert response.status_code == 200
    assert "Leg Day" in response.text
    assert "Morning Run" in response.text


def test_dashboard_shows_date_header_and_garmin_link(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 7, "running", "Morning Run", "", "2026-07-09 09:00:00", "h7", "pending", now)

    response = client.get("/")

    assert "9 Jul" in response.text
    assert "11:00" in response.text
    assert 'class="activity-list-head"' not in response.text
    assert "https://connect.garmin.com/modern/activity/7" in response.text


def test_dashboard_groups_activities_by_month_and_keeps_year_out_of_rows(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 11, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 1, "running", "July Run", "", "2026-07-09 09:00:00", "h1", "pending", now)
    db.insert_activity(conn, 2, "running", "June Run", "", "2026-06-09 09:00:00", "h2", "pending", now)

    response = client.get("/")

    assert "July 2026" in response.text
    assert "June 2026" in response.text
    july_index = response.text.index("July 2026")
    june_index = response.text.index("June 2026")
    assert july_index < june_index
    assert 'class="activity-date-year"' not in response.text


def test_dashboard_can_sort_activities_oldest_first(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 11, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 1, "running", "Newer Run", "", "2026-07-09 09:00:00", "h1", "pending", now)
    db.insert_activity(conn, 2, "running", "Older Run", "", "2026-06-09 09:00:00", "h2", "pending", now)

    response = client.get("/?sort=oldest")

    assert response.status_code == 200
    assert response.text.index("Older Run") < response.text.index("Newer Run")
    assert 'option value="oldest" selected' in response.text


def test_dashboard_paginates_activities(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 11, 10, 0, tzinfo=timezone.utc)
    for activity_id in range(1, 23):
        db.insert_activity(
            conn,
            activity_id,
            "running",
            f"Activity {activity_id}",
            "",
            f"2026-07-09 09:{activity_id:02d}:00",
            f"h{activity_id}",
            "pending",
            now,
        )

    first_page = client.get("/")
    second_page = client.get("/?page=2")
    partial_page = client.get("/api/activities/table?page=2")

    assert first_page.status_code == 200
    assert "Showing 1–20 of 22" in first_page.text
    assert ">Activity 22</h2>" in first_page.text
    assert ">Activity 2</h2>" not in first_page.text
    assert "Page 1 of 2" in first_page.text
    assert "Next" in first_page.text

    assert second_page.status_code == 200
    assert "Showing 21–22 of 22" in second_page.text
    assert ">Activity 2</h2>" in second_page.text
    assert "Page 2 of 2" in second_page.text
    assert "Previous" in second_page.text

    assert partial_page.status_code == 200
    assert "Showing 21–22 of 22" in partial_page.text
    assert ">Activity 1</h2>" in partial_page.text

    ten_per_page = client.get("/?page_size=10")
    assert ten_per_page.status_code == 200
    assert "Showing 1–10 of 22" in ten_per_page.text
    assert "Page 1 of 3" in ten_per_page.text
    assert 'option value="10" selected' in ten_per_page.text
    assert "page_size=10" in ten_per_page.text


def test_dashboard_can_filter_activities_by_status(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 11, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 1, "running", "Pending Run", "", "2026-07-09 09:00:00", "h1", "pending", now)
    db.insert_activity(conn, 2, "running", "Published Run", "", "2026-07-08 09:00:00", "h2", "published", now)

    response = client.get("/?status_filter=pending")

    assert response.status_code == 200
    assert "Pending Run" in response.text
    assert "Published Run" not in response.text
    assert 'option value="pending" selected' in response.text


def test_dashboard_shows_republish_button_for_missing_activity(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 3, "running", "Deleted Run", "", "2026-07-09 09:00:00", "h3", "missing", now)

    response = client.get("/")

    assert response.status_code == 200
    assert "Republish" in response.text


def test_dashboard_shows_edit_form_for_activity_metadata(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 8, "running", "Morning Run", "Easy Z2", "2026-07-09 09:00:00", "h8", "pending", now)

    response = client.get("/")

    assert response.status_code == 200
    assert "/api/activities/8/edit" in response.text
    assert 'name="title"' in response.text
    assert 'name="description"' in response.text


def test_dashboard_detail_actions_are_ordered_edit_exclude_publish(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 9, "running", "Morning Run", "Easy Z2", "2026-07-09 09:00:00", "h9", "held", now)

    response = client.get("/")

    assert response.status_code == 200
    # Scope to the dialog footer: the activity row has its own Publish button
    # earlier in the page, so searching the whole body finds the wrong one.
    footer = response.text.split('<footer class="drawer-actions">')[1].split("</footer>")[0]
    assert footer.index(">Edit<") < footer.index(">Exclude<") < footer.index(">Publish<")


def test_unexcluding_lives_in_the_details_dialog_not_the_row(tmp_path):
    """Exclude is only offered in the dialog, so its inverse belongs there too
    rather than on the row."""
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 4, "walking", "Evening Walk", "", "2026-07-09 09:00:00", "h4", "excluded", now)

    response = client.get("/")

    assert response.status_code == 200
    row = response.text.split('<div class="activity-row-actions">')[1].split("</div>")[0]
    assert "Un-exclude" not in row
    assert ">Details<" in row

    footer = response.text.split('<footer class="drawer-actions">')[1].split("</footer>")[0]
    assert ">Un-exclude<" in footer
    assert "/api/activities/4/unexclude" in footer


def test_the_wordmark_links_to_the_activities_page(tmp_path):
    conn, client = _logged_in_client(tmp_path)

    for path in ("/", "/settings"):
        page = client.get(path)
        assert page.status_code == 200
        lockup = page.text.split('class="brand-heading"')[1].split("</h1>")[0]
        assert '<a class="brand-link" href="/"' in lockup, f"wordmark is not a link on {path}"


def test_nav_pages_state_the_page_name_once(tmp_path):
    """The nav already says which page you are on; a matching h1 was pure
    duplication and cost a whole row of vertical space."""
    conn, client = _logged_in_client(tmp_path)

    dashboard = client.get("/")
    assert "<h1>Activities</h1>" not in dashboard.text
    assert 'aria-current="page"' in dashboard.text

    settings = client.get("/settings")
    assert "<h1>Settings</h1>" not in settings.text
    assert 'aria-current="page"' in settings.text


def test_setup_keeps_its_own_page_title(tmp_path):
    """setup.html has no nav, so its h1 is not redundant and must survive."""
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))

    page = client.get("/setup")
    assert 'class="page-kicker brand-lockup"' in page.text
    assert "Connect Garmin" in page.text


def test_dashboard_banner_names_the_broken_connection(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    db.set_config_value(conn, "garmin_credentials_verified", False)

    response = client.get("/")

    assert response.status_code == 200
    assert "Garmin disconnected" in response.text


def test_sync_garmin_route_runs_garmin_sync_and_returns_partial(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    fake_garmin = MagicMock()
    fake_garmin.fetch_recent_activities.return_value = []
    fake_strava = MagicMock()
    status_check = MagicMock()
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)
    monkeypatch.setattr(server_module.sync, "check_strava_status", status_check)

    response = client.post("/api/sync/garmin")

    assert response.status_code == 200
    fake_garmin.fetch_recent_activities.assert_called_once()
    status_check.assert_called_once()


def test_publish_to_strava_route_publishes_only_selected_activities(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 1, "running", "Run A", "", "2026-07-09 09:00:00", "h1", "pending", now)
    db.insert_activity(conn, 2, "running", "Run B", "", "2026-07-09 09:30:00", "h2", "pending", now)

    fake_garmin = MagicMock()
    fake_garmin.download_fit.return_value = b"FIT"
    fake_strava = MagicMock()
    fake_strava.find_existing_activity.return_value = None
    fake_strava.publish.return_value = 9001
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.post("/api/sync/strava/publish", data={"activity_ids": ["1"]})

    assert response.status_code == 200
    assert db.get_activity(conn, 1)["publish_status"] == "published"
    assert db.get_activity(conn, 2)["publish_status"] == "pending"


def test_publish_to_strava_route_with_no_selection_publishes_nothing(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 3, "running", "Run C", "", "2026-07-09 09:00:00", "h3", "pending", now)
    fake_strava = MagicMock()
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: MagicMock())

    response = client.post("/api/sync/strava/publish")

    assert response.status_code == 200
    assert db.get_activity(conn, 3)["publish_status"] == "pending"
    fake_strava.publish.assert_not_called()


def test_publish_route_reports_failed_uploads(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 1, "running", "Run A", "", "2026-07-09 09:00:00", "h1", "pending", now)

    fake_garmin = MagicMock()
    fake_garmin.download_fit.return_value = b"FIT"
    fake_strava = MagicMock()
    fake_strava.find_existing_activity.return_value = None
    # Strava refuses the upload — publish_pending swallows this per row and
    # counts it, so the route is the only place that can tell the user.
    fake_strava.publish.side_effect = StravaUploadError("upload rejected")
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.post("/api/sync/strava/publish", data={"activity_ids": ["1"]})

    assert response.status_code == 200
    assert "1 activity could not be published" in response.text
    assert db.get_activity(conn, 1)["publish_status"] == "pending"


def test_publish_route_says_nothing_when_every_upload_succeeds(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 1, "running", "Run A", "", "2026-07-09 09:00:00", "h1", "pending", now)
    fake_garmin = MagicMock()
    fake_garmin.download_fit.return_value = b"FIT"
    fake_strava = MagicMock()
    fake_strava.find_existing_activity.return_value = None
    fake_strava.publish.return_value = 9001
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.post("/api/sync/strava/publish", data={"activity_ids": ["1"]})

    assert "could not be published" not in response.text


def test_status_route_reports_a_rejected_token_instead_of_a_500(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    fake_strava = MagicMock()
    # What a revoked token or a missing activity:read_all scope raises.
    fake_strava.find_existing_activity.side_effect = StravaAuthError("Strava access token rejected")
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)
    monkeypatch.setattr(
        server_module.sync, "check_strava_status",
        MagicMock(side_effect=StravaAuthError("Strava access token rejected")),
    )

    response = client.post("/api/sync/strava/status")

    # 409 matches the sibling "Strava is disconnected" error and is opted into
    # swapping by the htmx:beforeSwap handler in base.html, so the message
    # actually lands in the table instead of being discarded as an error.
    assert response.status_code == 409
    assert "Strava access token rejected" in response.text


def test_publish_route_reports_a_rejected_token_instead_of_a_500(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 1, "running", "Run A", "", "2026-07-09 09:00:00", "h1", "pending", now)
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: MagicMock())
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())
    monkeypatch.setattr(
        server_module.sync, "publish_pending",
        MagicMock(side_effect=StravaAuthError("Strava refresh token was revoked")),
    )

    response = client.post("/api/sync/strava/publish", data={"activity_ids": ["1"]})

    assert response.status_code == 409
    assert "Strava refresh token was revoked" in response.text


def test_sync_strava_status_route_flags_missing_activity(tmp_path, monkeypatch):
    conn, client = _logged_in_client(tmp_path)
    cfg = db.get_config_value(conn, "settings") or {}
    cfg["lookback_days"] = 30
    db.set_config_value(conn, "settings", cfg)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 4, "running", "Run D", "", "2026-07-09 09:00:00", "h4", "published", now)
    db.set_published(conn, 4, strava_activity_id=8001, now=now)
    fake_strava = MagicMock()
    # Strava's window comes back without 8001 in it — the activity was deleted
    # on Strava's side.
    fake_strava.list_activities_between.return_value = []
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: fake_strava)

    response = client.post("/api/sync/strava/status")

    assert response.status_code == 200
    assert db.get_activity(conn, 4)["publish_status"] == "missing"


def test_strava_publish_requires_setup(tmp_path):
    """Bulk publish stays on the activities page, so it still answers with the
    table. Its sibling /api/sync/strava/status moved to Settings and answers
    with the manual sync fragment instead — covered in test_settings_routes.py."""
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))

    response = client.post("/api/sync/strava/publish", follow_redirects=False)

    assert response.status_code == 409
    assert 'id="activity-table"' in response.text


def test_selection_uses_a_hidden_input_not_a_checkbox_column(tmp_path):
    """The row lights up instead of growing a checkbox column, but the input
    stays in the DOM: it carries the checked state for keyboard and screen
    reader users, and hx-include still selects on it, so publish is unchanged."""
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 5, "running", "Pending Run", "", "2026-07-09 09:00:00", "h5", "pending", now)

    page = client.get("/")

    assert 'class="activity-row-select"' not in page.text
    assert 'name="activity_ids" value="5"' in page.text
    assert "activsync-select-input" in page.text


def test_only_publishable_rows_are_marked_selectable(tmp_path):
    """Whether a row can be selected is fixed at render time, so it ships as a
    class; the CSS dims everything without it and ignores clicks on it."""
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 5, "running", "Pending Run", "", "2026-07-09 09:00:00", "h5", "pending", now)
    db.insert_activity(conn, 6, "running", "Published Run", "", "2026-07-09 08:00:00", "h6", "published", now)
    db.insert_activity(conn, 7, "running", "Excluded Run", "", "2026-07-09 07:00:00", "h7", "excluded", now)

    page = client.get("/")

    rows = re.findall(r'<article class="(activity-row[^"]*)" data-gid="(\d+)"', page.text)
    selectable = {gid: ("is-selectable" in cls) for cls, gid in rows}
    assert selectable == {"5": True, "6": False, "7": False}


def test_selection_tools_render_after_the_list_so_they_can_stick(tmp_path):
    """Sticky-bottom needs the bar to come after every row in document order;
    sitting above the list, the bar scrolled away exactly when a long list
    needed it. Anchor on the LAST row, not the list's opening tag — the bar
    used to live inside the list, which would satisfy a laxer check."""
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 5, "running", "Pending Run", "", "2026-07-09 09:00:00", "h5", "pending", now)
    db.insert_activity(conn, 6, "running", "Held Run", "", "2026-07-08 09:00:00", "h6", "held", now)

    page = client.get("/")

    # Anchor on the opening tag, not class="activity-row": selectable rows carry
    # a second class, so the bare attribute does not appear on every row.
    assert page.text.rindex('<article class="activity-row') < page.text.index('class="multi-select-tools"')


def test_selection_mode_has_one_exit_not_two(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 5, "running", "Pending Run", "", "2026-07-09 09:00:00", "h5", "pending", now)

    page = client.get("/")

    assert "cancel-selection-button" not in page.text
    assert page.text.count("select-multiple-button") == 1


def test_publish_selected_ships_disabled_so_it_cannot_post_an_empty_selection(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 5, "running", "Pending Run", "", "2026-07-09 09:00:00", "h5", "pending", now)

    page = client.get("/")

    tools = page.text.split('class="multi-select-tools"')[1].split("</div>")[0]
    assert "disabled" in tools


def test_selection_logic_is_not_inline_onclick(tmp_path):
    """The state logic lives in static/js/activities.js, not in ~600 characters
    of onclick duplicated across three elements."""
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 5, "running", "Pending Run", "", "2026-07-09 09:00:00", "h5", "pending", now)

    page = client.get("/")

    assert "classList.toggle('is-selecting')" not in page.text
    assert "querySelectorAll('.activsync-select')" not in page.text
    assert "/static/js/activities.js" in page.text


def test_pages_link_scripts_that_are_actually_served(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))

    page = client.get("/setup")
    srcs = re.findall(r'<script src="(/static/[^"]+)"', page.text)
    assert srcs, "no local scripts linked"
    for src in srcs:
        assert "?v=" in src, f"{src} is not cache-busted"
        served = client.get(src.split("?")[0])
        assert served.status_code == 200, f"{src} is linked but not served"


def test_dashboard_shows_checkbox_for_pending_held_and_missing(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 5, "running", "Pending Run", "", "2026-07-09 09:00:00", "h5", "pending", now)
    db.insert_activity(conn, 6, "running", "Published Run", "", "2026-07-09 08:00:00", "h6", "published", now)
    db.insert_activity(conn, 7, "running", "Missing Run", "", "2026-07-09 07:00:00", "h7", "missing", now)

    response = client.get("/")

    assert response.text.count('name="activity_ids" value="5"') == 1
    assert response.text.count('name="activity_ids" value="7"') == 1
    assert 'name="activity_ids" value="6"' not in response.text


def test_logs_page_is_not_available(tmp_path):
    _, client = _logged_in_client(tmp_path)

    response = client.get("/logs")

    assert response.status_code == 404


def test_initial_sync_runs_and_redirects(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(conn, "garmin_credentials", {"email": "me@example.com", "password": "hunter2"})
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_credentials", {"client_id": "cid", "client_secret": "csecret"})
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "access", "refresh_token": "refresh", "expires_at": 4102444800})
    db.set_config_value(conn, "garmin_activity_types", [{"type_key": "running", "label": "Running"}])
    client = TestClient(create_app(conn))

    fake_garmin = MagicMock(); fake_garmin.fetch_recent_activities.return_value = []
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())
    status_check = MagicMock()
    monkeypatch.setattr(server_module.sync, "check_strava_status", status_check)

    response = client.post("/setup/initial-sync")

    assert response.status_code == 200
    assert "HX-Redirect" not in response.headers
    assert "Go to ActivSync" in response.text
    assert db.get_config_value(conn, "initial_sync_done") is True
    fake_garmin.fetch_recent_activities.assert_called_once()
    status_check.assert_called_once()


def test_initial_sync_failure_keeps_flag_false(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(conn, "garmin_credentials", {"email": "me@example.com", "password": "hunter2"})
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_credentials", {"client_id": "cid", "client_secret": "csecret"})
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "access", "refresh_token": "refresh", "expires_at": 4102444800})
    db.set_config_value(conn, "garmin_activity_types", [{"type_key": "running", "label": "Running"}])
    client = TestClient(create_app(conn))

    boom = MagicMock(side_effect=RuntimeError("garmin down"))
    monkeypatch.setattr(server_module.sync, "sync_garmin", boom)
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: MagicMock())
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())

    response = client.post("/setup/initial-sync")

    assert response.status_code == 200
    assert "garmin down" in response.text
    assert "Retry" in response.text
    assert db.get_config_value(conn, "initial_sync_done") in (None, False)


def test_initial_sync_first_run_loads_and_holds_categories_when_prefetch_deferred(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(conn, "garmin_credentials", {"email": "me@example.com", "password": "hunter2"})
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_credentials", {"client_id": "cid", "client_secret": "csecret"})
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "access", "refresh_token": "refresh", "expires_at": 4102444800})
    # connect-time prefetch failed -> no cached categories yet
    client = TestClient(create_app(conn))

    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.return_value = [
        {"type_key": "running", "label": "Running"}, {"type_key": "cycling", "label": "Cycling"}]
    fake_garmin.fetch_recent_activities.return_value = []
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())
    monkeypatch.setattr(server_module.sync, "check_strava_status", MagicMock())

    response = client.post("/setup/initial-sync")

    assert response.status_code == 200
    assert "HX-Redirect" not in response.headers
    assert "Go to ActivSync" in response.text
    from activsync import config
    assert sorted(config.load_config(conn)["held_activity_types"]) == ["cycling", "running"]
    assert db.get_config_value(conn, "initial_sync_done") is True


def test_initial_sync_first_run_category_fetch_failure_surfaces_error(tmp_path, monkeypatch):
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(conn, "garmin_credentials", {"email": "me@example.com", "password": "hunter2"})
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_credentials", {"client_id": "cid", "client_secret": "csecret"})
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "access", "refresh_token": "refresh", "expires_at": 4102444800})
    client = TestClient(create_app(conn))

    fake_garmin = MagicMock()
    fake_garmin.fetch_activity_types.side_effect = RuntimeError("types unavailable")
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: fake_garmin)
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())

    response = client.post("/setup/initial-sync")

    assert response.status_code == 200
    assert "types unavailable" in response.text
    assert "Retry" in response.text
    assert db.get_config_value(conn, "initial_sync_done") in (None, False)


def test_initial_sync_is_refused_after_setup_is_done(tmp_path, monkeypatch):
    """The wizard is strictly one-time. A stale open tab must not be able to
    re-post the initial sync after setup and run an unwanted normal-window sync."""
    conn = db.connect(str(tmp_path / "test.db"))
    db.set_config_value(conn, "garmin_credentials", {"email": "me@example.com", "password": "hunter2"})
    db.set_config_value(conn, "garmin_credentials_verified", True)
    db.set_config_value(conn, "strava_credentials", {"client_id": "cid", "client_secret": "csecret"})
    db.set_config_value(conn, "strava_tokens", {
        "access_token": "access", "refresh_token": "refresh", "expires_at": 4102444800})
    db.set_config_value(conn, "initial_sync_done", True)
    client = TestClient(create_app(conn))

    sync_garmin = MagicMock()
    monkeypatch.setattr(server_module.sync, "sync_garmin", sync_garmin)
    monkeypatch.setattr(server_module, "_build_garmin_client", lambda c: MagicMock())
    monkeypatch.setattr(server_module, "_build_strava_client", lambda c: MagicMock())

    response = client.post("/setup/initial-sync")

    assert response.headers["HX-Redirect"] == "/"
    sync_garmin.assert_not_called()


def test_dashboard_reports_the_catch_up_result(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    db.set_config_value(conn, "catch_up_report", {
        "new": 18, "held": 6, "linked": 12, "days": 22,
    })

    response = client.get("/")

    assert "18 activities" in response.text
    assert "6 held for review" in response.text


def test_dismissing_the_catch_up_report_clears_it(tmp_path):
    conn, client = _logged_in_client(tmp_path)
    _setup_done(conn)
    db.set_config_value(conn, "catch_up_report", {"new": 1, "held": 0, "linked": 0, "days": 8})

    response = client.post("/api/catch-up-report/dismiss", follow_redirects=False)

    assert response.status_code == 303
    assert db.get_config_value(conn, "catch_up_report") is None


def test_favicon_ico_is_served_at_the_root(tmp_path):
    """Browsers probe /favicon.ico at the root regardless of what the page
    links, so without this every visit logs a 404."""
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))

    response = client.get("/favicon.ico")

    assert response.status_code == 200
    assert response.headers["content-type"] == "image/png"
    assert response.content[:8] == b"\x89PNG\r\n\x1a\n"


def test_favicon_ico_is_the_same_image_the_pages_link(tmp_path):
    conn = db.connect(str(tmp_path / "test.db"))
    client = TestClient(create_app(conn))

    root = client.get("/favicon.ico")
    linked = client.get("/static/favicon.png")

    assert root.content == linked.content


def test_activity_dialog_leads_with_the_time_not_the_activity_type(tmp_path):
    """The timestamp is fixed-width and the type is not, so the type has to come
    second: leading with it pushes the timestamp to a different column on every
    row, and a long type wraps the line rather than just extending it."""
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 11, "backcountry_skiing_snowboarding_ws", "Backcountry day",
                       "", "2026-07-09 09:00:00", "h11", "published", now)

    response = client.get("/")

    kicker = re.search(r'<p class="dialog-kicker"[^>]*>([^<]+)</p>', response.text)
    assert kicker, "activity dialog is missing its kicker"
    assert kicker.group(1).strip() == "2026-07-09 11:00 · backcountry_skiing_snowboarding_ws"


def test_activity_dialog_kicker_keeps_its_full_text_reachable(tmp_path):
    """The kicker is a single truncating line, so a long activity type gets cut
    on screen. The full string has to stay somewhere the reader can get at it."""
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 12, "backcountry_skiing_snowboarding_ws", "Backcountry day",
                       "", "2026-07-09 09:00:00", "h12", "published", now)

    response = client.get("/")

    assert ('<p class="dialog-kicker" '
            'title="2026-07-09 11:00 · backcountry_skiing_snowboarding_ws">') in response.text


def test_activity_dialog_puts_the_status_badge_after_the_links(tmp_path):
    """Desktop right-aligns this row, so the last item lands on the row's right
    edge. The badge goes there because it is what gets scanned for, and it holds
    that edge whether or not the Strava pill is present — the links vary, the
    anchor should not."""
    conn, client = _logged_in_client(tmp_path)
    now = datetime(2026, 7, 9, 10, 0, tzinfo=timezone.utc)
    db.insert_activity(conn, 13, "running", "Morning Run", "", "2026-07-09 09:00:00", "h13",
                       "published", now)

    response = client.get("/")

    actions = re.search(
        r'<div class="activity-dialog-meta-actions">(.*?)</div>\s*</div>',
        response.text, re.S,
    )
    assert actions, "activity dialog is missing its meta actions row"
    body = actions.group(1)
    assert body.index("activity-card-links") < body.index("badge-published"), (
        "the badge must come last so it lands on the right edge"
    )
