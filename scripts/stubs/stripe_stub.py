#!/usr/bin/env python3
"""A local stand-in for the Stripe API, for scripts/tests/test_stripe.sh.

Not a Stripe simulator for production use — just enough of the
Checkout Sessions / PaymentIntents / Refunds surface for the
Stripe suite to run without a Stripe account:

  POST /v1/checkout/sessions       mint a session + payment intent
  GET  /v1/checkout/sessions/{id}  session, intent expanded
  GET  /v1/payment_intents/{id}    one payment intent, charge expanded
  POST /v1/payment_intents/{id}/capture
  POST /v1/payment_intents/{id}/cancel
  POST /v1/refunds                 add a refund to the intent's charge
  GET  /pay/{n}                    minimal hosted page
  POST /__pay/{intent_id}          flip a konbini intent to
                                   succeeded (the customer paid
                                   at the store)
  POST /__control                  {"fail_create": bool}
  GET  /__sessions                 how many sessions were minted

Card/PayPay intents are born `succeeded` (Checkout captures on
completion), konbini intents are born `requires_action` with a
voucher (deadline + hosted URL) in next_action. Amounts are
plain yen — JPY is zero-decimal at Stripe too. State is in
memory only; restart resets the world.

Environment, all optional — unset keeps the suite's behaviour:

  STRIPE_STUB_PORT           listen port (default 18101)
  STRIPE_STUB_SECRET_KEY     when set, /v1/* demands
                             Authorization: Bearer <this key>;
                             unset, no auth is checked
"""

import json
import os
import threading
import time
import urllib.parse
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Minted IDs carry the stub's start time: the app stores session
# IDs under a UNIQUE constraint, so a restarted stub must never
# reissue an earlier run's IDs.
RUN_TAG = str(int(time.time()))

PORT = int(os.environ.get("STRIPE_STUB_PORT") or 18101)
BASE = f"http://127.0.0.1:{PORT}"
SECRET_KEY = os.environ.get("STRIPE_STUB_SECRET_KEY") or None

STATE = {
    "sequence": 0,
    "sessions": {},
    "intents": {},
    "fail_create": False,
    "fail_refund": False,
}
LOCK = threading.Lock()


def intent_json(p):
    charge = {
        "id": p["charge_id"],
        "object": "charge",
        "created": p["charge_created"],
        "amount": p["amount"],
        "amount_refunded": p["amount_refunded"],
        "payment_intent": p["id"],
        "status": "succeeded" if p["status"] == "succeeded" else "pending",
    }
    body = {
        "id": p["id"],
        "object": "payment_intent",
        "status": p["status"],
        "amount": p["amount"],
        "currency": "jpy",
        "metadata": {"external_order_num": p["external_order_num"]},
        "latest_charge": charge,
        "next_action": p["next_action"],
    }
    return body


def session_json(s):
    intent = STATE["intents"][s["payment_intent"]]
    return {
        "id": s["id"],
        "object": "checkout.session",
        "status": "complete",
        "payment_status": "paid" if intent["status"] == "succeeded" else "unpaid",
        "url": s["url"],
        "client_reference_id": s["external_order_num"],
        "metadata": {"external_order_num": s["external_order_num"]},
        "payment_intent": intent_json(intent),
        "customer_email": s["email"],
        "amount_total": s["amount"],
        "currency": "jpy",
    }


