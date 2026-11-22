#!/usr/bin/env python3
"""A local stand-in for the KOMOJU API, for scripts/tests/test_payments.sh.

Not a KOMOJU simulator for production use — just enough of the
Sessions/Payments surface for the payment suite to run without
a merchant account, and to stand in for KOMOJU by hand when the
real service is unreachable:

  POST /api/v1/sessions            mint a session + payment
  GET  /api/v1/sessions/{id}       session with its payment
  GET  /api/v1/payments/{id}       one payment
  POST /api/v1/payments/{id}/capture
  POST /api/v1/payments/{id}/refund
  POST /api/v1/payments/{id}/cancel
  GET  /pay/{n}                    minimal hosted page: the
                                   payment's summary, and a pay
                                   button while it is authorized
  POST /__pay/{payment_id}         flip an authorized konbini
                                   payment to captured (the
                                   customer paid at the store)
  POST /__expire/{payment_id}      flip a payment to expired
                                   (its deadline passed unpaid)
  POST /__fail/{payment_id}        flip a payment to failed
  POST /__control                  {"fail_create": bool,
                                    "authorize_only": bool}
  GET  /__sessions                 how many sessions were minted

Card/PayPay payments are born `captured` (the app charges with
capture "auto"), konbini payments are born `authorized` with a
deadline and an instructions URL. With authorize_only set, card
payments are born `authorized` too — the abandoned-checkout
drill. State is in memory only; restart resets the world.

Environment, all optional — unset keeps the suite's behaviour:

  KOMOJU_STUB_PORT           listen port (default 18099)
  KOMOJU_STUB_SECRET_KEY     when set, /api/v1/* demands HTTP
                             Basic auth with this as the username
                             and an empty password (KOMOJU's own
                             model); unset, no auth is checked
  KOMOJU_STUB_WEBHOOK_URL    together with the secret below,
  KOMOJU_STUB_WEBHOOK_SECRET state changes are also delivered
                             as webhooks, signed the way KOMOJU
                             signs: hex HMAC-SHA256 over the raw
                             body in X-Komoju-Signature, a fresh
                             per-delivery id in X-Komoju-ID, and
                             a body of {"id", "type", "data"}
                             whose `id` is the dedupe key. Unset,
                             nothing is delivered — the suite
                             signs and posts its own webhooks.
"""

import base64
import hashlib
import hmac
import html
import json
import os
import sys
import threading
import time
import urllib.request
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Minted IDs carry the stub's start time: the app stores session
# IDs under a UNIQUE constraint, so a restarted stub must never
# reissue an earlier run's IDs.
RUN_TAG = str(int(time.time()))

PORT = int(os.environ.get("KOMOJU_STUB_PORT") or 18099)
BASE = f"http://127.0.0.1:{PORT}"
SECRET_KEY = os.environ.get("KOMOJU_STUB_SECRET_KEY") or None
WEBHOOK_URL = os.environ.get("KOMOJU_STUB_WEBHOOK_URL") or None
WEBHOOK_SECRET = os.environ.get("KOMOJU_STUB_WEBHOOK_SECRET") or None

STATE = {
    "sequence": 0,
    "events": 0,
    "sessions": {},
    "payments": {},
    "fail_create": False,
    "authorize_only": False,
}


def now_iso():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def payment_json(p):
    return {
        "id": p["id"],
        "status": p["status"],
        "amount": p["amount"],
        "tax": 0,
        "total": p["amount"],
        "currency": "JPY",
        "external_order_num": p["external_order_num"],
        "session_id": p["session_id"],
        "payment_deadline": p["payment_deadline"],
        "payment_details": p["payment_details"],
        "captured_at": p["captured_at"],
        "amount_refunded": p["amount_refunded"],
    }


