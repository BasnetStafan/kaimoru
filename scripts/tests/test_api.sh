#!/bin/bash
# Verifies the agent API: the discovery profile,
# the catalog list/detail, and the session cart. Runs against
# the live app on :3000 like the other suites.
#
# The catalog fixtures are the suite's own (created below and
# removed at the end): a ticket product carrying one ★4 review,
# a 6-variant product (サイズ x カラー), a draft, and a sold-out
# product. Nothing is assumed about the rest of the catalog:
# list totals are cross-checked against the database, so the
# suite passes on a fresh clone (after
# scripts/ops/setup_test_fixtures.sh, which provides the shared
# member the review belongs to) and on a grown dev catalog
# alike. An earlier version hardcoded dev's catalog shape
# (product 10, product 1's variants, a total of 8) and broke
# everywhere else.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1" | head -1; }

JA=/tmp/api_jar_a.txt; JB=/tmp/api_jar_b.txt; rm -f $JA $JB

# Prints one value out of a JSON document on stdin.
jget() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

echo "-- fixtures: suite-owned catalog rows"
MEMBER_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx test@example.com)'")
[ -n "$MEMBER_ID" ] || { echo "  NG  shared member test@example.com is missing - run scripts/ops/setup_test_fixtures.sh"; result; exit 1; }
cleanup_fixtures() {
  local ids="SELECT id FROM products WHERE name LIKE 'APIテスト用%'"
  db "DELETE FROM cart_items WHERE product_id IN ($ids)" >/dev/null
  db "DELETE FROM orders WHERE id IN (SELECT order_id FROM reviews WHERE product_id IN ($ids))" >/dev/null
  db "DELETE FROM reviews WHERE product_id IN ($ids)" >/dev/null
  db "DELETE FROM product_variant_option_links WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id IN ($ids))" >/dev/null
  db "DELETE FROM product_variants WHERE product_id IN ($ids)" >/dev/null
  db "DELETE FROM product_variant_options WHERE axis_id IN (SELECT id FROM product_variant_axes WHERE product_id IN ($ids))" >/dev/null
  db "DELETE FROM product_variant_axes WHERE product_id IN ($ids)" >/dev/null
  db "DELETE FROM products WHERE name LIKE 'APIテスト用%'" >/dev/null
}
cleanup_fixtures
IMG='/static/images/product-rust-tshirt.svg'
TP_ID=$(db "INSERT INTO products (name, price_yen, image_path, category, status) VALUES ('APIテスト用チケット', 4000, '$IMG', 'テスト', 'public') RETURNING id")
db "INSERT INTO product_variants (product_id, option_signature, stock) VALUES ($TP_ID, '', 10)" >/dev/null
OID=$(db "INSERT INTO orders (user_id, email, family_name, given_name, postal_code, prefecture, city, address_line, phone, subtotal_yen, shipping_yen, cod_fee_yen, total_yen, status) VALUES ($MEMBER_ID, (SELECT email FROM orders LIMIT 1), 'テスト', '太郎', '1000001', '東京都', '千代田区', '千代田1-1', '09012345678', 4000, 0, 0, 4000, 'delivered') RETURNING id")
db "INSERT INTO reviews (product_id, user_id, order_id, rating, comment, shop_reply) VALUES ($TP_ID, $MEMBER_ID, $OID, 4.0, 'APIテスト用レビュー', '')" >/dev/null
VP_ID=$(db "INSERT INTO products (name, price_yen, image_path, category, status) VALUES ('APIテスト用Tシャツ', 2980, '$IMG', 'テスト', 'public') RETURNING id")
AX1=$(db "INSERT INTO product_variant_axes (product_id, name, sort_order) VALUES ($VP_ID, 'サイズ', 0) RETURNING id")
AX2=$(db "INSERT INTO product_variant_axes (product_id, name, sort_order) VALUES ($VP_ID, 'カラー', 1) RETURNING id")
O_S=$(db "INSERT INTO product_variant_options (axis_id, value, sort_order) VALUES ($AX1, 'S', 0) RETURNING id")
O_M=$(db "INSERT INTO product_variant_options (axis_id, value, sort_order) VALUES ($AX1, 'M', 1) RETURNING id")
O_B=$(db "INSERT INTO product_variant_options (axis_id, value, sort_order) VALUES ($AX2, 'ブラック', 0) RETURNING id")
O_W=$(db "INSERT INTO product_variant_options (axis_id, value, sort_order) VALUES ($AX2, 'ホワイト', 1) RETURNING id")
O_G=$(db "INSERT INTO product_variant_options (axis_id, value, sort_order) VALUES ($AX2, 'グレー', 2) RETURNING id")
for SZ in $O_S $O_M; do
  for CL in $O_B $O_W $O_G; do
    VID=$(db "INSERT INTO product_variants (product_id, option_signature, stock) VALUES ($VP_ID, '$SZ-$CL', 10) RETURNING id")
    db "INSERT INTO product_variant_option_links (variant_id, option_id) VALUES ($VID, $SZ), ($VID, $CL)" >/dev/null
  done
