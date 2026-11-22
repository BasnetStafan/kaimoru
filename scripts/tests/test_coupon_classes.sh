#!/bin/bash
# Coupon Classes suite (Coupon Shopify-Core spec, 2026-11-08):
# value types, scope, combination classes, usage caps,
# automatic discounts, and the redemption release. Assumes
# the app is running at 127.0.0.1:3000 (./run.sh).
# Choreography notes (proven against the live app):
# - codes are unique per run (RUN suffix): caps and counts
#   make static codes non-idempotent across reruns;
# - every scenario re-logs in: the checkout flow (with its
#   applied codes) lives in the session, so a fresh jar is
#   the only clean slate;
# - the CSRF token is scraped from the product page: a
#   guest's empty cart page renders no form to scrape;
# - lines are the keyholder (product 5, variant 5, 880 yen)
#   and the venue towel (product 9, variant 7, 1800 yen),
#   whose stock this suite tops up when low.
source "$(dirname "$0")/lib.sh"

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
CT=$(printf 'csrf_%s' 'token')
BASE=http://127.0.0.1:3000
RUN=$(date +%s | tail -c 6)
P_CODE="CP${RUN}"   # percent 10, scope product 5
O_CODE="CO${RUN}"   # fixed 500, order
O2_CODE="CX${RUN}"  # fixed 100, order (same-class rival)
S_CODE="CS${RUN}"   # free shipping
A_CODE="CA${RUN}"   # fixed 300, automatic
C2_CODE="CC${RUN}"  # fixed 200, per-customer cap 2
T1_CODE="CT${RUN}"  # fixed 150, total cap 1
ONE_CODE="CN${RUN}" # fixed 100, per-customer cap 1 (release test)
PASS=0; FAIL=0
ok()   { PASS=$((PASS+1)); echo "ok   - $1"; }
ng()   { FAIL=$((FAIL+1)); echo "NG   - $1"; }
expect() { # desc, actual, expected
    if [ "$2" = "$3" ]; then ok "$1"; else ng "$1 (got: $2, want: $3)"; fi
}
grep_ok() { # desc, file, pattern
    if grep -q "$3" "$2"; then ok "$1"; else ng "$1 (pattern not found: $3)"; fi
}
grep_ng() { # desc, file, pattern (must be absent)
    if grep -q "$3" "$2"; then ng "$1 (pattern unexpectedly found: $3)"; else ok "$1"; fi
}

