#!/bin/bash
# Stage D verification: product detail page.
BASE=http://127.0.0.1:3000
# Shared harness (echo-only variant): lib.sh (ST-6).
SUITE_NO_COUNT=1
. "$(dirname "$0")/lib.sh"

J=/tmp/td_jar.txt; rm -f $J
echo "-- detail page for product 2 (stock 3)"
curl -s -c $J -w "\nHTTP:%{http_code}\n" $BASE/products/2 -o /tmp/td_p2.html
grep -q "<title>Agentic Store | Axum マグカップ</title>" /tmp/td_p2.html && ok "tab title is product name" || ng "tab title"
grep -q "残りわずか（あと3点）" /tmp/td_p2.html && ok "stock line with real count" || ng "stock line"
grep -q "商品説明" /tmp/td_p2.html && grep -q "マグカップです" /tmp/td_p2.html && ok "description section" || ng "description"
grep -q "同じカテゴリーの商品" /tmp/td_p2.html && ok "related section" || ng "related section"
grep -q 'href="/products/2"' /tmp/td_p2.html && ng "related excludes itself" || ok "related excludes itself"
# related cards have their own add buttons: count detail links
# Breadcrumbs render client-side (breadcrumbs.js owns the
# route table); the server contract is the empty mount nav.
grep -q 'data-breadcrumb' /tmp/td_p2.html && ok "breadcrumb mount" || ng "breadcrumb mount"

echo "-- 404 behaviour"
CODE=$(curl -s -o /tmp/td_404.html -w "%{http_code}" $BASE/products/99999)
[ "$CODE" = "404" ] && grep -q "商品が見つかりませんでした" /tmp/td_404.html && ok "unknown id -> 404 page" || ng "unknown id -> $CODE"
CODE=$(curl -s -o /tmp/td_404b.html -w "%{http_code}" $BASE/products/abc)
[ "$CODE" = "404" ] && ok "non-numeric id -> 404" || ng "non-numeric -> $CODE"

echo "-- sold-out product 3"
curl -s -b $J $BASE/products/3 -o /tmp/td_p3.html
grep -q "売り切れ" /tmp/td_p3.html && grep -q "disabled" /tmp/td_p3.html && ok "sold-out detail state" || ng "sold out state"

echo "-- preorder product 8"
curl -s -b $J $BASE/products/8 -o /tmp/td_p8.html
grep -q "予約受付中" /tmp/td_p8.html && ok "preorder label" || ng "preorder label"

echo "-- removed product 6: now a styled 404"
CODE=$(curl -s -o /tmp/td_p6.html -w "%{http_code}" $BASE/products/6)
[ "$CODE" = "404" ] && grep -q "商品が見つかりませんでした" /tmp/td_p6.html && ok "removed product -> 404 page" || ng "removed product -> $CODE"

echo "-- detail add with quantity 2 (product 2, stock 3)"
C=$(csrf /tmp/td_p2.html)
curl -s -b $J -c $J -X POST $BASE/cart/items --data-urlencode "product_id=2" --data-urlencode "quantity=2" --data-urlencode "source=detail" --data-urlencode "csrf_token=$C" -o /tmp/td_add.html
grep -q "Axum マグカップをカートに追加しました（2点）" /tmp/td_add.html && ok "detail add toast shows quantity" || ng "detail add toast shows quantity"
curl -s -b $J $BASE/cart -o /tmp/td_cart.html
grep -q "Axum マグカップ" /tmp/td_cart.html && ok "cart has the mug" || ng "cart has mug"
grep -q '<span class="qty-value">2</span>' /tmp/td_cart.html && ok "quantity 2 from detail" || ng "quantity from detail"

echo "-- guest vs member body identical (fresh guest vs member)"
J2=/tmp/td_m.txt; rm -f $J2
curl -s -c $J2 $BASE/login -o /tmp/td_login.html
curl -s -b $J2 -c $J2 -o /dev/null -X POST $BASE/login --data-urlencode "email=test@example.com" --data-urlencode "password=test1234" --data-urlencode "csrf_token=$(csrf /tmp/td_login.html)"
curl -s -b $J2 $BASE/products/2 -o /tmp/td_p2m.html
# The buybox favourite toggle differs by design (guests get a
# login link, members the toggle form), so its line is masked
# out of both bodies before the identical-body comparison.
sed -n '/product-detail/,/同じカテゴリー/p' /tmp/td_p2.html | sed 's/name="csrf_token" value="[^"]*"/name="csrf_token" value="MASKED"/' | sed '/buybox-favorite/d; /favorite-toggle/d' > /tmp/td_g_body.txt
sed -n '/product-detail/,/同じカテゴリー/p' /tmp/td_p2m.html | sed 's/name="csrf_token" value="[^"]*"/name="csrf_token" value="MASKED"/' | sed '/buybox-favorite/d; /favorite-toggle/d' > /tmp/td_m_body.txt
diff -q /tmp/td_g_body.txt /tmp/td_m_body.txt >/dev/null && ok "guest/member detail body identical" || ng "guest/member body differs"

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

echo "-- variants: picker, quick-add fragment, forged variant, per-combination cap"
make_variant_fixture 'バリエーション確認商品'
curl -s -b $J $BASE/products/$FVP -o /tmp/td_var.html
grep -q 'data-variant-picker' /tmp/td_var.html && ok "detail renders the variant picker" || ng "detail renders the variant picker"
grep -q 'variant-group-name">サイズ' /tmp/td_var.html && grep -q 'variant-group-name">カラー' /tmp/td_var.html \
  && ok "picker groups are headed by the axis names" || ng "picker groups are headed by the axis names"
grep -q 'data-variant-data' /tmp/td_var.html && ok "picker data block is embedded" || ng "picker data block is embedded"
curl -s -b $J $BASE/products/$FVP/quick-add -o /tmp/td_qa.html
grep -q 'data-variant-picker' /tmp/td_qa.html && grep -q 'バリエーション確認商品' /tmp/td_qa.html \
  && ok "quick-add fragment returns the picker for a variant product" || ng "quick-add fragment returns the picker"
T=$(csrf /tmp/td_var.html)
V2=$(dbq "SELECT id FROM product_variants WHERE product_id = 2")
CODE=$(curl -s -b $J -o /dev/null -w "%{http_code}" -X POST $BASE/cart/items \
  --data-urlencode "product_id=$FVP" --data-urlencode "variant_id=$V2" --data-urlencode "quantity=1" \
  --data-urlencode "csrf_token=$T")
[ "$CODE" = "404" ] && ok "forged variant from another product is a 404" || ng "forged variant from another product is a 404 ($CODE)"
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items \
  --data-urlencode "product_id=$FVP" --data-urlencode "variant_id=$FVSB" --data-urlencode "quantity=99" \
  --data-urlencode "csrf_token=$T"
Q=$(dbq "SELECT quantity FROM cart_items WHERE variant_id = $FVSB")
[ "$Q" = "2" ] && ok "add is capped at the combination's stock (99 -> 2)" || ng "add is capped at the combination's stock (got $Q)"
curl -s -b $J $BASE/cart -o /tmp/td_vcart.html
grep -q "サイズ: S / カラー: ブラック" /tmp/td_vcart.html && ok "cart line shows the combination label" || ng "cart line shows the combination label"
