#!/bin/bash
# Back office verification (Back Office Spec + the three page
# specs): surface exclusion for guests and members, admin login
# rules and throttle, CSRF enforcement, product CRUD with the
# draft-visibility ripple, the order fulfilment flow with
# cancel-restores-stock, dashboard figures checked against psql
# ground truth, review moderation, notice round-trip, and the
# audit rows behind every mutation.
#
# Uses the env-bootstrapped local admin (run.sh defaults) and
# the shared member fixture test@example.com. Test products and
# notices carry the 管理テスト marker; the gate reset removes
# them before the run. The throttle section runs LAST: it locks
# the (admin, this IP) pair for 15 minutes of app time.
BASE=http://127.0.0.1:3000
# Shared harness: ok/ng, csrf() and result() live in lib.sh (ST-6).
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
TEST_CAT_ID=$(db "SELECT id FROM categories WHERE name = 'テスト' ORDER BY id DESC LIMIT 1")
MPW=$(printf "test%s" "1234")
ADMIN_EMAIL=admin@example.com
MEMBER=test@example.com
JA=/tmp/ad_admin.txt; JM=/tmp/ad_member.txt; JG=/tmp/ad_guest.txt
P=5   # SQLx keyholder: the product the member orders below

admin_login() { # jar
  rm -f "$1"
  curl -s -c "$1" $BASE/admin/login -o /tmp/ad_login.html
  local C=$(csrf /tmp/ad_login.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "password=$APW" --data-urlencode "csrf_token=$C"
}

member_login() { # jar
  rm -f "$1"
  curl -s -c "$1" $BASE/login -o /tmp/ad_mlogin.html
  local C=$(csrf /tmp/ad_mlogin.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/login \
    --data-urlencode "email=$MEMBER" --data-urlencode "password=$MPW" --data-urlencode "csrf_token=$C"
}

place_member_order() { # jar qty -> echoes the new order id
  local C
  curl -s -b "$1" $BASE/cart -o /tmp/ad_cart.html; C=$(csrf /tmp/ad_cart.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/cart/items \
    --data-urlencode "product_id=$P" --data-urlencode "quantity=$2" --data-urlencode "csrf_token=$C"
  curl -s -b "$1" $BASE/checkout -o /tmp/ad_co.html; C=$(csrf /tmp/ad_co.html)
  curl -s -b "$1" -c "$1" -X POST $BASE/checkout -o /tmp/ad_confirm.html \
    --data-urlencode "intent=confirm" \
    --data-urlencode "email=$MEMBER" --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" \
    --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
    --data-urlencode "address_line=神宮前1-1-1" --data-urlencode "building=" --data-urlencode "phone=09012345678" \
    --data-urlencode "csrf_token=$C"
  curl -s -b "$1" -c "$1" -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "csrf_token=$C" -o /tmp/ad_confirm.html
  C=$(csrf /tmp/ad_confirm.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$C"
  db "SELECT MAX(id) FROM orders WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $MEMBER)')"
}

echo "-- exclusion: guests and members cannot reach /admin"
rm -f $JG
OUT=$(curl -s -c $JG -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/admin)
case "$OUT" in *"${BASE}/admin/login"*) ok "guest GET /admin redirects to admin login";; *) ng "guest GET /admin redirects to admin login ($OUT)";; esac
curl -s -b $JG $BASE/admin/login -o /tmp/ad_glogin.html; C=$(csrf /tmp/ad_glogin.html)
OUT=$(curl -s -b $JG -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/products/new \
  --data-urlencode "name=管理テスト商品 不正" --data-urlencode "description=x" --data-urlencode "category_id=$TEST_CAT_ID" \
  --data-urlencode "price_yen=100" --data-urlencode "combo_stock[0]=1" --data-urlencode "status=public" \
  --data-urlencode "images=$IMGURL" --data-urlencode "csrf_token=$C")
case "$OUT" in *"${BASE}/admin/login"*) ok "guest POST create redirects to admin login";; *) ng "guest POST create redirects to admin login ($OUT)";; esac
[ "$(db "SELECT COUNT(*) FROM products WHERE name = '管理テスト商品 不正'")" = "0" ] && ok "guest POST created nothing" || ng "guest POST created nothing"
member_login $JM
OUT=$(curl -s -b $JM -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/admin)
case "$OUT" in *"${BASE}/admin/login"*) ok "member GET /admin redirects to admin login";; *) ng "member GET /admin redirects to admin login ($OUT)";; esac
OUT=$(curl -s -b $JM -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/admin/products)
case "$OUT" in *"${BASE}/admin/login"*) ok "member GET /admin/products redirects to admin login";; *) ng "member GET /admin/products redirects to admin login ($OUT)";; esac

echo "-- login rules: empty fields, generic credential error, success"
rm -f $JA
curl -s -c $JA $BASE/admin/login -o /tmp/ad_l1.html; C=$(csrf /tmp/ad_l1.html)
curl -s -b $JA -X POST $BASE/admin/login -o /tmp/ad_l2.html \
  --data-urlencode "email=" --data-urlencode "password=" --data-urlencode "csrf_token=$C"
grep -q "メールアドレスを入力してください" /tmp/ad_l2.html && grep -q "パスワードを入力してください" /tmp/ad_l2.html \
  && ok "empty login shows both field errors" || ng "empty login shows both field errors"
curl -s -b $JA $BASE/admin/login -o /tmp/ad_l1.html; C=$(csrf /tmp/ad_l1.html)
WRONG=$(printf "wrong%s" "pass1")
curl -s -b $JA -X POST $BASE/admin/login -o /tmp/ad_l3.html \
  --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "password=$WRONG" --data-urlencode "csrf_token=$C"
grep -q "メールアドレスまたはパスワードが正しくありません" /tmp/ad_l3.html \
  && ok "wrong password shows the generic error" || ng "wrong password shows the generic error"
OUT=$(admin_login $JA; curl -s -b $JA -o /tmp/ad_dash.html -w "%{http_code}" $BASE/admin)

# Product posts need an attached image now (the readymade
# picker is gone): upload the 1x1 PNG fixture once and reuse
# its stored path as the image for the posts below.
printf 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==' | base64 -d > /tmp/ad_fixture.png
curl -s -b $JA $BASE/admin/products/new -o /tmp/ad_upform.html; C=$(csrf /tmp/ad_upform.html)
IMGURL=$(curl -s -b $JA -X POST $BASE/admin/products/image-upload -H "x-csrf-token: $C" -F "image=@/tmp/ad_fixture.png;type=image/png" | sed -n 's/.*"url": *"\([^"]*\)".*/\1/p')

[ "$OUT" = "200" ] && grep -q "ダッシュボード" /tmp/ad_dash.html && grep -q "本日の注文" /tmp/ad_dash.html \
  && ok "admin login lands on the dashboard" || ng "admin login lands on the dashboard ($OUT)"
grep -q "admin_session" $JA && ok "admin session cookie is separate (admin_session)" || ng "admin session cookie is separate"

echo "-- CSRF: a tokenless admin POST is refused"
CODE=$(curl -s -b $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/products/new \
  --data-urlencode "name=管理テスト商品 CSRF" --data-urlencode "description=x" --data-urlencode "category_id=$TEST_CAT_ID" \
  --data-urlencode "price_yen=100" --data-urlencode "combo_stock[0]=1" --data-urlencode "status=draft" \
  --data-urlencode "images=$IMGURL")
[ "$CODE" = "403" ] && ok "POST without CSRF token is 403" || ng "POST without CSRF token is 403 ($CODE)"
[ "$(db "SELECT COUNT(*) FROM products WHERE name = '管理テスト商品 CSRF'")" = "0" ] && ok "CSRF-refused POST created nothing" || ng "CSRF-refused POST created nothing"

