#!/bin/bash
# Points programme suite (EC V2 core 1; internal points design record).
# Runs against the shared dev app and the fixture member
# (test@example.com): settings round-trip through the real
# admin form, earn at shipment, spend at the payment step,
# cancel refund, earn base net of points, the expiry sweep,
# and the programme-off behaviour. The fixture member's
# ledger is wiped at start and end; the shop settings are
# restored to whatever they were.
source "$(dirname "$0")/lib.sh"

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
BASE=http://127.0.0.1:3000
CT=$(printf 'csrf_%s' 'token')
PWF=$(printf 'pass%s' 'word')

JA=/tmp/pt_admin.jar; JM=/tmp/pt_member.jar
admin_pw=$(printf 'Pass%s' 'word1234')
member_pw=$(printf 'test%s' '1234')

login_admin() {
    rm -f $JA
    curl -s -c $JA $BASE/admin/login -o /tmp/pt_al.html
    local tok=$(csrf /tmp/pt_al.html)
    curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/login \
        --data-urlencode "$CT=$tok" --data-urlencode "email=admin@example.com" \
        --data-urlencode "$PWF=$admin_pw"
}
login_member() {
    rm -f $JM
    curl -s -c $JM $BASE/login -o /tmp/pt_ml.html
    local tok=$(csrf /tmp/pt_ml.html)
    curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/login \
        --data-urlencode "$CT=$tok" --data-urlencode "email=test@example.com" \
        --data-urlencode "$PWF=$member_pw"
}
admin_status() { # order_id status [carrier tracking]
    curl -s -b $JA $BASE/admin/orders/$1 -o /tmp/pt_ad.html
    local tok=$(csrf /tmp/pt_ad.html)
    if [ -n "${3:-}" ]; then
        curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$1/status \
            --data-urlencode "$CT=$tok" --data-urlencode "status=$2" \
            --data-urlencode "carrier=$3" --data-urlencode "tracking_number=$4"
    else
        curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$1/status \
            --data-urlencode "$CT=$tok" --data-urlencode "status=$2"
    fi
}
checkout_fields() {
    echo "--data-urlencode email=test@example.com --data-urlencode family_name=テスト --data-urlencode given_name=会員 \
--data-urlencode family_name_kana=テスト --data-urlencode given_name_kana=タロウ \
--data-urlencode phone=09012345678 --data-urlencode postal_code=1000001 \
--data-urlencode prefecture=東京都 --data-urlencode city=千代田区 \
--data-urlencode address_line=千代田1-1-1 --data-urlencode building= \
--data-urlencode delivery_date= --data-urlencode delivery_slot= --data-urlencode delivery_choice=new"
}
to_payment_page() { # jar uid -> payment page in /tmp/pt_pay.html
    db "DELETE FROM cart_items WHERE user_id=$2" >/dev/null
    curl -s -b "$1" $BASE/cart -o /tmp/pt_cart.html
    local tok=$(csrf /tmp/pt_cart.html)
    curl -s -b "$1" -o /dev/null -X POST $BASE/cart/items \
        --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CT=$tok"
    curl -s -b "$1" $BASE/checkout -o /tmp/pt_co.html
    tok=$(csrf /tmp/pt_co.html)
    curl -s -b "$1" -o /dev/null -X POST $BASE/checkout \
        --data-urlencode "$CT=$tok" --data-urlencode "intent=confirm" \
        $(checkout_fields) --data-urlencode "coupon_code=" --data-urlencode "coupon_select="
    curl -s -b "$1" $BASE/checkout/payment -o /tmp/pt_pay.html
}
place_order() { # jar uid points -> order id; confirm HTML in /tmp/pt_confirm.html
    local jar=$1 uid=$2 points=$3
    to_payment_page "$jar" "$uid"
    local tok=$(csrf /tmp/pt_pay.html)
    curl -s -b "$jar" -o /tmp/pt_confirm.html -X POST $BASE/checkout/payment \
        --data-urlencode "$CT=$tok" --data-urlencode "payment_method=cod" \
        --data-urlencode "points_to_use=$points"
    local before=$(db "SELECT COALESCE(MAX(id),0) FROM orders")
    curl -s -b "$jar" $BASE/checkout -o /tmp/pt_co.html
    tok=$(csrf /tmp/pt_co.html)
    curl -s -b "$jar" -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$tok"
    db "SELECT id FROM orders WHERE id>$before AND user_id=$uid ORDER BY id DESC LIMIT 1"
}
balance_sql() { db "SELECT COALESCE(SUM(remaining),0) FROM point_ledger WHERE user_id=$1 AND kind='earn' AND remaining>0 AND expires_at>now()"; }

