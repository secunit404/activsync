"""The optional logo on the single sign-on button. Uploads are decoded and
re-encoded to a small PNG, so nothing the uploader wrote is ever served back."""

from __future__ import annotations

import hashlib
import io
import sqlite3
import warnings
from dataclasses import dataclass

from PIL import Image, ImageOps

from activsync import db

MAX_UPLOAD_BYTES = 1024 * 1024
MAX_PIXELS = 4096 * 4096
LOGO_HEIGHT = 96
LOGO_MAX_WIDTH = 4 * LOGO_HEIGHT

# Never the filename or Content-Type: both are the uploader's claims. SVG is
# left out because it can carry script.
_FORMATS = {"PNG": b"\x89PNG\r\n\x1a\n", "JPEG": b"\xff\xd8\xff"}

UNSUPPORTED = "Use a PNG, JPEG or WebP image."
UNREADABLE = "That image could not be read. Try exporting it again as PNG."
TOO_LARGE = "That image is too large. Use one under 1 MB."
TOO_MANY_PIXELS = "That image has too many pixels. Use one under 4096×4096."


class LogoError(ValueError):
    pass


@dataclass(frozen=True)
class Logo:
    png: bytes
    sha256: str


def sniff_format(data: bytes) -> str | None:
    for name, signature in _FORMATS.items():
        if data.startswith(signature):
            return name
    if data[:4] == b"RIFF" and data[8:12] == b"WEBP":
        return "WEBP"
    return None


def normalize(data: bytes) -> bytes:
    """Return the logo as a PNG at most LOGO_HEIGHT tall, or raise LogoError."""
    if len(data) > MAX_UPLOAD_BYTES:
        raise LogoError(TOO_LARGE)
    kind = sniff_format(data)
    if kind is None:
        raise LogoError(UNSUPPORTED)
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("error", Image.DecompressionBombWarning)
            with Image.open(io.BytesIO(data), formats=[kind]) as image:
                width, height = image.size
                if width * height > MAX_PIXELS:
                    raise LogoError(TOO_MANY_PIXELS)
                image.load()
                upright = ImageOps.exif_transpose(image).convert("RGBA")
    except LogoError:
        raise
    except (
        OSError,
        ValueError,
        SyntaxError,
        EOFError,
        Image.DecompressionBombError,
        Image.DecompressionBombWarning,
    ) as exc:
        raise LogoError(UNREADABLE) from exc
    upright.thumbnail((LOGO_MAX_WIDTH, LOGO_HEIGHT), Image.Resampling.LANCZOS)
    out = io.BytesIO()
    upright.save(out, format="PNG", optimize=True)
    return out.getvalue()


def load(conn: sqlite3.Connection) -> Logo | None:
    row = conn.execute("SELECT png, sha256 FROM auth_oidc_logo WHERE id = 1").fetchone()
    return Logo(png=bytes(row["png"]), sha256=row["sha256"]) if row else None


def current_url(conn: sqlite3.Connection) -> str | None:
    row = conn.execute("SELECT sha256 FROM auth_oidc_logo WHERE id = 1").fetchone()
    return f"/api/v1/auth/oidc/logo?v={row['sha256'][:12]}" if row else None


def save(conn: sqlite3.Connection, png: bytes, now: float) -> Logo:
    logo = Logo(png=png, sha256=hashlib.sha256(png).hexdigest())
    with db.transaction(conn):
        conn.execute(
            "INSERT INTO auth_oidc_logo (id, png, sha256, updated_at) VALUES (1, ?, ?, ?) "
            "ON CONFLICT(id) DO UPDATE SET png = excluded.png, "
            "sha256 = excluded.sha256, updated_at = excluded.updated_at",
            (png, logo.sha256, int(now)),
        )
    return logo


def delete(conn: sqlite3.Connection) -> None:
    with db.transaction(conn):
        conn.execute("DELETE FROM auth_oidc_logo")