def deliver_webhook(event_type, payment):
    """Delivers one signed webhook for a state change.

    Mirrors KOMOJU's delivery contract: the body is
    {"id", "type", "data"} with the payment object as `data`;
    the signature is the hex HMAC-SHA256 of the exact bytes
    sent, keyed with the webhook secret, in X-Komoju-Signature;
    X-Komoju-ID is a fresh per-delivery tracing number (the
    receiver dedupes on the body's `id`, never on it). The send
    runs on a daemon thread so a dead receiver can never block
    or fail the API call that caused the event, and nothing is
    sent at all unless both webhook env vars are set.
    """
    if not (WEBHOOK_URL and WEBHOOK_SECRET):
        return
    STATE["events"] += 1
    n = STATE["events"]
    body = json.dumps({
        "id": f"evt-stub-{RUN_TAG}-{n}",
        "type": event_type,
        "data": payment_json(payment),
    }).encode()
    signature = hmac.new(WEBHOOK_SECRET.encode(), body, hashlib.sha256).hexdigest()

    def send():
        request = urllib.request.Request(
            WEBHOOK_URL,
            data=body,
            headers={
                "Content-Type": "application/json",
                "X-Komoju-Signature": signature,
                "X-Komoju-ID": f"dlv-stub-{RUN_TAG}-{n}",
            },
            method="POST",
        )
        try:
            urllib.request.urlopen(request, timeout=3).read()
        except Exception as exc:  # a drill tool: report, never raise
            print(f"stub: webhook delivery failed: {exc}", file=sys.stderr)

    threading.Thread(target=send, daemon=True).start()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):  # keep the suite output clean
        pass

    def _send(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _send_html(self, code, page):
        body = page.encode()
        self.send_response(code)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _redirect(self, location):
        self.send_response(303)
        self.send_header("Location", location)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def _body(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b"{}"
        try:
            return json.loads(raw)
        except ValueError:
            return {}

    def _authorized(self):
        """KOMOJU's auth model: HTTP Basic, the secret key as the
        username and an empty password. Enforced only when the
        stub was started with KOMOJU_STUB_SECRET_KEY; the suite
        runs it without one and sees no auth at all."""
        if SECRET_KEY is None:
            return True
        header = self.headers.get("Authorization") or ""
        if not header.startswith("Basic "):
            return False
        try:
            decoded = base64.b64decode(header[6:]).decode()
        except Exception:
            return False
        username, _, _pw = decoded.partition(":")
        return hmac.compare_digest(username, SECRET_KEY)

    def _auth_guard(self):
        """True when the request may proceed; otherwise the 401
        has already been answered."""
        if self.path.startswith("/api/v1/") and not self._authorized():
            self._send(401, {"message": "unauthorized"})
            return False
        return True

    # -- the hosted payment page (manual stand-in use only) ----

    def _pay_page(self, n):
        session = STATE["sessions"].get(f"stub-session-{RUN_TAG}-{n}")
        if not session:
            return self._send(404, {"message": "session not found"})
        payment = STATE["payments"].get(session["payment_id"])
        if not payment:
            return self._send(404, {"message": "payment not found"})
        amount = f"¥{payment['amount']:,}"
        status = payment["status"]
        if status == "authorized":
            action = (
                f'<form method="post" action="/__pay_page/{payment["id"]}">'
                "<button type=\"submit\">支払う（スタブ）</button></form>"
                "<p>コンビニの場合は店頭で支払ったものとして扱います。</p>"
            )
        elif status == "captured":
            action = "<p>支払い済みです。</p>"
        else:
            action = f"<p>この支払いは {html.escape(status)} です。</p>"
        back = ""
        if session.get("return_url"):
            back = f'<p><a href="{html.escape(session["return_url"])}">ストアに戻る</a></p>'
        return self._send_html(200, (
            "<!DOCTYPE html><html lang=\"ja\"><head><meta charset=\"utf-8\">"
            "<title>KOMOJUスタブ</title></head><body>"
            "<h1>KOMOJUスタブ（ホストページ）</h1>"
            f"<p>金額: {amount}</p>"
            f"<p>状態: {html.escape(status)}</p>"
            f"{action}{back}"
            "</body></html>"
        ))

    # -- API surface --------------------------------------------

    def do_GET(self):
        if self.path == "/__sessions":
            return self._send(200, {"count": len(STATE["sessions"])})
        if self.path.startswith("/pay/"):
            return self._pay_page(self.path.rsplit("/", 1)[1])
        if not self._auth_guard():
            return
        if self.path.startswith("/api/v1/sessions/"):
            session = STATE["sessions"].get(self.path.rsplit("/", 1)[1])
            if not session:
                return self._send(404, {"message": "session not found"})
            payment = STATE["payments"].get(session["payment_id"])
            return self._send(200, {
                "id": session["id"],
                "status": "completed",
                "payment": payment_json(payment) if payment else None,
            })
        if self.path.startswith("/api/v1/payments/"):
            payment = STATE["payments"].get(self.path.rsplit("/", 1)[1])
            if not payment:
                return self._send(404, {"message": "payment not found"})
            return self._send(200, payment_json(payment))
        return self._send(404, {"message": "not found"})

    def do_POST(self):
        if self.path == "/__control":
            body = self._body()
            STATE["fail_create"] = bool(body.get("fail_create", False))
            STATE["authorize_only"] = bool(body.get("authorize_only", False))
            return self._send(200, {"ok": True})
        if self.path.startswith("/__pay_page/"):
            # The hosted page's pay button: flip, then send the
            # browser back to the store's return URL, like the
            # real hosted page does after a payment.
            payment = STATE["payments"].get(self.path.rsplit("/", 1)[1])
            if not payment:
                return self._send(404, {"message": "payment not found"})
            payment["status"] = "captured"
            payment["captured_at"] = now_iso()
            deliver_webhook("payment.captured", payment)
            session = STATE["sessions"].get(payment["session_id"]) or {}
            return self._redirect(session.get("return_url") or f"/pay/{payment['id'].rsplit('-', 1)[1]}")
        if self.path.startswith("/__pay/"):
            payment = STATE["payments"].get(self.path.rsplit("/", 1)[1])
            if not payment:
                return self._send(404, {"message": "payment not found"})
            payment["status"] = "captured"
            payment["captured_at"] = now_iso()
            deliver_webhook("payment.captured", payment)
            return self._send(200, payment_json(payment))
        if self.path.startswith("/__expire/") or self.path.startswith("/__fail/"):
            payment = STATE["payments"].get(self.path.rsplit("/", 1)[1])
            if not payment:
                return self._send(404, {"message": "payment not found"})
            if self.path.startswith("/__expire/"):
                payment["status"] = "expired"
                deliver_webhook("payment.expired", payment)
            else:
                payment["status"] = "failed"
                deliver_webhook("payment.failed", payment)
            return self._send(200, payment_json(payment))
        if not self._auth_guard():
            return
        if self.path == "/api/v1/sessions":
            if STATE["fail_create"]:
                return self._send(500, {"message": "stub outage (scripted)"})
            body = self._body()
            STATE["sequence"] += 1
            n = STATE["sequence"]
            session_id = f"stub-session-{RUN_TAG}-{n}"
            payment_id = f"stub-payment-{RUN_TAG}-{n}"
            method = (body.get("payment_types") or ["credit_card"])[0]
            external = (body.get("payment_data") or {}).get("external_order_num")
            if method == "konbini":
                status = "authorized"
                deadline = (datetime.now(timezone.utc) + timedelta(days=3)).strftime(
                    "%Y-%m-%dT%H:%M:%SZ"
                )
                details = {
                    "type": "konbini",
                    "store": "seven-eleven",
                    "instructions_url": f"{BASE}/instructions/{n}",
                }
                captured_at = None
            elif STATE["authorize_only"]:
                status, deadline, details, captured_at = "authorized", None, None, None
            else:
                status, deadline, details = "captured", None, None
                captured_at = now_iso()
            payment = {
                "id": payment_id,
                "session_id": session_id,
                "status": status,
                "amount": int(body.get("amount") or 0),
                "external_order_num": external,
                "payment_deadline": deadline,
                "payment_details": details,
                "captured_at": captured_at,
                "amount_refunded": 0,
            }
            STATE["payments"][payment_id] = payment
            STATE["sessions"][session_id] = {
                "id": session_id,
                "payment_id": payment_id,
                "return_url": body.get("return_url"),
            }
            return self._send(200, {
                "id": session_id,
                "session_url": f"{BASE}/pay/{n}",
            })
        parts = self.path.split("/")
        if (
            len(parts) == 6
            and parts[1:4] == ["api", "v1", "payments"]
            and parts[5] in ("capture", "refund", "cancel")
        ):
            payment = STATE["payments"].get(parts[4])
            if not payment:
                return self._send(404, {"message": "payment not found"})
            body = self._body()
            if parts[5] == "capture":
                amount = body.get("amount")
                if amount is not None:
                    # A partial capture settles for the captured
                    # amount (tax is always 0 here, so the total
                    # follows the amount).
                    payment["amount"] = int(amount)
                payment["status"] = "captured"
                payment["captured_at"] = payment["captured_at"] or now_iso()
                deliver_webhook("payment.captured", payment)
            elif parts[5] == "refund":
                amount = body.get("amount")
                if amount is None:
                    # No amount means the full remaining amount.
                    amount = payment["amount"] - payment["amount_refunded"]
                payment["amount_refunded"] += int(amount)
                if payment["amount_refunded"] >= payment["amount"]:
                    payment["status"] = "refunded"
                deliver_webhook("payment.refunded", payment)
            else:
                payment["status"] = "cancelled"
                deliver_webhook("payment.cancelled", payment)
            return self._send(200, payment_json(payment))
        return self._send(404, {"message": "not found"})


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
