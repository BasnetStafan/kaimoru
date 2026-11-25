#!/bin/bash
# Consumption-tax upgrade verification (Phase 2 of the product
# upgrade): per-class breakdown math, snapshots, the future-rate
# rule and the tax-exclusive entry conversion.
#
# Worked example, computed by hand (the expectations below):
#   standard product  ¥2,980 (10% class), reduced ¥1,080 (8%),
#   exempt ¥500, out-of-scope ¥700 → subtotal ¥5,260.
#   Subtotal >= ¥5,000 so shipping is ¥0; COD fee ¥330 joins the
#   STANDARD class base. Total = 5,260 + 0 + 330 = ¥5,590.
#   standard base = 2,980 + 330 = 3,310 → tax = floor(3310×10/110) = ¥300.
#   reduced  base = 1,080 → tax = floor(1080×8/108) = ¥80.
#   exempt / out-of-scope bases ¥500 / ¥700, tax ¥0.
# Included-tax convention: floor-of-tax since 2026-11-14
# (see the internal purchase test matrix).
BASE=http://127.0.0.1:3000
MAIL=http://127.0.0.1:8025
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -q -t -A -c "$1"; }
CT=$(printf "csrf_%s" "token")
PW=$(printf "pass%s" "word")
APW=$(printf "Password%s" "1234")

IMG=$(db "SELECT image_path FROM products WHERE id = 2")
CAT_ID=$(db "SELECT id FROM categories ORDER BY id LIMIT 1")

ensure_product() { # name price tax_class_id -> id
  local pid
  pid=$(db "SELECT id FROM products WHERE name = '$1'")
  if [ -z "$pid" ]; then
    pid=$(db "INSERT INTO products (name, description, price_yen, category, category_id, image_path, status, tax_class_id) SELECT '$1', '', $2, c.name, c.id, '$IMG', 'public', $3 FROM categories c WHERE c.id = $CAT_ID RETURNING id")
    db "INSERT INTO product_variants (product_id, option_signature, stock) VALUES ($pid, '', 10)" >/dev/null
  else
    db "UPDATE products SET price_yen = $2, tax_class_id = $3, status = 'public' WHERE id = $pid" >/dev/null
    db "INSERT INTO product_variants (product_id, option_signature, stock) SELECT $pid, '', 10 WHERE NOT EXISTS (SELECT 1 FROM product_variants WHERE product_id = $pid)" >/dev/null
    db "UPDATE product_variants SET stock = 10 WHERE product_id = $pid" >/dev/null
  fi
  echo "$pid"
}

P_STD=$(ensure_product "検証税 標準" 2980 1)
P_RED=$(ensure_product "検証税 軽減" 1080 2)
P_EX=$(ensure_product "検証税 非課税" 500 3)
P_OOS=$(ensure_product "検証税 不課税" 700 4)
[ -n "$P_STD" ] && [ -n "$P_RED" ] && [ -n "$P_EX" ] && [ -n "$P_OOS" ] && ok "tax fixtures ready" || ng "tax fixtures ready"

echo "-- guest checkout across all four classes"
J=/tmp/tax_jar.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/tax_home.html; C=$(csrf /tmp/tax_home.html)
for pid in $P_STD $P_RED $P_EX $P_OOS; do
  curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$pid" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
