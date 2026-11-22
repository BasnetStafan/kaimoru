#!/usr/bin/env python3
"""A local stand-in OIDC provider for scripts/tests/test_sns.sh.

Not a simulator for production use - just enough of the
OpenID Connect surface for the SNS suite to run without real
LINE / Google / Yahoo credentials:

  GET  <provider authorize paths>  records the request (state,
                                   nonce, PKCE challenge), then
                                   303s to the redirect_uri with
                                   a fresh code and the state
  POST <provider token paths>      verifies the PKCE verifier
                                   against the recorded
                                   challenge and returns a
                                   signed RS256 ID token
  GET  <provider JWKS paths>       the stub's public key set
  GET  /__scenario/<name>          sets the scenario cookie on
                                   the caller's jar: which
                                   claims the next tokens carry
  GET  /__last                     JSON of the last authorize
                                   request, for assertions

One RSA key (openssl, generated at startup in a temp dir)
signs every token. The issuer is chosen by the token path
that was hit, matching what the app expects per provider;
the audience is the client_id of the token request. Claims
per scenario:

  reg        sub stub-sub-reg, email sns-reg@example.com,
             verified, a display name
  reg2       sub stub-sub-reg2, email sns-reg2@example.com,
             verified (the second-provider registration)
  collision  sub stub-sub-collision, email of the suite's
             password member (sns-member@example.com), verified
  totplink   sub stub-sub-totp, the TOTP fixture's email,
             verified (linked from My Page in the suite)
  noemail    sub stub-sub-noemail, NO email claim at all
  unvemail   sub stub-sub-unv, email sns-unv@example.com,
             email_verified false
  dead       sub stub-sub-dead, email sns-dead@example.com,
             verified (the withdrawn-account fixture)

Environment: SNS_STUB_PORT (default 18098).
"""

import base64
import hashlib
import json
import os
import subprocess
import tempfile
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

PORT = int(os.environ.get("SNS_STUB_PORT") or 18098)
KID = "sns-stub-key-1"

ISSUERS = {
    "/oauth2/v2.1/token": "https://access.line.me",
    "/token": "https://accounts.google.com",
    "/yconnect/v2/token": "https://auth.login.yahoo.co.jp",
}
AUTHORIZE_PATHS = {"/oauth2/v2.1/authorize", "/o/oauth2/v2/auth", "/yconnect/v2/authorization"}
TOKEN_PATHS = set(ISSUERS)
JWKS_PATHS = {"/oauth2/v2.1/certs", "/oauth2/v3/certs", "/yconnect/v2/public-keys"}

SCENARIOS = {
    "reg": {"sub": "stub-sub-reg", "email": "sns-reg@example.com",
            "email_verified": True, "name": "SNS 登録"},
    "reg2": {"sub": "stub-sub-reg2", "email": "sns-reg2@example.com",
             "email_verified": True, "name": "SNS 登録2"},
    "collision": {"sub": "stub-sub-collision", "email": "sns-member@example.com",
                  "email_verified": True, "name": "SNS 衝突"},
    "totplink": {"sub": "stub-sub-totp", "email": "sns-totp@example.com",
                 "email_verified": True, "name": "SNS 二段階"},
    "noemail": {"sub": "stub-sub-noemail", "name": "SNS 無メール"},
    "unvemail": {"sub": "stub-sub-unv", "email": "sns-unv@example.com",
                 "email_verified": False, "name": "SNS 未確認"},
    "dead": {"sub": "stub-sub-dead", "email": "sns-dead@example.com",
             "email_verified": True, "name": "SNS 退会済"},
}

# ---------------------------------------------------------------- key setup

_TMP = tempfile.mkdtemp(prefix="sns-stub-")
_KEY = os.path.join(_TMP, "key.pem")
subprocess.run(
    ["openssl", "genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048",
     "-out", _KEY],
    check=True, capture_output=True,
)
_modulus_hex = subprocess.run(
    ["openssl", "rsa", "-in", _KEY, "-modulus", "-noout"],
    check=True, capture_output=True, text=True,
).stdout.strip().split("=", 1)[1]
_text = subprocess.run(
    ["openssl", "rsa", "-in", _KEY, "-text", "-noout"],
    check=True, capture_output=True, text=True,
).stdout
_exponent = int(next(l for l in _text.splitlines() if "Exponent:" in l).split()[1])


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


