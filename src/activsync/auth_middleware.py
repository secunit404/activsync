"""Request pipeline guards: peer capture, security headers, authentication,
and CSRF. Pure ASGI so they run before routing and cover every route,
including ones added later."""

from __future__ import annotations

import base64
import hashlib
import ipaddress
import logging
import re
import sqlite3
import time
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import quote, urlsplit

from starlette.concurrency import run_in_threadpool
from starlette.datastructures import MutableHeaders
from starlette.requests import Request
from starlette.responses import JSONResponse, PlainTextResponse, RedirectResponse, Response

from activsync import auth_body_cap, auth_core, auth_devices, net_gateways

logger = logging.getLogger("activsync.auth")

PUBLIC_ROUTES = frozenset({
    ("GET", "/health"),
    ("GET", "/favicon.ico"),
    ("GET", "/apple-touch-icon.png"),
    ("GET", "/api/v1/auth/status"),
    ("POST", "/api/v1/auth/login"),
    ("POST", "/api/v1/auth/claim"),
    ("GET", "/auth/oidc/login"),
    ("GET", "/auth/oidc/callback"),
    ("GET", "/api/v1/auth/oidc/logo"),
})
PUBLIC_PREFIXES = (("GET", "/assets/"),)
SPA_RESERVED_PREFIXES = ("api", "assets", "strava", "auth")
UNSAFE_METHODS = frozenset({"POST", "PUT", "PATCH", "DELETE"})

TRUST_GATEWAY_VARIABLE = "ACTIVSYNC_AUTH_PROXY_TRUST_GATEWAY"

UNKNOWN_HOST_MESSAGE = (
    "Unknown host — set ACTIVSYNC_PUBLIC_URL or add it to ACTIVSYNC_ALLOWED_HOSTS"
)
PUBLIC_HOST_MESSAGE = "Unknown host — ActivSync answers only on the host in ACTIVSYNC_PUBLIC_URL"

SESSION_COOKIE = "activsync_session"
SECURE_SESSION_COOKIE = "__Host-activsync_session"
DEVICE_COOKIE = "activsync_device"
SECURE_DEVICE_COOKIE = "__Host-activsync_device"


def route_path(scope: Mapping) -> str:
    """The path Starlette routes on. uvicorn prepends root_path to
    scope["path"], and a guard matching the prefixed form sees an unreserved
    first segment and waves /api/... through as a public SPA page."""
    path = scope["path"]
    root_path = scope.get("root_path", "")
    if not root_path or not path.startswith(root_path):
        return path
    if path == root_path:
        return ""
    if path[len(root_path)] == "/":
        return path[len(root_path):]
    return path


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


def device_cookie_name(scope: Mapping) -> str:
    return SECURE_DEVICE_COOKIE if is_https(scope) else DEVICE_COOKIE


_SINGLE_DOT_SEGMENTS = frozenset({".", "%2e"})
_DOUBLE_DOT_SEGMENTS = frozenset({"..", ".%2e", "%2e.", "%2e%2e"})


def _resolved_path(path: str) -> str:
    """Dot segments removed the way browsers do, including percent-encoded
    dots and a `..` at the root, which urllib and posixpath treat differently."""
    resolved: list[str] = []
    segments = path.split("/")[1:]
    for index, segment in enumerate(segments):
        last = index == len(segments) - 1
        lowered = segment.lower()
        if lowered in _DOUBLE_DOT_SEGMENTS:
            if resolved:
                resolved.pop()
            if last:
                resolved.append("")
        elif lowered in _SINGLE_DOT_SEGMENTS:
            if last:
                resolved.append("")
        else:
            resolved.append(segment)
    return "/" + "/".join(resolved)


def safe_next(value: str | None) -> str:
    if (
        not value
        or not value.startswith("/")
        or value.startswith("//")
        or "\\" in value
        or any(ord(ch) < 0x20 for ch in value)
        or _resolved_path(re.split(r"[?#]", value, maxsplit=1)[0]).startswith("//")
    ):
        return "/"
    return value


def client_ip(request: Request) -> str:
    return request.client.host if request.client else ""


def start_session(
    conn: sqlite3.Connection, request: Request, response: Response, method: str
) -> None:
    """Rotate on every login: drop whatever cookie the browser arrived with,
    then issue a fresh token. Every sign-in also trusts this browser."""
    cookie_name = session_cookie_name(request.scope)
    previous = request.cookies.get(cookie_name)
    if previous:
        auth_core.revoke_session(conn, auth_core.hash_token(previous))
    token = auth_core.create_session(
        conn,
        method=method,
        ip=client_ip(request),
        user_agent=request.headers.get("user-agent", ""),
        now=time.time(),
    )
    response.set_cookie(
        cookie_name,
        token,
        max_age=auth_core.SESSION_ABSOLUTE_SECONDS,
        path="/",
        secure=is_https(request.scope),
        httponly=True,
        samesite="lax",
    )
    issue_device_cookie(conn, request, response)


