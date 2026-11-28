#!/bin/bash
# MCP Phase 3 suite (approval-gated draft orders,
# docs/development/MCP_SERVER.md). Runs against the shared
# dev app with mcp_enabled = true in config.toml.
#
# Keys are minted through the real admin UI flow, like
# test_mcp_carts.sh; then the order tools are exercised
# over MCP: the scope model (J1 — an orders:write key
# runs the whole flow alone, a cart:write key can never
# draft), idempotent draft creation, the human approval
# page and its COD payment, the no-oracle order reads,
# the void-on-change rule, and expiry. The hosted path
# is checked for whichever outcome the dev gateway
# gives (a redirect to the provider, or the payment
# error page when the session cannot start). Suite
# keys, drafts, carts and lines are removed at the end;
# the guest orders placed stay, as in every suite.
source "$(dirname "$0")/lib.sh"

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
BASE=http://127.0.0.1:3000
MCP=$BASE/mcp
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
JA=/tmp/mp3_admin.txt; JG=/tmp/mp3_guest.txt
KEY_MARK="MCP注文テスト"
WRITE_MSG="a valid API key with the orders:write scope is required"
READ_MSG="a valid API key with the orders:read scope is required"
ADDR='{"email":"mcp-p3@example.com","family_name":"注文","given_name":"テスト","postal_code":"160-0023","prefecture":"東京都","city":"新宿区","address_line":"西新宿1-1-1","phone":"09012345678"}'

jqe() { python3 -c "import json,sys; d=json.loads(sys.argv[2]); print($1)" x "$2"; }
call() { # token tool args
  if [ -n "$1" ]; then AUTH=(-H "Authorization: Bearer $1"); else AUTH=(); fi
  curl -s -X POST "$MCP" -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' "${AUTH[@]}" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"$2\",\"arguments\":$3}}"
}

admin_login() {
  rm -f "$JA"
  curl -s -c "$JA" $BASE/admin/login -o /tmp/mp3_login.html
  local C=$(csrf /tmp/mp3_login.html)
  curl -s -b "$JA" -c "$JA" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "password=$APW" --data-urlencode "csrf_token=$C"
}

make_key() { # name scope -> echoes the full token, shown once on the create response
  curl -s -b "$JA" $BASE/admin/settings/api-keys -o /tmp/mp3_keys.html
  local C=$(csrf /tmp/mp3_keys.html)
  curl -s -b "$JA" -X POST $BASE/admin/settings/api-keys \
    --data-urlencode "name=$1" --data-urlencode "scope=$2" --data-urlencode "csrf_token=$C" \
    | grep -o 'kmcp_[a-f0-9_]*' | head -1
}