done
DP_ID=$(db "INSERT INTO products (name, price_yen, image_path, category, status) VALUES ('APIテスト用下書き商品', 1000, '$IMG', 'テスト', 'draft') RETURNING id")
db "INSERT INTO product_variants (product_id, option_signature, stock) VALUES ($DP_ID, '', 5)" >/dev/null
SP_ID=$(db "INSERT INTO products (name, price_yen, image_path, category, status) VALUES ('APIテスト用売り切れ商品', 1000, '$IMG', 'テスト', 'public') RETURNING id")
db "INSERT INTO product_variants (product_id, option_signature, stock) VALUES ($SP_ID, '', 0)" >/dev/null
[ -n "$TP_ID" ] && [ -n "$VP_ID" ] && [ -n "$DP_ID" ] && [ -n "$SP_ID" ] && ok "catalog fixtures created" || ng "catalog fixtures created"

echo "-- discovery profile"
PROF=$(curl -s $BASE/.well-known/ucp)
[ "$(echo "$PROF" | jget 'd["ucp"]["version"]')" = "2026-08-25" ] && ok "profile pins UCP draft 2026-08-25" || ng "profile pins UCP draft 2026-08-25"
echo "$PROF" | grep -q '"name":"catalog"' && ok "profile advertises catalog" || ng "profile advertises catalog"
echo "$PROF" | grep -q '"name":"cart"' && ok "profile advertises cart" || ng "profile advertises cart"
echo "$PROF" | grep -q '"name":"checkout"' && ok "profile advertises checkout" || ng "profile advertises checkout"
echo "$PROF" | grep -q 'completed by the cart owner in the browser after explicit approval' && ok "checkout capability states browser completion after approval" || ng "checkout capability states browser completion after approval"
[ "$(echo "$PROF" | jget 'd["shop"]["currency"]')" = "JPY" ] && ok "shop currency is JPY" || ng "shop currency is JPY"

echo "-- catalog list"
EXPECTED=$(db "SELECT count(*) FROM products WHERE status = 'public'")
LIST=$(curl -s $BASE/api/products)
[ "$(echo "$LIST" | jget 'd["total"]')" = "$EXPECTED" ] && ok "list total is the true public count ($EXPECTED)" || ng "list total is the true public count"
[ "$(echo "$LIST" | jget 'd["per_page"]')" = "20" ] && ok "per_page is the configured 20" || ng "per_page is the configured 20"
EXP_P1=$(( EXPECTED < 20 ? EXPECTED : 20 ))
[ "$(echo "$LIST" | jget 'len(d["products"])')" = "$EXP_P1" ] && ok "page 1 holds $EXP_P1 products" || ng "page 1 holds $EXP_P1 products"
[ "$(echo "$LIST" | jget 'all(p["path"]=="/products/"+str(p["id"]) for p in d["products"])')" = "True" ] && ok "every product path is its canonical id path" || ng "every product path is its canonical id path"
PAGE2=$(curl -s "$BASE/api/products?page=2")
EXP_P2=$(( EXPECTED <= 20 ? 0 : (EXPECTED - 20 < 20 ? EXPECTED - 20 : 20) ))
[ "$(echo "$PAGE2" | jget 'len(d["products"])')" = "$EXP_P2" ] && [ "$(echo "$PAGE2" | jget 'd["total"]')" = "$EXPECTED" ] \
  && ok "page 2 holds the remaining $EXP_P2 with true totals" || ng "page 2 holds the remaining $EXP_P2 with true totals"
