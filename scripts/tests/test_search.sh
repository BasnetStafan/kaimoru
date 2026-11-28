#!/bin/bash
# Verifies the Product Search Spec implementation.
BASE=http://127.0.0.1:3000
# Shared harness: ok/ng, csrf() and result() live in lib.sh (ST-6).
. "$(dirname "$0")/lib.sh"

J=/tmp/search_jar.txt; rm -f $J

echo "-- header search box"
curl -s -c $J $BASE/ -o /tmp/s_home.html
grep -q 'class="search-form"' /tmp/s_home.html && ok "search form in header" || ng "search form in header"
grep -q 'hx-get="/search/suggest"' /tmp/s_home.html && ok "htmx suggest wiring" || ng "htmx suggest wiring"
grep -q 'href="/search?category=' /tmp/s_home.html && ok "category nav reuses /search" || ng "category nav reuses /search"

echo "-- keyword search (guest)"
curl -s "$BASE/search?q=%E3%82%B7%E3%83%A3%E3%83%84" -o /tmp/s_q.html
grep -q "の検索結果" /tmp/s_q.html && ok "results heading" || ng "results heading"
grep -q "Tシャツ" /tmp/s_q.html && ok "shirt product found" || ng "shirt product found"

echo "-- wildcard is literal, not a pattern"
curl -s "$BASE/search?q=%25" -o /tmp/s_pct.html
grep -q "見つかりませんでした" /tmp/s_pct.html && ok "% returns zero results" || ng "% returns zero results"

echo "-- category filter"
curl -s "$BASE/search?category=%E3%82%B0%E3%83%83%E3%82%BA" -o /tmp/s_cat.html
grep -q "グッズ" /tmp/s_cat.html && ok "category page shows matches" || ng "category page shows matches"

echo "-- sort by price asc: prices in ascending order"
curl -s "$BASE/search?sort=price_asc" -o /tmp/s_sort.html
PRICES=$(grep -o '￥[0-9,]*' /tmp/s_sort.html | head -8 | tr -d '￥,' | tr '\n' ' ')
SORTED=$(echo $PRICES | tr ' ' '\n' | sort -n | tr '\n' ' ')
[ "$PRICES" = "$SORTED" ] && ok "prices ascending ($PRICES)" || ng "prices ascending ($PRICES vs $SORTED)"

echo "-- suggestions"
curl -s "$BASE/search/suggest?q=%E3%83%9E%E3%82%B0" -o /tmp/s_sug.html
grep -q "マグカップ" /tmp/s_sug.html && ok "suggest finds mug" || ng "suggest finds mug"
OUT=$(curl -s "$BASE/search/suggest?q=%E3%81%82")
[ -z "$OUT" ] && ok "1-char suggest returns empty" || ng "1-char suggest returns empty"

echo "-- guest and member see identical results"
curl -s -c $J $BASE/login -o /tmp/s_login.html
CSRF=$(csrf /tmp/s_login.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/login --data-urlencode "email=test@example.com" --data-urlencode "password=test1234" --data-urlencode "csrf_token=$CSRF"
grep -o "<h3>[^<]*</h3>" /tmp/s_q.html | sort > /tmp/s_guest_names.txt
curl -s -b $J "$BASE/search?q=%E3%82%B7%E3%83%A3%E3%83%84" | grep -o "<h3>[^<]*</h3>" | sort > /tmp/s_member_names.txt
diff -q /tmp/s_guest_names.txt /tmp/s_member_names.txt >/dev/null && ok "identical results guest/member" || ng "identical results guest/member"

echo "-- latency (results budget 200ms server side)"
T=$(curl -s -o /dev/null -w "%{time_total}" "$BASE/search?q=%E3%82%B7%E3%83%A3%E3%83%84")
echo "  total time: ${T}s"
awk "BEGIN{exit !($T < 0.5)}" && ok "search responds fast locally" || ng "search responds fast locally"

echo "-- suggest rate limit (30/min/IP)"
NONEMPTY=0
for i in $(seq 1 40); do
  R=$(curl -s "$BASE/search/suggest?q=%E3%82%B3%E3%83%BC")
  [ -n "$R" ] && NONEMPTY=$((NONEMPTY+1))
done
echo "  non-empty suggest responses out of 40 rapid calls: $NONEMPTY"
[ "$NONEMPTY" -le 31 ] && ok "rate limit engages" || ng "rate limit engages"


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

echo "-- variants: the card offers 選択する wired to the quick-add modal"
make_variant_fixture '検索バリエーション商品'
curl -s "$BASE/search?q=%E6%A4%9C%E7%B4%A2%E3%83%90%E3%83%AA%E3%82%A8%E3%83%BC%E3%82%B7%E3%83%A7%E3%83%B3" -o /tmp/s_var.html
grep -q '選択する' /tmp/s_var.html && ok "variant product card renders 選択する" || ng "variant product card renders 選択する"
grep -q "hx-get=\"/products/$FVP/quick-add\"" /tmp/s_var.html \
  && ok "選択する loads the quick-add fragment" || ng "選択する loads the quick-add fragment"

result
