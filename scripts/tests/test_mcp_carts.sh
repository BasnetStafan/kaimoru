#!/bin/bash
# MCP cart suite (Kaimoru MCP Phase 2,
# docs/development/MCP_SERVER.md). Runs against the shared
# dev app with mcp_enabled = true in config.toml.
#
# Keys are minted through the real admin UI flow (login,
# create, scrape the once-shown token), which also covers
# the admin page; then the cart tools are exercised over
# MCP: auth refusals (no key, malformed, read-scope,
# revoked), the no-oracle ownership rule, the cart happy
# path against REST ground truth, and the browser claim
# handoff with a cookie jar. Suite keys, carts and lines
# are removed at the end.
source "$(dirname "$0")/lib.sh"

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
BASE=http://127.0.0.1:3000
MCP=$BASE/mcp
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
JA=/tmp/mc_admin.txt; JG=/tmp/mc_guest.txt
KEY_MARK="MCPカートテスト"
AUTH_MSG="a valid API key with the cart:write scope is required"

jqe() { python3 -c "import json,sys; d=json.loads(sys.argv[2]); print($1)" x "$2"; }
call() { # token tool args
  if [ -n "$1" ]; then AUTH=(-H "Authorization: Bearer $1"); else AUTH=(); fi
  curl -s -X POST "$MCP" -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' "${AUTH[@]}" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"$2\",\"arguments\":$3}}"
}

admin_login() {
  rm -f "$JA"
  curl -s -c "$JA" $BASE/admin/login -o /tmp/mc_login.html
  local C=$(csrf /tmp/mc_login.html)
  curl -s -b "$JA" -c "$JA" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "password=$APW" --data-urlencode "csrf_token=$C"
}

make_key() { # name scope -> echoes the full token, shown once on the create response
  curl -s -b "$JA" $BASE/admin/settings/api-keys -o /tmp/mc_keys.html
  local C=$(csrf /tmp/mc_keys.html)
  curl -s -b "$JA" -X POST $BASE/admin/settings/api-keys \
    --data-urlencode "name=$1" --data-urlencode "scope=$2" --data-urlencode "csrf_token=$C" \
    | grep -o 'kmcp_[a-f0-9_]*' | head -1
}

revoke_key() { # prefix -> revokes through the list page's own form target
  curl -s -b "$JA" $BASE/admin/settings/api-keys -o /tmp/mc_keys.html
  local C=$(csrf /tmp/mc_keys.html)
  local ID=$(python3 - "$1" <<'PYEOF'
import re, sys
html = open('/tmp/mc_keys.html').read()
for row in html.split('<tr>'):
    if sys.argv[1] in row:
        m = re.search(r'/api-keys/(\d+)/revoke', row)
        if m:
            print(m.group(1))
        break
PYEOF
)
  curl -s -b "$JA" -o /dev/null -X POST "$BASE/admin/settings/api-keys/$ID/revoke" --data-urlencode "csrf_token=$C"
}

