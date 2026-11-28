#!/usr/bin/env python3
"""A local stand-in for an OpenAI-compatible chat endpoint,
for scripts/tests/test_agent_chat.sh.

Not a model simulator — just enough of the chat-completions
surface for the agent-chat suite to run deterministically
without the real LLM (the local Ollama), and to stand in by
hand when developing the chat flow:

  POST /v1/chat/completions   answer with the next canned
                              decision (see /__control). The
                              request body is captured.
  POST /__control             {"sequence": [decision, ...]}
                              sets the decisions to hand out,
                              in order; the last one repeats
                              once the sequence runs out. A
                              decision is either a JSON value
                              (serialised into the answer's
                              content) or a string (used as
                              the content verbatim — how the
                              suite feeds malformed output).
                              Setting the control also clears
                              the captured requests.
  GET  /__requests            {"requests": [body, ...]} —
                              every chat-completions body
                              received since the last control.
  GET  /__health              liveness probe for the suite.

With no control set, the answer content is "{}" — a decision
the app must refuse, which is also the suite's failure case.
State is in memory only; restart resets the world.

Environment, all optional:

  LLM_STUB_PORT              listen port (default 18123)
"""

import json
import os
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOCK = threading.Lock()
STATE = {"sequence": [], "requests": []}


class Handler(BaseHTTPRequestHandler):
    # The suite reads its own logs; keep stderr clean.
    def log_message(self, *args):
        pass

    def _send_json(self, code, obj):
        body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _read_json(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        try:
            return json.loads(raw or b"{}")
        except ValueError:
            return {}

    def do_GET(self):
        if self.path == "/__health":
            return self._send_json(200, {"ok": True})
        if self.path == "/__requests":
            with LOCK:
                return self._send_json(200, {"requests": STATE["requests"]})
        return self._send_json(404, {"error": "not found"})

    def do_POST(self):
        if self.path == "/__control":
            body = self._read_json()
            with LOCK:
                STATE["sequence"] = list(body.get("sequence", []))
                STATE["requests"] = []
            return self._send_json(200, {"ok": True})
        if self.path == "/v1/chat/completions":
            body = self._read_json()
            with LOCK:
                STATE["requests"].append(body)
                if len(STATE["sequence"]) > 1:
                    decision = STATE["sequence"].pop(0)
                elif STATE["sequence"]:
                    decision = STATE["sequence"][0]
                else:
                    decision = {}
            if isinstance(decision, str):
                content = decision
            else:
                content = json.dumps(decision, ensure_ascii=False)
            return self._send_json(200, {
                "id": "chatcmpl-stub",
                "object": "chat.completion",
                "model": body.get("model", "stub"),
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": content},
                    "finish_reason": "stop",
                }],
                "usage": {"prompt_tokens": 0, "completion_tokens": 0,
                          "total_tokens": 0},
            })
        return self._send_json(404, {"error": "not found"})


def main():
    port = int(os.environ.get("LLM_STUB_PORT", "18123"))
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"llm stub listening on 127.0.0.1:{port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
