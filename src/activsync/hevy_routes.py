"""Hevy settings routes: credentials, sync options, exercise mappings,
backfill. Registered from server.create_app — split out because server.py is
past the file-size cap, not because these routes behave differently."""

from __future__ import annotations

import logging
import sqlite3
from datetime import datetime, timezone

from fastapi import FastAPI, Form, Request
from fastapi.responses import (
    HTMLResponse,
    PlainTextResponse,
    RedirectResponse,
    Response,
)

from activsync import config, db, events, hevy_db, hevy_sync, view
from activsync.hevy_apply import _parse_ts
from activsync.hevy_client import HevyAuthError, HevyClient
from activsync.hevy_mapper import CATEGORY_NAMES, SUBCATEGORY_NAMES

logger = logging.getLogger("activsync.hevy_routes")

ALLOWED_STRATEGIES = ("replace", "merge", "describe")
GRACE_RANGE = (0, 1440)
INTERVAL_RANGE = (1, 120)


def _template_row(template: dict) -> dict:
    """Normalize a Hevy API exercise-template dict to the cache row shape."""
    return {
        "exercise_template_id": template.get("id"),
        "title": template.get("title", ""),
        "primary_muscle_group": template.get("primary_muscle_group"),
        "secondary_muscle_groups": template.get("secondary_muscle_groups", []),
        "equipment_category": (template.get("equipment_category")
                               or template.get("equipment")),
        "is_custom": template.get("is_custom", False),
    }


def _parse_since(raw: str) -> datetime | None:
    try:
        parsed = datetime.fromisoformat(raw.strip())
    except (ValueError, TypeError):
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def _workouts_since(client, since_dt: datetime) -> list[dict]:
    """Page through /workouts (newest first) down to the date boundary."""
    collected: list[dict] = []
    page = 1
    while True:
        data = client.get_workouts_page(page, page_size=10)
        workouts = data.get("workouts", []) or []
        older_seen = False
        for workout in workouts:
            start = _parse_ts(workout.get("start_time"))
            if start is None:
                continue
            if start < since_dt:
                older_seen = True
                continue
            collected.append(workout)
        page_count = data.get("page_count", page)
        if older_seen or page >= page_count or not workouts:
            break
        page += 1
    return collected


def _twin_activity(conn: sqlite3.Connection, workout: dict) -> dict | None:
    """An existing Garmin activity at exactly the workout's start time."""
    start = _parse_ts(workout.get("start_time"))
    if start is None:
        return None
    for activity in db.list_activities(conn):
        if activity["activity_type"] not in ("strength_training", "other"):
            continue
        if _parse_ts(activity["start_time"]) == start:
            return activity
    return None


