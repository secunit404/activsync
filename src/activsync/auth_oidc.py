"""OIDC sign-in: an Authlib client built from the stored `auth_oidc` config,
and the link/login callback logic that pins one (iss, sub) as the owner."""

from __future__ import annotations

import dataclasses
import sqlite3
from dataclasses import dataclass

from activsync import db

OIDC_KEY = "auth_oidc"
DEFAULT_SCOPES = "openid profile email"


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
    db.set_config_value(conn, OIDC_KEY, None)
