"""Sync reconciliation: Garmin activities <-> local status <-> Strava publish."""

from __future__ import annotations

import hashlib
import json
import logging
import sqlite3
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone

from activsync import db
from activsync.garmin_client import ActivityRecord, GarminClient
from activsync.strava_client import StravaClient, match_closest_activity

logger = logging.getLogger("activsync")

# How far a reconnect catch-up is allowed to widen the fetch window, so a
# long-dormant instance doesn't hammer Garmin's API and the inline reconnect
# request stays bounded.
CATCH_UP_MAX_DAYS = 90

# Why a row is 'held'. The two are NOT interchangeable: a category hold is a
# standing rule about an activity type and is lifted the moment the user makes
# that type an autosync category; a backlog hold is a statement about ONE
# activity ("you were disconnected, this is weeks old, decide for yourself").
# Only the user may lift a backlog hold — never an automatic promotion path.
HOLD_CATEGORY = "category"
HOLD_BACKLOG = "backlog"

# Distinguishes "caller passed no override" from "caller passed None", which is
# itself meaningful (None = Garmin has never synced successfully).
LAST_SYNC_UNSET = "__unset__"

# Widens the Strava window fetch beyond the lookback cutoff so an activity whose
# Strava start_date sits fractionally outside it still turns up — otherwise a
# boundary activity would look deleted and get flagged missing.
_WINDOW_PADDING = timedelta(minutes=30)


def _parse_start_time(row: dict) -> datetime:
    return datetime.strptime(row["start_time"], "%Y-%m-%d %H:%M:%S").replace(tzinfo=timezone.utc)


def _rows_within(
    conn: sqlite3.Connection, statuses: tuple[str, ...], cutoff: datetime
) -> list[tuple[str, dict, datetime]]:
    """Rows in the given statuses that start at or after the cutoff, with their
    parsed start times."""
    return [
        (status, row, start_time)
        for status in statuses
        for row in db.list_activities(conn, status=status)
        if (start_time := _parse_start_time(row)) >= cutoff
    ]


@dataclass
class GarminSyncStats:
    new: int = 0
    updated: int = 0
    removed: int = 0
    held_backlog: int = 0


@dataclass
class PublishStats:
    published: int = 0
    failed: int = 0


@dataclass
class StatusCheckStats:
    flagged_missing: int = 0
    linked_existing: int = 0


@dataclass
class CatchUpStats:
    garmin: GarminSyncStats
    status: StatusCheckStats
    lookback_days: int


def compute_content_hash(title: str, description: str, activity_type: str, garmin_data: str = "{}") -> str:
    raw = f"{title}\x00{description}\x00{activity_type}\x00{garmin_data}".encode("utf-8")
    return hashlib.sha256(raw).hexdigest()


_GARMIN_DATA_FIELDS = [
    "distance", "duration", "moving_duration", "elapsed_duration",
    "elevation_gain", "elevation_loss", "calories",
    "avg_hr", "max_hr", "avg_power", "max_power", "norm_power",
    "aerobic_te", "anaerobic_te", "training_load",
    "avg_cadence", "max_cadence",
    "total_sets", "total_reps", "total_volume",
]


def _garmin_data_json(act: ActivityRecord) -> str:
    """Serialize the optional ActivityRecord fields into a compact JSON string."""
    data: dict = {}
    for key in _GARMIN_DATA_FIELDS:
        val = getattr(act, key, None)
        if val is not None:
            data[key] = val
    return json.dumps(data) if data else "{}"


def _publish_row(conn: sqlite3.Connection, garmin: GarminClient, strava: StravaClient,
                  garmin_activity_id: int, now: datetime) -> None:
    row = db.get_activity(conn, garmin_activity_id)
    start_time = datetime.strptime(row["start_time"], "%Y-%m-%d %H:%M:%S").replace(tzinfo=timezone.utc)

    existing_strava_id = strava.find_existing_activity(start_time)
    if existing_strava_id is not None:
        db.set_published(conn, garmin_activity_id, existing_strava_id, now)
        return

    fit_bytes = garmin.download_fit(garmin_activity_id)
    strava_activity_id = strava.publish(
        garmin_activity_id, fit_bytes,
        name=row["title"],
        description=row.get("description") or None,
    )
    db.set_published(conn, garmin_activity_id, strava_activity_id, now)