echo "== key issuing through the admin page =="
admin_login
KEY_W=$(make_key "$KEY_MARK write" "cart:write")
KEY_W2=$(make_key "$KEY_MARK other" "cart:write")
KEY_R=$(make_key "$KEY_MARK read" "catalog:read")
KEY_REV=$(make_key "$KEY_MARK revoked" "cart:write")
[ -n "$KEY_W" ] && [ ${#KEY_W} -eq 82 ] && ok "admin create returns the once-shown token" || ng "admin create token (${#KEY_W} chars)"
curl -s -b "$JA" $BASE/admin/settings/api-keys -o /tmp/mc_keys.html
grep -q "$KEY_MARK write" /tmp/mc_keys.html && grep -q "${KEY_W:5:12}" /tmp/mc_keys.html \
  && ok "admin list shows the key and its prefix" || ng "admin list shows the key"
revoke_key "${KEY_REV:5:12}"
R=$(call "$KEY_REV" create_cart '{}')
[ "$(jqe "d['error']['code']" "$R")" = "-32001" ] && ok "revoked key is refused" || ng "revoked key answered"

echo "== authentication refusals =="
R=$(call "" create_cart '{}')
[ "$(jqe "d['error']['code']" "$R")" = "-32001" ] && [ "$(jqe "d['error']['message']" "$R")" = "$AUTH_MSG" ] \
  && ok "no key: the shared auth error" || ng "no key answer"
R=$(call "not-a-real-token" create_cart '{}')
[ "$(jqe "d['error']['message']" "$R")" = "$AUTH_MSG" ] && ok "malformed key: the same answer" || ng "malformed key answer"
R=$(call "$KEY_R" create_cart '{}')
[ "$(jqe "d['error']['message']" "$R")" = "$AUTH_MSG" ] && ok "read-scope key: the same answer" || ng "read-scope key answer"

echo "== cart happy path =="
R=$(call "$KEY_W" create_cart '{}')
CART=$(jqe "d['result']['structuredContent']['cart_id']" "$R")
case "$CART" in mcp_*) ok "create_cart returns an mcp_ handle";; *) ng "create_cart handle: $CART";; esac
P1_PRICE=$(curl -s "$BASE/api/products/1" | python3 -c "import json,sys; print(json.load(sys.stdin)['price_yen'])")
P1_NAME=$(db "SELECT name FROM products WHERE id=1")
FLAT=$(db "SELECT shipping_flat_yen FROM shop_settings LIMIT 1")
THRESH=$(db "SELECT free_shipping_threshold_yen FROM shop_settings LIMIT 1")
expect_total() { python3 -c "
subtotal = $P1_PRICE * $1
print(subtotal + (0 if subtotal >= $THRESH else $FLAT))"; }
R=$(call "$KEY_W" add_cart_item "{\"cart_id\":\"$CART\",\"product_id\":1,\"quantity\":2}")
[ "$(jqe "d['result']['structuredContent']['item_count']" "$R")" = "2" ] \
  && [ "$(jqe "d['result']['structuredContent']['total_yen']" "$R")" = "$(expect_total 2)" ] \
  && ok "add 2 x product 1: totals match the REST ground truth" || ng "add totals"
VID=$(jqe "d['result']['structuredContent']['lines'][0]['variant_id']" "$R")
R=$(call "$KEY_W" get_cart "{\"cart_id\":\"$CART\"}")
[ "$(jqe "d['result']['structuredContent']['total_yen']" "$R")" = "$(expect_total 2)" ] \
  && ok "get_cart agrees with the add read-back" || ng "get_cart total"
R=$(call "$KEY_W" update_cart_item "{\"cart_id\":\"$CART\",\"variant_id\":$VID,\"quantity\":3}")
[ "$(jqe "d['result']['structuredContent']['item_count']" "$R")" = "3" ] \
  && ok "update_cart_item sets the quantity" || ng "update quantity"
R=$(call "$KEY_W" update_cart_item "{\"cart_id\":\"$CART\",\"variant_id\":$VID,\"quantity\":0}")
[ "$(jqe "d['error']['message']" "$R")" = "quantity must be at least 1; use remove_cart_item to remove a line" ] \
  && ok "update to 0 is refused with the remove hint" || ng "update to 0 answer"
MULTI=$(db "SELECT p.id FROM products p JOIN product_variants v ON v.product_id = p.id WHERE p.status='public' GROUP BY p.id HAVING COUNT(v.id) > 1 ORDER BY p.id LIMIT 1")
R=$(call "$KEY_W" add_cart_item "{\"cart_id\":\"$CART\",\"product_id\":$MULTI,\"quantity\":1}")
[ "$(jqe "d['error']['message']" "$R")" = "product $MULTI has combinations; variant_id is required" ] \
  && ok "combination product without variant_id is refused" || ng "variant-required answer"
DRAFT_ID=$(db "SELECT id FROM products WHERE status <> 'public' ORDER BY id LIMIT 1")
R=$(call "$KEY_W" add_cart_item "{\"cart_id\":\"$CART\",\"product_id\":$DRAFT_ID,\"quantity\":1}")
[ "$(jqe "d['error']['message']" "$R")" = "no public product with id $DRAFT_ID" ] \
  && ok "draft product answers like a missing one" || ng "draft add answer"
R=$(call "$KEY_W" remove_cart_item "{\"cart_id\":\"$CART\",\"variant_id\":$VID}")
[ "$(jqe "d['result']['structuredContent']['item_count']" "$R")" = "0" ] \
  && ok "remove_cart_item empties the cart" || ng "remove line"

echo "== ownership: no oracle across keys =="
R=$(call "$KEY_W2" get_cart "{\"cart_id\":\"$CART\"}")
[ "$(jqe "d['error']['message']" "$R")" = "no cart with id $CART for this API key" ] \
  && ok "another key's cart answers as unknown" || ng "wrong-owner answer"
R=$(call "$KEY_W" get_cart "{\"cart_id\":\"mcp_00000000000000000000000000000000\"}")
[ "$(jqe "d['error']['message']" "$R")" = "no cart with id mcp_00000000000000000000000000000000 for this API key" ] \
  && ok "an unknown cart answers identically" || ng "unknown cart answer"

echo "== checkout handoff =="
R=$(call "$KEY_W" get_checkout_link "{\"cart_id\":\"$CART\"}")
[ "$(jqe "d['error']['message']" "$R")" = "the cart is empty" ] \
  && ok "empty cart gets no checkout link" || ng "empty cart link answer"
call "$KEY_W" add_cart_item "{\"cart_id\":\"$CART\",\"product_id\":1,\"quantity\":1}" > /dev/null
R=$(call "$KEY_W" get_checkout_link "{\"cart_id\":\"$CART\"}")
LINK=$(jqe "d['result']['structuredContent']['checkout_url']" "$R")
case "$LINK" in */cart/claim/*) ok "checkout link is a claim URL";; *) ng "checkout link: $LINK";; esac
CLAIM_PATH="/cart/claim/${LINK##*/cart/claim/}"
rm -f "$JG"
OUT=$(curl -s -c "$JG" -o /dev/null -w "%{http_code} %{redirect_url}" "$BASE$CLAIM_PATH")
case "$OUT" in "303 $BASE/cart") ok "claim redirects to the cart page";; *) ng "claim redirect: $OUT";; esac
curl -s -b "$JG" $BASE/cart -o /tmp/mc_cart.html
grep -q "カートを引き継ぎました" /tmp/mc_cart.html && grep -q "$P1_NAME" /tmp/mc_cart.html \
  && ok "claimed cart shows the product with the handover notice" || ng "claimed cart page"
