import io
import struct
import zlib

import pytest
from fastapi.testclient import TestClient
from PIL import Image, PngImagePlugin

from activsync import auth_oidc, auth_oidc_logo, db
from activsync.server import create_app
from auth_helpers import CSRF_HEADERS, OWNER_PASSWORD, owner_client

LOGO_URL = "/api/v1/auth/oidc/logo"
CONFIG = {
    "displayName": "Pocket ID",
    "issuer": "https://id.example.com",
    "clientId": "activsync",
    "clientSecret": "very-secret-value",
    "scopes": "openid profile email",
    "autoRedirect": False,
    "currentPassword": OWNER_PASSWORD,
}


@pytest.fixture
def conn(tmp_path):
    return db.connect(str(tmp_path / "logo.db"))


@pytest.fixture
def owner(conn):
    client = owner_client(create_app(conn))
    assert client.put("/api/v1/auth/oidc", json=CONFIG).status_code == 200
    return client


def _image(fmt: str, size=(400, 200), mode="RGBA", **save_args) -> bytes:
    image = Image.new(mode, size, "red" if mode != "1" else 1)
    if fmt == "JPEG" and image.mode == "RGBA":
        image = image.convert("RGB")
    out = io.BytesIO()
    image.save(out, format=fmt, **save_args)
    return out.getvalue()


def _upload(client, body: bytes, content_type="image/png"):
    return client.put(LOGO_URL, content=body, headers={"Content-Type": content_type})


def _served(client) -> Image.Image:
    response = client.get(LOGO_URL)
    assert response.status_code == 200
    return Image.open(io.BytesIO(response.content))


@pytest.mark.parametrize("fmt", ["PNG", "JPEG", "WEBP"])
def test_each_supported_format_is_stored_as_a_96px_png(owner, fmt):
    response = _upload(owner, _image(fmt), content_type="application/octet-stream")

    assert response.status_code == 200
    served = _served(owner)
    assert served.format == "PNG"
    assert served.size == (192, 96)


def test_the_type_comes_from_the_bytes_not_the_label(owner):
    svg = b'<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>'

    refused = _upload(owner, svg, content_type="image/png")
    accepted = _upload(owner, _image("PNG"), content_type="image/svg+xml")

    assert refused.status_code == 400
    assert refused.json()["detail"] == auth_oidc_logo.UNSUPPORTED
    assert accepted.status_code == 200


def test_gif_is_refused(owner, conn):
    response = _upload(owner, _image("GIF", mode="P"), content_type="image/gif")

    assert response.status_code == 400
    assert auth_oidc_logo.load(conn) is None


def test_the_upload_is_reencoded_without_its_metadata(owner):
    info = PngImagePlugin.PngInfo()
    info.add_text("Comment", "smuggled-payload")

    _upload(owner, _image("PNG", pnginfo=info))

    assert b"smuggled-payload" not in owner.get(LOGO_URL).content


def test_jpeg_orientation_is_applied(owner):
    exif = Image.Exif()
    exif[0x0112] = 6

    _upload(owner, _image("JPEG", size=(200, 100), exif=exif))

    assert _served(owner).size == (48, 96)


def test_a_wide_logo_is_capped_at_four_to_one(owner):
    _upload(owner, _image("PNG", size=(1000, 100)))

    assert _served(owner).size == (384, 38)


def test_a_small_logo_is_not_upscaled(owner):
    _upload(owner, _image("PNG", size=(32, 32)))

    assert _served(owner).size == (32, 32)


def test_an_oversized_body_is_refused_with_413(owner, conn):
    body = _image("PNG") + b"\0" * auth_oidc_logo.MAX_UPLOAD_BYTES

    response = _upload(owner, body)

    assert response.status_code == 413
    assert auth_oidc_logo.load(conn) is None


def _chunk(kind: bytes, data: bytes) -> bytes:
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))


def _png_header_only(width: int, height: int) -> bytes:
    """A PNG whose header claims `width`×`height` but carries no pixel data."""
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + _chunk(b"IHDR", ihdr) + _chunk(b"IDAT", b"")


def test_too_many_pixels_is_refused_before_decoding(owner, conn):
    response = _upload(owner, _png_header_only(5000, 5000))

    assert response.status_code == 400
    assert response.json()["detail"] == auth_oidc_logo.TOO_MANY_PIXELS
    assert auth_oidc_logo.load(conn) is None


def test_a_corrupt_image_is_refused(owner):
    response = _upload(owner, _png_header_only(100, 100) + b"garbage")

    assert response.status_code == 400
    assert response.json()["detail"] == auth_oidc_logo.UNREADABLE


def test_the_logo_is_served_publicly_with_locked_down_headers(owner, conn):
    _upload(owner, _image("PNG"))
    anonymous = TestClient(owner.app)

    response = anonymous.get(LOGO_URL)

    assert response.status_code == 200
    assert response.headers["content-type"] == "image/png"
    assert response.headers["x-content-type-options"] == "nosniff"
    assert response.headers["content-security-policy"] == "default-src 'none'; sandbox"
    assert response.headers["cache-control"] == "public, max-age=31536000, immutable"


def test_no_logo_is_a_404_even_when_signed_out(conn):
    assert TestClient(create_app(conn)).get(LOGO_URL).status_code == 404


def test_uploading_needs_a_session(owner, conn):
    anonymous = TestClient(owner.app, headers=CSRF_HEADERS)

    response = _upload(anonymous, _image("PNG"))

    assert response.status_code == 401
    assert auth_oidc_logo.load(conn) is None


def test_uploading_needs_single_sign_on_first(conn):
    client = owner_client(create_app(conn))

    response = _upload(client, _image("PNG"))

    assert response.status_code == 409


def test_status_and_settings_carry_a_versioned_logo_url(owner, conn):
    auth_oidc.set_linked(conn, iss="https://id.example.com", sub="owner-sub", label="andreas")
    _upload(owner, _image("PNG"))
    logo = auth_oidc_logo.load(conn)

    status = TestClient(owner.app).get("/api/v1/auth/status").json()

    expected = f"{LOGO_URL}?v={logo.sha256[:12]}"
    assert status["login"]["oidc"]["logoUrl"] == expected
    assert owner.get("/api/v1/auth/oidc").json()["logoUrl"] == expected


def test_removing_the_logo_restores_the_key(owner, conn):
    _upload(owner, _image("PNG"))

    assert owner.delete(LOGO_URL).status_code == 200

    assert owner.get(LOGO_URL).status_code == 404
    assert owner.get("/api/v1/auth/oidc").json()["logoUrl"] is None


def test_removing_single_sign_on_removes_the_logo(owner, conn):
    _upload(owner, _image("PNG"))

    owner.request("DELETE", "/api/v1/auth/oidc", json={"currentPassword": OWNER_PASSWORD})

    assert auth_oidc_logo.load(conn) is None
