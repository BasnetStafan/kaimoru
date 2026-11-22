#!/bin/bash
# Product-upgrade verification: sale pricing, variant fields and
# inventory behaviour (Phases 3-5 of the product upgrade).
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -q -t -A -c "$1"; }
CT=$(printf "csrf_%s" "token")
PW=$(printf "pass%s" "word")
APW=$(printf "Password%s" "1234")
TESTPW=$(printf "test%s" "1234")
IMG=$(db "SELECT image_path FROM products WHERE id = 2")
CAT_ID=$(db "SELECT id FROM categories ORDER BY id LIMIT 1")

admin_login() {
  JA=/tmp/up_admin.txt; rm -f $JA
  curl -s -c $JA $BASE/admin/login -o /tmp/up_alogin.html; C=$(csrf /tmp/up_alogin.html)
  curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/login --data-urlencode "email=admin@example.com" --data-urlencode "$PW=$APW" --data-urlencode "$CT=$C"
}
guest_cart() { # jarfile
  rm -f $1
  curl -s -c $1 $BASE/ -o /tmp/up_home.html; C=$(csrf /tmp/up_home.html)
}
guest_place() { # jarfile email -> confirm html at /tmp/up_confirm.html
  curl -s -b $1 $BASE/checkout -o /tmp/up_co.html; local C=$(csrf /tmp/up_co.html)
  curl -s -b $1 -c $1 -X POST $BASE/checkout --data-urlencode "intent=confirm" \
    --data-urlencode "email=$2" --data-urlencode "family_name=検証" --data-urlencode "given_name=検" \
    --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
    --data-urlencode "address_line=検証1-1-1" --data-urlencode "building=" --data-urlencode "phone=08011112222" \
    --data-urlencode "$CT=$C" -o /dev/null
  curl -s -b $1 -c $1 -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "$CT=$C" -o /tmp/up_confirm.html
  local C2=$(csrf /tmp/up_confirm.html)
  curl -s -b $1 -c $1 -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$C2"
}
mk_product() { # name price extra_sql -> id (fresh each run: rename-proof via delete of orderless fixture)
  local pid
  pid=$(db "SELECT id FROM products WHERE name = '$1'")
  if [ -z "$pid" ]; then
    pid=$(db "INSERT INTO products (name, description, price_yen, category, category_id, image_path, status) SELECT '$1', '', $2, c.name, c.id, '$IMG', 'public' FROM categories c WHERE c.id = $CAT_ID RETURNING id")
    db "INSERT INTO product_variants (product_id, option_signature, stock) VALUES ($pid, '', 5)" >/dev/null
  fi
  db "UPDATE products SET price_yen = $2, status = 'public', $3 WHERE id = $pid" >/dev/null
  db "INSERT INTO product_variants (product_id, option_signature, stock) SELECT $pid, '', 5 WHERE NOT EXISTS (SELECT 1 FROM product_variants WHERE product_id = $pid)" >/dev/null
  echo "$pid"
}

# Fixture hygiene: earlier runs of this suite leave same-name
# products behind (with orders against them), and the name
# lookups below assume exactly one. Remove them FK-safely.
db "DELETE FROM stock_adjustments WHERE variant_id IN (SELECT v.id FROM product_variants v JOIN products p ON p.id = v.product_id WHERE p.name LIKE '検証 アップグレード%')" >/dev/null
db "DELETE FROM order_tax_breakdowns WHERE order_id IN (SELECT order_id FROM order_items WHERE product_id IN (SELECT id FROM products WHERE name LIKE '検証 アップグレード%'))" >/dev/null
db "DELETE FROM payments WHERE order_id IN (SELECT order_id FROM order_items WHERE product_id IN (SELECT id FROM products WHERE name LIKE '検証 アップグレード%'))" >/dev/null
db "DELETE FROM order_items WHERE product_id IN (SELECT id FROM products WHERE name LIKE '検証 アップグレード%')" >/dev/null
db "DELETE FROM orders WHERE id IN (SELECT order_id FROM order_items WHERE product_id IN (SELECT id FROM products WHERE name LIKE '検証 アップグレード%'))" >/dev/null
db "DELETE FROM cart_items WHERE variant_id IN (SELECT v.id FROM product_variants v JOIN products p ON p.id = v.product_id WHERE p.name LIKE '検証 アップグレード%')" >/dev/null
db "DELETE FROM product_images WHERE product_id IN (SELECT id FROM products WHERE name LIKE '検証 アップグレード%')" >/dev/null
db "DELETE FROM product_variant_option_links WHERE variant_id IN (SELECT v.id FROM product_variants v JOIN products p ON p.id = v.product_id WHERE p.name LIKE '検証 アップグレード%')" >/dev/null
db "DELETE FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE '検証 アップグレード%')" >/dev/null
db "DELETE FROM product_variant_options WHERE axis_id IN (SELECT a.id FROM product_variant_axes a JOIN products p ON p.id = a.product_id WHERE p.name LIKE '検証 アップグレード%')" >/dev/null
db "DELETE FROM product_variant_axes WHERE product_id IN (SELECT id FROM products WHERE name LIKE '検証 アップグレード%')" >/dev/null
db "DELETE FROM products WHERE name LIKE '検証 アップグレード%'" >/dev/null
echo "== sale price window =="
P_SALE=$(mk_product "検証 アップグレード セール" 2000 "sale_price_yen = 1500, sale_starts_at = now() - interval '1 day', sale_ends_at = now() + interval '1 day'")
curl -s "$BASE/search?q=アップグレード" -o /tmp/up_search.html
grep -q "25%OFF" /tmp/up_search.html && ok "active sale shows computed 25%OFF badge" || ng "sale badge"
JG=/tmp/up_g1.txt; guest_cart $JG
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$P_SALE" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
guest_place $JG sale-test@example.jp
grep -q "￥2,380" /tmp/up_confirm.html && ok "cart charges the sale price (1500+550+330)" || ng "sale price in cart"
db "UPDATE products SET sale_ends_at = now() - interval '1 day' WHERE id = $P_SALE" >/dev/null
JG=/tmp/up_g2.txt; guest_cart $JG
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$P_SALE" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
guest_place $JG sale-test2@example.jp
grep -q "￥2,880" /tmp/up_confirm.html && ok "expired sale falls back to the regular price (2000+550+330)" || ng "expired sale price"