def issue_device_cookie(conn: sqlite3.Connection, request: Request, response: Response) -> None:
    cookie_name = device_cookie_name(request.scope)
    device_id = auth_devices.trust_device(
        conn, time.time(), replacing=request.cookies.get(cookie_name)
    )
    response.set_cookie(
        cookie_name,
        device_id,
        max_age=auth_devices.DEVICE_COOKIE_MAX_AGE,
        path="/",
        secure=is_https(request.scope),
        httponly=True,
        samesite="lax",
    )


def clear_session_cookie(request: Request, response: Response) -> None:
    response.delete_cookie(
        session_cookie_name(request.scope),
        path="/",
        secure=is_https(request.scope),
        httponly=True,
        samesite="lax",
    )


def login_redirect_target(request: Request) -> str:
    path = request.url.path
    if request.url.query:
        path = f"{path}?{request.url.query}"
    return f"/login?next={quote(path, safe='')}"


def external_url(request: Request, route_name: str, public_url: str | None) -> str:
    if public_url:
        return f"{public_url}{request.app.url_path_for(route_name)}"
    return str(request.url_for(route_name))


def _is_ip(value: str, kind: type[ipaddress.IPv4Address | ipaddress.IPv6Address]) -> bool:
    try:
        kind(value)
    except ValueError:
        return False
    return True


_DEFAULT_PORTS = {"http": "80", "https": "443"}
_HOST_PATTERN = re.compile(r"(?P<name>\[[^\]]*\]|[^:\[\]]*)(?::(?P<port>[^:\[\]]*))?")


def normalize_host(host: str, scheme: str | None = None) -> str:
    """Lowercase, no trailing dot on the name, and no port that is the default
    for `scheme` (either default when the scheme is unknown)."""
    host = host.strip().lower()
    match = _HOST_PATTERN.fullmatch(host)
    if match is None:
        return host
    name = match["name"].removesuffix(".")
    port = match["port"] or ""
    defaults = {_DEFAULT_PORTS[scheme]} if scheme in _DEFAULT_PORTS else _DEFAULT_PORTS.values()
    if port in defaults:
        port = ""
    return f"{name}:{port}" if port else name


def host_is_allowed(host: str | None, allowed_hosts: tuple[str, ...]) -> bool:
    """Without a public URL, a DNS-rebinding attacker's domain makes Origin
    equal Host, so the CSRF check alone would pass; only known names get in."""
    if not host:
        return False
    host = normalize_host(host)
    if host.startswith("["):
        literal, _, rest = host[1:].partition("]")
        return (rest == "" or rest.startswith(":")) and _is_ip(literal, ipaddress.IPv6Address)
    name = host.partition(":")[0]
    if _is_ip(name, ipaddress.IPv4Address):
        return True
    if name == "localhost" or name.endswith(".localhost"):
        return True
    for entry in filter(None, (normalize_host(entry) for entry in allowed_hosts)):
        if entry.startswith("."):
            if name == entry[1:] or name.endswith(entry):
                return True
        elif name == entry:
            return True
    return False


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


class PublicSchemeMiddleware:
    """An HTTPS public URL means every request the Host guard lets through
    reached the user over HTTPS, whether or not a trusted proxy said so, so the
    Secure and __Host- cookies and HSTS must not depend on X-Forwarded-Proto."""

    def __init__(self, app, *, public_url: str | None) -> None:
        self.app = app
        self.force_https = bool(public_url and urlsplit(public_url).scheme == "https")

    async def __call__(self, scope, receive, send) -> None:
        if self.force_https and scope["type"] == "http":
            scope = {**scope, "scheme": "https"}
        await self.app(scope, receive, send)


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


