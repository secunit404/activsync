"""Break-glass sign-in recovery: `docker compose exec activsync activsync-auth <cmd>`.

Shell access to the container already means full access to the database, so
these commands ask for nothing beyond that. They are safe to run while the
server is up: the connection takes the same lock and WAL the server uses.
"""

from __future__ import annotations

import argparse
import getpass
import os
import sqlite3
import sys
import time

from activsync import auth_core, auth_devices, auth_middleware, auth_oidc, db


def _reset_password(conn: sqlite3.Connection) -> int:
    password = getpass.getpass("New password: ")
    problem = auth_core.password_problem(password)
    if problem:
        print(problem, file=sys.stderr)
        return 1
    if getpass.getpass("Repeat it: ") != password:
        print("The two passwords did not match. Nothing changed.", file=sys.stderr)
        return 1
    auth_core.set_password(conn, password, time.time())
    auth_core.reset_throttle(conn)
    auth_core.revoke_all(conn)
    auth_devices.clear_trusted_devices(conn)
    print("Password reset. Every session was signed out.")
    return 0


def _revoke_sessions(conn: sqlite3.Connection) -> int:
    revoked = auth_core.revoke_all(conn)
    auth_devices.clear_trusted_devices(conn)
    print(f"Signed out {revoked} session(s).")
    return 0


def _disable_oidc(conn: sqlite3.Connection) -> int:
    auth_oidc.delete_config(conn)
    print("Single sign-on removed. Sign in with your password.")
    return 0


def _describe_oidc(oidc: auth_oidc.OidcConfig | None) -> str:
    if oidc is None:
        return "off"
    linked = "linked" if oidc.linked else "not linked"
    return f"{oidc.display_name} ({oidc.issuer}), {linked}"


def _describe_proxy(proxy: auth_middleware.ProxyAuthConfig | None) -> str:
    if proxy is None:
        return "off"
    return f"{proxy.header} from {', '.join(proxy.describe()['trusted'])}"


def _status(conn: sqlite3.Connection) -> int:
    lines = [
        f"Owner claimed:        {'yes' if auth_core.owner_claimed(conn) else 'no'}",
        f"Authentication:       {'DISABLED' if auth_core.auth_disabled() else 'on'}",
        f"Single sign-on:       {_describe_oidc(auth_oidc.load_config(conn))}",
        "Proxy header:         "
        f"{_describe_proxy(auth_middleware.ProxyAuthConfig.from_env(os.environ))}",
        f"Active sessions:      {len(auth_core.list_sessions(conn))}",
    ]
    print("\n".join(lines))
    return 0


COMMANDS = {
    "reset-password": (_reset_password, "Set a new password and sign out every session."),
    "revoke-sessions": (_revoke_sessions, "Sign out every session and forget trusted browsers."),
    "disable-oidc": (_disable_oidc, "Remove the single sign-on configuration."),
    "status": (_status, "Show which sign-in methods are on. Never prints secrets."),
}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="activsync-auth", description="Recover sign-in access to ActivSync."
    )
    commands = parser.add_subparsers(dest="command", required=True)
    for name, (_, help_text) in COMMANDS.items():
        commands.add_parser(name, help=help_text)
    args = parser.parse_args(argv)
    conn = db.connect(db.resolve_path(mock_mode=False))
    try:
        return COMMANDS[args.command][0](conn)
    finally:
        conn.close()


if __name__ == "__main__":
    raise SystemExit(main())
