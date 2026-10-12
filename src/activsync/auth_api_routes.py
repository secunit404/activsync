"""Typed sign-in, claim, session, and access-settings routes."""

from __future__ import annotations

import sqlite3
import time
from collections.abc import Callable
from datetime import datetime, timezone
from typing import Literal

from fastapi import APIRouter, HTTPException, Request, Response
from pydantic import Field

from activsync import auth_core, auth_devices, db
from activsync.api_routes import ApiModel
from activsync.auth_middleware import (
    AuthState,
    clear_session_cookie,
    client_ip,
    device_cookie_name,
    start_session,
)

AuthMethod = Literal["password", "oidc", "strava", "proxy", "disabled"]
PasswordForm = Literal["shown", "hidden"]


class OidcLoginOption(ApiModel):
    label: str
    auto_redirect: bool


class LoginOptions(ApiModel):
    password_form: PasswordForm
    oidc: OidcLoginOption | None
    strava: bool


class AuthStatus(ApiModel):
    authenticated: bool
    method: AuthMethod | None
    user: str | None
    owner_claimed: bool
    auth_disabled: bool
    login: LoginOptions


class ClaimRequest(ApiModel):
    setup_code: str = Field(max_length=64)
    password: str = Field(max_length=1024)


class LoginRequest(ApiModel):
    password: str = Field(max_length=1024)


class PasswordChangeRequest(ApiModel):
    current: str = Field(max_length=1024)
    new: str = Field(max_length=1024)


class AuthSettingsRequest(ApiModel):
    password_form: PasswordForm | None = None


class AuthSettingsState(ApiModel):
    password_form: PasswordForm


class SessionInfo(ApiModel):
    public_id: str
    method: str
    created_at: str
    last_seen_at: str
    ip: str
    user_agent: str
    current: bool


class AuthActionResult(ApiModel):
    message: str


def _iso(epoch: int) -> str:
    return datetime.fromtimestamp(epoch, timezone.utc).isoformat()


def _current_auth(request: Request) -> AuthState | None:
    return getattr(request.state, "auth", None)