@dataclass(frozen=True)
class ProxyAuthConfig:
    header: str
    trusted: tuple[ipaddress.IPv4Network | ipaddress.IPv6Network, ...]
    user: str
    trusted_gateways: tuple[net_gateways.IPAddress, ...] = ()

    @classmethod
    def from_env(
        cls,
        environ: Mapping[str, str],
        *,
        gateways: Callable[[], Iterable[net_gateways.IPAddress]] | None = None,
    ) -> ProxyAuthConfig | None:
        header = environ.get("ACTIVSYNC_AUTH_PROXY_HEADER", "").strip()
        if not header:
            return None
        trusted_raw = environ.get("ACTIVSYNC_AUTH_PROXY_TRUSTED", "").strip()
        user = environ.get("ACTIVSYNC_AUTH_PROXY_USER", "").strip()
        if not trusted_raw:
            raise ValueError(
                "ACTIVSYNC_AUTH_PROXY_HEADER is set, so ACTIVSYNC_AUTH_PROXY_TRUSTED "
                "must list the proxy's address range (for example 172.18.0.0/16)."
            )
        if not user:
            raise ValueError(
                "ACTIVSYNC_AUTH_PROXY_HEADER is set, so ACTIVSYNC_AUTH_PROXY_USER "
                "must name the user the proxy signs in as."
            )
        networks = []
        for item in (part.strip() for part in trusted_raw.split(",")):
            if not item:
                continue
            try:
                network = ipaddress.ip_network(item, strict=False)
            except ValueError as exc:
                raise ValueError(
                    f"ACTIVSYNC_AUTH_PROXY_TRUSTED has an invalid address range: {item}"
                ) from exc
            if network.prefixlen == 0:
                raise ValueError(
                    f"ACTIVSYNC_AUTH_PROXY_TRUSTED must not trust every address ({item})."
                )
            networks.append(network)
        in_range = tuple(
            gateway
            for gateway in (gateways or net_gateways.read_default_gateways)()
            if any(gateway in network for network in networks)
        )
        trust_gateway = environ.get(TRUST_GATEWAY_VARIABLE, "").strip().lower() in (
            "1", "true", "yes",
        )
        if in_range and not trust_gateway:
            raise ValueError(
                "ACTIVSYNC_AUTH_PROXY_TRUSTED must not include this container's "
                f"default gateway ({', '.join(map(str, in_range))}). Docker's gateway "
                "relays connections that come from outside the Docker network, so "
                "anyone who can reach the published port could send the proxy "
                "header and be signed in as you. Trust only the reverse proxy's own "
                "address: give it a fixed IP and trust its /32. If the proxy runs on "
                f"the Docker host itself, set {TRUST_GATEWAY_VARIABLE}=1 and keep "
                "the port reachable from that host alone."
            )
        return cls(
            header=header,
            trusted=tuple(networks),
            user=user,
            trusted_gateways=in_range if trust_gateway else (),
        )

    def warn_if_trusting_gateway(self) -> None:
        if self.trusted_gateways:
            logger.warning(
                "%s is set, so %s is trusted from the default gateway (%s). "
                "Docker relays every connection to the published port from that "
                "address; port 8381 must not be reachable except from the Docker "
                "host itself (publish it as 127.0.0.1:8381:8381).",
                TRUST_GATEWAY_VARIABLE,
                self.header,
                ", ".join(map(str, self.trusted_gateways)),
            )

    def trusts(self, peer: str | None) -> bool:
        try:
            address = ipaddress.ip_address(peer or "")
        except ValueError:
            return False
        return any(address in network for network in self.trusted)

    def describe(self) -> dict:
        return {
            "header": self.header,
            "trusted": [str(network) for network in self.trusted],
            "user": self.user,
        }


class PeerCaptureMiddleware:
    """Records the socket peer before ProxyHeadersMiddleware rewrites
    scope["client"] from X-Forwarded-For, which the client controls."""

    def __init__(self, app) -> None:
        self.app = app

    async def __call__(self, scope, receive, send) -> None:
        if scope["type"] == "http":
            client = scope.get("client")
            scope.setdefault("state", {})["peer"] = client[0] if client else None
        await self.app(scope, receive, send)


class _Rejected:
    pass


REJECTED = _Rejected()