JWKS = {"keys": [{
    "kty": "RSA", "kid": KID, "use": "sig", "alg": "RS256",
    "n": b64url(bytes.fromhex(_modulus_hex)),
    "e": b64url(_exponent.to_bytes((_exponent.bit_length() + 7) // 8, "big")),
}]}


def sign_rs256(signing_input: bytes) -> bytes:
    proc = subprocess.run(
        ["openssl", "dgst", "-sha256", "-sign", _KEY],
        input=signing_input, capture_output=True, check=True,
    )
    return proc.stdout


# ---------------------------------------------------------------- state

_codes = {}          # code -> {nonce, challenge, client_id, scenario}
_last_authorize = {} # most recent authorize request params
_counter = [0]


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):  # keep the suite output clean
        pass

    def _send_json(self, obj, status=200):
        body = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _scenario(self):
        cookie = self.headers.get("Cookie") or ""
        for part in cookie.split(";"):
            part = part.strip()
            if part.startswith("sns_scenario="):
                name = part.split("=", 1)[1]
                if name in SCENARIOS:
                    return name
        return "reg"

    def do_GET(self):
        url = urlparse(self.path)
        path = url.path
        if path.startswith("/__scenario/"):
            name = path.rsplit("/", 1)[1]
            body = b"ok"
            self.send_response(200)
            self.send_header("Set-Cookie", f"sns_scenario={name}; Path=/")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if path == "/__last":
            self._send_json(_last_authorize)
            return
        if path in JWKS_PATHS:
            self._send_json(JWKS)
            return
        if path in AUTHORIZE_PATHS:
            params = {k: v[0] for k, v in parse_qs(url.query).items()}
            _last_authorize.clear()
            _last_authorize.update(params)
            _counter[0] += 1
            code = f"stubcode-{_counter[0]}"
            _codes[code] = {
                "nonce": params.get("nonce", ""),
                "challenge": params.get("code_challenge", ""),
                "client_id": params.get("client_id", ""),
                "scenario": self._scenario(),
            }
            location = (
                f"{params['redirect_uri']}?code={code}"
                f"&state={params.get('state', '')}"
            )
            self.send_response(303)
            self.send_header("Location", location)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self._send_json({"error": "not_found"}, 404)

    def do_POST(self):
        url = urlparse(self.path)
        path = url.path
        if path not in TOKEN_PATHS:
            self._send_json({"error": "not_found"}, 404)
            return
        length = int(self.headers.get("Content-Length") or 0)
        form = {k: v[0] for k, v in
                parse_qs(self.rfile.read(length).decode()).items()}
        record = _codes.pop(form.get("code", ""), None)
        if record is None:
            self._send_json({"error": "invalid_grant"}, 400)
            return
        verifier = form.get("code_verifier", "")
        digest = hashlib.sha256(verifier.encode()).digest()
        if b64url(digest) != record["challenge"]:
            self._send_json({"error": "invalid_grant"}, 400)
            return
        claims = dict(SCENARIOS[record["scenario"]])
        now = int(time.time())
        claims.update({
            "iss": ISSUERS[path],
            "aud": form.get("client_id", record["client_id"]),
            "nonce": record["nonce"],
            "iat": now,
            "exp": now + 600,
        })
        header = {"alg": "RS256", "kid": KID, "typ": "JWT"}
        signing_input = (
            b64url(json.dumps(header).encode()) + "."
            + b64url(json.dumps(claims).encode())
        ).encode()
        token = signing_input.decode() + "." + b64url(sign_rs256(signing_input))
        self._send_json({
            "id_token": token,
            "access_token": "stub-access-token",
            "token_type": "Bearer",
            "expires_in": 3600,
        })


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