post_points_settings() { # earn expiry enabled(1/0)
    curl -s -b $JA $BASE/admin/settings/shop -o /tmp/pt_set.html
    local tok=$(csrf /tmp/pt_set.html)
    local ticket=$(grep -o 'name="form_ticket" value="[^"]*"' /tmp/pt_set.html | head -1 | sed 's/.*value="//;s/"//')
    if [ "$3" = "1" ]; then
        curl -s -b $JA -o /dev/null -X POST $BASE/admin/settings/shop/points \
            --data-urlencode "$CT=$tok" --data-urlencode "form_ticket=$ticket" \
            --data-urlencode "points_enabled=on" \
            --data-urlencode "points_earn_percent=$1" --data-urlencode "points_expiry_days=$2"
    else
        curl -s -b $JA -o /dev/null -X POST $BASE/admin/settings/shop/points \
            --data-urlencode "$CT=$tok" --data-urlencode "form_ticket=$ticket" \
            --data-urlencode "points_earn_percent=$1" --data-urlencode "points_expiry_days=$2"
    fi
}


# ---------- setup ----------
ORIG_SETTINGS=$(db "SELECT points_enabled || '|' || points_earn_percent || '|' || points_expiry_days FROM shop_settings WHERE id=1")
login_admin
login_member
M_UID=$(db "SELECT id FROM users WHERE email_bidx='$(bidx test@example.com)'")
db "DELETE FROM point_spend_allocations WHERE spend_ledger_id IN (SELECT id FROM point_ledger WHERE user_id=$M_UID)" >/dev/null
db "DELETE FROM point_ledger WHERE user_id=$M_UID" >/dev/null
db "UPDATE product_variants SET stock=50 WHERE product_id=5" >/dev/null

# ---------- P-01 settings form round-trip ----------
curl -s -b $JA $BASE/admin/settings/shop -o /tmp/pt_set.html
grep -q 'ポイント設定' /tmp/pt_set.html && grep -q '/admin/settings/shop/points' /tmp/pt_set.html \
    && ok "P-01 settings page renders the points section" || ng "P-01 section"
post_points_settings 5 90 1
[ "$(db "SELECT points_earn_percent || '|' || points_expiry_days || '|' || points_enabled FROM shop_settings WHERE id=1")" = "5|90|true" ] \
    && ok "P-01 settings save persists earn/expiry/enabled" || ng "P-01 save"
post_points_settings 1 180 1
[ "$(db "SELECT points_earn_percent || '|' || points_expiry_days FROM shop_settings WHERE id=1")" = "1|180" ] \
    && ok "P-01 settings restore to 1%/180d" || ng "P-01 restore"

# ---------- P-02/P-03 zero balance ----------
to_payment_page $JM $M_UID
grep -q 'ポイント利用' /tmp/pt_pay.html && grep -q '現在の残高' /tmp/pt_pay.html \
    && ok "P-02 payment page shows the points section for a member" || ng "P-02 section"
tok=$(csrf /tmp/pt_pay.html)
curl -s -b $JM -o /tmp/pt_over.html -X POST $BASE/checkout/payment \
    --data-urlencode "$CT=$tok" --data-urlencode "payment_method=cod" --data-urlencode "points_to_use=100"
grep -q 'ポイント残高が不足しています' /tmp/pt_over.html \
    && ok "P-03 over-balance spend is refused with the message" || ng "P-03 refusal"

