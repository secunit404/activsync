"""Typed JSON routes used by the React frontend."""

from __future__ import annotations

import sqlite3
from collections.abc import Callable
from datetime import datetime, timedelta, timezone
from math import ceil
from typing import Literal

from fastapi import APIRouter, HTTPException, Query
from pydantic import BaseModel, ConfigDict, Field

from activsync import (
    __version__,
    config,
    db,
    events,
    onboarding,
    sync,
    timeutil,
    update_check,
    view,
)
from activsync.garmin_client import GarminClient
from activsync.strava_client import StravaAuthError, StravaClient


def _to_camel(value: str) -> str:
    head, *tail = value.split("_")
    return head + "".join(part.capitalize() for part in tail)


class ApiModel(BaseModel):
    """Base response model with the frontend's camelCase wire format."""

    model_config = ConfigDict(alias_generator=_to_camel, populate_by_name=True)


class ServiceConnection(ApiModel):
    connected: bool
    status: str
    meta: str


class GarminConnection(ServiceConnection):
    email: str


class Connections(ApiModel):
    garmin: GarminConnection
    strava: ServiceConnection
    broken: list[Literal["garmin", "strava"]]


class SetupState(ApiModel):
    complete: bool
    step: Literal["garmin", "strava", "hevy", "syncing"] | None


class HevyState(ApiModel):
    enabled: bool
    connected: bool
    status: str


class CatchUpReport(ApiModel):
    new: int
    held: int
    linked: int
    days: int


class UpdateState(ApiModel):
    latest: str | None
    available: bool
    repo_url: str
    release_url: str


class AppState(ApiModel):
    name: Literal["ActivSync"] = "ActivSync"
    version: str
    development: bool
    setup: SetupState
    connections: Connections
    hevy: HevyState
    catch_up_report: CatchUpReport | None
    update: UpdateState


PublishStatus = Literal["pending", "held", "published", "missing", "excluded"]
SortOrder = Literal["newest", "oldest"]
PageSize = Literal[10, 20, 50, 100]
PUBLISH_STATUSES: tuple[PublishStatus, ...] = (
    "pending",
    "held",
    "published",
    "missing",
    "excluded",
)


class ActivityDetail(ApiModel):
    description: str | None
    distance: str
    duration: str
    moving_time: str
    elapsed_time: str
    pace: str
    speed: str
    elev_gain: str
    elev_loss: str
    calories: str
    avg_hr: str
    max_hr: str
    avg_power: str
    max_power: str
    norm_power: str
    aerobic_te: str
    anaerobic_te: str
    training_load: str
    avg_cadence: str
    max_cadence: str
    total_sets: str
    total_reps: str
    total_volume: str


class Activity(ApiModel):
    garmin_activity_id: int
    activity_type: str
    title: str
    description: str
    start_time: str
    publish_status: PublishStatus
    strava_activity_id: int | None
    hold_reason: str | None
    start_date_display: str
    start_month_year_display: str
    start_clock_display: str
    garmin_url: str
    strava_url: str | None
    hevy_badge: str | None
    detail: ActivityDetail


class WeekTotal(ApiModel):
    seconds: int
    display: str


class PaginationState(ApiModel):
    page: int
    page_size: PageSize
    page_count: int
    total_count: int
    first_item: int
    last_item: int


class ActivitiesPage(ApiModel):
    items: list[Activity]
    sort: SortOrder
    status: PublishStatus | None
    counts: dict[PublishStatus, int]
    week_total: WeekTotal
    pagination: PaginationState


class EditActivityRequest(ApiModel):
    title: str
    description: str = ""


class PublishActivitiesRequest(ApiModel):
    activity_ids: list[int] = Field(min_length=1)


class ActivityActionResult(ApiModel):
    message: str
    severity: Literal["success", "warning"] = "success"
    published_count: int = 0
    failed_count: int = 0
    blocked_count: int = 0


