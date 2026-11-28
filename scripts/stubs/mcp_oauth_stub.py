#!/usr/bin/env python3
"""A local stand-in OAuth authorization server for
scripts/tests/test_mcp_phase4.sh.

Not a simulator for production use - just enough of the
surface the MCP OAuth gate consumes, so the Phase 4 suite
runs without a real Auth0 tenant:

  GET  /.well-known/jwks.json   the stub's public key set
                                (one RSA key, generated at
                                startup, kid mcp-oauth-stub-1)
  POST /__mint                  JSON {sub, scope, aud,
                                lifetime_secs, sign_with}
                                -> a compact RS256 access
                                token as text/plain. The iss
                                claim is this stub's own
                                origin, taken from the Host
                                header of the mint request,
                                so it always matches the
                                issuer the suite configured.
                                sign_with "rogue" signs with
                                a second keypair that is
                                never published in the JWKS,
                                for the wrong-key check;
                                a negative lifetime_secs
                                mints an already-expired
                                token.

The port comes from MCP_OAUTH_STUB_PORT (default 18103).
"""

import base64
import json
import os
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import padding, rsa

KID = "mcp-oauth-stub-1"

MAIN_KEY = rsa.generate_private_key(public_exponent=65537, key_size=2048)
ROGUE_KEY = rsa.generate_private_key(public_exponent=65537, key_size=2048)


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def b64url_uint(value: int) -> str:
    raw = value.to_bytes((value.bit_length() + 7) // 8, "big")
    return b64url(raw)


def sign(key, claims: dict) -> str:
    header = {"alg": "RS256", "typ": "JWT", "kid": KID}
    signing_input = (
        b64url(json.dumps(header, separators=(",", ":")).encode())
        + "."
        + b64url(json.dumps(claims, separators=(",", ":")).encode())
    )
    signature = key.sign(
        signing_input.encode("ascii"),
        padding.PKCS1v15(),
        hashes.SHA256(),
    )
    return signing_input + "." + b64url(signature)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):  # keep the suite output clean
        pass

    def do_GET(self):
        if self.path == "/.well-known/jwks.json":
            numbers = MAIN_KEY.public_key().public_numbers()
            body = json.dumps(
                {
                    "keys": [
                        {
                            "kty": "RSA",
                            "kid": KID,
                            "use": "sig",
                            "alg": "RS256",
                            "n": b64url_uint(numbers.n),
                            "e": b64url_uint(numbers.e),
                        }
                    ]
                }
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        if self.path != "/__mint":
            self.send_response(404)
            self.end_headers()
            return
        length = int(self.headers.get("Content-Length") or 0)
        request = json.loads(self.rfile.read(length) or b"{}")
        now = int(time.time())
        lifetime = int(request.get("lifetime_secs", 600))
        claims = {
            "iss": f"http://{self.headers.get('Host')}",
            "sub": request.get("sub", "stub-subject"),
            "aud": request.get("aud", "test-audience"),
            "scope": request.get("scope", ""),
            "iat": now,
            "exp": now + lifetime,
        }
        key = ROGUE_KEY if request.get("sign_with") == "rogue" else MAIN_KEY
        body = sign(key, claims).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    port = int(os.environ.get("MCP_OAUTH_STUB_PORT", "18103"))
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
