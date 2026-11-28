#!/bin/bash
# MCP Phase 4 suite (OAuth 2.1 resource server,
# docs/development/MCP_SERVER.md). Hermetic: it owns the
# app lifecycle like test_stripe.sh — builds the server,
# starts a stub authorization server
# (scripts/stubs/mcp_oauth_stub.py) on port 18103, restarts
# the app port-scoped with MCP_OAUTH_ISSUER /
# MCP_OAUTH_AUDIENCE pointing at the stub, runs the checks,
# then restarts the app WITHOUT the OAuth env and verifies
# the normal state (PRM 404, 13 tools). No real Auth0
# tenant is involved; the tenant wiring is an operator
# step after this build.
#
# Checks: the protected-resource metadata on both paths,
# the OAuth happy path (cart + draft) with the principal
# rows it writes, the scope refusal, the 401 challenge for
# wrong-audience / expired / wrong-key tokens, the
# spoofed-header strip, the API-key flow still working
# while OAuth is active, and cross-principal isolation.
source "$(dirname "$0")/lib.sh"
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH"

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
BASE=http://127.0.0.1:3000
MCP=$BASE/mcp
STUB=http://127.0.0.1:18103
ISSUER=$STUB
AUD=test-audience
SUB=oauth-suite-subject
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
JA=/tmp/mp4_admin.txt
KEY_MARK="MCP OAuthテスト"
AUTH_MSG="a valid API key with the cart:write scope is required"
ADDR='{"email":"mcp-p4@example.com","family_name":"山田","given_name":"テスト","postal_code":"160-0023","prefecture":"東京都","city":"新宿区","address_line":"西新宿1-1-1","phone":"09012345678"}'
ALL_SCOPES="catalog:read cart:write orders:read orders:write"

eval "$(grep -E '^(DB_PASSWORD|APP_DB_PASSWORD|VERIFICATION_CODE_SECRET)=' .env | sed 's/^/export /')"
export ADMIN_EMAIL
export ADMIN_PASSWORD=$APW

jqe() { python3 -c "import json,sys; d=json.loads(sys.argv[2]); print($1)" x "$2"; }
call() { # token tool args [extra headers...]
  local TOKEN=$1 TOOL=$2 ARGS=$3; shift 3
  if [ -n "$TOKEN" ]; then AUTH=(-H "Authorization: Bearer $TOKEN"); else AUTH=(); fi
  curl -s -X POST "$MCP" -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' "${AUTH[@]}" "$@" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"$TOOL\",\"arguments\":$ARGS}}"
}
call_status() { # token tool args -> HTTP status; body in /tmp/mp4_body.json, headers in /tmp/mp4_headers.txt
  if [ -n "$1" ]; then AUTH=(-H "Authorization: Bearer $1"); else AUTH=(); fi
  curl -s -o /tmp/mp4_body.json -D /tmp/mp4_headers.txt -w "%{http_code}" -X POST "$MCP" \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' "${AUTH[@]}" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"$2\",\"arguments\":$3}}"
}
mint() { # sub scope aud lifetime_secs sign_with
  curl -s -X POST $STUB/__mint -H 'Content-Type: application/json' \
    -d "{\"sub\":\"$1\",\"scope\":\"$2\",\"aud\":\"$3\",\"lifetime_secs\":$4,\"sign_with\":\"$5\"}"
}

APPLOG=/tmp/mp4_app.log
stop_app() { fuser -k 3000/tcp 2>/dev/null; sleep 1; }
start_app() {
  stop_app
  "$@" ./target/debug/server > $APPLOG 2>&1 &
  for i in $(seq 1 60); do
    curl -s -o /dev/null $BASE/healthz && return 0
    sleep 1
  done
  echo "  app failed to start; log tail:"; tail -5 $APPLOG
  return 1
}

admin_login() {
  rm -f "$JA"
  curl -s -c "$JA" $BASE/admin/login -o /tmp/mp4_login.html
  local C=$(csrf /tmp/mp4_login.html)
  curl -s -b "$JA" -c "$JA" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "password=$APW" --data-urlencode "csrf_token=$C"
}
make_key() { # name scope -> echoes the full token, shown once on the create response
  curl -s -b "$JA" $BASE/admin/settings/api-keys -o /tmp/mp4_keys.html
  local C=$(csrf /tmp/mp4_keys.html)
  curl -s -b "$JA" -X POST $BASE/admin/settings/api-keys \
    --data-urlencode "name=$1" --data-urlencode "scope=$2" --data-urlencode "csrf_token=$C" \
    | grep -o 'kmcp_[a-f0-9_]*' | head -1
}