def register(app: FastAPI, conn: sqlite3.Connection, templates, *,
             settings_context, saved, is_htmx, mock_mode) -> None:
    """Attach the Hevy routes. The keyword helpers are create_app's closures
    (shared context/response conventions), passed in rather than duplicated."""

    def _client_for(api_key: str):
        if mock_mode():
            from activsync import dev_mock
            return dev_mock.MockHevyClient(conn)
        return HevyClient(api_key=api_key)

    def _card_error(request: Request, message: str, status_code: int = 400):
        if is_htmx(request):
            return PlainTextResponse(message, status_code=status_code)
        return templates.TemplateResponse(
            request, "settings.html", settings_context(hevy_error=message),
            status_code=status_code,
        )

    def _mappings_response(request: Request):
        if is_htmx(request):
            return templates.TemplateResponse(
                request, "partials/hevy_mappings.html",
                {"mappings": view.hevy_mappings_view(conn),
                 "category_names": CATEGORY_NAMES,
                 "subcategory_names": SUBCATEGORY_NAMES},
            )
        return RedirectResponse("/settings#hevy-mappings", status_code=303)

    @app.post("/settings/hevy-credentials")
    def hevy_credentials_submit(request: Request, api_key: str = Form("")):
        api_key = api_key.strip()
        if not api_key:
            return _card_error(request, "Enter your Hevy API key.")
        client = _client_for(api_key)
        try:
            client.get_user_info()
        except HevyAuthError:
            return _card_error(
                request, "Hevy rejected that API key — check it and try again.")
        except Exception as exc:
            logger.warning("hevy key validation failed: %s", exc)
            return _card_error(
                request, f"Could not reach Hevy to validate the key: {exc}",
                status_code=502)

        db.set_config_value(conn, "hevy_api_key", api_key)
        db.set_config_value(conn, "hevy_auth_ok", True)
        # Connect-time forward-only cursor: history is backfill's job, so an
        # existing cursor (reconnect) must not move.
        if not db.get_config_value(conn, hevy_sync.CURSOR_KEY):
            db.set_config_value(conn, hevy_sync.CURSOR_KEY,
                                datetime.now(timezone.utc).isoformat())
        try:
            for template in client.iter_all_exercise_templates():
                row = _template_row(template)
                if row["exercise_template_id"]:
                    hevy_db.upsert_template(conn, row)
        except Exception as exc:
            # The key is proven valid; a template hiccup only delays the
            # mappings view (the gate re-fetches unknown templates on demand).
            logger.warning("hevy template prefetch failed: %s", exc)
        return saved(request, "hevy")

    @app.post("/settings/hevy")
    def hevy_settings_submit(
        request: Request,
        hevy_enabled: bool = Form(False),
        hevy_watch_strategy: str = Form(...),
        hevy_grace_minutes: int = Form(...),
        hevy_poll_interval_minutes: int = Form(...),
        identity_manufacturer: str = Form(""),
        identity_product: str = Form(""),
        identity_serial: str = Form(""),
        profile_weight_kg: str = Form(""),
        profile_birth_year: str = Form(""),
        profile_vo2max: str = Form(""),
        profile_sex: str = Form(""),
    ):
        if hevy_watch_strategy not in ALLOWED_STRATEGIES:
            return _card_error(
                request, f"Unknown watch strategy: {hevy_watch_strategy}")
        if not GRACE_RANGE[0] <= hevy_grace_minutes <= GRACE_RANGE[1]:
            return _card_error(
                request, "Grace period must be between 0 and 1440 minutes.")
        if not INTERVAL_RANGE[0] <= hevy_poll_interval_minutes <= INTERVAL_RANGE[1]:
            return _card_error(
                request, "Poll interval must be between 1 and 120 minutes.")

        identity_fields = [field.strip() for field in
                           (identity_manufacturer, identity_product, identity_serial)]
        provided = [field for field in identity_fields if field]
        identity = None
        if provided:
            if len(provided) != 3:
                return _card_error(
                    request,
                    "Device identity override needs all three numbers "
                    "(manufacturer, product, serial) — or none to auto-detect.")
            try:
                identity = {
                    "manufacturer": int(identity_fields[0]),
                    "product": int(identity_fields[1]),
                    "serial": int(identity_fields[2]),
                }
            except ValueError:
                return _card_error(
                    request, "Device identity values must be whole numbers.")

        override: dict = {}
        try:
            if profile_weight_kg.strip():
                override["weight_kg"] = float(profile_weight_kg)
            if profile_birth_year.strip():
                override["birth_year"] = int(profile_birth_year)
            if profile_vo2max.strip():
                override["vo2max"] = float(profile_vo2max)
        except ValueError:
            return _card_error(request, "Profile override values must be numbers.")
        if profile_sex.strip():
            if profile_sex not in ("male", "female"):
                return _card_error(request, "Profile sex must be male or female.")
            override["sex"] = profile_sex

        cfg = config.load_config(conn)
        cfg.update({
            "hevy_enabled": hevy_enabled,
            "hevy_watch_strategy": hevy_watch_strategy,
            "hevy_grace_minutes": hevy_grace_minutes,
            "hevy_poll_interval_minutes": hevy_poll_interval_minutes,
            "hevy_device_identity": identity,
            "profile_override": override,
        })
        if not override:
            cfg.pop("profile_override", None)
        config.save_config(conn, cfg)
        return saved(request, "hevy")

    @app.post("/settings/hevy/disconnect")
    def hevy_disconnect(request: Request):
        db.set_config_value(conn, "hevy_api_key", None)
        db.set_config_value(conn, "hevy_auth_ok", None)
        cfg = config.load_config(conn)
        cfg["hevy_enabled"] = False
        config.save_config(conn, cfg)
        return saved(request, "hevy")

    @app.get("/settings/hevy/mappings", response_class=HTMLResponse)
    def hevy_mappings_section(request: Request):
        return templates.TemplateResponse(
            request, "partials/hevy_mappings.html",
            {"mappings": view.hevy_mappings_view(conn),
             "category_names": CATEGORY_NAMES,
             "subcategory_names": SUBCATEGORY_NAMES},
        )

    @app.post("/settings/hevy/mappings/{template_id}")
    def hevy_mapping_save(request: Request, template_id: str,
                          category: int = Form(...), subcategory: int = Form(...)):
        if (category not in CATEGORY_NAMES
                or subcategory not in SUBCATEGORY_NAMES.get(category, {})):
            return PlainTextResponse(
                "Unknown Garmin exercise category/subcategory pair.",
                status_code=400)
        hevy_db.save_mapping(conn, template_id, category, subcategory)
        woken = hevy_db.wake_needs_mapping(conn, template_id=template_id)
        if woken:
            events.bus.publish("refresh")
        return _mappings_response(request)

    @app.post("/settings/hevy/mappings/{template_id}/delete")
    def hevy_mapping_delete(request: Request, template_id: str):
        hevy_db.delete_mapping(conn, template_id)
        return _mappings_response(request)

    def _backfill_items(client, conn, since_dt: datetime) -> list[dict]:
        cfg = config.load_config(conn)
        items: list[dict] = []
        for workout in _workouts_since(client, since_dt):
            twin = _twin_activity(conn, workout)
            if twin is not None:
                action = "linked_existing"
            else:
                match = hevy_sync.find_watch_match(conn, {
                    "hevy_id": f"__preview_{workout.get('id')}__",
                    "start_time": workout.get("start_time"),
                    "end_time": workout.get("end_time"),
                })
                if isinstance(match, int):
                    action = cfg["hevy_watch_strategy"]
                elif match in ("multiple", "claimed"):
                    action = "needs_review"
                else:
                    action = "passive"
            items.append({"workout": workout, "twin": twin, "action": action})
        return items

    def _backfill_context(request: Request, since: str):
        api_key = db.get_config_value(conn, "hevy_api_key")
        if not api_key:
            return None, _card_error(request, "Connect Hevy before backfilling.")
        since_dt = _parse_since(since)
        if since_dt is None:
            return None, _card_error(
                request, "Enter the backfill start date as YYYY-MM-DD.")
        client = _client_for(api_key)
        try:
            items = _backfill_items(client, conn, since_dt)
        except HevyAuthError:
            return None, _card_error(
                request, "Hevy rejected the stored API key — reconnect Hevy.")
        except Exception as exc:
            logger.warning("hevy backfill fetch failed: %s", exc)
            return None, _card_error(
                request, f"Could not fetch workouts from Hevy: {exc}",
                status_code=502)
        return {"items": items, "since": since}, None

    # -- dashboard workout actions -------------------------------------------

    _TERMINAL_STATUSES = ("merged", "described", "replaced", "uploaded_passive",
                          "linked_existing")
    _RETRYABLE_STATUSES = ("needs_mapping", "failed", "needs_review")

    def _with_lease(hevy_id: str, mutate):
        """Run `mutate(row)` under the per-workout execution lease so a user
        action can never interleave with the poller applying the same row."""
        row = hevy_db.get_workout(conn, hevy_id)
        if row is None:
            return PlainTextResponse("Unknown Hevy workout.", status_code=404)
        token = hevy_db.acquire_lease(conn, hevy_id,
                                      datetime.now(timezone.utc))
        if not token:
            return PlainTextResponse(
                "Workout is being processed right now — try again in a moment.",
                status_code=409)
        try:
            response = mutate(row)
        finally:
            hevy_db.release_lease(conn, hevy_id, token)
        if response.status_code < 400:
            events.bus.publish("refresh")
        return response

    @app.post("/api/hevy/{hevy_id}/retry")
    def hevy_retry(request: Request, hevy_id: str):
        def mutate(row):
            if row["status"] not in _RETRYABLE_STATUSES:
                return PlainTextResponse(
                    f"Nothing to retry — workout is {row['status']}.",
                    status_code=400)
            op = hevy_db.get_open_operation(conn, hevy_id)
            if op is not None:
                if op["phase"] == "needs_review":
                    # Resume where the operation actually stopped. A confirmed
                    # target resumes finalizing; an unresolved upload resumes
                    # its snapshot diff; only an op that never uploaded may
                    # start over — resuming one that DID upload at `preparing`
                    # would re-upload.
                    if op["target_activity_id"]:
                        resume = "finalizing"
                    elif op["upload_id"]:
                        resume = "submission_unknown"
                    else:
                        resume = "preparing"
                    hevy_db.update_operation(
                        conn, op["id"], phase=resume, attempt_count=0,
                        delete_attempt_count=0, last_error=None)
                hevy_db.set_workout_status(conn, hevy_id, "syncing", error=None)
            else:
                hevy_db.set_workout_status(conn, hevy_id, "waiting_watch",
                                           error=None)
            return Response(status_code=204)

        return _with_lease(hevy_id, mutate)

    @app.post("/api/hevy/{hevy_id}/skip")
    def hevy_skip(request: Request, hevy_id: str):
        def mutate(row):
            if row["status"] in _TERMINAL_STATUSES or row["status"] == "skipped":
                return PlainTextResponse(
                    f"Cannot skip a {row['status']} workout.", status_code=400)
            if hevy_db.get_open_operation(conn, hevy_id) is not None:
                # Skipping mid-operation would orphan the journal and leave
                # the publish interlock stuck on the referenced activities.
                return PlainTextResponse(
                    "An operation is in progress for this workout — resolve or "
                    "retry it instead of skipping.", status_code=409)
            hevy_db.set_workout_status(conn, hevy_id, "skipped")
            return Response(status_code=204)

        return _with_lease(hevy_id, mutate)

    @app.post("/api/hevy/{hevy_id}/unskip")
    def hevy_unskip(request: Request, hevy_id: str):
        def mutate(row):
            if row["status"] != "skipped":
                return PlainTextResponse(
                    f"Workout is {row['status']}, not skipped.", status_code=400)
            hevy_db.set_workout_status(conn, hevy_id, "waiting_watch")
            return Response(status_code=204)

        return _with_lease(hevy_id, mutate)

    @app.post("/api/hevy/{hevy_id}/resync-fresh")
    def hevy_resync_fresh(request: Request, hevy_id: str):
        def mutate(row):
            if hevy_db.get_open_operation(conn, hevy_id) is not None:
                return PlainTextResponse(
                    "An operation is in progress for this workout — resolve it "
                    "before re-syncing fresh.", status_code=409)
            hevy_db.reset_links(conn, hevy_id)
            hevy_db.set_workout_status(conn, hevy_id, "waiting_watch",
                                       error=None)
            return Response(status_code=204)

        return _with_lease(hevy_id, mutate)

    @app.post("/settings/hevy/backfill/preview", response_class=HTMLResponse)
    def hevy_backfill_preview(request: Request, since: str = Form("")):
        context, error = _backfill_context(request, since)
        if error is not None:
            return error
        return templates.TemplateResponse(
            request, "partials/hevy_backfill_preview.html",
            {**context, "ran": False},
        )

    @app.post("/settings/hevy/backfill/run", response_class=HTMLResponse)
    def hevy_backfill_run(request: Request, since: str = Form("")):
        context, error = _backfill_context(request, since)
        if error is not None:
            return error
        linked = 0
        for item in context["items"]:
            workout = item["workout"]
            hevy_id = workout.get("id")
            if not hevy_id:
                continue
            hevy_db.upsert_workout(
                conn, hevy_id, workout.get("title", ""),
                workout.get("start_time", ""), workout.get("end_time", ""),
                workout.get("updated_at", ""), workout,
            )
            if item["twin"] is None:
                continue  # the normal flow picks it up next tick
            try:
                hevy_db.link_target(
                    conn, hevy_id, item["twin"]["garmin_activity_id"],
                    applied_strategy="external", provenance="backfill",
                )
            except sqlite3.IntegrityError:
                # Another workout already owns that activity — leave this row
                # to the normal flow, which parks conflicts for review.
                logger.warning("backfill twin for %s already claimed", hevy_id)
                continue
            # Publish status of the activity row is deliberately untouched:
            # linking records provenance on the Hevy side only.
            hevy_db.set_workout_status(conn, hevy_id, "linked_existing")
            linked += 1
        events.bus.publish("refresh")
        return templates.TemplateResponse(
            request, "partials/hevy_backfill_preview.html",
            {**context, "ran": True, "linked": linked},
        )