def sync_garmin(
    conn: sqlite3.Connection,
    garmin: GarminClient,
    cfg: dict,
    now: datetime,
    *,
    lookback_days: int | None = None,
    hold_before: datetime | None = None,
) -> GarminSyncStats:
    """Fetch recent Garmin activities, reconcile them into the local table,
    and record the outcome to config for the Settings page's connection status.

    lookback_days overrides cfg["lookback_days"] — used by the reconnect
    catch-up to widen the window across an outage.

    hold_before forces newly-inserted activities that started before it to be
    'held' regardless of their category, so an outage backlog is reviewed rather
    than blast-published. It affects the insert path only: an activity already in
    the table keeps whatever status the user left it in.
    """
    held_types = set(cfg["held_activity_types"])
    marker = cfg["hevy2garmin_marker"]
    marker_active = cfg["hevy2garmin_marker_enabled"] and bool(marker)
    lookback = cfg["lookback_days"] if lookback_days is None else lookback_days
    hold_cutoff = (
        None if hold_before is None
        else hold_before.astimezone(timezone.utc).strftime("%Y-%m-%d %H:%M:%S")
    )

    try:
        fetched = garmin.fetch_recent_activities(lookback)
    except Exception as e:
        db.set_config_value(conn, "garmin_last_sync_at", now.isoformat())
        db.set_config_value(conn, "garmin_last_sync_ok", False)
        db.set_config_value(conn, "garmin_last_sync_error", str(e))
        db.set_config_value(conn, "garmin_credentials_verified", False)
        logger.exception("garmin fetch failed")
        raise

    stats = GarminSyncStats()
    fetched_ids: set[int] = set()

    for act in fetched:
        fetched_ids.add(act.garmin_activity_id)
        existing = db.get_activity(conn, act.garmin_activity_id)
        garmin_data = _garmin_data_json(act)
        new_hash = compute_content_hash(act.title, act.description, act.activity_type, garmin_data)

        if existing is None:
            if hold_cutoff is not None and act.start_time < hold_cutoff:
                status, hold_reason = "held", HOLD_BACKLOG
                stats.held_backlog += 1
            elif act.activity_type in held_types:
                status, hold_reason = "held", HOLD_CATEGORY
            else:
                status, hold_reason = "pending", None
            db.insert_activity(
                conn, act.garmin_activity_id, act.activity_type, act.title,
                act.description, act.start_time, new_hash, status, now, garmin_data,
                hold_reason,
            )
            stats.new += 1
            continue

        if existing["content_hash"] == new_hash:
            continue

        status = existing["publish_status"]
        # The Hevy marker promotes a CATEGORY-held row to pending. A backlog-held
        # row is held because it predates the normal window, not because of its
        # category — an edit must not sneak the outage backlog past the hold.
        if (
            status == "held"
            and existing.get("hold_reason") != HOLD_BACKLOG
            and marker_active and marker in act.description
        ):
            status = "pending"
        db.update_activity_content(
            conn, act.garmin_activity_id, act.title, act.description,
            act.activity_type, new_hash, status, garmin_data,
        )
        stats.updated += 1

    # garmin_client.fetch_recent_activities computes its own cutoff from
    # datetime.now(timezone.utc) at the moment it actually calls the Garmin
    # API, which is always >= `now` above (clock drift between capturing
    # `now` and making the request). That makes the real fetch cutoff
    # slightly more recent than `now - lookback`, so the fetched span can be
    # narrower than this window. Shrink the removal window by a small buffer
    # so it never claims to cover a sliver that wasn't actually fetched —
    # otherwise an activity in that sliver would be marked removed merely
    # because it wasn't part of this particular fetch.
    window_start = (
        (now - timedelta(days=lookback) + timedelta(minutes=5))
        .astimezone(timezone.utc)
        .strftime("%Y-%m-%d %H:%M:%S")
    )
    for garmin_activity_id in db.list_active_ids_since(conn, window_start) - fetched_ids:
        db.mark_removed(conn, garmin_activity_id)
        stats.removed += 1

    db.set_config_value(conn, "garmin_last_sync_at", now.isoformat())
    db.set_config_value(conn, "garmin_last_sync_ok_at", now.isoformat())
    db.set_config_value(conn, "garmin_last_sync_ok", True)
    db.set_config_value(conn, "garmin_last_sync_error", None)
    db.set_config_value(conn, "garmin_credentials_verified", True)
    logger.info(
        "garmin sync: %d new, %d updated, %d removed", stats.new, stats.updated, stats.removed,
    )

    return stats