done
curl -s -b $J $BASE/checkout -o /tmp/tax_co.html; C=$(csrf /tmp/tax_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" \
  --data-urlencode "email=tax-test@example.jp" --data-urlencode "family_name=税務" --data-urlencode "given_name=検" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=税務1-1-1" --data-urlencode "building=" --data-urlencode "phone=08011112222" \
  --data-urlencode "$CT=$C" -o /tmp/tax_step.html
curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "$CT=$C" -o /tmp/tax_confirm.html
grep -q "入力内容の確認" /tmp/tax_confirm.html && ok "confirmation rendered" || ng "confirmation rendered"
grep -q "￥5,590" /tmp/tax_confirm.html && ok "total 5590 unchanged by tax work" || ng "total 5590 on confirm"
grep -q "税率ごとの内訳" /tmp/tax_confirm.html && ok "breakdown section on confirm" || ng "breakdown section on confirm"
grep -q "￥3,310" /tmp/tax_confirm.html && ok "standard base 3310 on confirm" || ng "standard base on confirm"
grep -q "￥300" /tmp/tax_confirm.html && ok "standard tax 300 on confirm" || ng "standard tax on confirm"
grep -q "￥80" /tmp/tax_confirm.html && ok "reduced tax 80 on confirm" || ng "reduced tax on confirm"
C=$(csrf /tmp/tax_confirm.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$C"
OID=$(db "SELECT id FROM orders ORDER BY id DESC LIMIT 1")
[ -n "$OID" ] && ok "order placed" || ng "order placed"
TOT=$(db "SELECT total_yen FROM orders WHERE id = $OID")
[ "$TOT" = "5590" ] && ok "order total 5590" || ng "order total ($TOT)"
B_STD=$(db "SELECT taxable_base_yen || '/' || tax_yen FROM order_tax_breakdowns WHERE order_id = $OID AND tax_class_code = 'standard'")
[ "$B_STD" = "3310/300" ] && ok "stored standard breakdown 3310/300" || ng "stored standard breakdown ($B_STD)"
B_RED=$(db "SELECT taxable_base_yen || '/' || tax_yen FROM order_tax_breakdowns WHERE order_id = $OID AND tax_class_code = 'reduced'")
[ "$B_RED" = "1080/80" ] && ok "stored reduced breakdown 1080/80" || ng "stored reduced breakdown ($B_RED)"
B_EX=$(db "SELECT taxable_base_yen || '/' || tax_yen FROM order_tax_breakdowns WHERE order_id = $OID AND tax_class_code = 'exempt'")
[ "$B_EX" = "500/0" ] && ok "stored exempt breakdown 500/0" || ng "stored exempt breakdown ($B_EX)"
B_OOS=$(db "SELECT taxable_base_yen || '/' || tax_yen FROM order_tax_breakdowns WHERE order_id = $OID AND tax_class_code = 'out_of_scope'")
[ "$B_OOS" = "700/0" ] && ok "stored out-of-scope breakdown 700/0" || ng "stored out-of-scope breakdown ($B_OOS)"
SNAP=$(db "SELECT tax_class_code || '/' || rate_percent FROM order_items WHERE order_id = $OID AND product_id = $P_RED")
[ "$SNAP" = "reduced/8" ] && ok "line snapshot reduced/8" || ng "line snapshot ($SNAP)"

echo "-- confirmation mail carries the breakdown"
sleep 2
MID=$(curl -s "$MAIL/api/v1/search?query=tax-test@example.jp" | grep -o '"ID":"[^"]*"' | head -1 | cut -d'"' -f4)
curl -s "$MAIL/api/v1/message/$MID" -o /tmp/tax_mail.json
grep -q "税率ごとの内訳" /tmp/tax_mail.json && ok "mail shows the breakdown" || ng "mail shows the breakdown"

echo "-- a future-dated rate changes nothing today"
JA=/tmp/tax_admin.txt; rm -f $JA
curl -s -c $JA $BASE/admin/login -o /tmp/tax_alogin.html; C=$(csrf /tmp/tax_alogin.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/login --data-urlencode "email=admin@example.com" --data-urlencode "$PW=$APW" --data-urlencode "$CT=$C"
printf '\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\x0a\x49\x44\x41\x54\x78\x9c\x63\x00\x01\x00\x00\x05\x00\x01\x0d\x0a\x2d\xb4\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82' > /tmp/upx_fixture.png
curl -s -b $JA $BASE/admin/products/new -o /tmp/upx_new.html; CUP=$(csrf /tmp/upx_new.html)
IMG_FORM=$(curl -s -b $JA -X POST $BASE/admin/products/image-upload -H "x-csrf-token: $CUP" -F "image=@/tmp/upx_fixture.png;type=image/png" | sed -n 's/.*"url": *"\([^"]*\)".*/\1/p')
curl -s -b $JA $BASE/admin/settings/tax -o /tmp/tax_page.html
grep -q "軽減税率" /tmp/tax_page.html && ok "tax admin page lists the classes" || ng "tax admin page lists the classes"
grep -q "不課税" /tmp/tax_page.html && ok "out-of-scope is a distinct class" || ng "out-of-scope class listed"
C=$(csrf /tmp/tax_page.html)
TOMORROW=$(date -d "+1 day" +%F)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/settings/tax --data-urlencode "tax_class_id=1" --data-urlencode "rate_percent=12" --data-urlencode "effective_from=$TOMORROW" --data-urlencode "$CT=$C"
RATE12=$(db "SELECT count(*) FROM tax_rates WHERE tax_class_id = 1 AND rate_percent = 12")
[ "$RATE12" = "1" ] && ok "future 12% rate registered" || ng "future rate registered ($RATE12)"

J2=/tmp/tax_jar2.txt; rm -f $J2
curl -s -c $J2 $BASE/ -o /tmp/tax_home2.html; C=$(csrf /tmp/tax_home2.html)
curl -s -b $J2 -c $J2 -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$P_STD" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
curl -s -b $J2 $BASE/checkout -o /tmp/tax_co2.html; C=$(csrf /tmp/tax_co2.html)
curl -s -b $J2 -c $J2 -X POST $BASE/checkout --data-urlencode "intent=confirm" \
  --data-urlencode "email=tax-test2@example.jp" --data-urlencode "family_name=税務" --data-urlencode "given_name=検" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=税務1-1-1" --data-urlencode "building=" --data-urlencode "phone=08011112222" \
  --data-urlencode "$CT=$C" -o /dev/null
curl -s -b $J2 -c $J2 -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "$CT=$C" -o /tmp/tax_confirm2.html
grep -q "￥3,860" /tmp/tax_confirm2.html && ok "second order total 3860" || ng "second order total"
C=$(csrf /tmp/tax_confirm2.html)
curl -s -b $J2 -c $J2 -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$C"
OID2=$(db "SELECT id FROM orders ORDER BY id DESC LIMIT 1")
B2=$(db "SELECT rate_percent || '/' || tax_yen FROM order_tax_breakdowns WHERE order_id = $OID2 AND tax_class_code = 'standard'")
[ "$B2" = "10/350" ] && ok "today's order still resolves 10% (tax 350)" || ng "future rate leaked into today ($B2)"
B1_AGAIN=$(db "SELECT tax_yen FROM order_tax_breakdowns WHERE order_id = $OID AND tax_class_code = 'standard'")
[ "$B1_AGAIN" = "300" ] && ok "first order's snapshot unchanged" || ng "first order snapshot moved ($B1_AGAIN)"
db "DELETE FROM tax_rates WHERE tax_class_id = 1 AND rate_percent = 12" >/dev/null

echo "-- tax-exclusive entry converts at the class rate"
curl -s -b $JA $BASE/admin/products/$P_STD/edit -o /tmp/tax_edit.html; C=$(csrf /tmp/tax_edit.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/$P_STD/edit \
  --data-urlencode "name=検証税 標準" --data-urlencode "description=" \
  --data-urlencode "category_id=$CAT_ID" --data-urlencode "price_yen=2980" \
  --data-urlencode "price_mode=excl" --data-urlencode "price_excl_yen=1000" \
  --data-urlencode "tax_class_id=1" --data-urlencode "status=public" \
  --data-urlencode "images=$IMG_FORM" --data-urlencode "combo_stock[0]=10" \
  --data-urlencode "$CT=$C"
P_AFTER=$(db "SELECT price_yen FROM products WHERE id = $P_STD")
[ "$P_AFTER" = "1100" ] && ok "exclusive 1000 at 10% stored as inclusive 1100" || ng "exclusive conversion ($P_AFTER)"
db "UPDATE products SET price_yen = 2980 WHERE id = $P_STD" >/dev/null

result
