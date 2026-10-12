"""Request pipeline guards: security headers, authentication, and CSRF.
Pure ASGI so they run before routing and cover every route, including ones
added later."""

from __future__ import annotations

import base64
import hashlib
import logging
import re
import sqlite3
import time
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import quote, urlsplit

from starlette.concurrency import run_in_threadpool
from starlette.datastructures import MutableHeaders
from starlette.requests import Request
from starlette.responses import JSONResponse, PlainTextResponse, RedirectResponse

from activsync import auth_core

logger = logging.getLogger("activsync.auth")

PUBLIC_ROUTES = frozenset({
    ("GET", "/health"),
    ("GET", "/favicon.ico"),
    ("GET", "/apple-touch-icon.png"),
    ("GET", "/api/v1/auth/status"),
    ("POST", "/api/v1/auth/login"),
    ("POST", "/api/v1/auth/claim"),
})
PUBLIC_PREFIXES = (("GET", "/assets/"),)
SPA_RESERVED_PREFIXES = ("api", "assets", "strava", "auth")
UNSAFE_METHODS = frozenset({"POST", "PUT", "PATCH", "DELETE"})

SESSION_COOKIE = "activsync_session"
SECURE_SESSION_COOKIE = "__Host-activsync_session"


def is_public(method: str, path: str) -> bool:
    method = "GET" if method == "HEAD" else method
    if (method, path) in PUBLIC_ROUTES:
        return True
    if any(method == m and path.startswith(prefix) for m, prefix in PUBLIC_PREFIXES):
        return True
    first_segment = path.lstrip("/").split("/", 1)[0]
    return method == "GET" and first_segment not in SPA_RESERVED_PREFIXES


@dataclass(frozen=True)
class AuthState:
    method: str
    token_hash: str | None = None
    public_id: str | None = None
    user: str | None = None


def is_https(scope: Mapping) -> bool:
    return scope.get("scheme") == "https"


def session_cookie_name(scope: Mapping) -> str:
    return SECURE_SESSION_COOKIE if is_https(scope) else SESSION_COOKIE


def login_redirect_target(request: Request) -> str:
    path = request.url.path
    if request.url.query:
        path = f"{path}?{request.url.query}"
    return f"/login?next={quote(path, safe='')}"


def external_url(request: Request, route_name: str, public_url: str | None) -> str:
    if public_url:
        return f"{public_url}{request.app.url_path_for(route_name)}"
    return str(request.url_for(route_name))


_INLINE_SCRIPT = re.compile(r"<script\b([^>]*)>(.*?)</script>", re.DOTALL | re.IGNORECASE)


def inline_script_hashes(html: str) -> list[str]:
    hashes = []
    for attributes, body in _INLINE_SCRIPT.findall(html):
        if "src=" in attributes or not body.strip():
            continue
        digest = hashlib.sha256(body.encode()).digest()
        hashes.append(base64.b64encode(digest).decode())
    return hashes


def content_security_policy(index_html: Path) -> str:
    """React Router's SPA build inlines its bootstrap scripts, so they are
    allowed by hash rather than with 'unsafe-inline'."""
    hashes = inline_script_hashes(index_html.read_text()) if index_html.is_file() else []
    script_src = " ".join(["'self'", *(f"'sha256-{value}'" for value in hashes)])
    return "; ".join([
        "default-src 'self'",
        f"script-src {script_src}",
        "style-src 'self' 'unsafe-inline'",
        "img-src 'self' data:",
        "font-src 'self' data:",
        "connect-src 'self'",
        "object-src 'none'",
        "base-uri 'self'",
        "form-action 'self'",
        "frame-ancestors 'none'",
    ])


