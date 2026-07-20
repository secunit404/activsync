"""Typed Hevy queue routes used by the React dashboard."""

from __future__ import annotations

import sqlite3
from collections.abc import Callable
from datetime import datetime, timezone

from fastapi import APIRouter, HTTPException

from activsync import events, hevy_db, view
from activsync.api_routes import ApiModel

TERMINAL_STATUSES = (
    "merged",
    "described",
    "replaced",
    "uploaded_passive",
    "linked_existing",
)
RETRYABLE_STATUSES = ("needs_mapping", "failed", "needs_review")


class HevyQueueItem(ApiModel):
    hevy_id: str
    title: str
    status: str
    error: str | None
    start_display: str
    needs_mapping: bool
    has_open_operation: bool
    resyncable: bool


class HevyQueueCounts(ApiModel):
    in_flight: int
    problems: int
    skipped: int


class HevyQueueState(ApiModel):
    enabled: bool
    in_flight: list[HevyQueueItem]
    problems: list[HevyQueueItem]
    skipped: list[HevyQueueItem]
    counts: HevyQueueCounts


class HevyQueueActionResult(ApiModel):
    message: str


def create_router(conn: sqlite3.Connection) -> APIRouter:
    router = APIRouter(prefix="/api/v1/hevy", tags=["frontend-hevy-queue"])

    @router.get("/queue", response_model=HevyQueueState)
    def queue_state() -> HevyQueueState:
        return HevyQueueState.model_validate(view.hevy_summary(conn))

    def with_lease(
        hevy_id: str,
        mutate: Callable[[dict], str],
    ) -> HevyQueueActionResult:
        row = hevy_db.get_workout(conn, hevy_id)
        if row is None:
            raise HTTPException(status_code=404, detail="Unknown Hevy workout.")
        token = hevy_db.acquire_lease(conn, hevy_id, datetime.now(timezone.utc))
        if not token:
            raise HTTPException(
                status_code=409,
                detail="Workout is being processed right now — try again in a moment.",
            )
        try:
            message = mutate(row)
        finally:
            hevy_db.release_lease(conn, hevy_id, token)
        events.bus.publish("refresh")
        return HevyQueueActionResult(message=message)

    @router.post("/{hevy_id}/retry", response_model=HevyQueueActionResult)
    def retry(hevy_id: str) -> HevyQueueActionResult:
        def mutate(row: dict) -> str:
            if row["status"] not in RETRYABLE_STATUSES:
                raise HTTPException(
                    status_code=409,
                    detail=f"Nothing to retry — workout is {row['status']}.",
                )
            operation = hevy_db.get_open_operation(conn, hevy_id)
            if operation is not None:
                if operation["phase"] == "needs_review":
                    if operation["target_activity_id"]:
                        resume = "finalizing"
                    elif operation["next_step"] == "resolve" or operation["upload_id"]:
                        resume = "submission_unknown"
                    else:
                        resume = "preparing"
                    hevy_db.update_operation(
                        conn,
                        operation["id"],
                        phase=resume,
                        attempt_count=0,
                        delete_attempt_count=0,
                        last_error=None,
                    )
                hevy_db.set_workout_status(conn, hevy_id, "syncing", error=None)
            else:
                hevy_db.set_workout_status(
                    conn,
                    hevy_id,
                    "waiting_watch",
                    error=None,
                )
            return f"Retry queued for {row['title'] or hevy_id}."

        return with_lease(hevy_id, mutate)

    @router.post("/{hevy_id}/skip", response_model=HevyQueueActionResult)
    def skip(hevy_id: str) -> HevyQueueActionResult:
        def mutate(row: dict) -> str:
            if row["status"] in TERMINAL_STATUSES or row["status"] == "skipped":
                raise HTTPException(
                    status_code=409,
                    detail=f"Cannot skip a {row['status']} workout.",
                )
            if hevy_db.get_open_operation(conn, hevy_id) is not None:
                raise HTTPException(
                    status_code=409,
                    detail=(
                        "An operation is in progress for this workout — resolve or "
                        "retry it instead of skipping."
                    ),
                )
            hevy_db.set_workout_status(conn, hevy_id, "skipped")
            return f"Skipped {row['title'] or hevy_id}."

        return with_lease(hevy_id, mutate)

    @router.post("/{hevy_id}/unskip", response_model=HevyQueueActionResult)
    def unskip(hevy_id: str) -> HevyQueueActionResult:
        def mutate(row: dict) -> str:
            if row["status"] != "skipped":
                raise HTTPException(
                    status_code=409,
                    detail=f"Workout is {row['status']}, not skipped.",
                )
            hevy_db.set_workout_status(conn, hevy_id, "waiting_watch")
            return f"Restored {row['title'] or hevy_id} to the sync queue."

        return with_lease(hevy_id, mutate)

    @router.post("/{hevy_id}/resync-fresh", response_model=HevyQueueActionResult)
    def resync_fresh(hevy_id: str) -> HevyQueueActionResult:
        def mutate(row: dict) -> str:
            if hevy_db.get_open_operation(conn, hevy_id) is not None:
                raise HTTPException(
                    status_code=409,
                    detail=(
                        "An operation is in progress for this workout — resolve it "
                        "before re-syncing fresh."
                    ),
                )
            if (
                row["status"] != "needs_review"
                or "deleted on Garmin" not in (row.get("error") or "")
            ):
                raise HTTPException(
                    status_code=409,
                    detail=(
                        "Re-sync as fresh is only available when the linked "
                        "activity was deleted on Garmin."
                    ),
                )
            hevy_db.reset_links(conn, hevy_id)
            hevy_db.set_workout_status(
                conn,
                hevy_id,
                "waiting_watch",
                error=None,
            )
            return f"Fresh sync queued for {row['title'] or hevy_id}."

        return with_lease(hevy_id, mutate)

    return router