member_login() {
    rm -f /tmp/ccl_m.jar
    curl -s -c /tmp/ccl_m.jar $BASE/login -o /tmp/ccl_ml.html
    local tok=$(csrf /tmp/ccl_ml.html)
    local pw=$(printf 'test%s' '1234')
    curl -s -b /tmp/ccl_m.jar -c /tmp/ccl_m.jar -o /dev/null -X POST $BASE/login \
        --data-urlencode "$CT=$tok" --data-urlencode "email=test@example.com" \
        --data-urlencode "password=$pw"
}
clear_cart() { db "DELETE FROM cart_items WHERE user_id=8" >/dev/null; }
add() { # jar, product_id, qty
    curl -s -b $1 -c $1 $BASE/products/$2 -o /tmp/ccl_cart.html
    local tok=$(csrf /tmp/ccl_cart.html)
    curl -s -b $1 -c $1 -o /dev/null -X POST $BASE/cart/items \
        --data-urlencode "product_id=$2" --data-urlencode "quantity=$3" \
        --data-urlencode "$CT=$tok"
}
checkout_get() { curl -s -b $1 -c $1 $BASE/checkout -o /tmp/ccl_co.html; }
apply() { # jar, code -> response in /tmp/ccl_resp.html
    checkout_get $1
    local tok=$(csrf /tmp/ccl_co.html)
    curl -s -b $1 -c $1 -o /tmp/ccl_resp.html -X POST $BASE/checkout \
        --data-urlencode "$CT=$tok" --data-urlencode "intent=apply_coupon" \
        --data-urlencode "coupon_code=$2" --data-urlencode "coupon_select=" \
        --data-urlencode "email=test@example.com" \
        --data-urlencode "family_name=テスト" --data-urlencode "given_name=会員" \
        --data-urlencode "phone=09012345678" --data-urlencode "postal_code=1000001" \
        --data-urlencode "prefecture=東京都" --data-urlencode "city=千代田区" \
        --data-urlencode "address_line=千代田1-1-1" --data-urlencode "building=" \
        --data-urlencode "delivery_date=" --data-urlencode "delivery_slot=" \
        --data-urlencode "delivery_choice=new"
}
delivery_submit() { # jar
    checkout_get $1
    local tok=$(csrf /tmp/ccl_co.html)
    curl -s -b $1 -c $1 -o /dev/null -X POST $BASE/checkout \
        --data-urlencode "$CT=$tok" --data-urlencode "intent=confirm" \
        --data-urlencode "coupon_code=" --data-urlencode "coupon_select=" \
        --data-urlencode "email=test@example.com" \
        --data-urlencode "family_name=テスト" --data-urlencode "given_name=会員" \
        --data-urlencode "phone=09012345678" --data-urlencode "postal_code=1000001" \
        --data-urlencode "prefecture=東京都" --data-urlencode "city=千代田区" \
        --data-urlencode "address_line=千代田1-1-1" --data-urlencode "building=" \
        --data-urlencode "delivery_date=" --data-urlencode "delivery_slot=" \
        --data-urlencode "delivery_choice=new"
}
place() { # jar -> new order id (COD)
    local before=$(db "SELECT COALESCE(MAX(id),0) FROM orders")
    delivery_submit $1
    curl -s -b $1 -c $1 $BASE/checkout/payment -o /tmp/ccl_pay.html
    local tok=$(csrf /tmp/ccl_pay.html)
    curl -s -b $1 -c $1 -X POST $BASE/checkout/payment -o /tmp/ccl_confirm.html \
        --data-urlencode "payment_method=cod" --data-urlencode "$CT=$tok"
    local tok2=$(csrf /tmp/ccl_confirm.html)
    curl -s -b $1 -c $1 -o /dev/null -X POST $BASE/checkout/confirm \
        --data-urlencode "$CT=$tok2"
    db "SELECT id FROM orders WHERE id > $before ORDER BY id LIMIT 1"
}

# Fixture premise: the towel line needs stock for two orders.
STOCK7=$(db "SELECT stock FROM product_variants WHERE id=7")
if [ "${STOCK7:-0}" -lt 3 ]; then
    db "UPDATE product_variants SET stock=10 WHERE id=7" >/dev/null
    db "INSERT INTO stock_adjustments (variant_id, delta, reason) VALUES (7, $((10 - ${STOCK7:-0})), 'manual')" >/dev/null
fi

# ---- definitions for this run ----
db "INSERT INTO coupons (code, discount_type, discount_yen, percent, scope, product_id, variant_id, max_uses_total, max_uses_per_customer, is_automatic, min_subtotal_yen, description, active)
    VALUES ('$P_CODE','percent',0,10,'product',5,NULL,NULL,NULL,false,0,'検証 商品10%',true),
           ('$O_CODE','fixed',500,NULL,'order',NULL,NULL,NULL,NULL,false,0,'検証 注文500',true),
           ('$O2_CODE','fixed',100,NULL,'order',NULL,NULL,NULL,NULL,false,0,'検証 注文100',true),
           ('$S_CODE','free_shipping',0,NULL,'order',NULL,NULL,NULL,NULL,false,0,'検証 送料無料',true),
           ('$C2_CODE','fixed',200,NULL,'order',NULL,NULL,NULL,2,false,0,'検証 1人2回',true),
           ('$T1_CODE','fixed',150,NULL,'order',NULL,NULL,1,NULL,false,0,'検証 全体1回',true),
           ('$ONE_CODE','fixed',100,NULL,'order',NULL,NULL,NULL,1,false,0,'検証 リリース',true)" >/dev/null