def create_router(
    conn: sqlite3.Connection,
    *,
    setup_code: auth_core.SetupCode,
    limiter: auth_core.AttemptLimiter,
    mock_mode: Callable[[], bool],
) -> APIRouter:
    router = APIRouter(prefix="/api/v1", tags=["auth"])

    def login_options() -> LoginOptions:
        settings = auth_core.load_settings(conn)
        return LoginOptions(
            password_form=settings["password_form"],
            oidc=None,
            strava=False,
        )

    def status_for(auth: AuthState | None) -> AuthStatus:
        return AuthStatus(
            authenticated=auth is not None,
            method=auth.method if auth else None,
            user=auth.user if auth else None,
            owner_claimed=auth_core.owner_claimed(conn),
            auth_disabled=auth_core.auth_disabled(),
            login=login_options(),
        )

    def limit_client(request: Request) -> None:
        wait = limiter.retry_after(client_ip(request), time.time())
        if wait:
            raise HTTPException(
                status_code=429,
                detail=f"Too many attempts. Try again in {wait} seconds.",
                headers={"Retry-After": str(wait)},
            )

    def trusted_device(request: Request) -> str | None:
        device_id = request.cookies.get(device_cookie_name(request.scope))
        return device_id if auth_devices.is_trusted(conn, device_id) else None

    def require_password(
        password: str, *, wrong_status: int, device_id: str | None
    ) -> None:
        check = auth_devices.check_password(
            conn, password, time.time(), device_id=device_id
        )
        if check.retry_after:
            raise HTTPException(
                status_code=429,
                detail=f"Too many wrong passwords. Try again in {check.retry_after} seconds.",
                headers={"Retry-After": str(check.retry_after)},
            )
        if not check.verified:
            raise HTTPException(status_code=wrong_status, detail="Wrong password.")

    def require_session(request: Request) -> AuthState:
        auth = _current_auth(request)
        if auth is None:
            raise HTTPException(status_code=401, detail="Not signed in")
        return auth

    @router.get("/auth/status", response_model=AuthStatus)
    def auth_status(request: Request) -> AuthStatus:
        return status_for(_current_auth(request))

    @router.post("/auth/claim", response_model=AuthStatus)
    def claim(payload: ClaimRequest, request: Request, response: Response) -> AuthStatus:
        if auth_core.owner_claimed(conn):
            raise HTTPException(status_code=409, detail="ActivSync is already claimed.")
        limit_client(request)
        problem = auth_core.password_problem(payload.password)
        if problem:
            raise HTTPException(status_code=400, detail=problem)
        if not setup_code.check(payload.setup_code):
            raise HTTPException(
                status_code=403,
                detail="That setup code is not right. Check the server log for the current one.",
            )
        auth_core.set_password(conn, payload.password, time.time())
        setup_code.discard()
        start_session(conn, request, response, "password")
        return status_for(AuthState(method="password"))

    @router.post("/auth/login", response_model=AuthStatus)
    def login(payload: LoginRequest, request: Request, response: Response) -> AuthStatus:
        if not auth_core.owner_claimed(conn):
            raise HTTPException(status_code=409, detail="ActivSync is not claimed yet.")
        device_id = trusted_device(request)
        if device_id is None:
            limit_client(request)
        require_password(payload.password, wrong_status=401, device_id=device_id)
        start_session(conn, request, response, "password")
        return status_for(AuthState(method="password"))

    @router.post("/auth/logout", status_code=204)
    def logout(request: Request, response: Response) -> None:
        auth = require_session(request)
        if auth.token_hash is None:
            raise HTTPException(
                status_code=409,
                detail="You are signed in through your proxy; sign out there.",
            )
        auth_core.revoke_session(conn, auth.token_hash)
        clear_session_cookie(request, response)

    @router.get("/auth/sessions", response_model=list[SessionInfo])
    def list_sessions(request: Request) -> list[SessionInfo]:
        auth = require_session(request)
        return [
            SessionInfo(
                public_id=session.public_id,
                method=session.method,
                created_at=_iso(session.created_at),
                last_seen_at=_iso(session.last_seen_at),
                ip=session.ip,
                user_agent=session.user_agent,
                current=session.token_hash == auth.token_hash,
            )
            for session in auth_core.list_sessions(conn)
        ]

    @router.delete("/auth/sessions/{public_id}", status_code=204)
    def revoke_one(public_id: str) -> None:
        if not auth_core.revoke_public_id(conn, public_id):
            raise HTTPException(status_code=404, detail="That session has already ended.")

    @router.delete("/auth/sessions", response_model=AuthActionResult)
    def revoke_others(request: Request) -> AuthActionResult:
        auth = require_session(request)
        revoked = auth_core.revoke_all(conn, keep_token_hash=auth.token_hash)
        auth_devices.clear_trusted_devices(conn)
        return AuthActionResult(message=f"Signed out {revoked} other session(s).")

    @router.put("/auth/password", response_model=AuthActionResult)
    def change_password(payload: PasswordChangeRequest, request: Request) -> AuthActionResult:
        auth = require_session(request)
        require_password(
            payload.current, wrong_status=400, device_id=trusted_device(request)
        )
        problem = auth_core.password_problem(payload.new)
        if problem:
            raise HTTPException(status_code=400, detail=problem)
        auth_core.set_password(conn, payload.new, time.time())
        auth_core.revoke_all(conn, keep_token_hash=auth.token_hash)
        auth_devices.clear_trusted_devices(conn)
        return AuthActionResult(message="Password changed. Other sessions were signed out.")

    @router.get("/auth/settings", response_model=AuthSettingsState)
    def auth_settings() -> AuthSettingsState:
        settings = auth_core.load_settings(conn)
        return AuthSettingsState(password_form=settings["password_form"])

    @router.put("/auth/settings", response_model=AuthActionResult)
    def save_auth_settings(payload: AuthSettingsRequest) -> AuthActionResult:
        auth_core.save_settings(conn, **payload.model_dump(exclude_none=True))
        return AuthActionResult(message="Sign-in settings saved.")

    @router.post("/dev/unclaim", include_in_schema=False)
    def dev_unclaim() -> dict:
        """Mock-only: drop the owner so an E2E spec can walk the claim step
        against the shared dev server. 404s outside mock mode."""
        if not mock_mode():
            raise HTTPException(status_code=404, detail="Not found")
        db.set_config_value(conn, auth_core.OWNER_KEY, None)
        return {"setupCode": setup_code.reissue()}

    return router