def create_router(
    conn: sqlite3.Connection,
    *,
    mock_mode: Callable[[], bool],
    build_garmin_client: Callable[[sqlite3.Connection], GarminClient],
    build_strava_client: Callable[[sqlite3.Connection], StravaClient],
) -> APIRouter:
    router = APIRouter(prefix="/api/v1", tags=["frontend"])

    @router.get("/app", response_model=AppState)
    def app_state() -> AppState:
        connections = view.connection_status(conn)
        step = onboarding.setup_step(conn, connections)
        hevy = view.hevy_settings_view(conn)
        cfg = config.load_config(conn)
        update = update_check.get_status()
        return AppState(
            version=__version__,
            development=mock_mode(),
            setup=SetupState(complete=step is None, step=step),
            connections=Connections.model_validate(connections),
            hevy=HevyState(
                enabled=bool(cfg.get("hevy_enabled")),
                connected=hevy["connected"],
                status=hevy["status"],
            ),
            catch_up_report=db.get_config_value(conn, "catch_up_report"),
            update=UpdateState(
                latest=update.latest,
                available=update.update_available,
                repo_url=update.repo_url,
                release_url=update.release_url,
            ),
        )

    @router.delete("/catch-up-report", status_code=204)
    def dismiss_catch_up_report() -> None:
        db.set_config_value(conn, "catch_up_report", None)
        events.bus.publish("refresh")

    @router.get("/activities", response_model=ActivitiesPage)
    def activities_page(
        sort: SortOrder = "newest",
        status: PublishStatus | None = None,
        page: int = Query(default=1, ge=1),
        page_size: int = Query(default=20, alias="pageSize"),
    ) -> ActivitiesPage:
        if page_size not in (10, 20, 50, 100):
            raise HTTPException(
                status_code=422,
                detail="pageSize must be one of 10, 20, 50, or 100",
            )
        all_activities = view.activities_view(conn, sort_order=sort)
        tz_name = config.load_config(conn)["display_timezone"]
        now_local = timeutil.to_local_now(tz_name)
        week_start = (now_local - timedelta(days=now_local.weekday())).replace(
            hour=0, minute=0, second=0, microsecond=0
        )
        counts: dict[PublishStatus, int] = {
            publish_status: 0 for publish_status in PUBLISH_STATUSES
        }
        week_seconds = 0
        for activity in all_activities:
            counts[activity["publish_status"]] += 1
            if activity["publish_status"] == "excluded":
                continue
            local_start = timeutil.to_local(activity["start_time"], tz_name)
            if local_start >= week_start:
                week_seconds += int(activity["duration_seconds"] or 0)

        filtered = (
            [activity for activity in all_activities if activity["publish_status"] == status]
            if status is not None
            else all_activities
        )
        total_count = len(filtered)
        page_count = max(1, ceil(total_count / page_size))
        current_page = min(page, page_count)
        first_index = (current_page - 1) * page_size
        page_items = filtered[first_index : first_index + page_size]

        activities = [
            Activity.model_validate(
                {
                    **activity,
                    "strava_url": (
                        f"https://www.strava.com/activities/{activity['strava_activity_id']}"
                        if activity["publish_status"] == "published"
                        and activity["strava_activity_id"] is not None
                        else None
                    ),
                }
            )
            for activity in page_items
        ]
        return ActivitiesPage(
            items=activities,
            sort=sort,
            status=status,
            counts=counts,
            week_total=WeekTotal(
                seconds=week_seconds,
                display=view._fmt_duration_coarse(week_seconds),
            ),
            pagination=PaginationState(
                page=current_page,
                page_size=page_size,
                page_count=page_count,
                total_count=total_count,
                first_item=first_index + 1 if total_count else 0,
                last_item=first_index + len(activities),
            ),
        )

    def require_activity(garmin_activity_id: int) -> dict:
        activity = db.get_activity(conn, garmin_activity_id)
        if activity is None:
            raise HTTPException(status_code=404, detail="Activity was not found")
        return activity

    def require_publish_connections() -> None:
        connections = view.connection_status(conn)
        if not connections["garmin"]["connected"] or not connections["strava"][
            "connected"
        ]:
            raise HTTPException(
                status_code=409,
                detail=(
                    "Both Garmin and Strava must be connected to publish. "
                    "Reconnect to continue."
                ),
            )

    @router.post(
        "/activities/{garmin_activity_id}/publish",
        response_model=ActivityActionResult,
    )
    def publish_activity(garmin_activity_id: int) -> ActivityActionResult:
        activity = require_activity(garmin_activity_id)
        if activity["publish_status"] not in ("held", "pending", "missing"):
            raise HTTPException(
                status_code=409,
                detail="Only pending, held, or missing activities can be published",
            )
        require_publish_connections()
        try:
            sync.publish_now(
                conn,
                build_garmin_client(conn),
                build_strava_client(conn),
                garmin_activity_id,
                datetime.now(timezone.utc),
            )
        except sync.PublishBlocked as exc:
            raise HTTPException(
                status_code=409,
                detail=f"Activity was blocked by Hevy sync: {exc.reason}.",
            ) from exc
        except StravaAuthError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
        except Exception as exc:
            raise HTTPException(
                status_code=502,
                detail=f"Could not publish activity: {exc}",
            ) from exc
        events.bus.publish("refresh")
        verb = "Republished" if activity["publish_status"] == "missing" else "Published"
        return ActivityActionResult(message=f"{verb} {activity['title']} to Strava")

    @router.post("/activities/publish", response_model=ActivityActionResult)
    def publish_activities(payload: PublishActivitiesRequest) -> ActivityActionResult:
        activity_ids = set(payload.activity_ids)
        activities = [require_activity(activity_id) for activity_id in activity_ids]
        invalid = [
            activity["title"]
            for activity in activities
            if activity["publish_status"] not in ("held", "pending", "missing")
        ]
        if invalid:
            raise HTTPException(
                status_code=409,
                detail="Some selected activities are no longer publishable. Refresh and try again.",
            )
        require_publish_connections()
        try:
            stats = sync.publish_pending(
                conn,
                build_garmin_client(conn),
                build_strava_client(conn),
                datetime.now(timezone.utc),
                garmin_activity_ids=activity_ids,
            )
        except StravaAuthError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
        events.bus.publish("refresh")

        issues = stats.failed + stats.blocked
        if issues:
            parts = [f"Published {stats.published}"]
            if stats.failed:
                parts.append(f"{stats.failed} failed and stayed ready to retry")
            if stats.blocked:
                parts.append(f"{stats.blocked} blocked by Hevy sync")
            return ActivityActionResult(
                message="; ".join(parts),
                severity="warning",
                published_count=stats.published,
                failed_count=stats.failed,
                blocked_count=stats.blocked,
            )
        noun = "activity" if stats.published == 1 else "activities"
        return ActivityActionResult(
            message=f"Published {stats.published} {noun} to Strava",
            published_count=stats.published,
        )

    @router.put(
        "/activities/{garmin_activity_id}", response_model=ActivityActionResult
    )
    def edit_activity(
        garmin_activity_id: int, payload: EditActivityRequest
    ) -> ActivityActionResult:
        activity = require_activity(garmin_activity_id)
        if not payload.title.strip():
            raise HTTPException(status_code=422, detail="Activity title cannot be blank")
        try:
            sync.edit_activity_metadata(
                conn,
                build_garmin_client(conn),
                build_strava_client(conn),
                garmin_activity_id,
                payload.title,
                payload.description,
            )
        except Exception as exc:
            raise HTTPException(
                status_code=502,
                detail=f"Could not save activity: {exc}",
            ) from exc
        events.bus.publish("refresh")
        return ActivityActionResult(message=f"Saved changes to {activity['title']}")

    @router.post(
        "/activities/{garmin_activity_id}/exclude",
        response_model=ActivityActionResult,
    )
    def exclude_activity(garmin_activity_id: int) -> ActivityActionResult:
        activity = require_activity(garmin_activity_id)
        if activity["publish_status"] not in ("held", "pending", "missing"):
            raise HTTPException(
                status_code=409,
                detail="Only pending, held, or missing activities can be excluded",
            )
        sync.exclude(conn, garmin_activity_id)
        events.bus.publish("refresh")
        return ActivityActionResult(message=f"Excluded {activity['title']}")

    @router.post(
        "/activities/{garmin_activity_id}/restore",
        response_model=ActivityActionResult,
    )
    def restore_activity(garmin_activity_id: int) -> ActivityActionResult:
        activity = require_activity(garmin_activity_id)
        if activity["publish_status"] != "excluded":
            raise HTTPException(status_code=409, detail="Activity is not excluded")
        sync.unexclude(conn, garmin_activity_id, config.load_config(conn))
        events.bus.publish("refresh")
        return ActivityActionResult(message=f"Restored {activity['title']}")

    return router
