#!/bin/bash
# Stage B smoke test: guest add-to-cart, badge, cart page, stepper.
BASE=http://127.0.0.1:3000
J=/tmp/tb_jar.txt; rm -f $J
# Shared harness (echo-only variant): lib.sh (ST-6).
SUITE_NO_COUNT=1
. "$(dirname "$0")/lib.sh"

curl -s -c $J $BASE/ -o /tmp/tb_home.html
grep -q 'id="cart-count" hidden' /tmp/tb_home.html && ok "badge hidden at 0" || ng "badge hidden at 0"
grep -q "カートに入れる" /tmp/tb_home.html && ok "add buttons on cards" || ng "add buttons on cards"
grep -q 'href="/products/1"' /tmp/tb_home.html && ok "cards link to detail" || ng "cards link to detail"
CSRF=$(csrf /tmp/tb_home.html)
echo "csrf len: ${#CSRF}"

echo "-- add product 2 (マグカップ, stock 3)"
curl -s -b $J -c $J -X POST $BASE/cart/items --data-urlencode "product_id=2" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$CSRF" -o /tmp/tb_add1.html
grep -q "Axum マグカップをカートに追加しました" /tmp/tb_add1.html && grep -q 'id="toast-region"' /tmp/tb_add1.html && ok "added toast (OOB)" || ng "added toast (OOB)"
grep -q 'hx-swap-oob="true"' /tmp/tb_add1.html && grep -q '>1<' /tmp/tb_add1.html && ok "OOB badge = 1" || ng "OOB badge = 1"

curl -s -b $J -c $J -X POST $BASE/cart/items --data-urlencode "product_id=2" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$CSRF" -o /tmp/tb_add2.html
grep -q '>2<' /tmp/tb_add2.html && ok "second add, badge = 2" || ng "second add, badge = 2"

echo "-- add beyond stock (stock 3, adding 5)"
curl -s -b $J -c $J -X POST $BASE/cart/items --data-urlencode "product_id=2" --data-urlencode "quantity=5" --data-urlencode "csrf_token=$CSRF" -o /tmp/tb_add3.html
grep -q "カートに追加しました" /tmp/tb_add3.html && ng "capped add shows no toast" || ok "capped add shows no toast"
grep -q '>3<' /tmp/tb_add3.html && ok "capped at stock, badge = 3" || ng "capped badge (expect 3)"

echo "-- sold-out product 3 cannot be added (forged POST)"
curl -s -b $J -c $J -X POST $BASE/cart/items --data-urlencode "product_id=3" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$CSRF" -o /tmp/tb_add4.html
grep -q "カートに追加しました" /tmp/tb_add4.html && ng "sold-out add rejected" || ok "sold-out add rejected"

echo "-- add product 5 (keyholder 880): subtotal crosses the free-shipping threshold"
curl -s -b $J -c $J -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$CSRF" -o /tmp/tb_add5.html
grep -q "カートに追加しました" /tmp/tb_add5.html && ok "keyholder added" || ng "keyholder added"

echo "-- regression: the toast is response-only, never re-rendered"
curl -s -b $J $BASE/cart -o /tmp/tb_nav1.html
grep -q 'class="toast"' /tmp/tb_nav1.html && ng "no toast on cart page after add" || ok "no toast on cart page after add"
grep -q "SQLx キーホルダーをカートに追加しました" /tmp/tb_nav1.html && ng "no cart flash message on cart page" || ok "no cart flash message on cart page"
curl -s -b $J $BASE/ -o /tmp/tb_nav2.html
grep -q 'class="toast"' /tmp/tb_nav2.html && ng "no toast on home page after add" || ok "no toast on home page after add"

echo "-- cart page"
curl -s -b $J $BASE/cart -o /tmp/tb_cart.html
grep -q "Axum マグカップ" /tmp/tb_cart.html && ok "cart shows mug x3" || ng "cart shows mug"
grep -q "SQLx キーホルダー" /tmp/tb_cart.html && ok "cart shows keyholder" || ng "cart shows keyholder"
# totals: mug 1650x3=4950 + keyholder 880 = 5830 subtotal -> free shipping; no COD fee on the cart page (it applies only when 代引き is chosen at checkout)
grep -q "￥5,830" /tmp/tb_cart.html && ok "totals 5830 + 0 ship = 5830" || ng "totals (expect 5,830)"