echo "===== SETUP: build, stub, OAuth-configured app ====="
cargo build -p server 2>/tmp/mp4_build.log | tail -1
[ -x target/debug/server ] && ok "server builds" || { ng "server builds"; tail -5 /tmp/mp4_build.log; exit 1; }
fuser -k 18103/tcp 2>/dev/null; sleep 1
MCP_OAUTH_STUB_PORT=18103 python3 scripts/stubs/mcp_oauth_stub.py >/tmp/mp4_stub.log 2>&1 &
STUB_PID=$!
sleep 1
curl -s $STUB/.well-known/jwks.json | grep -q 'mcp-oauth-stub-1' && ok "stub AS serves its JWKS" || ng "stub JWKS"
start_app env MCP_OAUTH_ISSUER=$ISSUER MCP_OAUTH_AUDIENCE=$AUD \
  && ok "app starts with OAuth configured" || ng "app starts with OAuth configured"

echo "== protected-resource metadata =="
PRM=$(curl -s $BASE/.well-known/oauth-protected-resource)
[ "$(jqe "d['resource']" "$PRM")" = "$BASE/mcp" ] \
  && [ "$(jqe "d['authorization_servers'][0]" "$PRM")" = "$ISSUER" ] \
  && [ "$(jqe "','.join(d['scopes_supported'])" "$PRM")" = "catalog:read,cart:write,orders:read,orders:write" ] \
  && [ "$(jqe "d['bearer_methods_supported'][0]" "$PRM")" = "header" ] \
  && [ "$(jqe "d['resource_name']" "$PRM")" = "Kaimoru MCP" ] \
  && ok "PRM document: resource, issuer, exact scopes" || ng "PRM document"
PRM2=$(curl -s $BASE/.well-known/oauth-protected-resource/mcp)
[ "$(jqe "d['resource']" "$PRM2")" = "$BASE/mcp" ] && ok "PRM also served on the /mcp-suffixed path" || ng "PRM suffixed path"

echo "== OAuth happy path =="
TOK=$(mint "$SUB" "$ALL_SCOPES" "$AUD" 600 main)
R=$(call "$TOK" create_cart '{}')
OCART=$(jqe "d['result']['structuredContent']['cart_id']" "$R")
case "$OCART" in mcp_*) ok "OAuth token creates a cart";; *) ng "OAuth create_cart: $R";; esac
R=$(call "$TOK" add_cart_item "{\"cart_id\":\"$OCART\",\"product_id\":1,\"quantity\":1}")
[ "$(jqe "d['result']['structuredContent']['item_count']" "$R")" = "1" ] && ok "OAuth token adds an item" || ng "OAuth add_cart_item"
R=$(call "$TOK" create_order_draft "{\"cart_id\":\"$OCART\",\"idempotency_key\":\"mp4-draft-1\",\"address\":$ADDR}")
DRAFT=$(jqe "d['result']['structuredContent']['draft_id']" "$R")
[ -n "$DRAFT" ] && [ "$DRAFT" != "None" ] && ok "OAuth token creates an order draft" || ng "OAuth create_order_draft: $R"
R=$(call "$TOK" get_order_draft "{\"draft_id\":$DRAFT}")
[ "$(jqe "d['result']['structuredContent']['status']" "$R")" = "pending" ] \
  && [ "$(jqe "d['result']['structuredContent']['order_id']" "$R")" = "None" ] \
  && ok "the draft reads back pending, no order placed" || ng "get_order_draft shape: $R"
ROW=$(db "SELECT principal_key || '|' || coalesce(api_key_id::text,'NULL') || '|' || coalesce(oauth_subject,'NULL') FROM mcp_carts WHERE cart_id='$OCART'")
[ "$ROW" = "oauth:$SUB|NULL|$SUB" ] && ok "cart row: oauth principal, no key provenance" || ng "cart row: $ROW"
ROW=$(db "SELECT principal_key || '|' || coalesce(api_key_id::text,'NULL') FROM mcp_order_drafts WHERE id=$DRAFT")
[ "$ROW" = "oauth:$SUB|NULL" ] && ok "draft row: oauth principal, no key provenance" || ng "draft row: $ROW"

