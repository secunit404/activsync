"""Typed sign-in, claim, session, and access-settings routes."""

from __future__ import annotations

import sqlite3
import time
from collections.abc import Callable
from datetime import datetime, timezone
from typing import Literal

from fastapi import APIRouter, HTTPException, Request, Response
from pydantic import Field
from starlette.concurrency import run_in_threadpool

from activsync import auth_core, auth_devices, auth_oidc, auth_oidc_logo, db
from activsync.api_routes import ApiModel
from activsync.auth_middleware import (
    AuthState,
    ProxyAuthConfig,
    clear_session_cookie,
    client_ip,
    device_cookie_name,
    own_origin,
    start_session,
)

AuthMethod = Literal["password", "oidc", "proxy", "disabled"]


class OidcLoginOption(ApiModel):
    label: str
    auto_redirect: bool
    logo_url: str | None


class LoginOptions(ApiModel):
    oidc: OidcLoginOption | None


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


class AuthorizeUrl(ApiModel):
    url: str


class PasswordChangeRequest(ApiModel):
    current: str = Field(max_length=1024)
    new: str = Field(max_length=1024)


class ProxyState(ApiModel):
    header: str
    trusted: list[str]
    user: str


class AuthSettingsState(ApiModel):
    proxy: ProxyState | None


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


class OidcConfigState(ApiModel):
    configured: bool
    display_name: str
    issuer: str
    client_id: str
    has_secret: bool
    scopes: str
    auto_redirect: bool
    redirect_uri: str
    linked_label: str | None
    logo_url: str | None


class OidcConfigRequest(ApiModel):
    display_name: str = Field(min_length=1, max_length=64)
    issuer: str = Field(min_length=1, max_length=512)
    client_id: str = Field(min_length=1, max_length=256)
    client_secret: str | None = Field(default=None, max_length=1024)
    scopes: str = Field(default=auth_oidc.DEFAULT_SCOPES, max_length=512)
    auto_redirect: bool = False
    current_password: str = Field(max_length=1024)


class CurrentPasswordRequest(ApiModel):
    current_password: str = Field(max_length=1024)


def _iso(epoch: int) -> str:
    return datetime.fromtimestamp(epoch, timezone.utc).isoformat()


def _current_auth(request: Request) -> AuthState | None:
    return getattr(request.state, "auth", None)


async def _read_capped_body(request: Request, limit: int) -> bytes:
    """413 as soon as the body passes `limit`, without buffering the rest."""
    declared = request.headers.get("content-length", "")
    if declared.isdigit() and int(declared) > limit:
        raise HTTPException(status_code=413, detail=auth_oidc_logo.TOO_LARGE)
    chunks: list[bytes] = []
    size = 0
    async for chunk in request.stream():
        size += len(chunk)
        if size > limit:
            raise HTTPException(status_code=413, detail=auth_oidc_logo.TOO_LARGE)
        chunks.append(chunk)
    return b"".join(chunks)


def create_router(
    conn: sqlite3.Connection,
    *,
    setup_code: auth_core.SetupCode,
    limiter: auth_core.AttemptLimiter,
    public_url: str | None,
    proxy: ProxyAuthConfig | None,
    mock_mode: Callable[[], bool],
) -> APIRouter:
    router = APIRouter(prefix="/api/v1", tags=["auth"])

    def login_options() -> LoginOptions:
        oidc_config = auth_oidc.load_config(conn)
        return LoginOptions(
            oidc=OidcLoginOption(
                label=oidc_config.display_name,
                auto_redirect=oidc_config.auto_redirect,
                logo_url=auth_oidc_logo.current_url(conn),
            )
            if oidc_config and oidc_config.linked
            else None,
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
        return AuthSettingsState(
            proxy=ProxyState.model_validate(proxy.describe()) if proxy else None,
        )

    @router.get("/auth/oidc", response_model=OidcConfigState)
    def oidc_config(request: Request) -> OidcConfigState:
        config = auth_oidc.load_config(conn)
        origin = public_url or own_origin(request.scope, request.headers)
        return OidcConfigState(
            configured=config is not None,
            display_name=config.display_name if config else "",
            issuer=config.issuer if config else "",
            client_id=config.client_id if config else "",
            has_secret=bool(config and config.client_secret),
            scopes=config.scopes if config else auth_oidc.DEFAULT_SCOPES,
            auto_redirect=bool(config and config.auto_redirect),
            redirect_uri=f"{origin}/auth/oidc/callback",
            linked_label=config.linked["label"] if config and config.linked else None,
            logo_url=auth_oidc_logo.current_url(conn),
        )

    @router.put("/auth/oidc", response_model=AuthActionResult)
    def save_oidc_config(payload: OidcConfigRequest, request: Request) -> AuthActionResult:
        require_password(
            payload.current_password, wrong_status=400, device_id=trusted_device(request)
        )
        try:
            config = auth_oidc.save_config(
                conn,
                display_name=payload.display_name.strip(),
                issuer=payload.issuer,
                client_id=payload.client_id.strip(),
                client_secret=payload.client_secret,
                scopes=payload.scopes,
                auto_redirect=payload.auto_redirect,
            )
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        if config.linked:
            return AuthActionResult(message="Single sign-on settings saved.")
        return AuthActionResult(
            message="Single sign-on settings saved. Link your account to finish."
        )

    @router.delete("/auth/oidc", response_model=AuthActionResult)
    def delete_oidc_config(payload: CurrentPasswordRequest, request: Request) -> AuthActionResult:
        require_password(
            payload.current_password, wrong_status=400, device_id=trusted_device(request)
        )
        auth_oidc.delete_config(conn)
        return AuthActionResult(message="Single sign-on removed.")

    @router.get("/auth/oidc/logo", include_in_schema=False)
    def oidc_logo() -> Response:
        logo = auth_oidc_logo.load(conn)
        if logo is None:
            raise HTTPException(status_code=404, detail="No logo uploaded")
        return Response(
            content=logo.png,
            media_type="image/png",
            headers={
                "Cache-Control": "public, max-age=31536000, immutable",
                "Content-Security-Policy": "default-src 'none'; sandbox",
                "X-Content-Type-Options": "nosniff",
            },
        )

    @router.put("/auth/oidc/logo", response_model=AuthActionResult)
    async def upload_oidc_logo(request: Request) -> AuthActionResult:
        if await run_in_threadpool(auth_oidc.load_config, conn) is None:
            raise HTTPException(status_code=409, detail="Set up single sign-on first.")
        data = await _read_capped_body(request, auth_oidc_logo.MAX_UPLOAD_BYTES)
        try:
            png = await run_in_threadpool(auth_oidc_logo.normalize, data)
        except auth_oidc_logo.LogoError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        await run_in_threadpool(auth_oidc_logo.save, conn, png, time.time())
        return AuthActionResult(message="Logo saved.")

    @router.delete("/auth/oidc/logo", response_model=AuthActionResult)
    def delete_oidc_logo() -> AuthActionResult:
        auth_oidc_logo.delete(conn)
        return AuthActionResult(message="Logo removed. The sign-in button shows a key again.")

    @router.post("/dev/unclaim", include_in_schema=False)
    def dev_unclaim() -> dict:
        """Mock-only: drop the owner so an E2E spec can walk the claim step
        against the shared dev server. 404s outside mock mode."""
        if not mock_mode():
            raise HTTPException(status_code=404, detail="Not found")
        db.set_config_value(conn, auth_core.OWNER_KEY, None)
        return {"setupCode": setup_code.reissue()}

    return router