echo "-- stepper down (mug 3 -> 2): subtotal 4180, shipping 550 returns"
CSRF2=$(csrf /tmp/tb_cart.html)
V2=$(docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "SELECT id FROM product_variants WHERE product_id=2")
curl -s -b $J -c $J -X POST $BASE/cart/items/update --data-urlencode "variant_id=$V2" --data-urlencode "quantity=2" --data-urlencode "csrf_token=$CSRF2" -o /tmp/tb_upd.html
grep -q "あと" /tmp/tb_upd.html && grep -q "送料無料" /tmp/tb_upd.html && ok "free-shipping progress note back" || ng "free-shipping progress note"
grep -q "￥4,730" /tmp/tb_upd.html && ok "totals 4180 + 550 ship = 4730" || ng "totals after update (expect 4,730)"

echo "-- remove keyholder line"
V5=$(docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "SELECT id FROM product_variants WHERE product_id=5")
curl -s -b $J -c $J -X POST $BASE/cart/items/remove --data-urlencode "variant_id=$V5" --data-urlencode "csrf_token=$CSRF2" -o /tmp/tb_rm.html
grep -q "SQLx キーホルダー" /tmp/tb_rm.html && ng "keyholder removed" || ok "keyholder removed"

echo "-- button restore endpoint is gone (buttons never change now)"
CODE=$(curl -s -b $J -o /dev/null -w "%{http_code}" "$BASE/cart/button/2")
[ "$CODE" = "404" ] && ok "restore endpoint removed" || ng "restore endpoint ($CODE)"