echo "== scope and token refusals =="
TOK_RO=$(mint "$SUB" "catalog:read" "$AUD" 600 main)
R=$(call "$TOK_RO" create_cart '{}')
[ "$(jqe "d['error']['message']" "$R")" = "$AUTH_MSG" ] && ok "catalog-only token: cart refused, standard message" || ng "catalog-only answer"
TOK_WA=$(mint "$SUB" "$ALL_SCOPES" "other-audience" 600 main)
ST=$(call_status "$TOK_WA" create_cart '{}')
[ "$ST" = "401" ] && grep -qi '^www-authenticate:' /tmp/mp4_headers.txt \
  && grep -qi 'resource_metadata="http://127.0.0.1:3000/.well-known/oauth-protected-resource"' /tmp/mp4_headers.txt \
  && [ "$(jqe "d['error']['code']" "$(cat /tmp/mp4_body.json)")" = "-32001" ] \
  && ok "wrong audience: 401 with the resource_metadata challenge" || ng "wrong-audience answer ($ST)"
TOK_EXP=$(mint "$SUB" "$ALL_SCOPES" "$AUD" -600 main)
ST=$(call_status "$TOK_EXP" create_cart '{}')
[ "$ST" = "401" ] && ok "expired token: 401" || ng "expired answer ($ST)"
TOK_ROGUE=$(mint "$SUB" "$ALL_SCOPES" "$AUD" 600 rogue)
ST=$(call_status "$TOK_ROGUE" create_cart '{}')
[ "$ST" = "401" ] && ok "token signed by an unpublished key: 401" || ng "rogue-key answer ($ST)"
R=$(call "" create_cart '{}' -H "x-mcp-oauth-subject: $SUB" -H "x-mcp-oauth-scopes: cart:write")
[ "$(jqe "d['error']['message']" "$R")" = "$AUTH_MSG" ] && ok "spoofed identity headers without a token are stripped" || ng "spoofed-header answer"

echo "== API keys still work while OAuth is active =="
admin_login
KEY_W=$(make_key "$KEY_MARK write" "cart:write")
R=$(call "$KEY_W" create_cart '{}')
KCART=$(jqe "d['result']['structuredContent']['cart_id']" "$R")
case "$KCART" in mcp_*) ok "API key creates a cart under OAuth-active config";; *) ng "key create_cart: $R";; esac
KID=$(db "SELECT id FROM api_keys WHERE name='$KEY_MARK write'")
ROW=$(db "SELECT principal_key FROM mcp_carts WHERE cart_id='$KCART'")
[ "$ROW" = "key:$KID" ] && ok "key cart row carries the key principal" || ng "key cart row: $ROW"

echo "== cross-principal isolation =="
R=$(call "$TOK" get_cart "{\"cart_id\":\"$KCART\"}")
[ "$(jqe "d['error']['message']" "$R")" = "no cart with id $KCART for this API key" ] \
  && ok "OAuth principal cannot read the key's cart" || ng "oauth->key cart answer"
R=$(call "$KEY_W" get_cart "{\"cart_id\":\"$OCART\"}")
[ "$(jqe "d['error']['message']" "$R")" = "no cart with id $OCART for this API key" ] \
  && ok "API key cannot read the OAuth principal's cart" || ng "key->oauth cart answer"

echo "===== CLEANUP and normal-state restore ====="
db "DELETE FROM cart_items WHERE session_id IN ('$OCART','$KCART')" >/dev/null
db "DELETE FROM mcp_order_drafts WHERE principal_key='oauth:$SUB'" >/dev/null
db "DELETE FROM mcp_carts WHERE cart_id IN ('$OCART','$KCART')" >/dev/null
db "DELETE FROM api_keys WHERE name LIKE '$KEY_MARK%'" >/dev/null
kill $STUB_PID 2>/dev/null
# The restore starts the binary the way run.sh does — the
# full .env sourced — so the shared dev app is left in its
# true normal state (gateway keys included), not the
# suite's reduced environment.
stop_app
( set -a; . ./.env; set +a; exec ./target/debug/server ) > $APPLOG 2>&1 &
RESTORED=0
for i in $(seq 1 60); do curl -s -o /dev/null $BASE/healthz && { RESTORED=1; break; }; sleep 1; done
[ "$RESTORED" = "1" ] && ok "app restored without OAuth env (full .env, run.sh-style)" || ng "app restored"
ST=$(curl -s -o /dev/null -w "%{http_code}" $BASE/.well-known/oauth-protected-resource)
[ "$ST" = "404" ] && ok "PRM is 404 while OAuth is inactive" || ng "PRM inactive answer ($ST)"
TL=$(curl -s -X POST "$MCP" -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}')
[ "$(jqe "len(d['result']['tools'])" "$TL")" = "13" ] && ok "tools/list back to the 13 Phase 1-3 tools" || ng "tools/list count"
result