# ---- TC-01: product-scoped percent prices from its lines only ----
member_login
clear_cart; add /tmp/ccl_m.jar 5 1; add /tmp/ccl_m.jar 9 1   # 880 + 1800 = 2680
apply /tmp/ccl_m.jar "$P_CODE"
grep_ok "TC-01 product % applied row" /tmp/ccl_resp.html "${P_CODE} を適用中［商品］（−￥88）"
OID=$(place /tmp/ccl_m.jar)
expect "TC-01 order discount" "$(db "SELECT discount_yen FROM orders WHERE id=$OID")" "88"
expect "TC-01 order total (2680-88+550+330)" "$(db "SELECT total_yen FROM orders WHERE id=$OID")" "3472"
expect "TC-01 redemption row" "$(db "SELECT class || ':' || discount_yen FROM coupon_redemptions WHERE order_id=$OID")" "product:88"
expect "TC-01 order coupon_code = product code" "$(db "SELECT coupon_code FROM orders WHERE id=$OID")" "$P_CODE"

# ---- TC-02: the three classes combine on one order ----
member_login
clear_cart; add /tmp/ccl_m.jar 5 1; add /tmp/ccl_m.jar 9 1
apply /tmp/ccl_m.jar "$P_CODE" >/dev/null
apply /tmp/ccl_m.jar "$O_CODE" >/dev/null
apply /tmp/ccl_m.jar "$S_CODE"
grep_ok "TC-02 order class applied" /tmp/ccl_resp.html "${O_CODE} を適用中［注文］（−￥500）"
grep_ok "TC-02 shipping class applied" /tmp/ccl_resp.html "${S_CODE} を適用中［送料］（−￥550）"
OID=$(place /tmp/ccl_m.jar)
expect "TC-02 discount 88+500" "$(db "SELECT discount_yen FROM orders WHERE id=$OID")" "588"
expect "TC-02 shipping net 0" "$(db "SELECT shipping_yen FROM orders WHERE id=$OID")" "0"
expect "TC-02 shipping discount 550" "$(db "SELECT shipping_discount_yen FROM orders WHERE id=$OID")" "550"
expect "TC-02 total (2680-588+0+330)" "$(db "SELECT total_yen FROM orders WHERE id=$OID")" "2422"
expect "TC-02 redemption count" "$(db "SELECT count(*) FROM coupon_redemptions WHERE order_id=$OID")" "3"
expect "TC-02 primary code = order class" "$(db "SELECT coupon_code FROM orders WHERE id=$OID")" "$O_CODE"
grep_ok "TC-02 confirm page showed 送料割引" /tmp/ccl_confirm.html "送料割引"

# ---- TC-03: a second code in an occupied class is refused ----
member_login
clear_cart; add /tmp/ccl_m.jar 5 1
apply /tmp/ccl_m.jar "$O_CODE" >/dev/null
apply /tmp/ccl_m.jar "$O2_CODE"
grep_ok "TC-03 generic refusal" /tmp/ccl_resp.html "クーポンコードが正しくないか"
grep_ok "TC-03 first code still applied" /tmp/ccl_resp.html "${O_CODE} を適用中"
grep_ng "TC-03 rival not applied" /tmp/ccl_resp.html "${O2_CODE} を適用中"

# ---- TC-04: automatic fills the class; an entered code displaces it ----
db "INSERT INTO coupons (code, discount_type, discount_yen, percent, scope, max_uses_per_customer, is_automatic, min_subtotal_yen, description, active)
    VALUES ('$A_CODE','fixed',300,NULL,'order',NULL,true,0,'検証 自動300',true)" >/dev/null
member_login
clear_cart; add /tmp/ccl_m.jar 5 1
checkout_get /tmp/ccl_m.jar
grep_ok "TC-04 automatic applied with no code" /tmp/ccl_co.html "自動適用：${A_CODE} を適用中［注文］（−￥300）"
apply /tmp/ccl_m.jar "$O_CODE"
grep_ok "TC-04 entered code applied" /tmp/ccl_resp.html "${O_CODE} を適用中"
grep_ng "TC-04 automatic displaced" /tmp/ccl_resp.html "自動適用：${A_CODE}"
rm -f /tmp/ccl_g.jar; add /tmp/ccl_g.jar 5 1
checkout_get /tmp/ccl_g.jar
grep_ok "TC-04 guest sees automatic" /tmp/ccl_co.html "自動適用：${A_CODE}"
apply /tmp/ccl_g.jar "$O_CODE"
grep_ok "TC-04 guest code refused (generic)" /tmp/ccl_resp.html "クーポンコードが正しくないか"
db "UPDATE coupons SET active=false WHERE code='$A_CODE'" >/dev/null