def error_body(message):
    return {"error": {"type": "invalid_request_error", "message": message}}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def _send(self, code, payload, content_type="application/json"):
        if isinstance(payload, (dict, list)):
            data = json.dumps(payload).encode()
        else:
            data = payload.encode()
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _form(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length).decode() if length else ""
        parsed = urllib.parse.parse_qs(raw)
        return {k: v[0] for k, v in parsed.items()}

    def _authorized(self):
        if SECRET_KEY is None:
            return True
        return self.headers.get("Authorization") == f"Bearer {SECRET_KEY}"

    def do_GET(self):
        path = urllib.parse.urlparse(self.path).path
        if path == "/__sessions":
            return self._send(200, {"count": len(STATE["sessions"])})
        if path.startswith("/pay/"):
            return self._send(
                200,
                "<html><body><h1>Stripe stub checkout</h1></body></html>",
                "text/html",
            )
        if path.startswith("/v1/checkout/sessions/"):
            if not self._authorized():
                return self._send(401, error_body("invalid API key"))
            session = STATE["sessions"].get(path.rsplit("/", 1)[1])
            if session is None:
                return self._send(404, error_body("no such checkout session"))
            return self._send(200, session_json(session))
        if path.startswith("/v1/payment_intents/"):
            if not self._authorized():
                return self._send(401, error_body("invalid API key"))
            intent = STATE["intents"].get(path.rsplit("/", 1)[1])
            if intent is None:
                return self._send(404, error_body("no such payment intent"))
            return self._send(200, intent_json(intent))
        return self._send(404, error_body("unknown route"))

    def do_POST(self):
        path = urllib.parse.urlparse(self.path).path
        if path == "/__control":
            length = int(self.headers.get("Content-Length") or 0)
            raw = self.rfile.read(length).decode() if length else "{}"
            with LOCK:
                STATE.update(json.loads(raw))
            return self._send(200, {"ok": True})
        if path.startswith("/__pay/"):
            intent = STATE["intents"].get(path.rsplit("/", 1)[1])
            if intent is None:
                return self._send(404, error_body("no such payment intent"))
            with LOCK:
                intent["status"] = "succeeded"
                intent["next_action"] = None
                intent["charge_created"] = int(time.time())
            return self._send(200, intent_json(intent))
        if not path.startswith("/v1/"):
            return self._send(404, error_body("unknown route"))
        if not self._authorized():
            return self._send(401, error_body("invalid API key"))
        form = self._form()
        if path == "/v1/checkout/sessions":
            return self._create_session(form)
        if path == "/v1/refunds":
            return self._create_refund(form)
        if path.startswith("/v1/payment_intents/"):
            intent_id = path.split("/")[3]
            intent = STATE["intents"].get(intent_id)
            if intent is None:
                return self._send(404, error_body("no such payment intent"))
            if path.endswith("/capture"):
                with LOCK:
                    intent["status"] = "succeeded"
                    intent["charge_created"] = int(time.time())
                return self._send(200, intent_json(intent))
            if path.endswith("/cancel"):
                with LOCK:
                    intent["status"] = "canceled"
                return self._send(200, intent_json(intent))
        return self._send(404, error_body("unknown route"))

    def _create_session(self, form):
        if STATE["fail_create"]:
            return self._send(500, error_body("stub forced failure"))
        method = form.get("payment_method_types[0]", "card")
        try:
            amount = int(form.get("line_items[0][price_data][unit_amount]", "0"))
        except ValueError:
            return self._send(400, error_body("bad amount"))
        external = form.get("payment_intent_data[metadata][external_order_num]") or form.get(
            "client_reference_id", ""
        )
        with LOCK:
            STATE["sequence"] += 1
            n = STATE["sequence"]
            session_id = f"cs_test_{RUN_TAG}_{n}"
            intent_id = f"pi_{RUN_TAG}_{n}"
            charge_id = f"ch_{RUN_TAG}_{n}"
            now = int(time.time())
            next_action = None
            status = "succeeded"
            if method == "konbini":
                status = "requires_action"
                expires = datetime.now(timezone.utc) + timedelta(days=3)
                next_action = {
                    "type": "konbini_display_details",
                    "konbini_display_details": {
                        "expires_at": int(expires.timestamp()),
                        "hosted_voucher_url": f"{BASE}/instructions/{intent_id}",
                        "payment_code": "12345678901234",
                    },
                }
            intent = {
                "id": intent_id,
                "status": status,
                "amount": amount,
                "external_order_num": external,
                "charge_id": charge_id,
                "charge_created": now,
                "amount_refunded": 0,
                "next_action": next_action,
            }
            STATE["intents"][intent_id] = intent
            session = {
                "id": session_id,
                "url": f"{BASE}/pay/{n}",
                "payment_intent": intent_id,
                "external_order_num": external,
                "email": form.get("customer_email", ""),
                "amount": amount,
            }
            STATE["sessions"][session_id] = session
        return self._send(200, session_json(session))

    def _create_refund(self, form):
        if STATE.get("fail_refund"):
            return self._send(500, error_body("stub forced refund failure"))
        intent = STATE["intents"].get(form.get("payment_intent", ""))
        if intent is None:
            return self._send(404, error_body("no such payment intent"))
        with LOCK:
            if "amount" in form:
                amount = int(form["amount"])
            else:
                amount = intent["amount"] - intent["amount_refunded"]
            intent["amount_refunded"] += amount
            refund_id = f"re_{RUN_TAG}_{STATE['sequence']}_{intent['amount_refunded']}"
        return self._send(
            200,
            {
                "id": refund_id,
                "object": "refund",
                "status": "succeeded",
                "amount": amount,
                "payment_intent": intent["id"],
                "currency": "jpy",
            },
        )


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
