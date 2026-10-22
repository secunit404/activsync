"""Browser-redirect OIDC sign-in flow under /auth."""

from __future__ import annotations

import logging
import sqlite3
import time
from typing import Literal
from urllib.parse import urlencode

from fastapi import APIRouter, Request
from fastapi.responses import RedirectResponse
from starlette.concurrency import run_in_threadpool

from activsync import auth_oidc
from activsync.auth_middleware import external_url, safe_next, start_session

logger = logging.getLogger("activsync.auth")


def _login_error(code: str) -> RedirectResponse:
    return RedirectResponse(f"/login?{urlencode({'error': code})}", status_code=303)


def _link_error(code: str) -> RedirectResponse:
    return RedirectResponse(f"/settings?{urlencode({'oidcError': code})}", status_code=303)


def create_router(
    conn: sqlite3.Connection,
    *,
    oidc: auth_oidc.OidcProvider,
    public_url: str | None,
) -> APIRouter:
    router = APIRouter(prefix="/auth", include_in_schema=False)

    @router.get("/oidc/login", name="oidc_login")
    async def oidc_login(
        request: Request,
        intent: Literal["login", "link"] = "login",
        next: str = "/",
    ):
        if intent == "link" and getattr(request.state, "auth", None) is None:
            return RedirectResponse("/login?next=%2Fsettings", status_code=303)
        if intent == "link" and not auth_oidc.consume_link_grant(request.session, time.time()):
            return _link_error("link_password")
        try:
            return await oidc.begin(
                request,
                redirect_uri=external_url(request, "oidc_callback", public_url),
                intent=intent,
                next_path=safe_next(next),
            )
        except auth_oidc.OidcError as exc:
            return _link_error(exc.code) if intent == "link" else _login_error(exc.code)

    @router.get("/oidc/callback", name="oidc_callback")
    async def oidc_callback(request: Request):
        try:
            result = await oidc.finish(request)
        except auth_oidc.OidcError as exc:
            return _link_error(exc.code) if exc.intent == "link" else _login_error(exc.code)

        if result.intent == "link":
            if getattr(request.state, "auth", None) is None:
                return _login_error("oidc_state")
            await run_in_threadpool(
                auth_oidc.set_linked,
                conn,
                iss=result.iss,
                sub=result.sub,
                label=result.label,
                keep_token_hash=request.state.auth.token_hash,
            )
            return RedirectResponse("/settings?oidc=linked", status_code=303)

        config = await run_in_threadpool(auth_oidc.load_config, conn)
        linked = config.linked if config else None
        if not linked or (linked["iss"], linked["sub"]) != (result.iss, result.sub):
            logger.warning(
                "OIDC sign-in by %s at %s refused; it is not the linked account",
                result.sub,
                result.iss,
            )
            return _login_error("oidc_mismatch")
        response = RedirectResponse(result.next_path, status_code=303)
        await run_in_threadpool(start_session, conn, request, response, "oidc")
        return response

    return router
