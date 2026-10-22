"""OIDC sign-in: an Authlib client built from the stored `auth_oidc` config,
and the link/login callback logic that pins one (iss, sub) as the owner."""

from __future__ import annotations

import dataclasses
import logging
import sqlite3
import threading
from dataclasses import dataclass

import httpx2
from authlib.integrations.starlette_client import OAuth, OAuthError
from authlib.integrations.base_client import MismatchingStateError
from joserfc.errors import JoseError
from starlette.requests import Request
from starlette.responses import RedirectResponse

from activsync import auth_oidc_logo, db

logger = logging.getLogger("activsync.auth.oidc")

OIDC_KEY = "auth_oidc"
DEFAULT_SCOPES = "openid profile email"
FLOW_KEY = "oidc_flow"


@dataclass(frozen=True)
class OidcConfig:
    display_name: str
    issuer: str
    client_id: str
    client_secret: str
    scopes: str
    auto_redirect: bool
    linked: dict | None


def load_config(conn: sqlite3.Connection) -> OidcConfig | None:
    stored = db.get_config_value(conn, OIDC_KEY)
    if not stored:
        return None
    return OidcConfig(
        display_name=stored["display_name"],
        issuer=stored["issuer"],
        client_id=stored["client_id"],
        client_secret=stored.get("client_secret", ""),
        scopes=stored.get("scopes", DEFAULT_SCOPES),
        auto_redirect=bool(stored.get("auto_redirect")),
        linked=stored.get("linked"),
    )


def _normalize_issuer(issuer: str) -> str:
    value = issuer.strip().rstrip("/")
    if not value.startswith(("https://", "http://")):
        raise ValueError("The issuer URL must start with https://")
    return value


def _store(conn: sqlite3.Connection, config: OidcConfig) -> None:
    db.set_config_value(conn, OIDC_KEY, {
        "display_name": config.display_name,
        "issuer": config.issuer,
        "client_id": config.client_id,
        "client_secret": config.client_secret,
        "scopes": config.scopes,
        "auto_redirect": config.auto_redirect,
        "linked": config.linked,
    })


def save_config(
    conn: sqlite3.Connection,
    *,
    display_name: str,
    issuer: str,
    client_id: str,
    client_secret: str | None,
    scopes: str,
    auto_redirect: bool,
) -> OidcConfig:
    """`client_secret=None` keeps the stored secret. A new issuer or client ID
    is a different identity provider, so the link is dropped."""
    normalized = _normalize_issuer(issuer)
    with db.transaction(conn):
        existing = load_config(conn)
        same_provider = (
            existing is not None
            and existing.issuer == normalized
            and existing.client_id == client_id
        )
        config = OidcConfig(
            display_name=display_name,
            issuer=normalized,
            client_id=client_id,
            client_secret=(
                client_secret
                if client_secret is not None
                else (existing.client_secret if existing else "")
            ),
            scopes=" ".join(scopes.split()) or DEFAULT_SCOPES,
            auto_redirect=auto_redirect,
            linked=existing.linked if same_provider else None,
        )
        _store(conn, config)
    return config


def set_linked(conn: sqlite3.Connection, *, iss: str, sub: str, label: str) -> None:
    with db.transaction(conn):
        config = load_config(conn)
        if config is None:
            return
        _store(conn, dataclasses.replace(
            config, linked={"iss": iss, "sub": sub, "label": label}
        ))


def delete_config(conn: sqlite3.Connection) -> None:
    auth_oidc_logo.delete(conn)
    db.set_config_value(conn, OIDC_KEY, None)


def discovery_url(issuer: str) -> str:
    return f"{issuer}/.well-known/openid-configuration"


def _scope_list(scopes: str) -> str:
    tokens = scopes.split()
    return " ".join(tokens if "openid" in tokens else ["openid", *tokens])


class OidcError(Exception):
    def __init__(self, code: str, intent: str = "login") -> None:
        super().__init__(code)
        self.code = code
        self.intent = intent


@dataclass(frozen=True)
class OidcResult:
    intent: str
    next_path: str
    iss: str
    sub: str
    label: str


