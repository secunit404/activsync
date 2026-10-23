"""An in-process OIDC provider: discovery, JWKS, authorize, token, userinfo.

Served through an httpx2.MockTransport handed to Authlib, so the real
client code runs end to end without a network.
"""

import base64
import hashlib
import secrets
import time
from urllib.parse import parse_qs, urlsplit

import httpx2
from joserfc import jwt
from joserfc.jwk import ECKey, KeySet, RSAKey

ISSUER = "https://idp.test"
CLIENT_ID = "activsync"
CLIENT_SECRET = "idp-secret"


def unreachable_transport() -> httpx2.MockTransport:
    def refuse(request: httpx2.Request) -> httpx2.Response:
        raise httpx2.ConnectError("connection refused", request=request)

    return httpx2.MockTransport(refuse)


class FakeIdp:
    def __init__(self, *, alg: str = "RS256", sub: str = "owner-sub") -> None:
        self.alg = alg
        self.sub = sub
        self.audience = CLIENT_ID
        self.issuer_claim = ISSUER
        self.nonce_override: str | None = None
        self.azp: str | None = None
        if alg == "ES256":
            self.key = ECKey.generate_key("P-256", parameters={"kid": "ec-1"})
        else:
            self.key = RSAKey.generate_key(2048, parameters={"kid": "rsa-1"})
        self.codes: dict[str, dict] = {}
        self.token_requests: list[dict] = []
        self.transport = httpx2.MockTransport(self._handle)

    def authorize(self, authorize_url: str) -> dict:
        """Play the user approving: returns the callback query parameters."""
        params = {key: values[0] for key, values in parse_qs(urlsplit(authorize_url).query).items()}
        code = secrets.token_urlsafe(16)
        self.codes[code] = params
        return {"code": code, "state": params["state"]}

    def _handle(self, request: httpx2.Request) -> httpx2.Response:
        path = request.url.path
        if path == "/.well-known/openid-configuration":
            return httpx2.Response(200, json={
                "issuer": ISSUER,
                "authorization_endpoint": f"{ISSUER}/authorize",
                "token_endpoint": f"{ISSUER}/token",
                "userinfo_endpoint": f"{ISSUER}/userinfo",
                "jwks_uri": f"{ISSUER}/jwks",
                "id_token_signing_alg_values_supported": [self.alg],
                "response_types_supported": ["code"],
            })
        if path == "/jwks":
            return httpx2.Response(200, json=KeySet([self.key]).as_dict(private=False))
        if path == "/token":
            return self._token(request)
        if path == "/userinfo":
            return httpx2.Response(200, json={"sub": self.sub, "preferred_username": "andreas"})
        return httpx2.Response(404)

    def _token(self, request: httpx2.Request) -> httpx2.Response:
        form = {key: values[0] for key, values in parse_qs(request.content.decode()).items()}
        self.token_requests.append(form)
        issued = self.codes.pop(form.get("code"), None)
        if issued is None:
            return httpx2.Response(400, json={"error": "invalid_grant"})
        verifier = form.get("code_verifier", "")
        challenge = base64.urlsafe_b64encode(
            hashlib.sha256(verifier.encode()).digest()
        ).rstrip(b"=").decode()
        if challenge != issued.get("code_challenge"):
            return httpx2.Response(400, json={"error": "invalid_grant"})
        now = int(time.time())
        id_token = jwt.encode(
            {"alg": self.alg, "kid": self.key.kid},
            {
                "iss": self.issuer_claim,
                "sub": self.sub,
                "aud": self.audience,
                "iat": now,
                "exp": now + 300,
                "nonce": self.nonce_override or issued.get("nonce"),
                **({"azp": self.azp} if self.azp else {}),
            },
            self.key,
        )
        return httpx2.Response(200, json={
            "access_token": "idp-access-token",
            "token_type": "Bearer",
            "expires_in": 300,
            "id_token": id_token,
        })