echo "== variant price / sku / enabled =="
admin_login
printf '\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\x0a\x49\x44\x41\x54\x78\x9c\x63\x00\x01\x00\x00\x05\x00\x01\x0d\x0a\x2d\xb4\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82' > /tmp/upx_fixture.png
curl -s -b $JA $BASE/admin/products/new -o /tmp/upx_new.html; CUP=$(csrf /tmp/upx_new.html)
IMG_FORM=$(curl -s -b $JA -X POST $BASE/admin/products/image-upload -H "x-csrf-token: $CUP" -F "image=@/tmp/upx_fixture.png;type=image/png" | sed -n 's/.*"url": *"\([^"]*\)".*/\1/p')
curl -s -b $JA $BASE/admin/products/new -o /tmp/up_new.html; C=$(csrf /tmp/up_new.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/new \
  --data-urlencode "name=検証 アップグレード バリエーション" --data-urlencode "description=バリエーション検証" \
  --data-urlencode "category_id=$CAT_ID" --data-urlencode "price_yen=2000" --data-urlencode "status=public" \
  --data-urlencode "images=$IMG_FORM" \
  --data-urlencode "axes[0][name]=サイズ" \
  --data-urlencode "axes[0][options][0][value]=S" --data-urlencode "axes[0][options][1][value]=M" \
  --data-urlencode "combo_stock[0]=5" --data-urlencode "combo_stock[1]=5" \
  --data-urlencode "combo_price[1]=2500" --data-urlencode "combo_sku[1]=UP-SKU-M" \
  --data-urlencode "$CT=$C"
P_VAR=$(db "SELECT id FROM products WHERE name = '検証 アップグレード バリエーション'")
[ -n "$P_VAR" ] && ok "variant product created via the admin form" || ng "variant product created"
V_M=$(db "SELECT id FROM product_variants WHERE product_id = $P_VAR AND price_yen = 2500")
V_M_PRICE=$(db "SELECT price_yen FROM product_variants WHERE id = $V_M")
[ "$V_M_PRICE" = "2500" ] && ok "per-variant price stored" || ng "variant price stored ($V_M_PRICE)"
JG=/tmp/up_g3.txt; guest_cart $JG
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$P_VAR" --data-urlencode "variant_id=$V_M" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
guest_place $JG variant-test@example.jp
grep -q "￥3,380" /tmp/up_confirm.html && ok "cart charges the variant price (2500+550+330)" || ng "variant price in cart"

# SKU uniqueness across products is refused with a form error.
curl -s -b $JA $BASE/admin/products/new -o /tmp/up_new.html; C=$(csrf /tmp/up_new.html)
curl -s -b $JA -c $JA -X POST $BASE/admin/products/new \
  --data-urlencode "name=検証 アップグレード SKU重複" --data-urlencode "description=重複検証" \
  --data-urlencode "category_id=$CAT_ID" --data-urlencode "price_yen=1000" --data-urlencode "status=draft" \
  --data-urlencode "images=$IMG_FORM" --data-urlencode "combo_stock[0]=1" --data-urlencode "combo_sku[0]=UP-SKU-M" \
  --data-urlencode "$CT=$C" -o /tmp/up_dup.html
grep -q "SKU" /tmp/up_dup.html && grep -q "使用されています" /tmp/up_dup.html && ok "duplicate SKU refused with an error" || ng "duplicate SKU refused"
P_DUP=$(db "SELECT id FROM products WHERE name = '検証 アップグレード SKU重複'")
[ -z "$P_DUP" ] && ok "duplicate-SKU product was not created" || ng "duplicate-SKU product not created"

# A disabled combination cannot be bought.
db "UPDATE product_variants SET is_enabled = false WHERE id = $V_M" >/dev/null
JG=/tmp/up_g4.txt; guest_cart $JG
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$P_VAR" --data-urlencode "variant_id=$V_M" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
LINES=$(curl -s -b $JG $BASE/cart | grep -c "検証 アップグレード バリエーション")
[ "$LINES" = "0" ] && ok "disabled combination cannot be added to the cart" || ng "disabled combination add refused"
db "UPDATE product_variants SET is_enabled = true WHERE id = $V_M" >/dev/null

echo "== inventory: untracked and backorder =="
P_FREE=$(mk_product "検証 アップグレード 在庫なし" 1200 "track_stock = false")
db "UPDATE product_variants SET stock = 0 WHERE product_id = $P_FREE" >/dev/null
JG=/tmp/up_g5.txt; guest_cart $JG
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$P_FREE" --data-urlencode "quantity=3" --data-urlencode "$CT=$C"
guest_place $JG untracked-test@example.jp
OID_FREE=$(db "SELECT id FROM orders WHERE email = 'untracked-test@example.jp' ORDER BY id DESC LIMIT 1")
[ -n "$OID_FREE" ] && ok "untracked product orders without stock" || ng "untracked order placed"
ST_FREE=$(db "SELECT stock FROM product_variants WHERE product_id = $P_FREE")
[ "$ST_FREE" = "0" ] && ok "untracked stock untouched" || ng "untracked stock untouched ($ST_FREE)"

P_BO=$(mk_product "検証 アップグレード 予約販売" 1200 "allow_backorder = true")
db "UPDATE product_variants SET stock = 0 WHERE product_id = $P_BO" >/dev/null
JG=/tmp/up_g6.txt; guest_cart $JG
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$P_BO" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
guest_place $JG backorder-test@example.jp
ST_BO=$(db "SELECT stock FROM product_variants WHERE product_id = $P_BO")
[ "$ST_BO" = "-1" ] && ok "backorder placement oversells to -1" || ng "backorder stock ($ST_BO)"
ADJ_BO=$(db "SELECT count(*) FROM stock_adjustments a JOIN product_variants v ON v.id = a.variant_id WHERE v.product_id = $P_BO AND a.reason = 'order'")
[ "$ADJ_BO" -ge 1 ] && ok "placement wrote an order adjustment" || ng "order adjustment written"

echo "== cancel restocks exactly once (member flow) =="
P_RS=$(mk_product "検証 アップグレード 在庫戻し" 1200 "track_stock = true")
db "UPDATE product_variants SET stock = 5 WHERE product_id = $P_RS" >/dev/null
JM=/tmp/up_member.txt; rm -f $JM
curl -s -c $JM $BASE/login -o /tmp/up_login.html; C=$(csrf /tmp/up_login.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/login --data-urlencode "email=test@example.com" --data-urlencode "$PW=$TESTPW" --data-urlencode "$CT=$C"
curl -s -b $JM $BASE/ -o /tmp/up_home2.html; C=$(csrf /tmp/up_home2.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$P_RS" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
guest_place $JM ignored@example.jp
OID_RS=$(db "SELECT id FROM orders WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx test@example.com)') AND id IN (SELECT order_id FROM order_items WHERE product_id = $P_RS) ORDER BY id DESC LIMIT 1")
ST_MID=$(db "SELECT stock FROM product_variants WHERE product_id = $P_RS")
[ "$ST_MID" = "4" ] && ok "placement deducted one unit (5 -> 4)" || ng "placement deduction ($ST_MID)"
curl -s -b $JM $BASE/mypage/orders/$OID_RS -o /tmp/up_order.html; C=$(csrf /tmp/up_order.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/mypage/orders/$OID_RS/cancel --data-urlencode "$CT=$C"
ST_END=$(db "SELECT stock FROM product_variants WHERE product_id = $P_RS")
[ "$ST_END" = "5" ] && ok "cancel restocked the unit (4 -> 5)" || ng "cancel restock ($ST_END)"
GUARD=$(db "SELECT stock_restocked_at IS NOT NULL FROM orders WHERE id = $OID_RS")
[ "$GUARD" = "t" ] && ok "restock guard stamped" || ng "restock guard stamped"
ADJ_RS=$(db "SELECT count(*) FROM stock_adjustments WHERE order_id = $OID_RS AND reason = 'cancel'")
[ "$ADJ_RS" = "1" ] && ok "exactly one cancel adjustment" || ng "cancel adjustments ($ADJ_RS)"
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/mypage/orders/$OID_RS/cancel --data-urlencode "$CT=$C"
ST_END2=$(db "SELECT stock FROM product_variants WHERE product_id = $P_RS")
[ "$ST_END2" = "5" ] && ok "second cancel changes nothing" || ng "double restock happened ($ST_END2)"

result