class OidcProvider:
    """Keeps one Authlib client per configuration, so discovery and the JWKS
    are fetched once and refetched only on an unknown `kid`."""

    def __init__(
        self,
        conn: sqlite3.Connection,
        *,
        transport: httpx2.AsyncBaseTransport | None = None,
    ) -> None:
        self._conn = conn
        self._transport = transport
        self._lock = threading.Lock()
        self._key: tuple | None = None
        self._client = None

    def client(self):
        config = load_config(self._conn)
        if config is None:
            return None, None
        key = (config.issuer, config.client_id, config.client_secret, config.scopes)
        with self._lock:
            if key != self._key:
                client_kwargs = {
                    "scope": _scope_list(config.scopes),
                    "code_challenge_method": "S256",
                    "timeout": 10,
                }
                if self._transport is not None:
                    client_kwargs["transport"] = self._transport
                self._client = OAuth().register(
                    "oidc",
                    client_id=config.client_id,
                    client_secret=config.client_secret or None,
                    server_metadata_url=discovery_url(config.issuer),
                    client_kwargs=client_kwargs,
                )
                self._key = key
            return config, self._client

    async def begin(
        self, request: Request, *, redirect_uri: str, intent: str, next_path: str
    ) -> RedirectResponse:
        config, client = self.client()
        if client is None:
            raise OidcError("oidc_unavailable", intent)
        request.session[FLOW_KEY] = {"intent": intent, "next": next_path}
        try:
            return await client.authorize_redirect(request, redirect_uri)
        except (httpx2.HTTPError, OAuthError, ValueError) as exc:
            logger.warning("could not start OIDC sign-in with %s: %s", config.issuer, exc)
            raise OidcError("oidc_unavailable", intent) from exc

    async def finish(self, request: Request) -> OidcResult:
        flow = request.session.pop(FLOW_KEY, None)
        intent = flow["intent"] if flow else "login"
        config, client = self.client()
        if client is None:
            raise OidcError("oidc_unavailable", intent)
        if flow is None:
            raise OidcError("oidc_state", intent)
        if request.query_params.get("error") == "access_denied":
            raise OidcError("oidc_denied", intent)
        try:
            metadata = await client.load_server_metadata()
            # Authlib only checks `aud` through `azp`, so a token minted for
            # another client that names this one as azp would pass; require
            # this client in `aud` outright, as OIDC Core 3.1.3.7 does.
            token = await client.authorize_access_token(
                request,
                claims_options={
                    "iss": {"essential": True, "values": [metadata["issuer"]]},
                    "aud": {"essential": True, "values": [config.client_id]},
                },
            )
        except MismatchingStateError as exc:
            raise OidcError("oidc_state", intent) from exc
        except JoseError as exc:
            logger.warning("rejected an OIDC ID token from %s: %s", config.issuer, exc)
            raise OidcError("oidc_state", intent) from exc
        except OAuthError as exc:
            logger.warning("OIDC sign-in with %s failed: %s", config.issuer, exc)
            code = "oidc_denied" if exc.error == "access_denied" else "oidc_unavailable"
            raise OidcError(code, intent) from exc
        except (httpx2.HTTPError, ValueError) as exc:
            logger.warning("OIDC provider %s is unreachable: %s", config.issuer, exc)
            raise OidcError("oidc_unavailable", intent) from exc

        claims = token.get("userinfo")
        if not claims or not claims.get("sub") or not claims.get("iss"):
            logger.warning("OIDC provider %s returned no ID token", config.issuer)
            raise OidcError("oidc_unavailable", intent)
        return OidcResult(
            intent=intent,
            next_path=flow["next"],
            iss=claims["iss"],
            sub=claims["sub"],
            label=await self._label(client, token, claims),
        )

    async def _label(self, client, token: dict, claims: dict) -> str:
        """Display only. Authelia leaves email out of the ID token by default,
        so ask the userinfo endpoint and tolerate it failing."""
        try:
            info = await client.userinfo(token=token)
        except Exception as exc:
            logger.info("OIDC userinfo lookup failed: %s", exc)
            info = {}
        for source in (info, claims):
            for key in ("preferred_username", "email", "name"):
                if source.get(key):
                    return str(source[key])
        return str(claims["sub"])