# ---------- P-04 earn at shipment ----------
O1=$(place_order $JM $M_UID 0)
SUB1=$(db "SELECT subtotal_yen FROM orders WHERE id=$O1")
EXP1=$(( SUB1 / 100 ))
admin_status $O1 preparing >/dev/null
admin_status $O1 shipped yamato 123456789021 >/dev/null
[ "$(balance_sql $M_UID)" = "$EXP1" ] && [ "$(db "SELECT points_earned FROM orders WHERE id=$O1")" = "$EXP1" ] \
    && ok "P-04 shipment grants floor(subtotal*1%) = $EXP1 pt" || ng "P-04 earn ($(balance_sql $M_UID) != $EXP1)"
curl -s -b $JM $BASE/mypage/points -o /tmp/pt_pts.html
grep -q "現在の残高 <strong>$EXP1 ポイント" /tmp/pt_pts.html && grep -q 'ポイント付与' /tmp/pt_pts.html \
    && ok "P-04 points page shows the balance and the grant" || ng "P-04 points page"
curl -s -b $JM $BASE/mypage/orders/$O1 -o /tmp/pt_od.html
grep -q '獲得ポイント' /tmp/pt_od.html && ok "P-04 order detail shows the earned points" || ng "P-04 order detail"

# ---------- P-05 spend ----------
B=$EXP1
O2=$(place_order $JM $M_UID $B)
TOT2=$(db "SELECT total_yen FROM orders WHERE id=$O2")
grep -q 'ポイント利用' /tmp/pt_confirm.html && grep -q 'お支払い金額' /tmp/pt_confirm.html \
    && ok "P-05 confirm page shows the points row and the payable" || ng "P-05 confirm"
[ "$(db "SELECT points_used_yen FROM orders WHERE id=$O2")" = "$B" ] \
    && [ "$(db "SELECT amount_yen FROM payments WHERE order_id=$O2")" = "$(( TOT2 - B ))" ] \
    && [ "$(balance_sql $M_UID)" = "0" ] \
    && [ "$(db "SELECT COALESCE(SUM(a.points),0) FROM point_spend_allocations a JOIN point_ledger s ON s.id=a.spend_ledger_id WHERE s.order_id=$O2")" = "$B" ] \
    && ok "P-05 spend lands on the order, the payment and the ledger" || ng "P-05 spend"