echo "== key issuing through the admin page =="
admin_login
KEY_O=$(make_key "$KEY_MARK write" "orders:write")
KEY_O2=$(make_key "$KEY_MARK other" "orders:write")
KEY_R=$(make_key "$KEY_MARK read" "orders:read")
KEY_C=$(make_key "$KEY_MARK cart" "cart:write")
KEY_CAT=$(make_key "$KEY_MARK catalog" "catalog:read")
[ -n "$KEY_O" ] && [ ${#KEY_O} -eq 82 ] && ok "admin create returns the orders:write token" || ng "admin create orders:write token"
curl -s -b "$JA" $BASE/admin/settings/api-keys -o /tmp/mp3_keys.html
grep -q "orders:write" /tmp/mp3_keys.html && grep -q "orders:read" /tmp/mp3_keys.html \
  && ok "admin list shows the new scopes" || ng "admin list scopes"

P1_PRICE=$(curl -s "$BASE/api/products/1" | python3 -c "import json,sys; print(json.load(sys.stdin)['price_yen'])")
P1_NAME=$(db "SELECT name FROM products WHERE id=1")
FLAT=$(db "SELECT shipping_flat_yen FROM shop_settings LIMIT 1")
THRESH=$(db "SELECT free_shipping_threshold_yen FROM shop_settings LIMIT 1")
COD_FEE=$(db "SELECT cod_fee_yen FROM shop_settings LIMIT 1")
expect_total() { python3 -c "
subtotal = $P1_PRICE * $1
print(subtotal + (0 if subtotal >= $THRESH else $FLAT))"; }
YEN_SYM=$(grep -oP 'yen_symbol = "\K[^"]+' "$(dirname "$0")/../../config.toml")
fmt_yen() { python3 -c "print('$YEN_SYM' + format($1, ','))"; }

echo "== scope refusals =="
R=$(call "" create_order_draft "{\"cart_id\":\"mcp_none\",\"idempotency_key\":\"x\",\"address\":$ADDR}")
[ "$(jqe "d['error']['code']" "$R")" = "-32001" ] && [ "$(jqe "d['error']['message']" "$R")" = "$WRITE_MSG" ] \
  && ok "no key: the orders:write auth error" || ng "no key draft answer"
R=$(call "$KEY_C" create_order_draft "{\"cart_id\":\"mcp_none\",\"idempotency_key\":\"x\",\"address\":$ADDR}")
[ "$(jqe "d['error']['message']" "$R")" = "$WRITE_MSG" ] \
  && ok "cart:write key can never draft: the same answer" || ng "cart key draft answer"
R=$(call "$KEY_R" create_order_draft "{\"cart_id\":\"mcp_none\",\"idempotency_key\":\"x\",\"address\":$ADDR}")
[ "$(jqe "d['error']['message']" "$R")" = "$WRITE_MSG" ] \
  && ok "orders:read key can never draft: the same answer" || ng "read key draft answer"
R=$(call "$KEY_CAT" get_order_status '{"order_id":1}')
[ "$(jqe "d['error']['message']" "$R")" = "$READ_MSG" ] \
  && ok "catalog:read key reading status: the orders:read auth error" || ng "catalog key status answer"
R=$(call "$KEY_C" get_order_status '{"order_id":1}')
[ "$(jqe "d['error']['message']" "$R")" = "$READ_MSG" ] \
  && ok "cart:write key reading status: the same answer" || ng "cart key status answer"
R=$(call "$KEY_R" get_order_status '{"order_id":99999999}')
[ "$(jqe "d['error']['message']" "$R")" = "no order with id 99999999 for this API key" ] \
  && ok "orders:read key passes auth and meets the no-oracle answer" || ng "read key no-oracle answer"

echo "== happy path: draft, approve, pay COD =="
R=$(call "$KEY_O" create_cart '{}')
CART=$(jqe "d['result']['structuredContent']['cart_id']" "$R")
case "$CART" in mcp_*) ok "orders:write key creates its own cart (J1)";; *) ng "orders key cart: $CART";; esac
R=$(call "$KEY_O" add_cart_item "{\"cart_id\":\"$CART\",\"product_id\":1,\"quantity\":2}")
[ "$(jqe "d['result']['structuredContent']['item_count']" "$R")" = "2" ] \
  && [ "$(jqe "d['result']['structuredContent']['total_yen']" "$R")" = "$(expect_total 2)" ] \
  && ok "orders:write key fills its cart" || ng "orders key add"
R=$(call "$KEY_O" create_order_draft "{\"cart_id\":\"$CART\",\"idempotency_key\":\"p3-main-1\",\"address\":$ADDR}")
DRAFT=$(jqe "d['result']['structuredContent']['draft_id']" "$R")
APPROVAL_URL=$(jqe "d['result']['structuredContent']['approval_url']" "$R")
[ "$DRAFT" -gt 0 ] 2>/dev/null && [ "$(jqe "d['result']['structuredContent']['status']" "$R")" = "pending" ] \
  && [ "$(jqe "d['result']['structuredContent']['total_yen']" "$R")" = "$(expect_total 2)" ] \
  && ok "draft freezes the cart totals" || ng "draft create"
case "$APPROVAL_URL" in */orders/draft/*) ok "draft carries its approval URL";; *) ng "approval URL: $APPROVAL_URL";; esac
TOKEN=${APPROVAL_URL##*/}
R=$(call "$KEY_O" create_order_draft "{\"cart_id\":\"$CART\",\"idempotency_key\":\"p3-main-1\",\"address\":$ADDR}")
[ "$(jqe "d['result']['structuredContent']['draft_id']" "$R")" = "$DRAFT" ] \
  && [ "$(jqe "d['result']['structuredContent']['approval_url']" "$R")" = "None" ] \
  && [ "$(db "SELECT count(*) FROM mcp_order_drafts WHERE idempotency_key='p3-main-1'")" = "1" ] \
  && ok "idempotent replay: same draft, no URL, exactly one row" || ng "idempotent replay"
rm -f "$JG"
CODE=$(curl -s -c "$JG" -o /tmp/mp3_page.html -w "%{http_code}" "$BASE/orders/draft/$TOKEN")
[ "$CODE" = "200" ] && grep -q "$P1_NAME" /tmp/mp3_page.html && grep -q "$(fmt_yen $(expect_total 2))" /tmp/mp3_page.html \
  && grep -q "/orders/draft/$TOKEN/pay" /tmp/mp3_page.html \
  && ok "approval page shows the product, the totals and the pay form" || ng "approval page"