def publish_pending(
    conn: sqlite3.Connection,
    garmin: GarminClient,
    strava: StravaClient,
    now: datetime,
    garmin_activity_ids: set[int] | None = None,
) -> PublishStats:
    """Publish activities currently in 'pending' status.

    The poller (garmin_activity_ids is None) publishes ONLY 'pending' rows.
    'missing' rows were deleted on Strava and stay that way until the user
    republishes; 'held' rows — whether held by category or as an outage backlog
    — are the user's call and are never published automatically.

    When garmin_activity_ids is given (manual bulk-publish from the dashboard),
    the user has explicitly checked those rows, so every status the dashboard
    lets them check is publishable — that is how a backlog row gets published
    deliberately.
    """
    stats = PublishStats()
    manual = garmin_activity_ids is not None
    statuses = ("pending", "missing", "held") if manual else ("pending",)
    for status in statuses:
        for row in db.list_activities(conn, status=status):
            if manual and row["garmin_activity_id"] not in garmin_activity_ids:
                continue
            try:
                _publish_row(conn, garmin, strava, row["garmin_activity_id"], now)
                stats.published += 1
            except Exception:
                logger.exception("publish failed for activity %s", row["garmin_activity_id"])
                stats.failed += 1
    if stats.published or stats.failed:
        logger.info("strava publish: %d published, %d failed", stats.published, stats.failed)
    return stats


def check_strava_status(
    conn: sqlite3.Connection,
    strava: StravaClient,
    cfg: dict,
    now: datetime,
    *,
    lookback_days: int | None = None,
) -> StatusCheckStats:
    """Check Strava for pending activities that were already published outside
    this app (e.g. via Garmin's native Strava sync) and link them, plus flag
    any previously-published activities deleted on Strava's side as 'missing'.

    Activities held by category are also checked. Held means the app should
    not publish them automatically; it does not mean an activity cannot have
    reached Strava through Garmin's native sync or another upload path.

    lookback_days overrides cfg["lookback_days"] — used by the reconnect
    catch-up to widen the window across an outage."""
    stats = StatusCheckStats()
    lookback = cfg["lookback_days"] if lookback_days is None else lookback_days
    cutoff = now - timedelta(days=lookback)

    to_link = _rows_within(conn, ("pending", "held"), cutoff)
    to_verify = _rows_within(conn, ("published",), cutoff)
    if not to_link and not to_verify:
        return stats

    # One window fetch answers both questions below. Asking Strava per activity
    # instead — a search per pending row, an existence check per published row —
    # burns the short-term quota within the hour on any busy week, after which
    # every call 429s and no reconciliation happens at all.
    on_strava = strava.list_activities_between(
        cutoff - _WINDOW_PADDING, now + _WINDOW_PADDING
    )

    # Pending/held activities may already be on Strava via Garmin Connect's
    # native connection or another upload path.
    unclaimed = list(on_strava)
    for status, row, start_time in to_link:
        existing_id = match_closest_activity(unclaimed, start_time)
        if existing_id is None:
            continue
        # Each Strava entry can only be the twin of one Garmin activity;
        # without this, two activities minutes apart would both claim it and
        # the real second one would never get published.
        unclaimed = [a for a in unclaimed if a["id"] != existing_id]
        db.set_published(conn, row["garmin_activity_id"], existing_id, now)
        stats.linked_existing += 1
        logger.debug(
            "linked %s activity %s to existing Strava %s",
            status, row["garmin_activity_id"], existing_id,
        )

    # Published activities absent from the window were deleted on Strava's side.
    live_ids = {a["id"] for a in on_strava}
    for _status, row, _start_time in to_verify:
        if row["strava_activity_id"] not in live_ids:
            db.set_publish_status(conn, row["garmin_activity_id"], "missing")
            stats.flagged_missing += 1

    if stats.flagged_missing:
        logger.info("strava status check: %d activities flagged missing", stats.flagged_missing)
    if stats.linked_existing:
        logger.info("strava status check: %d activities linked to existing Strava entries", stats.linked_existing)

    return stats