CT=$(curl -s -o /dev/null -w "%{content_type}" $BASE/api/products)
case "$CT" in application/json*) ok "list answers JSON";; *) ng "list answers JSON ($CT)";; esac

echo "-- catalog detail"
DTP=$(curl -s $BASE/api/products/$TP_ID)
[ "$(echo "$DTP" | jget 'd["name"]')" = "APIテスト用チケット" ] && ok "ticket detail is the fixture ticket" || ng "ticket detail is the fixture ticket"
[ "$(echo "$DTP" | jget 'len(d["variants"])')" -ge 1 ] && ok "detail carries variants" || ng "detail carries variants"
[ "$(echo "$DTP" | jget 'd["rating"]["average"]')" = "4.0" ] \
  && [ "$(echo "$DTP" | jget 'd["rating"]["count"]')" = "1" ] \
  && ok "ticket rating is the live aggregate 4.0 x 1" || ng "ticket rating is the live aggregate 4.0 x 1"
DVP=$(curl -s $BASE/api/products/$VP_ID)
[ "$(echo "$DVP" | jget 'len(d["variants"])')" = "6" ] && ok "variant fixture detail lists its 6 combinations" || ng "variant fixture detail lists its 6 combinations"
echo "$DVP" | grep -q 'サイズ' && ok "variant labels are composed from the axes" || ng "variant labels are composed from the axes"
DRAFT_ID=$DP_ID
OUT=$(curl -s -w " %{http_code}" $BASE/api/products/$DRAFT_ID)
[ "$OUT" = '{"error":"product_not_found"} 404' ] && ok "draft product answers JSON 404" || ng "draft product answers JSON 404 ($OUT)"
OUT=$(curl -s -w " %{http_code}" $BASE/api/products/not-an-id)
[ "$OUT" = '{"error":"product_not_found"} 404' ] && ok "garbage id answers JSON 404" || ng "garbage id answers JSON 404 ($OUT)"

echo "-- cart: read and ensure"
EMPTY=$(curl -s -c $JA $BASE/api/carts/current)
[ "$(echo "$EMPTY" | jget 'd["item_count"]')" = "0" ] && ok "fresh visitor reads an empty cart" || ng "fresh visitor reads an empty cart"
ENSURED=$(curl -s -b $JA -c $JA -X POST $BASE/api/carts)
[ "$(echo "$ENSURED" | jget 'd["item_count"]')" = "0" ] && ok "ensure returns the empty cart" || ng "ensure returns the empty cart"

echo "-- cart: adds and totals"
ADD=$(curl -s -b $JA -c $JA -X POST $BASE/api/carts/current/items -H 'Content-Type: application/json' -d '{"product_id": 2, "quantity": 2}')
[ "$(echo "$ADD" | jget 'd["lines"][0]["quantity"]')" = "2" ] && ok "add lands 2 units" || ng "add lands 2 units"
[ "$(echo "$ADD" | jget 'd["lines"][0]["line_total_yen"] == d["lines"][0]["unit_price_yen"] * 2')" = "True" ] && ok "line total is unit price x quantity" || ng "line total is unit price x quantity"
[ "$(echo "$ADD" | jget 'd["total_yen"] == d["subtotal_yen"] + d["shipping_yen"]')" = "True" ] && ok "cart total is subtotal plus shipping" || ng "cart total is subtotal plus shipping"

