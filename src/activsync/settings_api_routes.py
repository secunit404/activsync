"""Typed setup and settings routes used by the React frontend."""

from __future__ import annotations

import logging
import sqlite3
from collections.abc import Callable
from datetime import datetime, timezone
from typing import Literal

from fastapi import APIRouter, HTTPException
from pydantic import Field

from activsync import (
    __version__,
    config,
    db,
    dev_mock,
    events,
    hevy_db,
    hevy_sync,
    logging_setup,
    onboarding,
    sync,
    timeutil,
    update_check,
    view,
)
from activsync.api_routes import ApiModel, Connections, UpdateState
from activsync.garmin_client import GarminClient, MfaRequired
from activsync.hevy_client import HevyAuthError, HevyClient
from activsync.strava_client import StravaAuthError, StravaClient

logger = logging.getLogger("activsync.settings_api_routes")

SetupStep = onboarding.SetupStep
HevyStrategy = Literal["replace", "merge", "describe"]


class SetupProgress(ApiModel):
    complete: bool
    step: SetupStep | None
    mfa_required: bool


class CredentialState(ApiModel):
    garmin_email: str
    garmin_password_saved: bool
    strava_client_id: str
    strava_client_secret_saved: bool


class PreferenceState(ApiModel):
    display_timezone: str
    garmin_poll_interval_minutes: int
    strava_poll_interval_minutes: int
    lookback_days: int
    hevy2garmin_marker: str
    hevy2garmin_marker_enabled: bool


class ActivityTypeState(ApiModel):
    type_key: str
    label: str
    autosync: bool


class DeviceIdentity(ApiModel):
    manufacturer: int | None = None
    product: int | None = None
    serial: int | None = None


class ProfileOverride(ApiModel):
    weight_kg: float | None = None
    birth_year: int | None = None
    vo2max: float | None = None
    sex: Literal["male", "female"] | None = None


class HevySettingsState(ApiModel):
    connected: bool
    status: str
    api_key_saved: bool
    enabled: bool
    watch_strategy: HevyStrategy
    grace_minutes: int
    poll_interval_minutes: int
    identity: DeviceIdentity
    identity_display: str
    profile_override: ProfileOverride


class SettingsState(ApiModel):
    version: str
    update: UpdateState
    development: bool
    setup: SetupProgress
    connections: Connections
    credentials: CredentialState
    preferences: PreferenceState
    timezones: list[str]
    activity_types: list[ActivityTypeState]
    hevy: HevySettingsState


class ActionResult(ApiModel):
    message: str
    setup_step: SetupStep | None = None
    mfa_required: bool = False


class GarminSetupRequest(ApiModel):
    email: str
    password: str = ""
    lookback_days: int = Field(default=7, ge=1)
    detected_timezone: str = ""


class GarminReconnectRequest(ApiModel):
    email: str
    password: str = ""


class MfaRequest(ApiModel):
    code: str


class StravaCredentialsRequest(ApiModel):
    client_id: str
    client_secret: str = ""


class PreferencesRequest(ApiModel):
    display_timezone: str
    garmin_poll_interval_minutes: int = Field(ge=1)
    strava_poll_interval_minutes: int = Field(ge=1)
    lookback_days: int = Field(ge=1)
    hevy2garmin_marker: str
    hevy2garmin_marker_enabled: bool


class ActivityTypesRequest(ApiModel):
    autosync_types: list[str]


class HevyCredentialsRequest(ApiModel):
    api_key: str


class HevySettingsRequest(ApiModel):
    enabled: bool
    watch_strategy: HevyStrategy
    grace_minutes: int = Field(ge=0, le=1440)
    poll_interval_minutes: int = Field(ge=1, le=120)
    identity: DeviceIdentity = Field(default_factory=DeviceIdentity)
    profile_override: ProfileOverride = Field(default_factory=ProfileOverride)