echo "-- parallel adds never exceed stock (R2-20: the cap lives in the upsert)"
# Two adds of 2 against stock 2 racing each other: the old
# read-then-write flow could land on 4. The member cart is used
# so the result is checkable in the DB; the line is removed
# again so later suites see the account as they expect it.
JM=/tmp/tb_member.txt; rm -f $JM
curl -s -c $JM $BASE/login -o /tmp/tb_login.html
CSRFM=$(csrf /tmp/tb_login.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/login --data-urlencode "email=test@example.com" --data-urlencode "password=test1234" --data-urlencode "csrf_token=$CSRFM"
curl -s -b $JM $BASE/ -o /tmp/tb_mhome.html
CSRFM=$(csrf /tmp/tb_mhome.html)
curl -s -b $JM -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=9" --data-urlencode "quantity=2" --data-urlencode "csrf_token=$CSRFM" &
curl -s -b $JM -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=9" --data-urlencode "quantity=2" --data-urlencode "csrf_token=$CSRFM" &
wait
QTY=$(docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "SELECT quantity FROM cart_items WHERE user_id=(SELECT id FROM users WHERE email_bidx='$(bidx test@example.com)') AND variant_id=(SELECT id FROM product_variants WHERE product_id=9)")
[ "$QTY" = "2" ] && ok "parallel adds clamped at stock (qty=2)" || ng "parallel adds qty=$QTY (stock 2)"
curl -s -b $JM $BASE/cart -o /tmp/tb_mcart.html
CSRFM=$(csrf /tmp/tb_mcart.html)
V9=$(docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "SELECT id FROM product_variants WHERE product_id=9")
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/cart/items/remove --data-urlencode "variant_id=$V9" --data-urlencode "csrf_token=$CSRFM"

dbq() { docker exec agentic-db psql -U agentic -d agentic_commerce -qAt -c "$1"; }
# Builds one variant fixture product (サイズ S/M x カラー
# ブラック/ホワイト; stocks S/黒 2, S/白 0, M/黒 3, M/白 1) with
# direct inserts; sets FVP and the four variant ids.
make_variant_fixture() { # $1 = product name
  dbq "DELETE FROM cart_items WHERE variant_id IN (SELECT v.id FROM product_variants v JOIN products p ON p.id = v.product_id WHERE p.name = '$1')" >/dev/null
  dbq "DELETE FROM products WHERE name = '$1'" >/dev/null
  FVP=$(dbq "INSERT INTO products (name, price_yen, image_path, category) VALUES ('$1', 1980, '/static/images/product-rust-tshirt.svg', 'テスト') RETURNING id")
  local a1=$(dbq "INSERT INTO product_variant_axes (product_id, name, sort_order) VALUES ($FVP, 'サイズ', 0) RETURNING id")
  local a2=$(dbq "INSERT INTO product_variant_axes (product_id, name, sort_order) VALUES ($FVP, 'カラー', 1) RETURNING id")
  local os=$(dbq "INSERT INTO product_variant_options (axis_id, value, sort_order) VALUES ($a1, 'S', 0) RETURNING id")
  local om=$(dbq "INSERT INTO product_variant_options (axis_id, value, sort_order) VALUES ($a1, 'M', 1) RETURNING id")
  local ob=$(dbq "INSERT INTO product_variant_options (axis_id, value, sort_order) VALUES ($a2, 'ブラック', 0) RETURNING id")
  local ow=$(dbq "INSERT INTO product_variant_options (axis_id, value, sort_order) VALUES ($a2, 'ホワイト', 1) RETURNING id")
  FVSB=$(dbq "INSERT INTO product_variants (product_id, option_signature, stock) VALUES ($FVP, '$os-$ob', 2) RETURNING id")
  dbq "INSERT INTO product_variant_option_links (variant_id, option_id) VALUES ($FVSB, $os), ($FVSB, $ob)" >/dev/null
  FVSW=$(dbq "INSERT INTO product_variants (product_id, option_signature, stock) VALUES ($FVP, '$os-$ow', 0) RETURNING id")
  dbq "INSERT INTO product_variant_option_links (variant_id, option_id) VALUES ($FVSW, $os), ($FVSW, $ow)" >/dev/null
  FVMB=$(dbq "INSERT INTO product_variants (product_id, option_signature, stock) VALUES ($FVP, '$om-$ob', 3) RETURNING id")
  dbq "INSERT INTO product_variant_option_links (variant_id, option_id) VALUES ($FVMB, $om), ($FVMB, $ob)" >/dev/null
  FVMW=$(dbq "INSERT INTO product_variants (product_id, option_signature, stock) VALUES ($FVP, '$om-$ow', 1) RETURNING id")
  dbq "INSERT INTO product_variant_option_links (variant_id, option_id) VALUES ($FVMW, $om), ($FVMW, $ow)" >/dev/null
}

echo "-- variants: two combinations are two lines, labels shown, same combination merges"
make_variant_fixture 'カートバリエーション商品'
curl -s -b $J $BASE/products/$FVP -o /tmp/tb_var.html; T=$(csrf /tmp/tb_var.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items \
  --data-urlencode "product_id=$FVP" --data-urlencode "variant_id=$FVMB" --data-urlencode "quantity=1" \
  --data-urlencode "csrf_token=$T"
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items \
  --data-urlencode "product_id=$FVP" --data-urlencode "variant_id=$FVSB" --data-urlencode "quantity=1" \
  --data-urlencode "csrf_token=$T"
curl -s -b $J $BASE/cart -o /tmp/tb_vcart.html
grep -q "サイズ: M / カラー: ブラック" /tmp/tb_vcart.html && grep -q "サイズ: S / カラー: ブラック" /tmp/tb_vcart.html \
  && ok "two combinations render as two labelled lines" || ng "two combinations render as two labelled lines"
[ "$(dbq "SELECT COUNT(*) FROM cart_items WHERE variant_id IN ($FVMB, $FVSB)")" = "2" ] \
  && ok "two combinations are two cart rows" || ng "two combinations are two cart rows"
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items \
  --data-urlencode "product_id=$FVP" --data-urlencode "variant_id=$FVMB" --data-urlencode "quantity=1" \
  --data-urlencode "csrf_token=$T"
[ "$(dbq "SELECT quantity FROM cart_items WHERE variant_id = $FVMB")" = "2" ] \
  && [ "$(dbq "SELECT COUNT(*) FROM cart_items WHERE variant_id IN ($FVMB, $FVSB)")" = "2" ] \
  && ok "re-adding the same combination merges into its line" || ng "re-adding the same combination merges into its line"