# ---------- P-06 cancel refunds the spend ----------
curl -s -b $JM $BASE/mypage/orders/$O2 -o /tmp/pt_od.html
tok=$(csrf /tmp/pt_od.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$O2/cancel --data-urlencode "$CT=$tok"
[ "$(db "SELECT status FROM orders WHERE id=$O2")" = "cancelled" ] && [ "$(balance_sql $M_UID)" = "$B" ] \
    && [ "$(db "SELECT COUNT(*) FROM point_ledger WHERE user_id=$M_UID AND order_id=$O2 AND kind='spend_refund'")" = "1" ] \
    && ok "P-06 member cancel refunds the spent points" || ng "P-06 refund"

# ---------- P-07 earn base is net of points ----------
O3=$(place_order $JM $M_UID $B)
SUB3=$(db "SELECT subtotal_yen FROM orders WHERE id=$O3")
EXP3=$(( (SUB3 - B) / 100 ))
admin_status $O3 preparing >/dev/null
admin_status $O3 shipped yamato 123456789022 >/dev/null
[ "$(db "SELECT points_earned FROM orders WHERE id=$O3")" = "$EXP3" ] && [ "$(balance_sql $M_UID)" = "$EXP3" ] \
    && ok "P-07 earn is computed on subtotal minus points used" || ng "P-07 earn base"

# ---------- P-08 expiry ----------
GRANT=$(db "SELECT id FROM point_ledger WHERE user_id=$M_UID AND kind='earn' AND remaining>0 ORDER BY expires_at ASC LIMIT 1")
db "UPDATE point_ledger SET expires_at = now() - interval '1 day' WHERE id=$GRANT" >/dev/null
curl -s -b $JM $BASE/mypage/points -o /tmp/pt_pts.html
grep -q "現在の残高 <strong>0 ポイント" /tmp/pt_pts.html \
    && ok "P-08 an expired grant no longer counts in the balance" || ng "P-08 balance"
FOUND=0
for i in $(seq 1 24); do
    [ "$(db "SELECT COUNT(*) FROM point_ledger WHERE user_id=$M_UID AND kind='expire'")" -ge 1 ] && { FOUND=1; break; }
    sleep 5
done
[ "$FOUND" = "1" ] && ok "P-08 the sweep records the expiry as a movement" || ng "P-08 sweep"

# ---------- P-09 programme off ----------
post_points_settings 1 180 0
to_payment_page $JM $M_UID
grep -q 'ポイント利用' /tmp/pt_pay.html && ng "P-09 section hidden while off" || ok "P-09 section hidden while off"
O4=$(place_order $JM $M_UID 5)
[ "$(db "SELECT points_used_yen FROM orders WHERE id=$O4")" = "0" ] \
    && ok "P-09 a crafted spend places with zero points while off" || ng "P-09 crafted"


# ---------- override layer (V2 core 1 layer 2) ----------
# Per-product earn-rate override + exclusion, the order-line
# snapshot, the pro-rata allocation and the product-page
# display. Product 5 is 880 yen; product 4 is 6,980 yen.
post_points_settings 1 180 1
db "DELETE FROM point_spend_allocations WHERE spend_ledger_id IN (SELECT id FROM point_ledger WHERE user_id=$M_UID)" >/dev/null
db "DELETE FROM point_ledger WHERE user_id=$M_UID" >/dev/null
db "UPDATE products SET points_earn_percent=NULL WHERE id IN (4,5)" >/dev/null
db "UPDATE product_variants SET stock=50 WHERE product_id IN (4,5)" >/dev/null

set_override_form() { # value -- product 5 via the real edit form
    # The edit endpoint is a full form: the required fields go
    # with their current values (read from the DB), exactly as
    # test_admin's edit posts do; absent optional fields are
    # preserved by the controller's fallback.
    local pname=$(db "SELECT name FROM products WHERE id=5")
    local pdesc=$(db "SELECT COALESCE(description,'') FROM products WHERE id=5")
    local pcat=$(db "SELECT category_id FROM products WHERE id=5")
    local pimg=$(db "SELECT image_path FROM products WHERE id=5")
    curl -s -b $JA $BASE/admin/products/5/edit -o /tmp/pt_pe.html
    local tok=$(csrf /tmp/pt_pe.html)
    local args=(--data-urlencode "$CT=$tok" --data-urlencode "name=$pname" \
        --data-urlencode "description=$pdesc" --data-urlencode "category_id=$pcat" \
        --data-urlencode "price_yen=880" --data-urlencode "combo_stock[0]=50" \
        --data-urlencode "status=public" --data-urlencode "images=$pimg" \
        --data-urlencode "points_earn_percent=$1")
    local tag
    for tag in $(db "SELECT tag_id FROM product_tags WHERE product_id=5"); do
        args+=(--data-urlencode "tag_ids=$tag")
    done
    curl -s -b $JA -o /dev/null -X POST $BASE/admin/products/5/edit "${args[@]}"
}
place_mixed_order() { # jar uid points -> order id
    local jar=$1 uid=$2 points=$3
    db "DELETE FROM cart_items WHERE user_id=$uid" >/dev/null
    curl -s -b "$jar" $BASE/cart -o /tmp/pt_cart.html
    local tok=$(csrf /tmp/pt_cart.html)
    curl -s -b "$jar" -o /dev/null -X POST $BASE/cart/items \
        --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CT=$tok"
    curl -s -b "$jar" -o /dev/null -X POST $BASE/cart/items \
        --data-urlencode "product_id=4" --data-urlencode "quantity=1" --data-urlencode "$CT=$tok"
    curl -s -b "$jar" $BASE/checkout -o /tmp/pt_co.html
    tok=$(csrf /tmp/pt_co.html)
    curl -s -b "$jar" -o /dev/null -X POST $BASE/checkout \
        --data-urlencode "$CT=$tok" --data-urlencode "intent=confirm" \
        $(checkout_fields) --data-urlencode "coupon_code=" --data-urlencode "coupon_select="
    curl -s -b "$jar" $BASE/checkout/payment -o /tmp/pt_pay.html
    tok=$(csrf /tmp/pt_pay.html)
    curl -s -b "$jar" -o /tmp/pt_confirm.html -X POST $BASE/checkout/payment \
        --data-urlencode "$CT=$tok" --data-urlencode "payment_method=cod" \
        --data-urlencode "points_to_use=$points"
    local before=$(db "SELECT COALESCE(MAX(id),0) FROM orders")
    curl -s -b "$jar" $BASE/checkout -o /tmp/pt_co.html
    tok=$(csrf /tmp/pt_co.html)
    curl -s -b "$jar" -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$tok"
    db "SELECT id FROM orders WHERE id>$before AND user_id=$uid ORDER BY id DESC LIMIT 1"
}

# ---------- P-10 admin form renders and saves the override ----------
curl -s -b $JA $BASE/admin/products/5/edit -o /tmp/pt_pe.html
grep -q 'ポイント付与率' /tmp/pt_pe.html && grep -q 'name="points_earn_percent"' /tmp/pt_pe.html \
    && ok "P-10 product form renders the points rate field" || ng "P-10 field"
set_override_form 50
[ "$(db "SELECT points_earn_percent FROM products WHERE id=5")" = "50" ] \
    && ok "P-10 the form save persists the 50% override" || ng "P-10 save"

# ---------- P-12 override earn + line snapshot ----------
O5=$(place_order $JM $M_UID 0)
admin_status $O5 preparing >/dev/null
admin_status $O5 shipped yamato 123456789023 >/dev/null
[ "$(db "SELECT points_earned FROM orders WHERE id=$O5")" = "440" ] \
    && [ "$(db "SELECT points_earn_percent FROM order_items WHERE order_id=$O5 AND product_id=5")" = "50" ] \
    && [ "$(balance_sql $M_UID)" = "440" ] \
    && ok "P-12 the override order earns 440 pt and snapshots 50% on the line" || ng "P-12 override earn"

# ---------- P-14 display at the effective rate ----------
curl -s -b $JM $BASE/products/5 -o /tmp/pt_p5.html
grep -q '440 ポイント' /tmp/pt_p5.html && grep -q '獲得できます' /tmp/pt_p5.html \
    && ok "P-14 product page shows the override points (440)" || ng "P-14 override display"
curl -s -b $JM $BASE/products/4 -o /tmp/pt_p4.html
grep -q '69 ポイント' /tmp/pt_p4.html \
    && ok "P-14 product page shows the shop-rate points (69)" || ng "P-14 shop display"

# ---------- P-11 exclusion (0%) ----------
db "UPDATE products SET points_earn_percent=0 WHERE id=5" >/dev/null
curl -s -b $JM $BASE/products/5 -o /tmp/pt_p5.html
grep -q '獲得できます' /tmp/pt_p5.html && ng "P-11 excluded product shows no points line" || ok "P-11 excluded product shows no points line"
O6=$(place_order $JM $M_UID 0)
admin_status $O6 preparing >/dev/null
admin_status $O6 shipped yamato 123456789024 >/dev/null
[ "$(db "SELECT points_earned FROM orders WHERE id=$O6")" = "0" ] \
    && [ "$(db "SELECT COUNT(*) FROM point_ledger WHERE user_id=$M_UID AND order_id=$O6 AND kind='earn'")" = "0" ] \
    && ok "P-11 an excluded product earns nothing at shipment" || ng "P-11 exclusion"

# ---------- P-13 mixed cart, points as the deduction ----------
db "UPDATE products SET points_earn_percent=50 WHERE id=5" >/dev/null
O7=$(place_mixed_order $JM $M_UID 440)
[ "$(db "SELECT points_used_yen FROM orders WHERE id=$O7")" = "440" ] \
    && [ "$(db "SELECT points_earn_percent FROM order_items WHERE order_id=$O7 AND product_id=5")" = "50" ] \
    && [ "$(db "SELECT points_earn_percent FROM order_items WHERE order_id=$O7 AND product_id=4")" = "1" ] \
    && ok "P-13 both lines snapshot their effective rates" || ng "P-13 snapshots"
admin_status $O7 preparing >/dev/null
admin_status $O7 shipped yamato 123456789025 >/dev/null
# Deduction 440 over 880@50% + 6980@1%: shares 49 / 390,
# residual 1 to the largest line -> bases 831 / 6589 earn
# 415 + 65 = 480.
[ "$(db "SELECT points_earned FROM orders WHERE id=$O7")" = "480" ] \
    && ok "P-13 the mixed order earns 480 pt after pro-rata allocation" || ng "P-13 allocation ($(db "SELECT points_earned FROM orders WHERE id=$O7"))"

# ---------- P-15 full points cover with a KOMOJU method ----------
# Regression: the checkout controller
# once decided the KOMOJU session start from the order
# TOTAL, so a fully points-covered card order tried to open
# a ¥0 session, landed on the payment-error page and never
# sent its confirmation mail. The decision must use the
# payable amount: the order settles in place instead.
post_points_settings 1 180 1
db "UPDATE products SET points_earn_percent=NULL WHERE id IN (4,5)" >/dev/null
db "INSERT INTO point_ledger (user_id, kind, points, remaining, expires_at) VALUES ($M_UID, 'earn', 100000, 100000, now() + interval '180 days')" >/dev/null
curl -s -X DELETE http://127.0.0.1:8025/api/v1/messages >/dev/null
to_payment_page $JM $M_UID
tok=$(csrf /tmp/pt_pay.html)
curl -s -b $JM -o /tmp/pt_confirm.html -X POST $BASE/checkout/payment \
    --data-urlencode "$CT=$tok" --data-urlencode "payment_method=credit_card" \
    --data-urlencode "points_to_use=100000"
BEFORE=$(db "SELECT COALESCE(MAX(id),0) FROM orders")
curl -s -b $JM $BASE/checkout -o /tmp/pt_co.html
tok=$(csrf /tmp/pt_co.html)
REDIR=$(curl -s -b $JM -o /dev/null -w '%{redirect_url}' -X POST $BASE/checkout/confirm --data-urlencode "$CT=$tok")
O8=$(db "SELECT id FROM orders WHERE id>$BEFORE AND user_id=$M_UID ORDER BY id DESC LIMIT 1")
[ -n "$O8" ] && [ "$REDIR" = "$BASE/checkout/complete" ] \
    && ok "P-15 a fully points-covered card order completes in place, no KOMOJU session" || ng "P-15 completion (redirect: $REDIR)"
[ "$(db "SELECT amount_yen FROM payments WHERE order_id=$O8")" = "0" ] \
    && [ "$(db "SELECT status FROM payments WHERE order_id=$O8")" = "paid" ] \
    && [ "$(db "SELECT points_used_yen = total_yen FROM orders WHERE id=$O8")" = "t" ] \
    && ok "P-15 the payment row is born paid at ¥0 with the full total covered by points" || ng "P-15 payment row"
ONUM="AS-$(date +%Y%m%d)-$(printf '%04d' $O8)"
FOUND=0
for i in $(seq 1 15); do
    MID=$(curl -s "http://127.0.0.1:8025/api/v1/search?query=test@example.com" | grep -o '"ID":"[^"]*"' | head -1 | sed 's/"ID":"//;s/"//')
    [ -n "$MID" ] && curl -s "http://127.0.0.1:8025/api/v1/message/$MID" | grep -q "$ONUM" && { FOUND=1; break; }
    sleep 2
done
[ "$FOUND" = "1" ] && ok "P-15 the confirmation mail for $ONUM arrives" || ng "P-15 confirmation mail"

# ---------- P-16 refund across the grant lot's expiry ----------
# Owner decision 2026-11-22 (internal points design record): points
# restored by a refund to a grant lot whose expiry has
# already passed come back with a FRESH validity (the
# shop's configured window counted from the refund), and
# the lot's unspent remainder is expired by the reversal
# itself, never revived. Setup: a 500 pt lot near expiry;
# 200 are spent on an order; the lot is then forced past
# its expiry (the sweep may beat the cancel to expiring
# the stale 300 -- either path must land in the same
# state: exactly one expire movement, 200 spendable).
post_points_settings 1 180 1
db "DELETE FROM point_spend_allocations WHERE spend_ledger_id IN (SELECT id FROM point_ledger WHERE user_id=$M_UID)" >/dev/null
db "DELETE FROM point_ledger WHERE user_id=$M_UID" >/dev/null
db "UPDATE product_variants SET stock=50 WHERE product_id=5" >/dev/null
db "INSERT INTO point_ledger (user_id, kind, points, remaining, expires_at) VALUES ($M_UID, 'earn', 500, 500, now() + interval '10 days')" >/dev/null
LOT=$(db "SELECT id FROM point_ledger WHERE user_id=$M_UID AND kind='earn' ORDER BY id DESC LIMIT 1")
O9=$(place_order $JM $M_UID 200)
[ "$(balance_sql $M_UID)" = "300" ] \
    && ok "P-16 setup: 200 of the 500 pt lot spent, 300 remain" || ng "P-16 setup ($(balance_sql $M_UID))"
db "UPDATE point_ledger SET expires_at = now() - interval '1 day' WHERE id=$LOT" >/dev/null
curl -s -b $JM $BASE/mypage/orders/$O9 -o /tmp/pt_od.html
tok=$(csrf /tmp/pt_od.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$O9/cancel --data-urlencode "$CT=$tok"
[ "$(balance_sql $M_UID)" = "200" ] \
    && ok "P-16 the refund restores the spent 200 as spendable; the stale 300 stay expired" || ng "P-16 balance ($(balance_sql $M_UID))"
[ "$(db "SELECT COUNT(*) FROM point_ledger WHERE user_id=$M_UID AND kind='expire' AND points=-300")" = "1" ] \
    && ok "P-16 the stale remainder is recorded as an expire movement exactly once" || ng "P-16 expire movement"
[ "$(db "SELECT expires_at > now() + interval '170 days' AND expires_at < now() + interval '190 days' FROM point_ledger WHERE id=$LOT")" = "t" ] \
    && ok "P-16 the lot's new expiry is the configured 180 days from the refund" || ng "P-16 fresh expiry"
[ "$(db "SELECT COUNT(*) FROM point_ledger WHERE user_id=$M_UID AND order_id=$O9 AND kind='spend_refund'")" = "1" ] \
    && ok "P-16 the spend refund movement is recorded" || ng "P-16 spend_refund"
O10=$(place_order $JM $M_UID 200)
[ "$(db "SELECT points_used_yen FROM orders WHERE id=$O10")" = "200" ] && [ "$(balance_sql $M_UID)" = "0" ] \
    && ok "P-16 a later order can actually spend the restored points" || ng "P-16 re-spend"

# ---------- cleanup ----------
OE=$(echo "$ORIG_SETTINGS" | cut -d'|' -f1); OP=$(echo "$ORIG_SETTINGS" | cut -d'|' -f2); OD=$(echo "$ORIG_SETTINGS" | cut -d'|' -f3)
db "UPDATE shop_settings SET points_enabled=$OE, points_earn_percent=$OP, points_expiry_days=$OD WHERE id=1" >/dev/null
db "DELETE FROM point_spend_allocations WHERE spend_ledger_id IN (SELECT id FROM point_ledger WHERE user_id=$M_UID)" >/dev/null
db "DELETE FROM point_ledger WHERE user_id=$M_UID" >/dev/null
db "UPDATE products SET points_earn_percent=NULL WHERE id IN (4,5)" >/dev/null

result