# ---- TC-05: per-customer cap 2 ----
member_login
clear_cart; add /tmp/ccl_m.jar 5 1
apply /tmp/ccl_m.jar "$C2_CODE" >/dev/null; OID=$(place /tmp/ccl_m.jar)
expect "TC-05 first use placed" "$(db "SELECT count(*) FROM coupon_redemptions WHERE order_id=$OID AND code='$C2_CODE'")" "1"
clear_cart; add /tmp/ccl_m.jar 5 1
apply /tmp/ccl_m.jar "$C2_CODE"
grep_ok "TC-05 second use applies" /tmp/ccl_resp.html "${C2_CODE} を適用中"
OID=$(place /tmp/ccl_m.jar)
clear_cart; add /tmp/ccl_m.jar 5 1
apply /tmp/ccl_m.jar "$C2_CODE"
grep_ok "TC-05 third use refused" /tmp/ccl_resp.html "クーポンコードが正しくないか"

# ---- TC-06: total cap 1 ----
member_login
clear_cart; add /tmp/ccl_m.jar 5 1
apply /tmp/ccl_m.jar "$T1_CODE" >/dev/null; OID=$(place /tmp/ccl_m.jar)
expect "TC-06 first use placed" "$(db "SELECT count(*) FROM coupon_redemptions WHERE order_id=$OID AND code='$T1_CODE'")" "1"
clear_cart; add /tmp/ccl_m.jar 5 1
apply /tmp/ccl_m.jar "$T1_CODE"
grep_ok "TC-06 second use refused (total cap)" /tmp/ccl_resp.html "クーポンコードが正しくないか"

# ---- TC-07: a pre-settlement cancel releases the coupon ----
member_login
db "INSERT INTO user_coupons (user_id, code) VALUES (8, '$ONE_CODE')" >/dev/null
clear_cart; add /tmp/ccl_m.jar 5 1
apply /tmp/ccl_m.jar "$ONE_CODE" >/dev/null; OID=$(place /tmp/ccl_m.jar)
expect "TC-07 use recorded" "$(db "SELECT count(*) FROM coupon_redemptions WHERE order_id=$OID AND code='$ONE_CODE'")" "1"
expect "TC-07 holder stamp set (cap 1 reached)" "$(db "SELECT used_at IS NOT NULL FROM user_coupons WHERE user_id=8 AND code='$ONE_CODE'")" "t"
curl -s -b /tmp/ccl_m.jar -c /tmp/ccl_m.jar $BASE/mypage/orders/$OID/cancel -o /tmp/ccl_cx.html
TOK=$(csrf /tmp/ccl_cx.html)
curl -s -b /tmp/ccl_m.jar -c /tmp/ccl_m.jar -o /dev/null -X POST $BASE/mypage/orders/$OID/cancel \
    --data-urlencode "$CT=$TOK"
expect "TC-07 order cancelled" "$(db "SELECT status FROM orders WHERE id=$OID")" "cancelled"
expect "TC-07 redemptions released" "$(db "SELECT count(*) FROM coupon_redemptions WHERE order_id=$OID")" "0"
expect "TC-07 holder stamp cleared" "$(db "SELECT used_at IS NULL FROM user_coupons WHERE user_id=8 AND code='$ONE_CODE'")" "t"
clear_cart; add /tmp/ccl_m.jar 5 1
apply /tmp/ccl_m.jar "$ONE_CODE"
grep_ok "TC-07 coupon usable again after release" /tmp/ccl_resp.html "${ONE_CODE} を適用中"

echo "----"
echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" = "0" ]