def reconcile_held_activities(conn: sqlite3.Connection, held_activity_types: list[str]) -> int:
    """Un-hold activities whose type is no longer in held_activity_types (i.e.
    the category was just switched to autosync). Returns the number flipped.

    Backlog holds are left alone: making 'running' an autosync category says
    nothing about whether a three-week-old run from an outage should be
    published, and promoting it here would hand the whole backlog to the next
    poller tick — exactly what the hold exists to prevent. The user un-holds a
    backlog row deliberately, by publishing it.
    """
    held_types = set(held_activity_types)
    flipped = 0
    for row in db.list_activities(conn, status="held"):
        if row.get("hold_reason") == HOLD_BACKLOG:
            continue
        if row["activity_type"] not in held_types:
            db.set_publish_status(conn, row["garmin_activity_id"], "pending")
            flipped += 1
    return flipped


def publish_now(
    conn: sqlite3.Connection, garmin: GarminClient, strava: StravaClient,
    garmin_activity_id: int, now: datetime,
) -> None:
    _publish_row(conn, garmin, strava, garmin_activity_id, now)


def edit_activity_metadata(
    conn: sqlite3.Connection,
    garmin: GarminClient,
    strava: StravaClient,
    garmin_activity_id: int,
    title: str,
    description: str,
) -> None:
    title = title.strip()
    description = description.strip()
    if not title:
        raise ValueError("Activity title cannot be blank")

    row = db.get_activity(conn, garmin_activity_id)
    if row is None:
        raise ValueError(f"Activity {garmin_activity_id} was not found")

    garmin.update_activity_metadata(garmin_activity_id, title, description)
    if row["publish_status"] == "published" and row.get("strava_activity_id"):
        strava.update_activity_metadata(row["strava_activity_id"], title, description)

    content_hash = compute_content_hash(
        title,
        description,
        row["activity_type"],
        row.get("garmin_data") or "{}",
    )
    db.update_activity_metadata(conn, garmin_activity_id, title, description, content_hash)
    logger.debug("updated metadata for activity %s", garmin_activity_id)


def catch_up_lookback_days(
    conn: sqlite3.Connection,
    cfg: dict,
    now: datetime,
    *,
    last_sync_ok_at: str | None = LAST_SYNC_UNSET,
) -> int:
    """How far back a reconnect must look to cover the outage.

    Never narrower than the user's configured lookback — the cap only bounds
    how far the OUTAGE widens the window, it must never shrink an ordinary
    sync's span (that would leave hold_before predating the entire fetched
    span, so nothing is ever held).

    last_sync_ok_at overrides the stored garmin_last_sync_ok_at (see
    catch_up_sync); None means "never synced".
    """
    last_ok = (
        db.get_config_value(conn, "garmin_last_sync_ok_at")
        if last_sync_ok_at is LAST_SYNC_UNSET else last_sync_ok_at
    )
    if last_ok is None:
        return cfg["lookback_days"]          # never synced — nothing to catch up on
    outage_days = (now - datetime.fromisoformat(last_ok)).days + 1
    return max(cfg["lookback_days"], min(outage_days, CATCH_UP_MAX_DAYS))