class SecurityHeadersMiddleware:
    def __init__(self, app, *, content_security_policy: str) -> None:
        self.app = app
        self.csp = content_security_policy

    async def __call__(self, scope, receive, send) -> None:
        if scope["type"] != "http":
            await self.app(scope, receive, send)
            return
        https = is_https(scope)

        async def send_with_headers(message) -> None:
            if message["type"] == "http.response.start":
                headers = MutableHeaders(scope=message)
                headers.setdefault("Content-Security-Policy", self.csp)
                headers.setdefault("Referrer-Policy", "same-origin")
                headers.setdefault("X-Content-Type-Options", "nosniff")
                if https:
                    headers.setdefault("Strict-Transport-Security", "max-age=31536000")
            await send(message)

        await self.app(scope, receive, send_with_headers)


class AuthenticateMiddleware:
    def __init__(
        self,
        app,
        *,
        conn: sqlite3.Connection,
        public_url: str | None,
        clock: Callable[[], float] = time.time,
    ) -> None:
        self.app = app
        self.conn = conn
        self.public_host = urlsplit(public_url).netloc if public_url else None
        self.clock = clock

    async def __call__(self, scope, receive, send) -> None:
        if scope["type"] != "http":
            await self.app(scope, receive, send)
            return
        request = Request(scope)
        path = scope["path"]
        if (
            self.public_host
            and path != "/health"
            and request.headers.get("host") != self.public_host
        ):
            await PlainTextResponse("Unknown host", status_code=400)(scope, receive, send)
            return

        # A sync DB read; off the event loop so a long transaction elsewhere
        # cannot stall every open request and the SSE stream with it.
        auth = await run_in_threadpool(self._authenticate, request)
        scope.setdefault("state", {})["auth"] = auth

        if auth is None and not is_public(scope["method"], path):
            if path.startswith("/api/"):
                response = JSONResponse({"detail": "Not signed in"}, status_code=401)
            else:
                response = RedirectResponse(login_redirect_target(request), status_code=303)
            await response(scope, receive, send)
            return
        await self.app(scope, receive, send)

    def _authenticate(self, request: Request) -> AuthState | None:
        if auth_core.auth_disabled():
            return AuthState(method="disabled")
        token = request.cookies.get(session_cookie_name(request.scope))
        if not token:
            return None
        session = auth_core.resolve_session(self.conn, token, self.clock())
        if session is None:
            return None
        return AuthState(
            method=session.method,
            token_hash=session.token_hash,
            public_id=session.public_id,
        )


def own_origin(scope: Mapping, headers: Mapping[str, str]) -> str:
    return f"{scope.get('scheme', 'http')}://{headers.get('host', '')}"


def _origin_of(url: str) -> str:
    parts = urlsplit(url)
    return f"{parts.scheme}://{parts.netloc}"


def csrf_problem(headers: Mapping[str, str], expected_origin: str) -> str | None:
    if headers.get("x-activsync-request") != "1":
        return "missing X-ActivSync-Request header"
    origin = headers.get("origin")
    if origin == "null":
        return "opaque Origin"
    site = headers.get("sec-fetch-site")
    if site is not None:
        return None if site == "same-origin" else f"Sec-Fetch-Site is {site}"
    if origin:
        return None if origin == expected_origin else f"Origin {origin} does not match"
    referer = headers.get("referer")
    if referer:
        return None if _origin_of(referer) == expected_origin else "Referer does not match"
    return "no Origin or Referer"


class CsrfMiddleware:
    def __init__(self, app, *, public_url: str | None) -> None:
        self.app = app
        self.public_origin = _origin_of(public_url) if public_url else None

    async def __call__(self, scope, receive, send) -> None:
        if scope["type"] == "http" and scope["method"] in UNSAFE_METHODS:
            request = Request(scope)
            expected = self.public_origin or own_origin(scope, request.headers)
            problem = csrf_problem(request.headers, expected)
            if problem:
                logger.warning(
                    "blocked %s %s: %s", scope["method"], scope["path"], problem
                )
                await JSONResponse(
                    {"detail": "This request was blocked as cross-site."},
                    status_code=403,
                )(scope, receive, send)
                return
        await self.app(scope, receive, send)
