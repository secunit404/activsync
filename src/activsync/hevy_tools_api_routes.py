"""Typed Hevy mapping and history tools for the React Settings page."""

from __future__ import annotations

import logging
import sqlite3
from collections.abc import Callable

from fastapi import APIRouter, BackgroundTasks, HTTPException

from activsync import (
    db,
    dev_mock,
    events,
    fit_profile,
    hevy_backfill,
    hevy_db,
    view,
)
from activsync.api_routes import ApiModel
from activsync.fit_profile import CATEGORY_NAMES, SUBCATEGORY_NAMES
from activsync.hevy_client import HevyAuthError, HevyClient
from activsync.hevy_workout_detail import HevyWorkoutDetail, workout_detail

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
    suggested: bool
    # "user" | "automatic" | "" — see view.hevy_mappings_view.
    source: str
    has_standard_mapping: bool
    standard_category: int | None
    standard_subcategory: int | None
    category: int | None
    subcategory: int | None
    category_name: str | None
    subcategory_name: str | None


class HevyToolsState(ApiModel):
    mappings: list[ExerciseMapping]
    categories: list[MappingCategory]


class DeviceOption(ApiModel):
    value: int
    label: str


class DeviceOptions(ApiModel):
    manufacturers: list[DeviceOption]
    products: list[DeviceOption]


class MappingRequest(ApiModel):
    category: int
    subcategory: int


class ToolActionResult(ApiModel):
    message: str


class BackfillRequest(ApiModel):
    since: str
    # `None` (the default) means the field was omitted or sent as `null` —
    # preserves the original "import everything the since-window preview
    # surfaces" behavior for back-compatibility. A list, including `[]`,
    # means "import exactly these ids" — see `_select_backfill_items`.
    hevy_ids: list[str] | None = None


class BackfillItem(ApiModel):
    hevy_id: str
    title: str
    start_time: str
    action: str
    twin_activity_id: int | None
    garmin_url: str | None
    strava_activity_id: int | None
    strava_url: str | None
    missing_template_ids: list[str] = []
    workout: HevyWorkoutDetail


class BackfillResult(ApiModel):
    message: str
    since: str
    ran: bool
    linked: int
    items: list[BackfillItem]


def _select_backfill_items(
    raw_items: list[dict], hevy_ids: list[str] | None
) -> list[dict]:
    """Narrow a preview's raw items down to what `/backfill/run` should
    actually ingest.

    `hevy_ids is None` means the field was omitted (or sent `null`) on the
    wire — return every item unfiltered, so a client that only ever sends
    `since` keeps its original "import everything the preview surfaced"
    behavior. Once a list is present, even an empty one, it is treated as an
    explicit selection: `[]` deliberately imports nothing rather than
    silently falling back to "import everything" — the backfill screen can
    reach that state (every previewed row locked, none selected), and the
    two readings mean opposite things.

    Two guardrails apply to every explicit selection:
    - Ids that don't match any item in `raw_items` (a stale client, a typo,
      a workout that fell out of the since-window between preview and run)
      are dropped rather than causing an error or a wider import — the
      selection is a ceiling on what may be imported, never a trigger for
      importing something else instead.
    - Items whose preview action is `needs_mapping` are dropped even if
      explicitly named. The server is the actual enforcement point — a client
      could send one anyway.
    """
    if hevy_ids is None:
        return raw_items
    requested = set(hevy_ids)
    return [
        item
        for item in raw_items
        if item["workout"].get("id") in requested
        and item["action"] != "needs_mapping"
    ]


def _humanise_enum_name(name: str) -> str:
    """`FENIX3` -> `Fenix3`, `FR945_LTE` -> `Fr945 Lte`.

    Purely cosmetic: the integer value is what gets stored in
    `hevy_device_identity`, so changing a label is never a data change."""
    return name.replace("_", " ").title()


def _enum_options(names: dict[int, str]) -> list[DeviceOption]:
    """Value/label pairs from a FIT profile enum, sorted by label."""
    return sorted(
        (
            DeviceOption(value=value, label=_humanise_enum_name(name))
            for value, name in names.items()
        ),
        key=lambda option: option.label,
    )


