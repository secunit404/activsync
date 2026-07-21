"""Typed Hevy mapping and history tools for the React Settings page."""

from __future__ import annotations

import logging
import sqlite3
from collections.abc import Callable

from fastapi import APIRouter, HTTPException

from activsync import db, dev_mock, events, hevy_backfill, hevy_db, view
from activsync.api_routes import ApiModel
from activsync.hevy_client import HevyAuthError, HevyClient
from activsync.hevy_mapper import CATEGORY_NAMES, SUBCATEGORY_NAMES

logger = logging.getLogger("activsync.hevy_tools_api_routes")


class MappingSubcategory(ApiModel):
    value: int
    label: str


class MappingCategory(ApiModel):
    value: int
    label: str
    subcategories: list[MappingSubcategory]


class ExerciseMapping(ApiModel):
    template_id: str
    title: str
    is_custom: bool
    muscle_group: str
    mapped: bool
    unmapped: bool
    garmin_rejected: bool
    suggested: bool
    category: int | None
    subcategory: int | None
    category_name: str | None
    subcategory_name: str | None


class HevyToolsState(ApiModel):
    mappings: list[ExerciseMapping]
    categories: list[MappingCategory]


class MappingRequest(ApiModel):
    category: int
    subcategory: int


class ToolActionResult(ApiModel):
    message: str


class BackfillRequest(ApiModel):
    since: str


class BackfillItem(ApiModel):
    hevy_id: str
    title: str
    start_time: str
    action: str
    twin_activity_id: int | None
    missing_template_ids: list[str] = []


class BackfillResult(ApiModel):
    message: str
    since: str
    ran: bool
    linked: int
    items: list[BackfillItem]


def create_router(
    conn: sqlite3.Connection,
    *,
    mock_mode: Callable[[], bool],
) -> APIRouter:
    router = APIRouter(prefix="/api/v1/settings/hevy", tags=["frontend-hevy"])

    def client_for(api_key: str):
        if mock_mode():
            return dev_mock.MockHevyClient(conn)
        return HevyClient(api_key=api_key)

    def require_api_key() -> str:
        api_key = db.get_config_value(conn, "hevy_api_key")
        if not api_key:
            raise HTTPException(
                status_code=409, detail="Connect Hevy before using history tools."
            )
        return api_key

    def build_backfill(since: str) -> tuple[list[dict], list[BackfillItem]]:
        api_key = require_api_key()
        since_dt = hevy_backfill.parse_since(since)
        if since_dt is None:
            raise HTTPException(
                status_code=400, detail="Enter the backfill start date as YYYY-MM-DD."
            )
        try:
            raw_items = hevy_backfill.preview_items(
                conn, client_for(api_key), since_dt
            )
        except HevyAuthError as exc:
            raise HTTPException(
                status_code=401,
                detail="Hevy rejected the stored API key — reconnect Hevy.",
            ) from exc
        except Exception as exc:
            logger.warning("hevy backfill fetch failed: %s", exc)
            raise HTTPException(
                status_code=502,
                detail=f"Could not fetch workouts from Hevy: {exc}",
            ) from exc
        items = [
            BackfillItem(
                hevy_id=item["workout"].get("id", ""),
                title=(
                    item["workout"].get("title")
                    or item["workout"].get("id")
                    or "Untitled workout"
                ),
                start_time=item["workout"].get("start_time", ""),
                action=item["action"].replace(" ", "_"),
                twin_activity_id=(
                    item["twin"]["garmin_activity_id"]
                    if item["twin"] is not None
                    else None
                ),
                missing_template_ids=item.get("missing_template_ids", []),
            )
            for item in raw_items
        ]
        return raw_items, items

    @router.get("/tools", response_model=HevyToolsState)
    def tools_state() -> HevyToolsState:
        mappings = view.hevy_mappings_view(conn)
        categories = [
            MappingCategory(
                value=category,
                label=label,
                subcategories=[
                    MappingSubcategory(value=subcategory, label=subcategory_label)
                    for subcategory, subcategory_label in sorted(
                        SUBCATEGORY_NAMES.get(category, {}).items()
                    )
                ],
            )
            for category, label in sorted(CATEGORY_NAMES.items())
        ]
        return HevyToolsState(
            mappings=[
                ExerciseMapping.model_validate(mapping) for mapping in mappings
            ],
            categories=categories,
        )

    @router.put(
        "/mappings/{template_id}",
        response_model=ToolActionResult,
    )
    def save_mapping(
        template_id: str, payload: MappingRequest
    ) -> ToolActionResult:
        if (
            payload.category not in CATEGORY_NAMES
            or payload.subcategory
            not in SUBCATEGORY_NAMES.get(payload.category, {})
        ):
            raise HTTPException(
                status_code=400,
                detail="Unknown Garmin exercise category/subcategory pair.",
            )
        known_templates = {
            mapping["template_id"] for mapping in view.hevy_mappings_view(conn)
        }
        if template_id not in known_templates:
            raise HTTPException(status_code=404, detail="Exercise template not found.")
        hevy_db.save_mapping(
            conn, template_id, payload.category, payload.subcategory
        )
        woken = hevy_db.wake_needs_mapping(conn, template_id=template_id)
        events.bus.publish("refresh")
        suffix = f" {woken} waiting workout{'s' if woken != 1 else ''} resumed." if woken else ""
        return ToolActionResult(message=f"Exercise mapping saved.{suffix}")

    @router.delete(
        "/mappings/{template_id}",
        response_model=ToolActionResult,
    )
    def delete_mapping(template_id: str) -> ToolActionResult:
        hevy_db.delete_mapping(conn, template_id)
        events.bus.publish("refresh")
        return ToolActionResult(message="Exercise mapping removed.")

    @router.post("/backfill/preview", response_model=BackfillResult)
    def preview_backfill(payload: BackfillRequest) -> BackfillResult:
        _, items = build_backfill(payload.since)
        count = len(items)
        message = (
            f"{count} workout{'s' if count != 1 else ''} found. "
            "Nothing has been written yet."
            if count
            else f"No Hevy workouts found since {payload.since}."
        )
        return BackfillResult(
            message=message,
            since=payload.since,
            ran=False,
            linked=0,
            items=items,
        )

    @router.post("/backfill/run", response_model=BackfillResult)
    def run_backfill(payload: BackfillRequest) -> BackfillResult:
        raw_items, items = build_backfill(payload.since)
        linked = hevy_backfill.run_items(conn, raw_items)
        events.bus.publish("refresh")
        count = len(items)
        return BackfillResult(
            message=(
                f"Backfill complete: {count} workout"
                f"{'s' if count != 1 else ''} ingested, {linked} linked "
                "to existing Garmin activities."
            ),
            since=payload.since,
            ran=True,
            linked=linked,
            items=items,
        )

    return router