class AuthenticateMiddleware:
    def __init__(
        self,
        app,
        *,
        conn: sqlite3.Connection,
        public_url: str | None,
        allowed_hosts: tuple[str, ...],
        proxy: ProxyAuthConfig | None = None,
        clock: Callable[[], float] = time.time,
    ) -> None:
        self.app = app
        self.conn = conn
        public = urlsplit(public_url) if public_url else None
        self.public_scheme = public.scheme if public else None
        self.public_host = normalize_host(public.netloc, public.scheme) if public else None
        self.allowed_hosts = allowed_hosts
        self.proxy = proxy
        self.clock = clock
        self._last_untrusted_warning = 0.0
        self._last_unknown_host_warning = 0.0

    async def __call__(self, scope, receive, send) -> None:
        if scope["type"] != "http":
            await self.app(scope, receive, send)
            return
        request = Request(scope)
        path = route_path(scope)
        if path != "/health":
            host = request.headers.get("host")
            if not self._host_accepted(host):
                self._warn_unknown_host(host)
                message = PUBLIC_HOST_MESSAGE if self.public_host else UNKNOWN_HOST_MESSAGE
                await PlainTextResponse(message, status_code=400)(
                    scope, receive, send
                )
                return

        # A sync DB read; off the event loop so a long transaction elsewhere
        # cannot stall every open request and the SSE stream with it.
        auth = await run_in_threadpool(self._authenticate, request)
        if auth is REJECTED:
            await JSONResponse({"detail": "Not signed in"}, status_code=401)(
                scope, receive, send
            )
            return
        scope.setdefault("state", {})["auth"] = auth

        if auth is None and not is_public(scope["method"], path):
            if path.startswith("/api/"):
                response = JSONResponse({"detail": "Not signed in"}, status_code=401)
            else:
                response = RedirectResponse(login_redirect_target(request), status_code=303)
            await response(scope, receive, send)
            return
        if auth is None and scope["method"] in UNSAFE_METHODS:
            capped = await auth_body_cap.capped_receive(
                scope, receive, auth_body_cap.PUBLIC_BODY_LIMIT
            )
            if capped is None:
                await JSONResponse(
                    {"detail": auth_body_cap.TOO_LARGE}, status_code=413
                )(scope, receive, send)
                return
            receive = capped
        await self.app(scope, receive, send)

    def _host_accepted(self, host: str | None) -> bool:
        if self.public_host:
            return normalize_host(host or "", self.public_scheme) == self.public_host
        return host_is_allowed(host, self.allowed_hosts)

    def _warn_unknown_host(self, host: str | None) -> None:
        now = self.clock()
        if now - self._last_unknown_host_warning < 60:
            return
        self._last_unknown_host_warning = now
        if self.public_host:
            logger.warning(
                "rejected a request for Host %r; ACTIVSYNC_PUBLIC_URL allows only %r",
                (host or "")[:255],
                self.public_host,
            )
            return
        logger.warning(
            "rejected a request for Host %r; set ACTIVSYNC_PUBLIC_URL or add "
            "the name to ACTIVSYNC_ALLOWED_HOSTS",
            (host or "")[:255],
        )

    def _authenticate(self, request: Request) -> AuthState | _Rejected | None:
        if auth_core.auth_disabled():
            return AuthState(method="disabled")
        from_proxy = self._from_proxy(request)
        if from_proxy is not None:
            return from_proxy
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

    def _from_proxy(self, request: Request) -> AuthState | _Rejected | None:
        if self.proxy is None:
            return None
        value = request.headers.get(self.proxy.header)
        if value is None:
            return None
        peer = request.scope.get("state", {}).get("peer")
        if not self.proxy.trusts(peer):
            now = self.clock()
            if now - self._last_untrusted_warning >= 60:
                self._last_untrusted_warning = now
                logger.warning(
                    "ignored %s from untrusted peer %s; is something bypassing the proxy?",
                    self.proxy.header,
                    peer,
                )
            return None
        if value == self.proxy.user:
            return AuthState(method="proxy", user=value)
        logger.warning(
            "proxy at %s asserted %s=%r, which is not the configured owner",
            peer,
            self.proxy.header,
            value,
        )
        return REJECTED


def own_origin(scope: Mapping, headers: Mapping[str, str]) -> str:
    return f"{scope.get('scheme', 'http')}://{headers.get('host', '')}"


def _origin_of(url: str) -> str:
    try:
        parts = urlsplit(url)
    except ValueError:
        return url
    return f"{parts.scheme}://{normalize_host(parts.netloc, parts.scheme)}"


def csrf_problem(headers: Mapping[str, str], expected_origin: str) -> str | None:
    expected_origin = _origin_of(expected_origin)
    if headers.get("x-activsync-request") != "1":
        return "missing X-ActivSync-Request header"
    origin = headers.get("origin")
    if origin == "null":
        return "opaque Origin"
    site = headers.get("sec-fetch-site")
    if site is not None:
        return None if site == "same-origin" else f"Sec-Fetch-Site is {site}"
    if origin:
        if _origin_of(origin) == expected_origin:
            return None
        return f"Origin {origin} does not match"
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
                    "blocked %s %s: %s", scope["method"], route_path(scope), problem
                )
                await JSONResponse(
                    {"detail": "This request was blocked as cross-site."},
                    status_code=403,
                )(scope, receive, send)
                return
        await self.app(scope, receive, send)