def create_router(
    conn: sqlite3.Connection,
    *,
    mock_mode: Callable[[], bool],
    build_garmin_client: Callable[[sqlite3.Connection], GarminClient],
    build_strava_client: Callable[[sqlite3.Connection], StravaClient],
    begin_garmin_login: Callable[[str, str], object],
    complete_garmin_login: Callable[[object, str], object],
    pending_garmin_mfa: dict,
) -> APIRouter:
    router = APIRouter(prefix="/api/v1", tags=["frontend-settings"])

    def fetch_and_store_categories(*, hold_all: bool) -> None:
        onboarding.store_garmin_categories(
            conn,
            build_garmin_client(conn),
            hold_all=hold_all,
        )

    def finalize_garmin_connect() -> None:
        db.set_config_value(conn, "garmin_credentials_verified", True)
        db.set_config_value(conn, "garmin_last_sync_error", None)
        first_run = not db.get_config_value(conn, "initial_sync_done", default=False)
        try:
            fetch_and_store_categories(hold_all=first_run)
        except Exception:
            logger.exception("Garmin category prefetch after connect failed")

    def save_and_verify_garmin(email: str, password: str) -> Literal["ok", "mfa"]:
        pending_garmin_mfa.pop("auth", None)
        pending_garmin_mfa.pop("credentials", None)
        try:
            begin_garmin_login(email, password)
        except MfaRequired as exc:
            pending_garmin_mfa["auth"] = exc.pending_auth
            pending_garmin_mfa["credentials"] = {
                "email": email,
                "password": password,
            }
            return "mfa"
        except Exception as exc:
            raise HTTPException(
                status_code=502,
                detail=f"Could not connect to Garmin: {exc}",
            ) from exc
        db.set_config_value(
            conn, "garmin_credentials", {"email": email, "password": password}
        )
        finalize_garmin_connect()
        return "ok"

    def hevy_client(api_key: str):
        if mock_mode():
            return dev_mock.MockHevyClient(conn)
        return HevyClient(api_key=api_key)

    def connect_hevy(api_key: str, *, in_setup: bool) -> None:
        api_key = api_key.strip()
        if not api_key:
            raise HTTPException(status_code=400, detail="Enter your Hevy API key.")
        client = hevy_client(api_key)
        try:
            client.get_user_info()
        except HevyAuthError as exc:
            raise HTTPException(
                status_code=401,
                detail="Hevy rejected that API key — check it and try again.",
            ) from exc
        except Exception as exc:
            raise HTTPException(
                status_code=502,
                detail=f"Could not reach Hevy to validate the key: {exc}",
            ) from exc
        db.set_config_value(conn, "hevy_api_key", api_key)
        db.set_config_value(conn, "hevy_auth_ok", True)
        if not db.get_config_value(conn, hevy_sync.CURSOR_KEY):
            db.set_config_value(
                conn, hevy_sync.CURSOR_KEY, datetime.now(timezone.utc).isoformat()
            )
        try:
            for template in client.iter_all_exercise_templates():
                template_id = template.get("id")
                if template_id:
                    hevy_db.upsert_template(
                        conn,
                        {
                            "exercise_template_id": template_id,
                            "title": template.get("title", ""),
                            "primary_muscle_group": template.get("primary_muscle_group"),
                            "secondary_muscle_groups": template.get(
                                "secondary_muscle_groups", []
                            ),
                            "equipment_category": template.get("equipment_category")
                            or template.get("equipment"),
                            "is_custom": template.get("is_custom", False),
                        },
                    )
        except Exception:
            logger.exception("Hevy template prefetch after connect failed")
        if in_setup:
            db.set_config_value(conn, "setup_hevy_done", True)
            cfg = config.load_config(conn)
            cfg["hevy_enabled"] = True
            config.save_config(conn, cfg)

    def action(message: str, *, mfa_required: bool = False) -> ActionResult:
        return ActionResult(
            message=message,
            setup_step=onboarding.setup_step(conn),
            mfa_required=mfa_required,
        )

    @router.get("/settings", response_model=SettingsState)
    def settings_state() -> SettingsState:
        connections = view.connection_status(conn)
        step = onboarding.setup_step(conn, connections)
        cfg = config.load_config(conn)
        update = update_check.get_status()
        garmin_credentials = db.get_config_value(conn, "garmin_credentials") or {}
        strava_credentials = db.get_config_value(conn, "strava_credentials") or {}
        held = set(cfg["held_activity_types"])
        hevy_view = view.hevy_settings_view(conn)
        return SettingsState(
            version=__version__,
            update=UpdateState(
                latest=update.latest,
                available=update.update_available,
                repo_url=update.repo_url,
                release_url=update.release_url,
            ),
            development=mock_mode(),
            setup=SetupProgress(
                complete=step is None,
                step=step,
                mfa_required="auth" in pending_garmin_mfa,
            ),
            connections=Connections.model_validate(connections),
            credentials=CredentialState(
                garmin_email=garmin_credentials.get("email", ""),
                garmin_password_saved=bool(garmin_credentials.get("password")),
                strava_client_id=strava_credentials.get("client_id", ""),
                strava_client_secret_saved=bool(strava_credentials.get("client_secret")),
            ),
            preferences=PreferenceState.model_validate(cfg),
            timezones=timeutil.common_timezones(),
            activity_types=[
                ActivityTypeState(
                    type_key=item["type_key"],
                    label=item["label"],
                    autosync=item["type_key"] not in held,
                )
                for item in db.get_config_value(
                    conn, "garmin_activity_types", default=[]
                )
            ],
            hevy=HevySettingsState(
                connected=hevy_view["connected"],
                status=hevy_view["status"],
                api_key_saved=hevy_view["api_key_saved"],
                enabled=bool(cfg["hevy_enabled"]),
                watch_strategy=cfg["hevy_watch_strategy"],
                grace_minutes=int(cfg["hevy_grace_minutes"]),
                poll_interval_minutes=int(cfg["hevy_poll_interval_minutes"]),
                identity=DeviceIdentity.model_validate(hevy_view["identity"]),
                identity_display=hevy_view["identity_display"],
                profile_override=ProfileOverride.model_validate(
                    hevy_view["profile_override"]
                ),
            ),
        )

    @router.post("/setup/garmin", response_model=ActionResult)
    def setup_garmin(payload: GarminSetupRequest) -> ActionResult:
        if db.get_config_value(conn, "initial_sync_done", default=False):
            raise HTTPException(status_code=409, detail="Setup is already complete.")
        existing = db.get_config_value(conn, "garmin_credentials") or {}
        email = (payload.email or existing.get("email", "")).strip()
        password = payload.password or existing.get("password", "")
        if not email or not password:
            raise HTTPException(
                status_code=400, detail="Enter your Garmin email and password."
            )
        cfg = config.load_config(conn)
        cfg["lookback_days"] = payload.lookback_days
        if timeutil.is_valid_timezone(payload.detected_timezone):
            cfg["display_timezone"] = payload.detected_timezone
        config.save_config(conn, cfg)
        outcome = save_and_verify_garmin(email, password)
        if outcome == "mfa":
            return action("Garmin sent a verification code.", mfa_required=True)
        events.bus.publish("refresh")
        return action("Garmin connected.")

    @router.post("/settings/garmin/reconnect", response_model=ActionResult)
    def reconnect_garmin(payload: GarminReconnectRequest) -> ActionResult:
        existing = db.get_config_value(conn, "garmin_credentials") or {}
        email = (payload.email or existing.get("email", "")).strip()
        password = payload.password or existing.get("password", "")
        if not email or not password:
            raise HTTPException(
                status_code=400, detail="Enter your Garmin email and password."
            )
        last_sync_ok_at = db.get_config_value(conn, "garmin_last_sync_ok_at")
        outcome = save_and_verify_garmin(email, password)
        if outcome == "mfa":
            return action("Garmin sent a verification code.", mfa_required=True)
        onboarding.run_catch_up(
            conn,
            build_garmin_client(conn),
            build_strava_client(conn),
            last_sync_ok_at=last_sync_ok_at,
        )
        return action("Garmin reconnected and catch-up sync started.")

    @router.post("/garmin/mfa", response_model=ActionResult)
    def complete_mfa(payload: MfaRequest) -> ActionResult:
        pending = pending_garmin_mfa.get("auth")
        if pending is None:
            raise HTTPException(
                status_code=409, detail="The Garmin verification session expired."
            )
        last_sync_ok_at = db.get_config_value(conn, "garmin_last_sync_ok_at")
        try:
            complete_garmin_login(pending, payload.code)
        except Exception as exc:
            raise HTTPException(status_code=401, detail=str(exc)) from exc
        pending_garmin_mfa.pop("auth", None)
        credentials = pending_garmin_mfa.pop("credentials", None)
        if credentials is not None:
            db.set_config_value(conn, "garmin_credentials", credentials)
        finalize_garmin_connect()
        if db.get_config_value(conn, "initial_sync_done", default=False):
            onboarding.run_catch_up(
                conn,
                build_garmin_client(conn),
                build_strava_client(conn),
                last_sync_ok_at=last_sync_ok_at,
            )
        else:
            events.bus.publish("refresh")
        return action("Garmin verified and connected.")

    @router.delete("/garmin/mfa", response_model=ActionResult)
    def cancel_mfa() -> ActionResult:
        pending_garmin_mfa.pop("auth", None)
        pending_garmin_mfa.pop("credentials", None)
        return action("Garmin verification cancelled.")

    @router.put("/settings/strava-credentials", response_model=ActionResult)
    def save_strava_credentials(payload: StravaCredentialsRequest) -> ActionResult:
        existing = db.get_config_value(conn, "strava_credentials") or {}
        client_id = payload.client_id.strip()
        client_secret = payload.client_secret or existing.get("client_secret", "")
        if not client_id or not client_secret:
            raise HTTPException(
                status_code=400,
                detail="Enter both Strava client credentials before saving.",
            )
        onboarding.persist_strava_credentials(conn, client_id, client_secret)
        events.bus.publish("refresh")
        return action("Strava credentials saved. Continue to authorization.")

    @router.post("/setup/hevy", response_model=ActionResult)
    def setup_hevy(payload: HevyCredentialsRequest) -> ActionResult:
        if db.get_config_value(conn, "initial_sync_done", default=False):
            raise HTTPException(status_code=409, detail="Setup is already complete.")
        connect_hevy(payload.api_key, in_setup=True)
        events.bus.publish("refresh")
        return action("Hevy connected.")

    @router.post("/setup/hevy/skip", response_model=ActionResult)
    def skip_hevy() -> ActionResult:
        if db.get_config_value(conn, "initial_sync_done", default=False):
            raise HTTPException(status_code=409, detail="Setup is already complete.")
        db.set_config_value(conn, "setup_hevy_done", True)
        events.bus.publish("refresh")
        return action("Hevy skipped. You can connect it later in Settings.")

    @router.post("/setup/initial-sync", response_model=ActionResult)
    def initial_sync() -> ActionResult:
        if db.get_config_value(conn, "initial_sync_done", default=False):
            raise HTTPException(status_code=409, detail="Setup is already complete.")
        connections = view.connection_status(conn)
        if not connections["garmin"]["connected"] or not connections["strava"]["connected"]:
            raise HTTPException(
                status_code=409, detail="Connect Garmin and Strava before syncing."
            )
        try:
            now = datetime.now(timezone.utc)
            cfg = config.load_config(conn)
            if not db.get_config_value(conn, "garmin_activity_types", default=[]):
                fetch_and_store_categories(hold_all=True)
                cfg = config.load_config(conn)
            sync.sync_garmin(conn, build_garmin_client(conn), cfg, now)
            sync.check_strava_status(conn, build_strava_client(conn), cfg, now)
        except Exception as exc:
            raise HTTPException(
                status_code=502, detail=f"Initial sync failed: {exc}"
            ) from exc
        db.set_config_value(conn, "initial_sync_done", True)
        events.bus.publish("refresh")
        return action("Setup complete. Your activities are ready.")

    @router.put("/settings/preferences", response_model=ActionResult)
    def save_preferences(payload: PreferencesRequest) -> ActionResult:
        if not timeutil.is_valid_timezone(payload.display_timezone):
            raise HTTPException(
                status_code=400,
                detail=f"Unknown timezone: {payload.display_timezone}",
            )
        cfg = config.load_config(conn)
        cfg.update(payload.model_dump())
        config.save_config(conn, cfg)
        logging_setup.set_log_timezone(payload.display_timezone)
        events.bus.publish("refresh")
        return action("Preferences saved.")

    @router.put("/settings/activity-types", response_model=ActionResult)
    def save_activity_types(payload: ActivityTypesRequest) -> ActionResult:
        if not view.connection_status(conn)["garmin"]["connected"]:
            raise HTTPException(
                status_code=409,
                detail="Connect to Garmin before changing activity categories.",
            )
        known = {
            item["type_key"]
            for item in db.get_config_value(conn, "garmin_activity_types", default=[])
        }
        requested = set(payload.autosync_types)
        unknown = requested - known
        if unknown:
            raise HTTPException(status_code=400, detail="Unknown activity category.")
        held = sorted(known - requested)
        cfg = config.load_config(conn)
        cfg["held_activity_types"] = held
        config.save_config(conn, cfg)
        sync.reconcile_held_activities(conn, held)
        events.bus.publish("refresh")
        return action("Autosync categories saved.")

    @router.post("/settings/activity-types/refresh", response_model=ActionResult)
    def refresh_activity_types() -> ActionResult:
        if not view.connection_status(conn)["garmin"]["connected"]:
            raise HTTPException(
                status_code=409,
                detail="Connect to Garmin before refreshing activity categories.",
            )
        fetch_and_store_categories(hold_all=False)
        events.bus.publish("refresh")
        return action("Garmin activity categories refreshed.")

    @router.post("/settings/sync/{service}", response_model=ActionResult)
    def manual_sync(service: Literal["garmin", "strava"]) -> ActionResult:
        connections = view.connection_status(conn)
        cfg = config.load_config(conn)
        now = datetime.now(timezone.utc)
        if service == "garmin":
            if not connections["garmin"]["connected"]:
                raise HTTPException(
                    status_code=409,
                    detail="Garmin is disconnected — reconnect it to sync.",
                )
            sync.sync_garmin(conn, build_garmin_client(conn), cfg, now)
            if connections["strava"]["connected"]:
                sync.check_strava_status(conn, build_strava_client(conn), cfg, now)
            message = "Garmin synced."
        else:
            if not connections["strava"]["connected"]:
                raise HTTPException(
                    status_code=409,
                    detail="Strava is disconnected — reconnect it to sync.",
                )
            try:
                sync.check_strava_status(conn, build_strava_client(conn), cfg, now)
            except StravaAuthError as exc:
                raise HTTPException(status_code=409, detail=str(exc)) from exc
            message = "Strava synced."
        events.bus.publish("refresh")
        return action(message)

    @router.post("/settings/hevy/credentials", response_model=ActionResult)
    def save_hevy_credentials(payload: HevyCredentialsRequest) -> ActionResult:
        connect_hevy(payload.api_key, in_setup=False)
        events.bus.publish("refresh")
        return action("Hevy connected.")

    @router.put("/settings/hevy", response_model=ActionResult)
    def save_hevy_settings(payload: HevySettingsRequest) -> ActionResult:
        identity_values = payload.identity.model_dump()
        provided = [value for value in identity_values.values() if value is not None]
        if provided and len(provided) != 3:
            raise HTTPException(
                status_code=400,
                detail="Device identity override needs all three numbers — or none.",
            )
        cfg = config.load_config(conn)
        cfg.update(
            {
                "hevy_enabled": payload.enabled,
                "hevy_watch_strategy": payload.watch_strategy,
                "hevy_grace_minutes": payload.grace_minutes,
                "hevy_poll_interval_minutes": payload.poll_interval_minutes,
                "hevy_device_identity": identity_values if provided else None,
            }
        )
        profile = {
            key: value
            for key, value in payload.profile_override.model_dump().items()
            if value is not None
        }
        if profile:
            cfg["profile_override"] = profile
        else:
            cfg.pop("profile_override", None)
        config.save_config(conn, cfg)
        events.bus.publish("refresh")
        return action("Hevy settings saved.")

    @router.delete("/settings/hevy", response_model=ActionResult)
    def disconnect_hevy() -> ActionResult:
        db.set_config_value(conn, "hevy_api_key", None)
        db.set_config_value(conn, "hevy_auth_ok", None)
        cfg = config.load_config(conn)
        cfg["hevy_enabled"] = False
        config.save_config(conn, cfg)
        events.bus.publish("refresh")
        return action("Hevy disconnected.")

    @router.delete("/settings/strava", response_model=ActionResult)
    def disconnect_strava() -> ActionResult:
        build_strava_client(conn).disconnect()
        events.bus.publish("refresh")
        return action("Strava disconnected. Publishing is paused.")

    return router