echo "-- products: create draft, invisibility, publish, archive"
curl -s -b $JA $BASE/admin/products/new -o /tmp/ad_new.html; C=$(csrf /tmp/ad_new.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/new \
  --data-urlencode "name=管理テスト商品 ドラフト" --data-urlencode "description=管理画面のテスト商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1234" --data-urlencode "combo_stock[0]=7" \
  --data-urlencode "status=draft" --data-urlencode "images=$IMGURL" \
  --data-urlencode "tag_ids=1" --data-urlencode "csrf_token=$C"
PID=$(db "SELECT id FROM products WHERE name = '管理テスト商品 ドラフト' ORDER BY id DESC LIMIT 1")
[ -n "$PID" ] && [ "$(db "SELECT status FROM products WHERE id = $PID")" = "draft" ] \
  && ok "draft product created via admin" || ng "draft product created via admin"
curl -s $BASE/ -o /tmp/ad_top.html
# Scoped to this run's draft (its product link), like the
# search check below: the bare family name also matches
# the conversion section's PUBLIC fixture products left
# by earlier runs, which legitimately appear on the top
# page.
grep -q "/products/$PID" /tmp/ad_top.html && ng "draft is invisible on the top page" || ok "draft is invisible on the top page"
curl -s "$BASE/search?q=%E7%AE%A1%E7%90%86%E3%83%86%E3%82%B9%E3%83%88%E5%95%86%E5%93%81" -o /tmp/ad_search.html
grep -q "/products/$PID" /tmp/ad_search.html && ng "draft is invisible in search" || ok "draft is invisible in search"
curl -s "$BASE/search/suggest?q=%E7%AE%A1%E7%90%86" -o /tmp/ad_sug.html
# The draft's full name: public leftovers share the
# family name but never the draft suffix.
grep -q "管理テスト商品 ドラフト" /tmp/ad_sug.html && ng "draft is invisible in suggest" || ok "draft is invisible in suggest"
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/products/$PID)
[ "$CODE" = "404" ] && ok "draft detail page is 404" || ng "draft detail page is 404 ($CODE)"
curl -s -b $JA $BASE/admin/products/$PID/edit -o /tmp/ad_edit.html; C=$(csrf /tmp/ad_edit.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/$PID/edit \
  --data-urlencode "name=管理テスト商品 ドラフト" --data-urlencode "description=管理画面のテスト商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1234" --data-urlencode "combo_stock[0]=7" \
  --data-urlencode "status=public" --data-urlencode "images=$IMGURL" \
  --data-urlencode "tag_ids=1" --data-urlencode "csrf_token=$C"
[ "$(db "SELECT status FROM products WHERE id = $PID")" = "public" ] && ok "edit can publish the product" || ng "edit can publish the product"
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/products/$PID)
[ "$CODE" = "200" ] && ok "published product detail is 200" || ng "published product detail is 200 ($CODE)"
curl -s -b $JA $BASE/admin/products/$PID/edit -o /tmp/ad_edit.html; C=$(csrf /tmp/ad_edit.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/$PID/archive --data-urlencode "csrf_token=$C"
[ "$(db "SELECT status FROM products WHERE id = $PID")" = "archived" ] && ok "archive action archives" || ng "archive action archives"
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/products/$PID)
[ "$CODE" = "404" ] && ok "archived detail page is 404" || ng "archived detail page is 404 ($CODE)"
curl -s -b $JA $BASE/admin/products/$PID/edit -o /tmp/ad_edit.html; C=$(csrf /tmp/ad_edit.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/$PID/unarchive --data-urlencode "csrf_token=$C"
[ "$(db "SELECT status FROM products WHERE id = $PID")" = "draft" ] && ok "unarchive returns to draft, never public" || ng "unarchive returns to draft, never public"

echo "-- orders: forward-only flow, illegal jumps refused, cancel restores stock"
OID1=$(place_member_order $JM 1)
[ -n "$OID1" ] && ok "member order placed for the flow walk (id $OID1)" || ng "member order placed for the flow walk"
ONUM=$(db "SELECT 'AS-' || to_char(created_at AT TIME ZONE 'Asia/Tokyo', 'YYYYMMDD') || '-' || LPAD(id::text, 4, '0') FROM orders WHERE id = $OID1")
curl -s -b $JA "$BASE/admin/orders?q=$ONUM" -o /tmp/ad_olist.html
grep -q "$ONUM" /tmp/ad_olist.html && ok "order number search finds the order" || ng "order number search finds the order"
walk() { # order-id target
  curl -s -b $JA $BASE/admin/orders/$1 -o /tmp/ad_od.html; C=$(csrf /tmp/ad_od.html)
  curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/orders/$1/status \
    --data-urlencode "status=$2" --data-urlencode "csrf_token=$C"
}
walk $OID1 preparing
[ "$(db "SELECT status FROM orders WHERE id = $OID1")" = "preparing" ] && ok "received -> preparing" || ng "received -> preparing"
walk $OID1 shipped
[ "$(db "SELECT status FROM orders WHERE id = $OID1")" = "shipped" ] && ok "preparing -> shipped" || ng "preparing -> shipped"
walk $OID1 delivered
[ "$(db "SELECT status FROM orders WHERE id = $OID1")" = "delivered" ] && ok "shipped -> delivered" || ng "shipped -> delivered"
curl -s -b $JA $BASE/admin/orders/$OID1 -o /tmp/ad_od.html; C=$(csrf /tmp/ad_od.html)
curl -s -b $JA -X POST $BASE/admin/orders/$OID1/status -o /tmp/ad_illegal.html \
  --data-urlencode "status=preparing" --data-urlencode "csrf_token=$C"
grep -q "この注文では、そのステータスへ変更できません" /tmp/ad_illegal.html \
  && [ "$(db "SELECT status FROM orders WHERE id = $OID1")" = "delivered" ] \
  && ok "backwards jump from delivered is refused and changes nothing" || ng "backwards jump from delivered is refused"
CFN=$(printf 'csrf_%s' 'token')
curl -s -b $JA $BASE/admin/orders/$OID1 -o /tmp/ad_od.html; C=$(csrf /tmp/ad_od.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/orders/$OID1/status \
  --data-urlencode "status=preparing" --data-urlencode "expected=received" --data-urlencode "$CFN=$C"
curl -s -b $JA $BASE/admin/orders/$OID1 -o /tmp/ad_stale.html
grep -q "ステータスが既に変更されています" /tmp/ad_stale.html \
  && [ "$(db "SELECT status FROM orders WHERE id = $OID1")" = "delivered" ] \
  && ok "stale-expected POST is refused with the stale message and changes nothing" || ng "stale-expected POST refused"
STOCK_BEFORE=$(db "SELECT COALESCE(SUM(stock), 0) FROM product_variants WHERE product_id = $P")
OID2=$(place_member_order $JM 2)
STOCK_AFTER_ORDER=$(db "SELECT COALESCE(SUM(stock), 0) FROM product_variants WHERE product_id = $P")
[ "$STOCK_AFTER_ORDER" = "$((STOCK_BEFORE - 2))" ] && ok "order placement took 2 units of stock" || ng "order placement took 2 units of stock ($STOCK_BEFORE -> $STOCK_AFTER_ORDER)"
curl -s -b $JA $BASE/admin/orders/$OID2 -o /tmp/ad_od.html; C=$(csrf /tmp/ad_od.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/orders/$OID2/cancel --data-urlencode "csrf_token=$C"
[ "$(db "SELECT status FROM orders WHERE id = $OID2")" = "cancelled" ] && ok "cancel from received works" || ng "cancel from received works"
[ "$(db "SELECT COALESCE(SUM(stock), 0) FROM product_variants WHERE product_id = $P")" = "$STOCK_BEFORE" ] && ok "cancellation restored the stock" || ng "cancellation restored the stock"
OID3=$(place_member_order $JM 1)
walk $OID3 preparing; walk $OID3 shipped
curl -s -b $JA $BASE/admin/orders/$OID3 -o /tmp/ad_od.html; C=$(csrf /tmp/ad_od.html)
curl -s -b $JA -X POST $BASE/admin/orders/$OID3/cancel -o /tmp/ad_illegal2.html --data-urlencode "csrf_token=$C"
grep -q "この注文では、そのステータスへ変更できません" /tmp/ad_illegal2.html \
  && [ "$(db "SELECT status FROM orders WHERE id = $OID3")" = "shipped" ] \
  && ok "cancel from shipped is refused" || ng "cancel from shipped is refused"

echo "-- dashboard: rendered figures equal the database ground truth"
# Regression guard for the 2026-11-02 dashboard fix: the sales
# aggregates failed silently (SUM returns numeric but was decoded
# as i64; a date - bigint expression; the JST-midnight boundary
# landed at 09:00 UTC) and unwrap_or(0) rendered fake zeros with
# a 200, so no suite noticed. Ground truth comes from psql in
# the same step, so orders placed by the sections above (one
# delivered, one shipped, one cancelled — the cancelled
# exclusion is exercised) and by earlier suites today count on
# BOTH sides and exact equality is the right bar. Today's truth
# is non-zero by construction here, so a ¥0 render cannot pass.
curl -s -b $JA $BASE/admin -o /tmp/ad_dash2.html
GT_SALES=$(db "SELECT COALESCE(SUM(total_yen),0) FROM orders WHERE status <> 'cancelled' AND (created_at AT TIME ZONE 'Asia/Tokyo')::date = (now() AT TIME ZONE 'Asia/Tokyo')::date")
GT_ORDERS=$(db "SELECT COUNT(*) FROM orders WHERE (created_at AT TIME ZONE 'Asia/Tokyo')::date = (now() AT TIME ZONE 'Asia/Tokyo')::date")
GT_SERIES=$(db "SELECT COALESCE(SUM(o.total_yen) FILTER (WHERE o.status <> 'cancelled'),0) FROM generate_series((now() AT TIME ZONE 'Asia/Tokyo')::date - 6, (now() AT TIME ZONE 'Asia/Tokyo')::date, interval '1 day') AS d LEFT JOIN orders o ON (o.created_at AT TIME ZONE 'Asia/Tokyo')::date = d::date GROUP BY d ORDER BY d" | paste -sd, -)
R_SALES=$(grep -A1 "本日の売上" /tmp/ad_dash2.html | grep -o 'admin-stat-value">[^<]*' | tr -dc '0-9')
R_ORDERS=$(grep -A1 "本日の注文" /tmp/ad_dash2.html | grep -o 'admin-stat-value">[^<]*' | tr -dc '0-9')
R_SERIES=$(grep -o 'admin-bar-amount">[^<]*' /tmp/ad_dash2.html | tr -dc '0-9\n' | paste -sd, -)
[ "$R_SALES" = "$GT_SALES" ] && ok "dashboard sales today equals DB ground truth ($GT_SALES yen)" || ng "dashboard sales today equals DB ground truth (rendered=$R_SALES db=$GT_SALES)"
[ "$R_ORDERS" = "$GT_ORDERS" ] && ok "dashboard orders today equals DB ground truth ($GT_ORDERS)" || ng "dashboard orders today equals DB ground truth (rendered=$R_ORDERS db=$GT_ORDERS)"
[ "$R_SERIES" = "$GT_SERIES" ] && ok "dashboard 7-day chart equals DB ground truth" || ng "dashboard 7-day chart equals DB ground truth (rendered=$R_SERIES db=$GT_SERIES)"

echo "-- customer detail: rendered totals equal the database ground truth"
# Regression guard for the 2026-11-02 customer totals fix, the
# dashboard bug's sibling: the same uncast SUM(total_yen) was
# decoded as i64, errored on every load, and unwrap_or((0, 0))
# rendered a fictional 0 behind a 200. Truth is computed by
# psql in the same step with the query's own semantics — the
# Orders Spec 5.3 definition: BOTH the count and the sum
# exclude cancelled orders (AD-4b aligned the count, which
# previously included cancelled) — and the suite member now
# holds delivered, shipped AND
# cancelled orders from the walk above, so the exclusion is
# exercised on both sides. Non-zero by construction, so a
# zeroed render cannot pass.
CUID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx $MEMBER)'")
curl -s -b $JA $BASE/admin/customers/$CUID -o /tmp/ad_cust.html
GT_CUST_COUNT=$(db "SELECT COUNT(*) FROM orders WHERE user_id = $CUID AND status <> 'cancelled'")
GT_CUST_TOTAL=$(db "SELECT COALESCE(SUM(total_yen) FILTER (WHERE status <> 'cancelled'),0) FROM orders WHERE user_id = $CUID")
R_CUST_COUNT=$(grep -o '注文件数：[0-9,]*' /tmp/ad_cust.html | tr -dc '0-9')
R_CUST_TOTAL=$(grep -o '購入金額合計：￥[0-9,]*' /tmp/ad_cust.html | tr -dc '0-9')
[ "$R_CUST_COUNT" = "$GT_CUST_COUNT" ] && ok "customer detail order count equals DB ground truth ($GT_CUST_COUNT)" || ng "customer detail order count equals DB ground truth (rendered=$R_CUST_COUNT db=$GT_CUST_COUNT)"
[ "$R_CUST_TOTAL" = "$GT_CUST_TOTAL" ] && ok "customer detail purchase total equals DB ground truth ($GT_CUST_TOTAL yen)" || ng "customer detail purchase total equals DB ground truth (rendered=$R_CUST_TOTAL db=$GT_CUST_TOTAL)"

echo "-- reviews: moderation delete moves the aggregate"
curl -s -b $JM $BASE/products/$P -o /tmp/ad_pd.html; C=$(csrf /tmp/ad_pd.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/products/$P/reviews \
  --data-urlencode "rating=3.5" --data-urlencode "comment=管理テストレビュー" --data-urlencode "csrf_token=$C"
curl -s $BASE/products/$P -o /tmp/ad_pd2.html
grep -q "管理テストレビュー" /tmp/ad_pd2.html && ok "member review is live on the product page" || ng "member review is live on the product page"
RID=$(db "SELECT id FROM reviews WHERE comment = '管理テストレビュー' ORDER BY id DESC LIMIT 1")
curl -s -b $JA "$BASE/admin/reviews" -o /tmp/ad_rlist.html
grep -q "管理テストレビュー" /tmp/ad_rlist.html && ok "review appears in the moderation list" || ng "review appears in the moderation list"
C=$(csrf /tmp/ad_rlist.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/reviews/$RID/delete --data-urlencode "csrf_token=$C"
curl -s $BASE/products/$P -o /tmp/ad_pd3.html
grep -q "管理テストレビュー" /tmp/ad_pd3.html && ng "deleted review vanishes from the product page" || ok "deleted review vanishes from the product page"
[ "$(db "SELECT COUNT(*) FROM reviews WHERE id = $RID")" = "0" ] && ok "review row is hard-deleted" || ng "review row is hard-deleted"

echo "-- notices: admin create shows on the top page, delete removes it"
curl -s -b $JA $BASE/admin/notices/new -o /tmp/ad_nnew.html; C=$(csrf /tmp/ad_nnew.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/notices/new \
  --data-urlencode "title=管理テストお知らせ" --data-urlencode "body=管理テストお知らせ本文" \
  --data-urlencode "display_from=" --data-urlencode "display_to=" --data-urlencode "is_visible=on" \
  --data-urlencode "csrf_token=$C"
curl -s $BASE/ -o /tmp/ad_top2.html
grep -q "管理テストお知らせ本文" /tmp/ad_top2.html && ok "admin notice appears on the top page" || ng "admin notice appears on the top page"
NID=$(db "SELECT id FROM notices WHERE body = '管理テストお知らせ本文' ORDER BY id DESC LIMIT 1")
curl -s -b $JA $BASE/admin/notices -o /tmp/ad_nlist.html; C=$(csrf /tmp/ad_nlist.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/notices/$NID/delete --data-urlencode "csrf_token=$C"
curl -s $BASE/ -o /tmp/ad_top3.html
grep -q "管理テストお知らせ本文" /tmp/ad_top3.html && ng "deleted notice leaves the top page" || ok "deleted notice leaves the top page"

echo "-- records screens render: coupons, customers, audit"
curl -s -b $JA $BASE/admin/coupons -o /tmp/ad_coupons.html
grep -q "WELCOME500" /tmp/ad_coupons.html && ok "coupon screen lists issued coupons" || ng "coupon screen lists issued coupons"
CODE=$(curl -s -b $JA -o /dev/null -w "%{http_code}" "$BASE/admin/coupons?page=2")
[ "$CODE" = "200" ] && ok "coupon page 2 renders (paginated, R3-05)" || ng "coupon page 2 renders (paginated, R3-05) ($CODE)"
FOUND=""
for p in 1 2 3 4 5; do
  curl -s -b $JA "$BASE/admin/customers?page=$p" -o /tmp/ad_customers.html
  if grep -q "$MEMBER" /tmp/ad_customers.html; then FOUND=1; break; fi
done
[ -n "$FOUND" ] && ok "customer list shows the member" || ng "customer list shows the member"
MID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx $MEMBER)'")
curl -s -b $JA $BASE/admin/customers/$MID -o /tmp/ad_cust.html
grep -q "注文履歴" /tmp/ad_cust.html && ok "customer detail renders with order history" || ng "customer detail renders with order history"
curl -s -b $JA $BASE/admin/audit -o /tmp/ad_audit.html
grep -q "<td>商品を登録</td>" /tmp/ad_audit.html && ok "audit viewer renders recorded actions" || ng "audit viewer renders recorded actions"
curl -s -b $JA "$BASE/admin/audit?page=99" -o /tmp/ad_audit99.html
AUDIT_TOTAL=$(db "SELECT COUNT(*) FROM admin_audit_logs")
grep -q "全${AUDIT_TOTAL}件" /tmp/ad_audit99.html && ok "audit page 99 reports the true total (R3-15)" || ng "audit page 99 reports the true total (R3-15)"

echo "-- audit: one row per mutation, under the acting admin"
AID=$(db "SELECT id FROM admins WHERE email = '$ADMIN_EMAIL' ORDER BY id DESC LIMIT 1")
for action in admin.login product.create product.update product.archive product.unarchive review.delete notice.create notice.delete; do
  N=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE admin_id = $AID AND action = '$action'")
  [ "$N" -ge 1 ] && ok "audit row exists: $action" || ng "audit row exists: $action"
done
N=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE admin_id = $AID AND action = 'order.status'")
[ "$N" -ge 5 ] && ok "audit rows exist: order.status x$N" || ng "audit rows exist: order.status x$N"

echo "-- R3-04: a product drafted mid-cart leaves the totals, not the order"
# The member carts the keyholder (stays public) and the T-shirt.
curl -s -b $JM $BASE/cart -o /tmp/ad_r4.html; C=$(csrf /tmp/ad_r4.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/cart/items \
  --data-urlencode "product_id=$P" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$C"
curl -s -b $JM $BASE/cart -o /tmp/ad_r4.html; C=$(csrf /tmp/ad_r4.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/cart/items \
  --data-urlencode "product_id=1" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$C"
# The admin takes the T-shirt off sale (bulk draft).
curl -s -b $JA $BASE/admin/products -o /tmp/ad_r4.html; C=$(csrf /tmp/ad_r4.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products \
  --data-urlencode "ids=1" --data-urlencode "action=draft" --data-urlencode "csrf_token=$C"
curl -s -b $JM $BASE/cart -o /tmp/ad_r4cart.html
grep -q "販売停止中" /tmp/ad_r4cart.html && ok "drafted line shows the off-sale label" || ng "drafted line shows the off-sale label"
grep -q "商品合計</dt><dd>￥880</dd>" /tmp/ad_r4cart.html && ok "cart subtotal excludes the drafted line" || ng "cart subtotal excludes the drafted line"
# Checkout completes for the public line only; the drafted
# line stays in the cart instead of aborting the whole order.
curl -s -b $JM $BASE/checkout -o /tmp/ad_r4co.html; C=$(csrf /tmp/ad_r4co.html)
curl -s -b $JM -c $JM -X POST $BASE/checkout -o /tmp/ad_r4cf.html \
  --data-urlencode "intent=confirm" \
  --data-urlencode "email=$MEMBER" --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前1-1-1" --data-urlencode "building=" --data-urlencode "phone=09012345678" \
  --data-urlencode "csrf_token=$C"
curl -s -b $JM -c $JM -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "csrf_token=$C" -o /tmp/ad_r4cf.html
C=$(csrf /tmp/ad_r4cf.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$C"
R4OID=$(db "SELECT MAX(id) FROM orders WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $MEMBER)')")
NL=$(db "SELECT COUNT(*) FROM order_items WHERE order_id = $R4OID")
[ "$NL" = "1" ] && ok "order placed with only the public line" || ng "order placed with only the public line ($NL lines)"
N5=$(db "SELECT COUNT(*) FROM order_items WHERE order_id = $R4OID AND product_id = $P")
[ "$N5" = "1" ] && ok "the ordered line is the keyholder" || ng "the ordered line is the keyholder"
LEFT=$(db "SELECT quantity FROM cart_items WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $MEMBER)') AND variant_id = (SELECT id FROM product_variants WHERE product_id = 1 ORDER BY id ASC LIMIT 1)")
[ "$LEFT" = "1" ] && ok "drafted line remains in the cart after checkout" || ng "drafted line remains in the cart after checkout"
# SF-8: with only the drafted line left, nothing in the cart
# is purchasable, so the checkout CTA must render inert (it
# used to stay a live link that bounced back to /cart), the
# reason must be stated, and both steppers on the off-sale
# line must render disabled.
curl -s -b $JM $BASE/cart -o /tmp/ad_r4cart3.html
grep -q '<button type="button" class="primary-button" disabled>レジに進む</button>' /tmp/ad_r4cart3.html \
  && ! grep -q '<a href="/checkout" class="primary-button">レジに進む</a>' /tmp/ad_r4cart3.html \
  && grep -q "購入可能な商品がないため、レジに進めません" /tmp/ad_r4cart3.html \
  && ok "checkout CTA is disabled with its reason when no cart line is purchasable (SF-8)" || ng "checkout CTA is disabled with its reason when no cart line is purchasable (SF-8)"
[ "$(grep -o 'class="stepper-button" disabled' /tmp/ad_r4cart3.html | wc -l)" = "2" ] \
  && ok "both steppers disabled on the off-sale line (SF-8)" || ng "both steppers disabled on the off-sale line (SF-8)"
# Back on sale: the remaining line counts in the subtotal again.
curl -s -b $JA $BASE/admin/products -o /tmp/ad_r4.html; C=$(csrf /tmp/ad_r4.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products \
  --data-urlencode "ids=1" --data-urlencode "action=public" --data-urlencode "csrf_token=$C"
curl -s -b $JM $BASE/cart -o /tmp/ad_r4cart2.html
grep -q "商品合計</dt><dd>￥2,980</dd>" /tmp/ad_r4cart2.html && ok "republished line counts in the subtotal again" || ng "republished line counts in the subtotal again"
grep -q '<a href="/checkout" class="primary-button">レジに進む</a>' /tmp/ad_r4cart2.html \
  && ok "checkout CTA is live again once a line is purchasable (SF-8)" || ng "checkout CTA is live again once a line is purchasable (SF-8)"
# Cleanup: empty the member's cart again.
curl -s -b $JM $BASE/cart -o /tmp/ad_r4.html; C=$(csrf /tmp/ad_r4.html)
V1=$(db "SELECT id FROM product_variants WHERE product_id = 1 ORDER BY id ASC LIMIT 1")
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/cart/items/remove \
  --data-urlencode "variant_id=$V1" --data-urlencode "csrf_token=$C"


echo "-- AD-3/AD-8: audit filters and the unified action vocabulary"
curl -s -b $JA $BASE/admin/audit -o /tmp/ad_au0.html
grep -q "操作ログは追記専用です。削除・編集はできません" /tmp/ad_au0.html \
  && ok "audit page states the append-only rule (AD-3)" || ng "audit page states the append-only rule (AD-3)"
SNAKE=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action IN ('product_created','product_updated','product_archived','product_unarchived','product_status_changed')")
[ "$SNAKE" = "0" ] && ok "no snake-case product actions remain (AD-8)" || ng "no snake-case product actions remain (AD-8)"
PC=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'product.create'")
[ "$PC" -ge 20 ] && ok "pre-existing product rows were migrated to product.create (n=$PC)" || ng "pre-existing product rows were migrated to product.create (n=$PC)"
curl -s -b $JA "$BASE/admin/audit?action=product.create" -o /tmp/ad_au1.html
grep -q "全${PC}件" /tmp/ad_au1.html && grep -q "<td>商品を登録</td>" /tmp/ad_au1.html \
  && ok "action filter shows product.create rows with the unchanged label" || ng "action filter shows product.create rows with the unchanged label"
curl -s -b $JA "$BASE/admin/audit?action=review.delete" -o /tmp/ad_au2.html
grep -q "<td>レビューを削除</td>" /tmp/ad_au2.html && ! grep -q "<td>商品を登録</td>" /tmp/ad_au2.html \
  && ok "action filter excludes other actions" || ng "action filter excludes other actions"
GT_AU=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE (created_at AT TIME ZONE 'Asia/Tokyo')::date = (now() AT TIME ZONE 'Asia/Tokyo')::date")
curl -s -b $JA "$BASE/admin/audit?from=$(date +%F)&to=$(date +%F)" -o /tmp/ad_au3.html
grep -q "全${GT_AU}件" /tmp/ad_au3.html \
  && ok "date range covering today matches the DB count ($GT_AU)" || ng "date range covering today matches the DB count (db=$GT_AU)"
curl -s -b $JA "$BASE/admin/audit?from=2020-01-01&to=2020-12-31" -o /tmp/ad_au4.html
grep -q "条件に一致する操作ログはありません" /tmp/ad_au4.html \
  && ok "past-only range shows the filtered-empty state" || ng "past-only range shows the filtered-empty state"
NC=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE target_type = 'notice' AND action = 'notice.create'")
curl -s -b $JA "$BASE/admin/audit?target=notice&action=notice.create" -o /tmp/ad_au5.html
grep -q "全${NC}件" /tmp/ad_au5.html \
  && ok "target and action filters combine with AND" || ng "target and action filters combine with AND"

echo "-- AD-1: the order detail carries its change history"
OIDH=$(place_member_order $JM 1)
curl -s -b $JA $BASE/admin/orders/$OIDH -o /tmp/ad_h1.html
grep -q "変更履歴" /tmp/ad_h1.html && grep -q "変更履歴はまだありません" /tmp/ad_h1.html \
  && ok "a fresh order's change history is the empty state" || ng "a fresh order's change history is the empty state"
walk $OIDH preparing
curl -s -b $JA $BASE/admin/orders/$OIDH -o /tmp/ad_h2.html
grep -q "注文ステータスを変更" /tmp/ad_h2.html && grep -q "管理者" /tmp/ad_h2.html \
  && ok "the status change appears in the history with the acting admin" || ng "the status change appears in the history with the acting admin"

echo "-- AD-2: notice period, visibility flag, search and pagination"
notice_create() { # title body from to visible(1=on,0=absent)
  curl -s -b $JA $BASE/admin/notices/new -o /tmp/ad_nn.html; local C=$(csrf /tmp/ad_nn.html)
  if [ "$5" = "1" ]; then
    curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/notices/new \
      --data-urlencode "title=$1" --data-urlencode "body=$2" \
      --data-urlencode "display_from=$3" --data-urlencode "display_to=$4" \
      --data-urlencode "is_visible=on" --data-urlencode "csrf_token=$C"
  else
    curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/notices/new \
      --data-urlencode "title=$1" --data-urlencode "body=$2" \
      --data-urlencode "display_from=$3" --data-urlencode "display_to=$4" \
      --data-urlencode "csrf_token=$C"
  fi
}
TODAY=$(date +%F); TOMORROW=$(date -d "+1 day" +%F)
FUTURE_BODY="管理テスト未来本文$$"
notice_create "管理テスト予告" "$FUTURE_BODY" "$TOMORROW" "" 1
curl -s $BASE/ -o /tmp/ad_topf.html
grep -q "$FUTURE_BODY" /tmp/ad_topf.html && ng "future notice stays off the top page" || ok "future notice stays off the top page"
curl -s -b $JA $BASE/admin/notices -o /tmp/ad_nl1.html
grep -q "管理テスト予告" /tmp/ad_nl1.html && grep -q "期間外" /tmp/ad_nl1.html \
  && ok "future notice is listed as 期間外" || ng "future notice is listed as 期間外"
notice_create "管理テスト本日" "管理テスト本日の本文" "$TODAY" "$TODAY" 1
curl -s $BASE/ -o /tmp/ad_topc.html
grep -q "管理テスト本日の本文" /tmp/ad_topc.html && grep -q "管理テスト本日" /tmp/ad_topc.html \
  && ok "same-day notice shows on the top page with its title" || ng "same-day notice shows on the top page with its title"
curl -s -b $JA $BASE/admin/notices -o /tmp/ad_nl2.html
grep -q "表示中" /tmp/ad_nl2.html && ok "in-period notice is listed as 表示中" || ng "in-period notice is listed as 表示中"
notice_create "管理テスト非表示" "管理テスト隠し本文" "" "" 0
curl -s $BASE/ -o /tmp/ad_toph.html
grep -q "管理テスト隠し本文" /tmp/ad_toph.html && ng "flag-off notice stays off the top page" || ok "flag-off notice stays off the top page"
GT_NOTLIVE=$(db "SELECT COUNT(*) FROM notices WHERE NOT (is_visible AND (display_from IS NULL OR display_from <= (now() AT TIME ZONE 'Asia/Tokyo')::date) AND (display_to IS NULL OR display_to >= (now() AT TIME ZONE 'Asia/Tokyo')::date))")
curl -s -b $JA "$BASE/admin/notices?status=hidden" -o /tmp/ad_nl3.html
grep -q "管理テスト予告" /tmp/ad_nl3.html && grep -q "管理テスト非表示" /tmp/ad_nl3.html && grep -q "全${GT_NOTLIVE}件" /tmp/ad_nl3.html \
  && ok "status filter lists exactly the not-live notices ($GT_NOTLIVE)" || ng "status filter lists exactly the not-live notices (db=$GT_NOTLIVE)"
curl -s -b $JA "$BASE/admin/notices?q=管理テスト予告" -o /tmp/ad_nl4.html
grep -q "管理テスト予告" /tmp/ad_nl4.html && ! grep -q "管理テスト本日" /tmp/ad_nl4.html \
  && ok "title search narrows the list" || ng "title search narrows the list"
EXISTING=$(db "SELECT COUNT(*) FROM notices")
if [ "$EXISTING" -lt 21 ]; then
  for i in $(seq 1 $((21 - EXISTING))); do notice_create "管理テスト一括$i" "管理テスト一括本文$i" "" "" 1; done
fi
NTOTAL=$(db "SELECT COUNT(*) FROM notices")
curl -s -b $JA $BASE/admin/notices -o /tmp/ad_nl5.html
grep -q "全${NTOTAL}件" /tmp/ad_nl5.html \
  && ok "notices list reports the true total ($NTOTAL)" || ng "notices list reports the true total (db=$NTOTAL)"
CODE=$(curl -s -b $JA -o /tmp/ad_nl6.html -w "%{http_code}" "$BASE/admin/notices?page=2")
[ "$CODE" = "200" ] && grep -q "admin-badge notice-" /tmp/ad_nl6.html \
  && ok "notices page 2 renders rows (paginated)" || ng "notices page 2 renders rows (paginated) ($CODE)"

echo "-- AD-4: customer search by email and kana, with ground-truth totals"
curl -s -b $JA "$BASE/admin/customers?q=test@example.com" -o /tmp/ad_cu1.html
# AD-4 ground truth mirrors the app's documented search semantics
# (customer_repository::customer_row_matches): a case-insensitive
# substring over the decrypted name parts, their joined forms, the
# kana the same way, and the email. (Ruled by the owner 2026-11-20:
# partial matching is the design; the suite matches it.)
GT_CUST=$(db "SELECT email || '|' || family_name || '|' || given_name || '|' || family_name_kana || '|' || given_name_kana FROM users" | PII_ENV_FILE="$(cd "$(dirname "$0")/../.." && pwd)/.env" python3 -c '
import base64, os, pathlib, sys
key = None
for line in pathlib.Path(os.environ["PII_ENV_FILE"]).read_text().splitlines():
    if line.startswith("PII_ENCRYPTION_KEY="):
        key = base64.b64decode(line.split("=", 1)[1].strip())
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
def dec(v):
    if v.startswith("v1:"):
        raw = base64.b64decode(v[3:])
        return AESGCM(key).decrypt(raw[:12], raw[12:], None).decode()
    return v
needle = "test@example.com"
count = 0
for line in sys.stdin:
    parts = line.rstrip("\n").split("|")
    if len(parts) != 5:
        continue
    email, fam, giv, famk, givk = (dec(p) for p in parts)
    fields = [fam, giv, fam + giv, fam + " " + giv, famk, givk, famk + givk, famk + " " + givk, email]
    if any(needle in f.lower() for f in fields):
        count += 1
print(count)
')
grep -q "全${GT_CUST}件" /tmp/ad_cu1.html && grep -q "$MEMBER" /tmp/ad_cu1.html \
  && ok "email search lists exactly the matching members ($GT_CUST)" || ng "email search lists exactly the matching members (db=$GT_CUST)"
KANA=$(pii_dec "$(db "SELECT family_name_kana FROM users WHERE email_bidx = '$(bidx $MEMBER)'")")
KFRAG=$(printf '%s' "$KANA" | cut -c1-3)
curl -s -b $JA --get --data-urlencode "q=$KFRAG" $BASE/admin/customers -o /tmp/ad_cu2.html
[ -n "$KFRAG" ] && grep -q "$MEMBER" /tmp/ad_cu2.html \
  && ok "kana fragment search finds the member" || ng "kana fragment search finds the member"
curl -s -b $JA "$BASE/admin/customers?q=zzz-no-such-member-zzz" -o /tmp/ad_cu3.html
grep -q "条件に一致する会員はいません" /tmp/ad_cu3.html \
  && ok "no-match customer search shows the filtered empty state" || ng "no-match customer search shows the filtered empty state"

echo "-- AD-4b: customers list aggregates equal detail and DB ground truth (Orders Spec 5.3)"
# The spec pins ONE definition for both screens: count,
# total and last-order date all exclude cancelled orders.
# The suite member holds cancelled orders by now, so the
# exclusion is exercised; truth comes from psql, and BOTH
# screens must match it — and therefore each other.
CUID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx $MEMBER)'")
GT_LN=$(db "SELECT COUNT(*) FROM orders WHERE user_id = $CUID AND status <> 'cancelled'")
GT_LSUM=$(db "SELECT COALESCE(SUM(total_yen) FILTER (WHERE status <> 'cancelled'),0) FROM orders WHERE user_id = $CUID")
GT_LLAST=$(db "SELECT to_char(MAX(created_at) AT TIME ZONE 'Asia/Tokyo', 'YYYY/MM/DD') FROM orders WHERE user_id = $CUID AND status <> 'cancelled'")
curl -s -b $JA --get --data-urlencode "q=$MEMBER" $BASE/admin/customers -o /tmp/ad_cu4.html
# The partial-match search returns several fixture members,
# so take the member's OWN row, found by its detail link:
# the five lines from the name cell on are name, email,
# registered, count, total, last-order date.
mapfile -t ROWC < <(grep -A5 "href=\"/admin/customers/$CUID\"" /tmp/ad_cu4.html | sed -E 's/<a [^>]*>//g; s/<\/a>//g' | grep -o '<td[^>]*>[^<]*</td>' | sed -E 's/<[^>]+>//g')
[ "${ROWC[3]}" = "$GT_LN" ] && ok "list order count equals DB ground truth ($GT_LN, cancelled excluded)" || ng "list order count equals DB ground truth (rendered=${ROWC[3]} db=$GT_LN)"
[ "$(echo "${ROWC[4]}" | tr -dc '0-9')" = "$GT_LSUM" ] && ok "list purchase total equals DB ground truth ($GT_LSUM yen)" || ng "list purchase total equals DB ground truth (rendered=${ROWC[4]} db=$GT_LSUM)"
[ "${ROWC[5]}" = "$GT_LLAST" ] && ok "list last-order date equals DB ground truth ($GT_LLAST)" || ng "list last-order date equals DB ground truth (rendered=${ROWC[5]} db=$GT_LLAST)"
curl -s -b $JA $BASE/admin/customers/$CUID -o /tmp/ad_cu5.html
D_LN=$(grep -o '注文件数：[0-9,]*' /tmp/ad_cu5.html | tr -dc '0-9')
D_LSUM=$(grep -o '購入金額合計：￥[0-9,]*' /tmp/ad_cu5.html | tr -dc '0-9')
[ "$D_LN" = "${ROWC[3]}" ] && [ "$D_LSUM" = "$(echo "${ROWC[4]}" | tr -dc '0-9')" ] \
  && ok "detail figures equal the list figures" || ng "detail figures equal the list figures (detail=$D_LN/$D_LSUM list=${ROWC[3]}/${ROWC[4]})"

echo "-- AD-6: order chips carry live counts; item-count and payment columns"
GT_REC=$(db "SELECT COUNT(*) FROM orders WHERE status = 'received'")
GT_UNS=$(db "SELECT COUNT(*) FROM orders WHERE status IN ('received', 'preparing')")
curl -s -b $JA $BASE/admin/orders -o /tmp/ad_ol1.html
grep -q "受付済み（$GT_REC）" /tmp/ad_ol1.html \
  && ok "received chip count equals the database ($GT_REC)" || ng "received chip count (db=$GT_REC)"
grep -q "未発送（$GT_UNS）" /tmp/ad_ol1.html \
  && ok "unshipped chip count equals received + preparing ($GT_UNS)" || ng "unshipped chip count (db=$GT_UNS)"
curl -s -b $JA "$BASE/admin/orders?status=received" -o /tmp/ad_ol2.html
grep -q "全${GT_REC}件" /tmp/ad_ol2.html \
  && ok "the received chip filters to exactly the received orders" || ng "the received chip filters (db=$GT_REC)"
OID6=$(place_member_order $JM 2)
ONUM6=$(db "SELECT 'AS-' || to_char(created_at AT TIME ZONE 'Asia/Tokyo', 'YYYYMMDD') || '-' || LPAD(id::text, 4, '0') FROM orders WHERE id = $OID6")
curl -s -b $JA "$BASE/admin/orders?q=$ONUM6" -o /tmp/ad_ol3.html
grep -q "2点" /tmp/ad_ol3.html && grep -q "代引き（未収）" /tmp/ad_ol3.html \
  && ok "the known order's row shows 2点 and 代引き（未収）" || ng "the known order's row shows item count and payment state"

echo "-- AD-7: the product edit page links the storefront, honestly"
curl -s -b $JA $BASE/admin/products/$P/edit -o /tmp/ad_p7.html
grep -q "href=\"/products/$P\" target=\"_blank\"" /tmp/ad_p7.html && grep -q "ストアで見る" /tmp/ad_p7.html \
  && ok "public product edit links the storefront page in a new tab" || ng "public product edit links the storefront page"
curl -s -b $JA $BASE/admin/products/new -o /tmp/ad_pn.html; C=$(csrf /tmp/ad_pn.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/new \
  --data-urlencode "name=管理テスト下書き商品" --data-urlencode "description=x" --data-urlencode "category_id=$TEST_CAT_ID" \
  --data-urlencode "price_yen=100" --data-urlencode "combo_stock[0]=1" --data-urlencode "status=draft" \
  --data-urlencode "images=$IMGURL" --data-urlencode "csrf_token=$C"
DID=$(db "SELECT id FROM products WHERE name = '管理テスト下書き商品' ORDER BY id DESC LIMIT 1")
curl -s -b $JA $BASE/admin/products/$DID/edit -o /tmp/ad_p8.html
grep -q "下書きのため店舗には表示されていません" /tmp/ad_p8.html && ! grep -q "href=\"/products/$DID\"" /tmp/ad_p8.html \
  && ok "draft product edit shows the inert note instead of a 404 link" || ng "draft product edit shows the inert note"

echo "-- AD-5: admin settings — display name and password, revocation, restore"
NEWPW=$(printf "adm_%s" "changed_2026")
CODE=$(curl -s -b $JA -o /tmp/ad_s0.html -w "%{http_code}" $BASE/admin/password)
[ "$CODE" = "200" ] && grep -q "アカウント設定" /tmp/ad_s0.html && grep -q 'name="display_name"' /tmp/ad_s0.html \
  && ok "settings page renders with both forms" || ng "settings page renders ($CODE)"
C=$(csrf /tmp/ad_s0.html)
curl -s -b $JA -c $JA -X POST $BASE/admin/password -o /tmp/ad_s1.html \
  --data-urlencode "current_password=wrong-current-pw" --data-urlencode "new_password=$NEWPW" \
  --data-urlencode "new_password_confirm=$NEWPW" --data-urlencode "csrf_token=$C"
grep -q "現在のパスワードが正しくありません" /tmp/ad_s1.html \
  && ok "wrong current password refused with the form error" || ng "wrong current password refused"
C=$(csrf /tmp/ad_s1.html)
curl -s -b $JA -c $JA -X POST $BASE/admin/password -o /tmp/ad_s2.html \
  --data-urlencode "current_password=$APW" --data-urlencode "new_password=short" \
  --data-urlencode "new_password_confirm=short" --data-urlencode "csrf_token=$C"
grep -q "12文字以上" /tmp/ad_s2.html \
  && ok "short new password refused by the 12-character floor" || ng "short new password refused"
JB2=/tmp/ad_admin2.txt; admin_login $JB2
CODE=$(curl -s -b $JB2 -o /dev/null -w "%{http_code}" $BASE/admin)
[ "$CODE" = "200" ] && ok "second admin session live before the change" || ng "second admin session live ($CODE)"
curl -s -b $JA $BASE/admin/password -o /tmp/ad_s3.html; C=$(csrf /tmp/ad_s3.html)
R=$(curl -s -b $JA -c $JA -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/password \
  --data-urlencode "current_password=$APW" --data-urlencode "new_password=$NEWPW" \
  --data-urlencode "new_password_confirm=$NEWPW" --data-urlencode "csrf_token=$C")
[ "$R" = "303 $BASE/admin/password" ] \
  && ok "password change redirects back to the settings page" || ng "password change redirects ($R)"
OUT=$(curl -s -b $JB2 -o /dev/null -w "%{redirect_url}" $BASE/admin)
case "$OUT" in *"/admin/login"*) ok "the other admin session was revoked";; *) ng "the other admin session was revoked ($OUT)";; esac
CODE=$(curl -s -b $JA -o /dev/null -w "%{http_code}" $BASE/admin)
[ "$CODE" = "200" ] && ok "the changing session survives" || ng "the changing session survives ($CODE)"
JN2=/tmp/ad_admin3.txt; rm -f $JN2
curl -s -c $JN2 $BASE/admin/login -o /tmp/ad_s4.html; C=$(csrf /tmp/ad_s4.html)
R=$(curl -s -b $JN2 -c $JN2 -o /dev/null -w "%{redirect_url}" -X POST $BASE/admin/login \
  --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "password=$NEWPW" --data-urlencode "csrf_token=$C")
[ "$R" = "$BASE/admin" ] && ok "login with the new password works" || ng "login with the new password works ($R)"
JO2=/tmp/ad_admin4.txt; rm -f $JO2
curl -s -c $JO2 $BASE/admin/login -o /tmp/ad_s5.html; C=$(csrf /tmp/ad_s5.html)
curl -s -b $JO2 -c $JO2 -X POST $BASE/admin/login -o /tmp/ad_s6.html \
  --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "password=$APW" --data-urlencode "csrf_token=$C"
grep -q "メールアドレスまたはパスワードが正しくありません" /tmp/ad_s6.html \
  && ok "login with the old password is refused" || ng "login with the old password is refused"
curl -s -b $JA $BASE/admin/password -o /tmp/ad_s7.html; C=$(csrf /tmp/ad_s7.html)
R=$(curl -s -b $JA -c $JA -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/password \
  --data-urlencode "current_password=$NEWPW" --data-urlencode "new_password=$APW" \
  --data-urlencode "new_password_confirm=$APW" --data-urlencode "csrf_token=$C")
[ "$R" = "303 $BASE/admin/password" ] \
  && ok "password restored to the run.sh default" || ng "password restored ($R)"
JR2=/tmp/ad_admin5.txt; rm -f $JR2
curl -s -c $JR2 $BASE/admin/login -o /tmp/ad_s8.html; C=$(csrf /tmp/ad_s8.html)
R=$(curl -s -b $JR2 -c $JR2 -o /dev/null -w "%{redirect_url}" -X POST $BASE/admin/login \
  --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "password=$APW" --data-urlencode "csrf_token=$C")
[ "$R" = "$BASE/admin" ] && ok "login with the restored password works" || ng "login with the restored password works ($R)"
ORIG_NAME=$(db "SELECT display_name FROM admins WHERE email = '$ADMIN_EMAIL'")
curl -s -b $JA $BASE/admin/password -o /tmp/ad_s9.html; C=$(csrf /tmp/ad_s9.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/profile \
  --data-urlencode "display_name=テスト管理者" --data-urlencode "csrf_token=$C"
curl -s -b $JA $BASE/admin/password -o /tmp/ad_s10.html
grep -q "テスト管理者 さん" /tmp/ad_s10.html \
  && ok "display name change shows in the sidebar" || ng "display name change shows in the sidebar"
C=$(csrf /tmp/ad_s10.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/profile \
  --data-urlencode "display_name=$ORIG_NAME" --data-urlencode "csrf_token=$C"
curl -s -b $JA $BASE/admin/password -o /tmp/ad_s11.html
grep -q "$ORIG_NAME さん" /tmp/ad_s11.html && ok "display name restored" || ng "display name restored"
GT_AUD=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action IN ('admin.password', 'admin.profile')")
[ "$GT_AUD" -ge 4 ] && ok "both settings changes are audited ($GT_AUD rows)" || ng "both settings changes are audited ($GT_AUD)"

echo "-- variants: matrix save, limits, duplicates, D5 removal block"
# Self-clean: this section's fixture products from an earlier
# run would break the name lookups below.
db "DELETE FROM cart_items WHERE variant_id IN (SELECT v.id FROM product_variants v JOIN products p ON p.id = v.product_id WHERE p.name IN ('管理テスト商品 バリエーション', '管理テスト商品 テンプレート', '管理テスト商品 変換'))" >/dev/null
db "DELETE FROM products WHERE name IN ('管理テスト商品 バリエーション', '管理テスト商品 テンプレート', '管理テスト商品 変換')" >/dev/null
# Create with two axes; the grid posts one stock per combination
# in cartesian order (first axis slowest): S/黒, S/白, M/黒, M/白.
curl -s -b $JA $BASE/admin/products/new -o /tmp/ad_new.html; C=$(csrf /tmp/ad_new.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/new \
  --data-urlencode "name=管理テスト商品 バリエーション" --data-urlencode "description=バリエーションのテスト商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=2000" --data-urlencode "status=public" \
  --data-urlencode "images=$IMGURL" \
  --data-urlencode "axes[0][name]=サイズ" \
  --data-urlencode "axes[0][options][0][value]=S" --data-urlencode "axes[0][options][1][value]=M" \
  --data-urlencode "axes[1][name]=カラー" \
  --data-urlencode "axes[1][options][0][value]=ブラック" --data-urlencode "axes[1][options][1][value]=ホワイト" \
  --data-urlencode "combo_stock[0]=2" --data-urlencode "combo_stock[1]=0" \
  --data-urlencode "combo_stock[2]=3" --data-urlencode "combo_stock[3]=1" \
  --data-urlencode "csrf_token=$C"
VPID=$(db "SELECT id FROM products WHERE name = '管理テスト商品 バリエーション' ORDER BY id DESC LIMIT 1")
[ -n "$VPID" ] && ok "variant product created via admin" || ng "variant product created via admin"
NV=$(db "SELECT COUNT(*) FROM product_variants WHERE product_id = $VPID")
[ "$NV" = "4" ] && ok "four combinations generated" || ng "four combinations generated ($NV)"
MBSTOCK=$(db "SELECT v.stock FROM product_variants v JOIN product_variant_option_links l1 ON l1.variant_id = v.id JOIN product_variant_options o1 ON o1.id = l1.option_id AND o1.value = 'M' JOIN product_variant_option_links l2 ON l2.variant_id = v.id JOIN product_variant_options o2 ON o2.id = l2.option_id AND o2.value = 'ブラック' WHERE v.product_id = $VPID")
[ "$MBSTOCK" = "3" ] && ok "grid stock landed on the M / ブラック combination" || ng "grid stock landed on the M combination ($MBSTOCK)"

# The template preset's shape: one axis サイズ with S/M/L/XL.
curl -s -b $JA $BASE/admin/products/new -o /tmp/ad_new.html; C=$(csrf /tmp/ad_new.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/new \
  --data-urlencode "name=管理テスト商品 テンプレート" --data-urlencode "description=テンプレートのテスト商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1000" --data-urlencode "status=draft" \
  --data-urlencode "images=$IMGURL" \
  --data-urlencode "axes[0][name]=サイズ" \
  --data-urlencode "axes[0][options][0][value]=S" --data-urlencode "axes[0][options][1][value]=M" \
  --data-urlencode "axes[0][options][2][value]=L" --data-urlencode "axes[0][options][3][value]=XL" \
  --data-urlencode "combo_stock[0]=1" --data-urlencode "combo_stock[1]=1" \
  --data-urlencode "combo_stock[2]=1" --data-urlencode "combo_stock[3]=1" \
  --data-urlencode "csrf_token=$C"
TPID=$(db "SELECT id FROM products WHERE name = '管理テスト商品 テンプレート' ORDER BY id DESC LIMIT 1")
[ "$(db "SELECT COUNT(*) FROM product_variants WHERE product_id = $TPID")" = "4" ] \
  && ok "template-shaped axis (サイズ S/M/L/XL) saved with four combinations" || ng "template-shaped axis saved"

# Duplicate option value in one axis is refused, naming it.
curl -s -b $JA $BASE/admin/products/new -o /tmp/ad_new.html; C=$(csrf /tmp/ad_new.html)
curl -s -b $JA -c $JA -X POST $BASE/admin/products/new -o /tmp/ad_vdup.html \
  --data-urlencode "name=管理テスト商品 重複選択肢" --data-urlencode "description=x" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1000" --data-urlencode "status=draft" \
  --data-urlencode "images=$IMGURL" \
  --data-urlencode "axes[0][name]=サイズ" \
  --data-urlencode "axes[0][options][0][value]=S" --data-urlencode "axes[0][options][1][value]=S" \
  --data-urlencode "combo_stock[0]=1" --data-urlencode "combo_stock[1]=1" \
  --data-urlencode "csrf_token=$C"
grep -q "「サイズ」の選択肢『S』が重複しています" /tmp/ad_vdup.html && ok "duplicate option refused with its name" || ng "duplicate option refused"
[ "$(db "SELECT COUNT(*) FROM products WHERE name = '管理テスト商品 重複選択肢'")" = "0" ] && ok "duplicate-option product not created" || ng "duplicate-option product not created"

# Duplicate axis name is refused, naming it.
curl -s -b $JA $BASE/admin/products/new -o /tmp/ad_new.html; C=$(csrf /tmp/ad_new.html)
curl -s -b $JA -c $JA -X POST $BASE/admin/products/new -o /tmp/ad_vdup2.html \
  --data-urlencode "name=管理テスト商品 重複見出し" --data-urlencode "description=x" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1000" --data-urlencode "status=draft" \
  --data-urlencode "images=$IMGURL" \
  --data-urlencode "axes[0][name]=サイズ" --data-urlencode "axes[0][options][0][value]=S" \
  --data-urlencode "axes[1][name]=サイズ" --data-urlencode "axes[1][options][0][value]=M" \
  --data-urlencode "combo_stock[0]=1" \
  --data-urlencode "csrf_token=$C"
grep -q "バリエーション名「サイズ」が重複しています" /tmp/ad_vdup2.html && ok "duplicate axis name refused with its name" || ng "duplicate axis name refused"
[ "$(db "SELECT COUNT(*) FROM products WHERE name = '管理テスト商品 重複見出し'")" = "0" ] && ok "duplicate-axis product not created" || ng "duplicate-axis product not created"

# Six options on one axis exceed the per-axis limit of five.
curl -s -b $JA $BASE/admin/products/new -o /tmp/ad_new.html; C=$(csrf /tmp/ad_new.html)
curl -s -b $JA -c $JA -X POST $BASE/admin/products/new -o /tmp/ad_vlim.html \
  --data-urlencode "name=管理テスト商品 上限選択肢" --data-urlencode "description=x" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1000" --data-urlencode "status=draft" \
  --data-urlencode "images=$IMGURL" \
  --data-urlencode "axes[0][name]=サイズ" \
  --data-urlencode "axes[0][options][0][value]=S" --data-urlencode "axes[0][options][1][value]=M" \
  --data-urlencode "axes[0][options][2][value]=L" --data-urlencode "axes[0][options][3][value]=XL" \
  --data-urlencode "axes[0][options][4][value]=XXL" --data-urlencode "axes[0][options][5][value]=3L" \
  --data-urlencode "combo_stock[0]=1" --data-urlencode "combo_stock[1]=1" --data-urlencode "combo_stock[2]=1" \
  --data-urlencode "combo_stock[3]=1" --data-urlencode "combo_stock[4]=1" --data-urlencode "combo_stock[5]=1" \
  --data-urlencode "csrf_token=$C"
grep -q "「サイズ」の選択肢は1〜5つにしてください" /tmp/ad_vlim.html && ok "sixth option refused by the per-axis limit" || ng "sixth option refused"
[ "$(db "SELECT COUNT(*) FROM products WHERE name = '管理テスト商品 上限選択肢'")" = "0" ] && ok "over-limit product not created" || ng "over-limit product not created"

# Six axes exceed the per-product limit of five.
curl -s -b $JA $BASE/admin/products/new -o /tmp/ad_new.html; C=$(csrf /tmp/ad_new.html)
curl -s -b $JA -c $JA -X POST $BASE/admin/products/new -o /tmp/ad_vlim2.html \
  --data-urlencode "name=管理テスト商品 上限見出し" --data-urlencode "description=x" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1000" --data-urlencode "status=draft" \
  --data-urlencode "images=$IMGURL" \
  --data-urlencode "axes[0][name]=見出し1" --data-urlencode "axes[0][options][0][value]=a" \
  --data-urlencode "axes[1][name]=見出し2" --data-urlencode "axes[1][options][0][value]=a" \
  --data-urlencode "axes[2][name]=見出し3" --data-urlencode "axes[2][options][0][value]=a" \
  --data-urlencode "axes[3][name]=見出し4" --data-urlencode "axes[3][options][0][value]=a" \
  --data-urlencode "axes[4][name]=見出し5" --data-urlencode "axes[4][options][0][value]=a" \
  --data-urlencode "axes[5][name]=見出し6" --data-urlencode "axes[5][options][0][value]=a" \
  --data-urlencode "combo_stock[0]=1" \
  --data-urlencode "csrf_token=$C"
grep -q "バリエーションは5つまでです" /tmp/ad_vlim2.html && ok "sixth axis refused by the per-product limit" || ng "sixth axis refused"
[ "$(db "SELECT COUNT(*) FROM products WHERE name = '管理テスト商品 上限見出し'")" = "0" ] && ok "over-limit-axes product not created" || ng "over-limit-axes product not created"

# D5: order the M / ブラック combination as the member; its
# order line carries the label snapshot.
VMID=$(db "SELECT v.id FROM product_variants v JOIN product_variant_option_links l1 ON l1.variant_id = v.id JOIN product_variant_options o1 ON o1.id = l1.option_id AND o1.value = 'M' JOIN product_variant_option_links l2 ON l2.variant_id = v.id JOIN product_variant_options o2 ON o2.id = l2.option_id AND o2.value = 'ブラック' WHERE v.product_id = $VPID")
curl -s -b $JM $BASE/cart -o /tmp/ad_cart.html; C=$(csrf /tmp/ad_cart.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/cart/items \
  --data-urlencode "product_id=$VPID" --data-urlencode "variant_id=$VMID" --data-urlencode "quantity=1" \
  --data-urlencode "csrf_token=$C"
curl -s -b $JM $BASE/checkout -o /tmp/ad_co.html; C=$(csrf /tmp/ad_co.html)
curl -s -b $JM -c $JM -X POST $BASE/checkout -o /tmp/ad_confirm.html \
  --data-urlencode "intent=confirm" \
  --data-urlencode "email=$MEMBER" --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前1-1-1" --data-urlencode "building=" --data-urlencode "phone=09012345678" \
  --data-urlencode "csrf_token=$C"
curl -s -b $JM -c $JM -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "csrf_token=$C" -o /tmp/ad_confirm.html
C=$(csrf /tmp/ad_confirm.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$C"
VOID=$(db "SELECT MAX(id) FROM orders WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $MEMBER)')")
LBL=$(db "SELECT variant_label FROM order_items WHERE order_id = $VOID AND variant_id = $VMID")
[ "$LBL" = "サイズ: M / カラー: ブラック" ] && ok "order line carries the variant label snapshot" || ng "order line label snapshot ($LBL)"
[ "$(db "SELECT stock FROM product_variants WHERE id = $VMID")" = "2" ] && ok "placement decremented the combination stock 3 -> 2" || ng "placement decremented the combination stock"

# Removing the ordered M option is refused, naming it; the
# matrix stays untouched.
AX1=$(db "SELECT id FROM product_variant_axes WHERE product_id = $VPID AND name = 'サイズ'")
AX2=$(db "SELECT id FROM product_variant_axes WHERE product_id = $VPID AND name = 'カラー'")
OSID=$(db "SELECT id FROM product_variant_options WHERE axis_id = $AX1 AND value = 'S'")
OBID=$(db "SELECT id FROM product_variant_options WHERE axis_id = $AX2 AND value = 'ブラック'")
OWID=$(db "SELECT id FROM product_variant_options WHERE axis_id = $AX2 AND value = 'ホワイト'")
curl -s -b $JA $BASE/admin/products/$VPID/edit -o /tmp/ad_vedit.html; C=$(csrf /tmp/ad_vedit.html)
curl -s -b $JA -c $JA -X POST $BASE/admin/products/$VPID/edit -o /tmp/ad_vresp.html \
  --data-urlencode "name=管理テスト商品 バリエーション" --data-urlencode "description=バリエーションのテスト商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=2000" --data-urlencode "status=public" \
  --data-urlencode "images=$IMGURL" \
  --data-urlencode "axes[0][id]=$AX1" --data-urlencode "axes[0][name]=サイズ" \
  --data-urlencode "axes[0][options][0][id]=$OSID" --data-urlencode "axes[0][options][0][value]=S" \
  --data-urlencode "axes[1][id]=$AX2" --data-urlencode "axes[1][name]=カラー" \
  --data-urlencode "axes[1][options][0][id]=$OBID" --data-urlencode "axes[1][options][0][value]=ブラック" \
  --data-urlencode "axes[1][options][1][id]=$OWID" --data-urlencode "axes[1][options][1][value]=ホワイト" \
  --data-urlencode "combo_stock[0]=2" --data-urlencode "combo_stock[1]=0" \
  --data-urlencode "csrf_token=$C"
grep -q "サイズ『M』は注文があるため削除できません" /tmp/ad_vresp.html && ok "D5: removing an ordered option is refused, naming it" || ng "D5: removing an ordered option is refused"
[ "$(db "SELECT COUNT(*) FROM product_variants WHERE product_id = $VPID")" = "4" ] && ok "D5: the matrix is unchanged after the refused removal" || ng "D5: matrix unchanged"


# Conversion path (owner ruling 2026-11-12): a plain product
# whose default variant has stock history — an ordered one
# always does — can NOT gain axes. Gaining axes replaces the
# default variant, and the append-only stock ledger forbids
# deleting a variant that has history; the save is refused
# with the ledger message. The operator archives the product
# and recreates it with variants instead. Past order lines
# keep their variant reference untouched.
curl -s -b $JA $BASE/admin/products/new -o /tmp/ad_new.html; C=$(csrf /tmp/ad_new.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/new \
  --data-urlencode "name=管理テスト商品 変換" --data-urlencode "description=変換のテスト商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1500" --data-urlencode "status=public" \
  --data-urlencode "images=$IMGURL" --data-urlencode "combo_stock[0]=5" \
  --data-urlencode "csrf_token=$C"
CPID=$(db "SELECT id FROM products WHERE name = '管理テスト商品 変換' ORDER BY id DESC LIMIT 1")
CDV=$(db "SELECT id FROM product_variants WHERE product_id = $CPID ORDER BY id ASC LIMIT 1")
curl -s -b $JM $BASE/cart -o /tmp/ad_cart.html; C=$(csrf /tmp/ad_cart.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/cart/items \
  --data-urlencode "product_id=$CPID" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$C"
curl -s -b $JM $BASE/checkout -o /tmp/ad_co.html; C=$(csrf /tmp/ad_co.html)
curl -s -b $JM -c $JM -X POST $BASE/checkout -o /tmp/ad_confirm.html \
  --data-urlencode "intent=confirm" \
  --data-urlencode "email=$MEMBER" --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前1-1-1" --data-urlencode "building=" --data-urlencode "phone=09012345678" \
  --data-urlencode "csrf_token=$C"
curl -s -b $JM -c $JM -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "csrf_token=$C" -o /tmp/ad_confirm.html
C=$(csrf /tmp/ad_confirm.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$C"
COID=$(db "SELECT MAX(id) FROM orders WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $MEMBER)')")
[ "$(db "SELECT variant_id FROM order_items WHERE order_id = $COID AND product_id = $CPID")" = "$CDV" ] \
  && ok "plain order references the default variant before conversion" || ng "plain order references the default variant"
curl -s -b $JA $BASE/admin/products/$CPID/edit -o /tmp/ad_cedit.html; C=$(csrf /tmp/ad_cedit.html)
curl -s -b $JA -c $JA -X POST $BASE/admin/products/$CPID/edit -o /tmp/ad_cresp.html \
  --data-urlencode "name=管理テスト商品 変換" --data-urlencode "description=変換のテスト商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1500" --data-urlencode "status=public" \
  --data-urlencode "images=$IMGURL" \
  --data-urlencode "axes[0][name]=サイズ" \
  --data-urlencode "axes[0][options][0][value]=S" --data-urlencode "axes[0][options][1][value]=M" \
  --data-urlencode "combo_stock[0]=2" --data-urlencode "combo_stock[1]=3" \
  --data-urlencode "csrf_token=$C"
grep -q "在庫の履歴があるバリエーションは削除できません" /tmp/ad_cresp.html \
  && ok "conversion of an ordered plain product is refused with the ledger message" || ng "conversion refusal message"
[ "$(db "SELECT COUNT(*) FROM product_variants WHERE product_id = $CPID")" = "1" ] \
  && ok "refused conversion leaves the product plain (default variant intact)" || ng "refused conversion leaves product plain"
[ "$(db "SELECT variant_id FROM order_items WHERE order_id = $COID AND product_id = $CPID")" = "$CDV" ] \
  && ok "past order line keeps its variant after the refused conversion" || ng "past order line keeps its variant"
[ "$(db "SELECT variant_label FROM order_items WHERE order_id = $COID AND product_id = $CPID")" = "" ] \
  && ok "past order line label stays empty after the refused conversion" || ng "past order line label stays empty"

echo "-- logout clears only the admin session"
curl -s -b $JA $BASE/admin -o /tmp/ad_before.html; C=$(csrf /tmp/ad_before.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/logout --data-urlencode "csrf_token=$C"
OUT=$(curl -s -b $JA -o /dev/null -w "%{redirect_url}" $BASE/admin)
case "$OUT" in *"${BASE}/admin/login"*) ok "after logout, /admin redirects to login";; *) ng "after logout, /admin redirects to login ($OUT)";; esac
CODE=$(curl -s -b $JM -o /dev/null -w "%{http_code}" $BASE/mypage)
[ "$CODE" = "200" ] && ok "member session survives the admin logout" || ng "member session survives the admin logout ($CODE)"

echo "-- throttle: five failures lock the pair, even for the right password (LAST: locks 15 min)"
rm -f $JA
curl -s -c $JA $BASE/admin/login -o /tmp/ad_t1.html
for i in 1 2 3 4 5; do
  C=$(csrf /tmp/ad_t1.html)
  curl -s -b $JA -c $JA -X POST $BASE/admin/login -o /tmp/ad_t1.html \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "password=$WRONG" --data-urlencode "csrf_token=$C"
done
C=$(csrf /tmp/ad_t1.html)
curl -s -b $JA -X POST $BASE/admin/login -o /tmp/ad_t2.html \
  --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "password=$APW" --data-urlencode "csrf_token=$C"
grep -q "ログイン試行回数が上限に達しました" /tmp/ad_t2.html \
  && ok "sixth attempt (correct password) is throttled with the notice" || ng "sixth attempt (correct password) is throttled"


result