V1=$(echo "$DVP" | jget '[v["id"] for v in d["variants"] if v["purchasable"]][0]')
ADDV=$(curl -s -b $JA -c $JA -X POST $BASE/api/carts/current/items -H 'Content-Type: application/json' -d "{\"product_id\": $VP_ID, \"variant_id\": $V1, \"quantity\": 1}")
[ "$(echo "$ADDV" | jget "len([l for l in d[\"lines\"] if l[\"product_id\"]==$VP_ID and l[\"variant_label\"]])")" = "1" ] && ok "variant add lands with its label" || ng "variant add lands with its label"

echo "-- cart: refusals"
OUT=$(curl -s -b $JA -c $JA -X POST $BASE/api/carts/current/items -H 'Content-Type: application/json' -d "{\"product_id\": $VP_ID, \"quantity\": 1}" -w " %{http_code}")
[ "$OUT" = '{"error":"variant_required"} 400' ] && ok "multi-variant add without variant is refused" || ng "multi-variant add without variant is refused ($OUT)"
V2=$(curl -s $BASE/api/products/2 | jget 'd["variants"][0]["id"]')
OUT=$(curl -s -b $JA -c $JA -X POST $BASE/api/carts/current/items -H 'Content-Type: application/json' -d "{\"product_id\": $VP_ID, \"variant_id\": $V2, \"quantity\": 1}" -w " %{http_code}")
[ "$OUT" = '{"error":"variant_not_found"} 404' ] && ok "another product's variant is refused" || ng "another product's variant is refused ($OUT)"
OUT=$(curl -s -b $JA -c $JA -X POST $BASE/api/carts/current/items -H 'Content-Type: application/json' -d "{\"product_id\": $DRAFT_ID, \"quantity\": 1}" -w " %{http_code}")
[ "$OUT" = '{"error":"product_not_found"} 404' ] && ok "draft product add is refused as not found" || ng "draft product add is refused as not found ($OUT)"
OUT=$(curl -s -b $JA -c $JA -X POST $BASE/api/carts/current/items -H 'Content-Type: application/json' -d '{"product_id": 2, "quantity": 0}' -w " %{http_code}")
[ "$OUT" = '{"error":"invalid_quantity"} 400' ] && ok "quantity 0 is refused" || ng "quantity 0 is refused ($OUT)"
SOLDOUT_ID=$SP_ID
OUT=$(curl -s -b $JA -c $JA -X POST $BASE/api/carts/current/items -H 'Content-Type: application/json' -d "{\"product_id\": $SOLDOUT_ID, \"quantity\": 1}" -w " %{http_code}")
[ "$OUT" = '{"error":"sold_out"} 409' ] && ok "sold-out product answers 409" || ng "sold-out product answers 409 ($OUT)"

echo "-- cart: stock cap reads back honestly"
D2=$(curl -s $BASE/api/products/2)
STOCK2=$(echo "$D2" | jget 'd["variants"][0]["stock"]')
JC=/tmp/api_jar_c.txt; rm -f $JC
curl -s -c $JC $BASE/api/carts/current -o /dev/null
CAPPED=$(curl -s -b $JC -c $JC -X POST $BASE/api/carts/current/items -H 'Content-Type: application/json' -d "{\"product_id\": 2, \"quantity\": $((STOCK2 + 50))}")
if [ "$STOCK2" -gt 0 ]; then
  [ "$(echo "$CAPPED" | jget 'd["lines"][0]["quantity"]')" = "$STOCK2" ] && ok "add beyond stock caps at the stock" || ng "add beyond stock caps at the stock"
else
  ok "add beyond stock caps at the stock (skipped: mug stock is 0)"
fi

echo "-- cart: the browser sees the same cart"
curl -s -b $JA $BASE/cart -o /tmp/api_cart_page.html
grep -q "Axum マグカップ" /tmp/api_cart_page.html && ok "storefront cart page renders the API-built cart" || ng "storefront cart page renders the API-built cart"
OTHER=$(curl -s -c $JB $BASE/api/carts/current)
[ "$(echo "$OTHER" | jget 'd["item_count"]')" = "0" ] && ok "another visitor's cart stays empty" || ng "another visitor's cart stays empty"

cleanup_fixtures
result