def strava_status_lookback_days(conn: sqlite3.Connection, cfg: dict, now: datetime) -> int:
    """How far back a reconnect must check Strava publish status.

    Measured from the OLDEST activity still awaiting a decision, NOT from the
    Garmin outage: during a STRAVA-only outage Garmin sync keeps succeeding, so
    the Garmin-derived outage is ~0 days while pending rows quietly pile up for
    weeks. Those rows are older than the normal window, so a normal-window
    status check skips them — but publish_pending has no window at all and
    would upload every one of them, duplicating whatever Garmin's own native
    Strava sync already pushed. The status check has to cover the real backlog.
    """
    oldest = db.oldest_unpublished_start_time(conn)
    if oldest is None:
        return cfg["lookback_days"]
    oldest_dt = datetime.strptime(oldest, "%Y-%m-%d %H:%M:%S").replace(tzinfo=timezone.utc)
    backlog_days = (now - oldest_dt).days + 1
    return max(cfg["lookback_days"], min(backlog_days, CATCH_UP_MAX_DAYS))


def catch_up_sync(
    conn: sqlite3.Connection,
    garmin: GarminClient,
    strava: StravaClient,
    cfg: dict,
    now: datetime,
    *,
    last_sync_ok_at: str | None = LAST_SYNC_UNSET,
) -> CatchUpStats:
    """Bring the activity list back up to date after a reconnect.

    Widens the window to span the outage, then reconciles Strava publish status
    across the backlog — during an outage Garmin's own native Strava sync may
    already have pushed activities, and linking those is what stops us
    re-uploading duplicates.

    Activities visible only because the window was widened are inserted as
    'held': publishing something from three weeks ago should be deliberate.

    last_sync_ok_at overrides the stored garmin_last_sync_ok_at, so a caller can
    size the window from the value it observed AT RECONNECT TIME. The poller
    shares this sqlite connection and can otherwise land a normal-window sync
    between the reconnect and this call, overwriting the timestamp with `now` —
    the catch-up would then measure a 0-day outage, never widen, and silently
    fetch nothing of the backlog.
    """
    lookback = catch_up_lookback_days(conn, cfg, now, last_sync_ok_at=last_sync_ok_at)
    hold_before = now - timedelta(days=cfg["lookback_days"])

    # The Garmin half is skipped when Garmin is not connected: a user who
    # reconnects STRAVA first would otherwise have the Garmin fetch raise and
    # abort the whole catch-up, so the Strava reconciliation they just asked
    # for — the half that prevents duplicate uploads — would never run.
    garmin_stats = GarminSyncStats()
    if db.get_config_value(conn, "garmin_credentials_verified", default=False):
        garmin_stats = sync_garmin(
            conn, garmin, cfg, now, lookback_days=lookback, hold_before=hold_before,
        )
    else:
        logger.info("catch-up: garmin is not connected, skipping the garmin fetch")

    # Computed AFTER the Garmin fetch so the freshly-inserted backlog counts.
    status_lookback = strava_status_lookback_days(conn, cfg, now)
    status_stats = check_strava_status(conn, strava, cfg, now, lookback_days=status_lookback)
    logger.info(
        "catch-up sync over %d days (status %d days): %d new (%d held as backlog), %d linked",
        lookback, status_lookback, garmin_stats.new, garmin_stats.held_backlog,
        status_stats.linked_existing,
    )
    return CatchUpStats(garmin=garmin_stats, status=status_stats, lookback_days=lookback)


def exclude(conn: sqlite3.Connection, garmin_activity_id: int) -> None:
    db.set_publish_status(conn, garmin_activity_id, "excluded")


def unexclude(conn: sqlite3.Connection, garmin_activity_id: int, cfg: dict) -> None:
    row = db.get_activity(conn, garmin_activity_id)
    held_types = set(cfg["held_activity_types"])
    marker = cfg["hevy2garmin_marker"]
    marker_active = cfg["hevy2garmin_marker_enabled"] and bool(marker)

    if row["activity_type"] in held_types and not (marker_active and marker in row["description"]):
        db.set_publish_status(conn, garmin_activity_id, "held", HOLD_CATEGORY)
    else:
        db.set_publish_status(conn, garmin_activity_id, "pending")