def create_router(
    conn: sqlite3.Connection,
    *,
    mock_mode: Callable[[], bool],
    process_hevy_workout: Callable[[str], str] | None = None,
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
        description_template = db.get_config_value(conn, "hevy_description_template")
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
                garmin_url=(
                    view.GARMIN_ACTIVITY_URL.format(
                        item["twin"]["garmin_activity_id"]
                    )
                    if item["twin"] is not None
                    else None
                ),
                strava_activity_id=(
                    item["twin"]["strava_activity_id"]
                    if item["twin"] is not None
                    and item["twin"].get("publish_status") == "published"
                    else None
                ),
                strava_url=(
                    view.STRAVA_ACTIVITY_URL.format(
                        item["twin"]["strava_activity_id"]
                    )
                    if item["twin"] is not None
                    and item["twin"].get("publish_status") == "published"
                    and item["twin"].get("strava_activity_id") is not None
                    else None
                ),
                missing_template_ids=item.get("missing_template_ids", []),
                workout=HevyWorkoutDetail.model_validate(
                    workout_detail(
                        item["workout"],
                        hevy_id=item["workout"].get("id", ""),
                        title=item["workout"].get("title", ""),
                        start_time=item["workout"].get("start_time", ""),
                        end_time=item["workout"].get("end_time", ""),
                        description_template=description_template,
                    )
                ),
            )
            for item in raw_items
        ]
        return raw_items, items

    @router.get("/device-options", response_model=DeviceOptions)
    def device_options() -> DeviceOptions:
        """Manufacturer and Garmin product pickers for the device-identity
        override in Settings.

        Read-only reference data straight out of the FIT profile — no DB
        access and no Hevy API key needed, so it stays available before Hevy
        is connected (the identity fields render in Settings regardless).

        Manufacturers are deliberately narrowed to Garmin. ActivSync writes
        Garmin FIT files and uploads them to Garmin Connect; `fit_builder`'s
        only real identity is GENERIC_GARMIN_IDENTITY, and the one other
        constant there (DEVELOPMENT_IDENTITY, manufacturer 255) is documented
        as reference/tests only. Offering the profile's remaining 242
        manufacturers would let a user pick one that cannot work."""
        garmin = fit_profile.GARMIN_MANUFACTURER
        return DeviceOptions(
            manufacturers=_enum_options(
                {garmin: fit_profile.MANUFACTURER_NAMES[garmin]}
            ),
            products=_enum_options(fit_profile.GARMIN_PRODUCT_NAMES),
        )

    @router.get("/tools", response_model=HevyToolsState)
    def tools_state() -> HevyToolsState:
        mappings = view.hevy_mappings_view(conn)
        categories = [
            MappingCategory(
                value=category,
                label=view.garmin_exercise_label(label),
                subcategories=[
                    MappingSubcategory(
                        value=subcategory,
                        label=view.garmin_exercise_label(subcategory_label),
                    )
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
        mapping = next(
            (
                row
                for row in view.hevy_mappings_view(conn)
                if row["template_id"] == template_id
            ),
            None,
        )
        if mapping is None:
            raise HTTPException(status_code=404, detail="Exercise template not found.")
        hevy_db.delete_mapping(conn, template_id)
        restored_standard = bool(mapping["has_standard_mapping"])
        woken = (
            hevy_db.wake_needs_mapping(conn, template_id=template_id)
            if restored_standard
            else 0
        )
        events.bus.publish("refresh")
        message = (
            "Standard mapping restored."
            if restored_standard
            else "Exercise mapping removed."
        )
        if woken:
            message += f" {woken} waiting workout{'s' if woken != 1 else ''} resumed."
        return ToolActionResult(message=message)

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
    def run_backfill(
        payload: BackfillRequest, background_tasks: BackgroundTasks
    ) -> BackfillResult:
        raw_items, items = build_backfill(payload.since)
        selected_items = _select_backfill_items(raw_items, payload.hevy_ids)
        matched = hevy_backfill.run_items(conn, selected_items)
        if process_hevy_workout is not None:
            for item in selected_items:
                if (
                    item["twin"] is not None
                    and item["action"] in ("merge", "replace", "describe")
                ):
                    background_tasks.add_task(
                        process_hevy_workout, item["workout"]["id"]
                    )
        events.bus.publish("refresh")
        # Reflects what was actually handed to `run_items`, not the full
        # preview length — once the import is selective those two can
        # differ, and the confirmation must not overstate the work done.
        count = len(selected_items)
        return BackfillResult(
            message=(
                f"Backfill complete: {count} workout"
                f"{'s' if count != 1 else ''} ingested, {matched} matched "
                "with existing Garmin activities."
            ),
            since=payload.since,
            ran=True,
            linked=matched,
            items=items,
        )

    return router
