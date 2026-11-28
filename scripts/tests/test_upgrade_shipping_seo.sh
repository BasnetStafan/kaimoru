#!/bin/bash
# Product-upgrade verification: shipping integration, the
# publication window and SEO/slug behaviour (Phases 6 and 8).
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -q -t -A -c "$1"; }
CT=$(printf "csrf_%s" "token")
IMG=$(db "SELECT image_path FROM products WHERE id = 2")
CAT_ID=$(db "SELECT id FROM categories ORDER BY id LIMIT 1")

mk_product() { # name price extra_sql -> id
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
guest_buy() { # jarfile pid email prefecture -> confirm page at /tmp/us_confirm.html, then places
  rm -f $1
  curl -s -c $1 $BASE/ -o /tmp/us_home.html; local C=$(csrf /tmp/us_home.html)
  curl -s -b $1 -c $1 -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$2" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
  curl -s -b $1 $BASE/checkout -o /tmp/us_co.html; C=$(csrf /tmp/us_co.html)
  curl -s -b $1 -c $1 -X POST $BASE/checkout --data-urlencode "intent=confirm" \
    --data-urlencode "email=$3" --data-urlencode "family_name=配送" --data-urlencode "given_name=検" \
    --data-urlencode "postal_code=9000001" --data-urlencode "prefecture=$4" --data-urlencode "city=検証市" \
    --data-urlencode "address_line=配送1-1-1" --data-urlencode "building=" --data-urlencode "phone=08011112222" \
    --data-urlencode "$CT=$C" -o /tmp/us_step.html
  curl -s -b $1 -c $1 -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "$CT=$C" -o /tmp/us_confirm.html
}

echo "== all-free-shipping cart pays no shipping =="
P_FREE=$(mk_product "検証 アップグレード 送料無料" 1000 "free_shipping = true, undeliverable_regions = '', publish_starts_at = NULL, publish_ends_at = NULL, slug = NULL")
guest_buy /tmp/us_g1.txt $P_FREE shipfree@example.jp 東京都
grep -q "￥1,330" /tmp/us_confirm.html && ok "free-shipping total is 1000 + 0 + 330 COD" || ng "free-shipping total"
grep -q "￥550" /tmp/us_confirm.html && ng "flat shipping still charged" || ok "no flat shipping on an all-free cart"

echo "== undeliverable prefecture is blocked =="
P_UND=$(mk_product "検証 アップグレード 配送不可" 1000 "free_shipping = false, undeliverable_regions = '沖縄県'")
PRE_O=$(db "SELECT count(*) FROM orders WHERE email = 'undeliverable@example.jp'")
guest_buy /tmp/us_g2.txt $P_UND undeliverable@example.jp 沖縄県
C=$(csrf /tmp/us_confirm.html)
curl -s -b /tmp/us_g2.txt -c /tmp/us_g2.txt -X POST $BASE/checkout/confirm --data-urlencode "$CT=$C" -o /tmp/us_place.html
grep -q "配送できません" /tmp/us_place.html && ok "checkout refuses the undeliverable prefecture" || ng "undeliverable block"
POST_O=$(db "SELECT count(*) FROM orders WHERE email = 'undeliverable@example.jp'")
[ "$PRE_O" = "$POST_O" ] && ok "no order was placed for the blocked checkout" || ng "blocked checkout placed an order"

echo "== publication window hides the product everywhere =="
P_WIN=$(mk_product "検証 アップグレード 公開期間" 1000 "publish_starts_at = now() + interval '1 day', publish_ends_at = NULL")
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/products/$P_WIN)
[ "$CODE" = "404" ] && ok "future-window product 404s on the detail URL" || ng "future-window detail ($CODE)"
curl -s "$BASE/search?q=公開期間" -o /tmp/us_search.html
grep -q "検証 アップグレード 公開期間" /tmp/us_search.html && ng "future-window product listed in search" || ok "future-window product absent from search"
db "UPDATE products SET publish_starts_at = NULL, publish_ends_at = now() - interval '1 day' WHERE id = $P_WIN" >/dev/null
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/products/$P_WIN)
[ "$CODE" = "404" ] && ok "expired-window product 404s too" || ng "expired-window detail ($CODE)"
db "UPDATE products SET publish_ends_at = NULL WHERE id = $P_WIN" >/dev/null
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/products/$P_WIN)
[ "$CODE" = "200" ] && ok "clearing the window restores visibility" || ng "window cleared visibility ($CODE)"

echo "== slug URL and per-product SEO =="
P_SEO=$(mk_product "検証 アップグレード SEO" 1000 "slug = 'kensyo-seo-item', seo_title = '検証SEOタイトル', meta_description = '検証用のメタ説明文です。'")
CODE=$(curl -s -o /tmp/us_slug.html -w "%{http_code}" $BASE/products/kensyo-seo-item)
[ "$CODE" = "200" ] && ok "slug URL resolves" || ng "slug URL resolves ($CODE)"
grep -q "検証 アップグレード SEO" /tmp/us_slug.html && ok "slug page shows the product" || ng "slug page content"
grep -q "<title>検証SEOタイトル</title>" /tmp/us_slug.html && ok "seo_title emitted" || ng "seo_title emitted"
grep -q 'content="検証用のメタ説明文です。"' /tmp/us_slug.html && ok "meta description emitted" || ng "meta description emitted"
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/products/$P_SEO)
[ "$CODE" = "200" ] && ok "id URL stays canonical and works" || ng "id URL works ($CODE)"

result