curl -s -b "$JG" -o /dev/null "$BASE$CLAIM_PATH"
curl -s -b "$JG" $BASE/cart -o /tmp/mc_cart.html
grep -q "有効期限が切れているか" /tmp/mc_cart.html \
  && ok "second claim use is refused with the expiry notice" || ng "second claim answer"
R=$(call "$KEY_W" get_cart "{\"cart_id\":\"$CART\"}")
[ "$(jqe "d['result']['structuredContent']['item_count']" "$R")" = "0" ] \
  && ok "the MCP cart is empty after the claim" || ng "MCP cart after claim"

echo "== anonymous surface unchanged =="
R=$(curl -s -X POST "$MCP" -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}')
[ "$(jqe "len(d['result']['tools'])" "$R")" = "10" ] && ok "tools/list answers anonymously with 10 tools" || ng "tools/list count"

echo "== cleanup =="
db "DELETE FROM cart_items WHERE session_id IN (SELECT cart_id FROM mcp_carts WHERE api_key_id IN (SELECT id FROM api_keys WHERE name LIKE '$KEY_MARK%'))" > /dev/null
db "DELETE FROM mcp_carts WHERE api_key_id IN (SELECT id FROM api_keys WHERE name LIKE '$KEY_MARK%')" > /dev/null
db "DELETE FROM api_keys WHERE name LIKE '$KEY_MARK%'" > /dev/null

result