C=$(csrf /tmp/mp3_page.html)
curl -s -b "$JG" -c "$JG" -o /dev/null -D /tmp/mp3_pay.headers -X POST "$BASE/orders/draft/$TOKEN/pay" \
  --data-urlencode "csrf_token=$C" --data-urlencode "payment_method=cod"
head -1 /tmp/mp3_pay.headers | grep -q "303" && grep -qi "location: /checkout/complete" /tmp/mp3_pay.headers \
  && ok "COD pay lands on the completion page" || ng "COD pay redirect"
DSTATE=$(db "SELECT status || '|' || coalesce(order_id::text,'') FROM mcp_order_drafts WHERE id=$DRAFT")
OID=${DSTATE##*|}
[ "${DSTATE%%|*}" = "consumed" ] && [ -n "$OID" ] && ok "draft consumed onto its order" || ng "draft state: $DSTATE"
OROW=$(db "SELECT (user_id IS NULL) || '|' || placed_via || '|' || total_yen || '|' || status FROM orders WHERE id=$OID")
[ "$OROW" = "true|agent|$(($(expect_total 2) + COD_FEE))|received" ] \
  && ok "guest order placed with cart totals + COD fee" || ng "order row: $OROW"
[ "$(db "SELECT count(*) FROM cart_items WHERE session_id='$CART'")" = "0" ] \
  && ok "placement consumed the MCP cart's lines" || ng "cart lines after placement"
R=$(call "$KEY_O" get_order_draft "{\"draft_id\":$DRAFT}")
[ "$(jqe "d['result']['structuredContent']['status']" "$R")" = "consumed" ] \
  && [ "$(jqe "d['result']['structuredContent']['order_id']" "$R")" = "$OID" ] \
  && ok "get_order_draft shows consumed + order_id (J2)" || ng "draft read-back"
R=$(call "$KEY_O" get_order_status "{\"order_id\":$OID}")
[ "$(jqe "d['result']['structuredContent']['status']" "$R")" = "received" ] \
  && [ "$(jqe "d['result']['structuredContent']['total_yen']" "$R")" = "$(($(expect_total 2) + COD_FEE))" ] \
  && ok "get_order_status matches the placed order" || ng "status read-back"
R=$(call "$KEY_O2" get_order_status "{\"order_id\":$OID}")
[ "$(jqe "d['error']['message']" "$R")" = "no order with id $OID for this API key" ] \
  && ok "another key's order: the no-oracle answer" || ng "wrong-key status answer"
R=$(call "$KEY_O2" get_order_draft "{\"draft_id\":$DRAFT}")
[ "$(jqe "d['error']['message']" "$R")" = "no draft with id $DRAFT for this API key" ] \
  && ok "another key's draft: the no-oracle answer" || ng "wrong-key draft answer"

echo "== void on change =="
R=$(call "$KEY_O" create_cart '{}')
CART2=$(jqe "d['result']['structuredContent']['cart_id']" "$R")
call "$KEY_O" add_cart_item "{\"cart_id\":\"$CART2\",\"product_id\":1,\"quantity\":1}" > /dev/null
R=$(call "$KEY_O" create_order_draft "{\"cart_id\":\"$CART2\",\"idempotency_key\":\"p3-void-1\",\"address\":$ADDR}")
DRAFT2=$(jqe "d['result']['structuredContent']['draft_id']" "$R")
URL2=$(jqe "d['result']['structuredContent']['approval_url']" "$R")
TOKEN2=${URL2##*/}
call "$KEY_O" add_cart_item "{\"cart_id\":\"$CART2\",\"product_id\":1,\"quantity\":1}" > /dev/null
R=$(call "$KEY_O" get_order_draft "{\"draft_id\":$DRAFT2}")
[ "$(jqe "d['result']['structuredContent']['status']" "$R")" = "void" ] \
  && ok "a cart mutation voids the pending draft" || ng "void after mutation"
ORDERS_BEFORE=$(db "SELECT count(*) FROM orders")
rm -f "$JG"
curl -s -c "$JG" -o /tmp/mp3_void.html "$BASE/orders/draft/$TOKEN2"
# The state page carries no form (nothing to pay), so the
# token for the direct POST comes from the cart page of
# the same session — CSRF tokens are session-bound.
curl -s -b "$JG" -c "$JG" -o /tmp/mp3_cartpage.html "$BASE/cart"
C=$(csrf /tmp/mp3_cartpage.html)
curl -s -b "$JG" -c "$JG" -o /tmp/mp3_void_pay.html -X POST "$BASE/orders/draft/$TOKEN2/pay" \
  --data-urlencode "csrf_token=$C" --data-urlencode "payment_method=cod"
[ "$(db "SELECT count(*) FROM orders")" = "$ORDERS_BEFORE" ] \
  && [ "$(db "SELECT order_id IS NULL FROM mcp_order_drafts WHERE id=$DRAFT2")" = "t" ] \
  && ok "paying a void draft places no order" || ng "void pay"

echo "== expiry =="
R=$(call "$KEY_O" create_cart '{}')
CART3=$(jqe "d['result']['structuredContent']['cart_id']" "$R")
call "$KEY_O" add_cart_item "{\"cart_id\":\"$CART3\",\"product_id\":1,\"quantity\":1}" > /dev/null
R=$(call "$KEY_O" create_order_draft "{\"cart_id\":\"$CART3\",\"idempotency_key\":\"p3-exp-1\",\"address\":$ADDR}")
DRAFT3=$(jqe "d['result']['structuredContent']['draft_id']" "$R")
URL3=$(jqe "d['result']['structuredContent']['approval_url']" "$R")
TOKEN3=${URL3##*/}
db "UPDATE mcp_order_drafts SET expires_at = now() - interval '1 hour' WHERE id=$DRAFT3" > /dev/null
ORDERS_BEFORE=$(db "SELECT count(*) FROM orders")
rm -f "$JG"
curl -s -c "$JG" -o /tmp/mp3_exp.html "$BASE/orders/draft/$TOKEN3"
curl -s -b "$JG" -c "$JG" -o /tmp/mp3_cartpage.html "$BASE/cart"
C=$(csrf /tmp/mp3_cartpage.html)
curl -s -b "$JG" -c "$JG" -o /dev/null -X POST "$BASE/orders/draft/$TOKEN3/pay" \
  --data-urlencode "csrf_token=$C" --data-urlencode "payment_method=cod"
[ "$(db "SELECT count(*) FROM orders")" = "$ORDERS_BEFORE" ] \
  && [ "$(db "SELECT status FROM mcp_order_drafts WHERE id=$DRAFT3")" = "expired" ] \
  && ok "an expired draft refuses payment and reads expired" || ng "expired pay"

echo "== hosted method (dev gateway outcome) =="
if grep -q "komoju_enabled = true" "$(dirname "$0")/../../config.toml"; then
  R=$(call "$KEY_O" create_cart '{}')
  CART4=$(jqe "d['result']['structuredContent']['cart_id']" "$R")
  call "$KEY_O" add_cart_item "{\"cart_id\":\"$CART4\",\"product_id\":1,\"quantity\":1}" > /dev/null
  R=$(call "$KEY_O" create_order_draft "{\"cart_id\":\"$CART4\",\"idempotency_key\":\"p3-hosted-1\",\"address\":$ADDR}")
  URL4=$(jqe "d['result']['structuredContent']['approval_url']" "$R")
  TOKEN4=${URL4##*/}
  rm -f "$JG"
  curl -s -c "$JG" -o /tmp/mp3_hosted.html "$BASE/orders/draft/$TOKEN4"
  C=$(csrf /tmp/mp3_hosted.html)
  curl -s -b "$JG" -c "$JG" -o /tmp/mp3_hosted_pay.html -D /tmp/mp3_hosted.headers -X POST "$BASE/orders/draft/$TOKEN4/pay" \
    --data-urlencode "csrf_token=$C" --data-urlencode "payment_method=credit_card"
  if head -1 /tmp/mp3_hosted.headers | grep -q "303" && grep -qi "location: https://komoju.com" /tmp/mp3_hosted.headers; then
    ok "hosted pay redirects to the provider (KOMOJU)"
  elif grep -q "お支払い手続き" /tmp/mp3_hosted_pay.html; then
    ok "hosted pay reached the payment error page (session could not start in dev)"
  else
    ng "hosted pay outcome"
  fi
else
  echo "komoju disabled in dev config; hosted path skipped"
fi

echo "== cleanup =="
db "DELETE FROM mcp_order_drafts WHERE api_key_id IN (SELECT id FROM api_keys WHERE name LIKE '$KEY_MARK%')" > /dev/null
db "DELETE FROM cart_items WHERE session_id IN (SELECT cart_id FROM mcp_carts WHERE api_key_id IN (SELECT id FROM api_keys WHERE name LIKE '$KEY_MARK%'))" > /dev/null
db "DELETE FROM mcp_carts WHERE api_key_id IN (SELECT id FROM api_keys WHERE name LIKE '$KEY_MARK%')" > /dev/null
db "DELETE FROM api_keys WHERE name LIKE '$KEY_MARK%'" > /dev/null
[ "$(db "SELECT count(*) FROM api_keys WHERE name LIKE '$KEY_MARK%'")" = "0" ] \
  && ok "suite keys, drafts and carts removed" || ng "cleanup"

result
