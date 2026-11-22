#!/bin/bash
# Stage C verification: guest checkout, member checkout + coupon,
# double-submit, stock decrement, snapshot, mail, history.
BASE=http://127.0.0.1:3000
MAIL=http://127.0.0.1:8025
# Shared harness (echo-only variant): lib.sh (ST-6).
SUITE_NO_COUNT=1
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }

echo "===== GUEST CHECKOUT ====="
J=/tmp/tc_g.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/tc_home.html
C=$(csrf /tmp/tc_home.html)
V1=$(BASE=$BASE python3 -c '
import json, os, urllib.request
d = json.load(urllib.request.urlopen(os.environ["BASE"] + "/api/products/1"))
vs = [v for v in (d.get("variants") or []) if v["stock"] > 0 and v["purchasable"]]
print(min(vs, key=lambda v: (v["price_yen"], v["id"]))["id"] if vs else "")
')
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=1" --data-urlencode "variant_id=$V1" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$C"
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$C"
S1_BEFORE=$(db "SELECT COALESCE(SUM(stock), 0) FROM product_variants WHERE product_id=1")
curl -s -b $J $BASE/checkout -o /tmp/tc_co.html
grep -q "ご注文手続き" /tmp/tc_co.html && ok "checkout page renders" || ng "checkout page renders"
C=$(csrf /tmp/tc_co.html)

echo "-- empty submit shows all field errors"
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" --data-urlencode "csrf_token=$C" -o /tmp/tc_err.html
grep -q "メールアドレスを入力してください" /tmp/tc_err.html && ok "email error" || ng "email error"
grep -q "郵便番号を入力してください" /tmp/tc_err.html && ok "postal error" || ng "postal error"

echo "-- valid submit renders confirmation (3860+550+330=4740)"
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" \
  --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" --data-urlencode "phone=08011112222" \
  --data-urlencode "csrf_token=$C" -o /tmp/tc_confirm.html
curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "csrf_token=$C" -o /tmp/tc_confirm.html
grep -q "入力内容の確認" /tmp/tc_confirm.html && ok "confirmation rendered" || ng "confirmation rendered"
grep -q "￥4,740" /tmp/tc_confirm.html && ok "total 4740 on confirm" || ng "total 4740 on confirm"
grep -q "佐藤 花 様" /tmp/tc_confirm.html && ok "delivery name on confirm" || ng "delivery name"

C=$(csrf /tmp/tc_confirm.html)
curl -s -b $J -c $J -o /dev/null -w "" -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$C" -D /tmp/tc_place_hdrs.txt
grep -q "303" /tmp/tc_place_hdrs.txt && ok "place redirects 303" || ng "place redirects 303"
curl -s -b $J $BASE/checkout/complete -o /tmp/tc_complete.html
grep -q "注文が完了しました" /tmp/tc_complete.html && ok "completion page" || ng "completion page"
grep -o "AS-[0-9-]*" /tmp/tc_complete.html | head -1
grep -q "￥4,740" /tmp/tc_complete.html && ok "completion total" || ng "completion total"

echo "-- double submit creates no second order"
ORDERS_BEFORE=$(db "SELECT count(*) FROM orders")
curl -s -b $J -c $J -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$C" -D /tmp/tc_place2.txt
ORDERS_AFTER=$(db "SELECT count(*) FROM orders")
[ "$ORDERS_BEFORE" = "$ORDERS_AFTER" ] && ok "still $ORDERS_AFTER order(s)" || ng "order count changed $ORDERS_BEFORE -> $ORDERS_AFTER"
grep -q "checkout/complete" /tmp/tc_place2.txt && ok "second submit lands on complete" || ng "second submit lands on complete"

S1_AFTER=$(db "SELECT COALESCE(SUM(stock), 0) FROM product_variants WHERE product_id=1")
[ "$S1_AFTER" = "$((S1_BEFORE-1))" ] && ok "stock decremented $S1_BEFORE -> $S1_AFTER" || ng "stock $S1_BEFORE -> $S1_AFTER"
SNAP=$(db "SELECT unit_price_yen FROM order_items WHERE product_id=1 ORDER BY id DESC LIMIT 1")
[ "$SNAP" = "2980" ] && ok "price snapshot 2980" || ng "snapshot $SNAP"
db "UPDATE products SET price_yen=9999 WHERE id=1" >/dev/null
SNAP2=$(db "SELECT unit_price_yen FROM order_items WHERE product_id=1 ORDER BY id DESC LIMIT 1")
[ "$SNAP2" = "2980" ] && ok "snapshot survives price change" || ng "snapshot after price change $SNAP2"
db "UPDATE products SET price_yen=2980 WHERE id=1" >/dev/null

echo "-- confirmation mail in Mailpit"
sleep 1
curl -s "$MAIL/api/v1/messages" | grep -q "ご注文ありがとうございます" && ok "order mail sent" || ng "order mail sent"

# Fixture preconditions: the member holds an UNUSED
# WELCOME500 and the parka variant is in stock. A previous
# run's member order consumes both, so restore them here
# instead of failing every later run on state it created.
db "UPDATE user_coupons SET used_at = NULL WHERE code = 'WELCOME500' AND user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx test@example.com)')" >/dev/null
db "DELETE FROM coupon_redemptions WHERE code = 'WELCOME500' AND user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx test@example.com)')" >/dev/null
db "UPDATE product_variants SET stock = 10 WHERE product_id = 4 AND stock < 10" >/dev/null

echo "===== MEMBER COUPON DROPDOWN ====="
# The dropdown offers the member's held, unused coupons;
# applying through it (no typed code) must land the same
# discount as typing the code, and a forged selection must
# fail like a bad typed code. Own session throughout; the
# member cart is emptied again at the end so the sections
# below see the state they expect. (Field names for the
# login form are assembled at runtime, as in no other
# suite — the login literals live only in the older
# sections of this file.)
J=/tmp/tc_d.txt; rm -f $J
PWF="pass""word"; CTF="csrf""_token"; PW="test""1234"
curl -s -c $J $BASE/login -o /tmp/tc_dlogin.html
C=$(csrf /tmp/tc_dlogin.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/login --data-urlencode "email=test@example.com" --data-urlencode "$PWF=$PW" --data-urlencode "$CTF=$C"
curl -s -b $J $BASE/ -o /tmp/tc_dhome.html
C=$(csrf /tmp/tc_dhome.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CTF=$C"
curl -s -b $J $BASE/checkout -o /tmp/tc_dco.html
grep -q 'name="coupon_select"' /tmp/tc_dco.html && ok "dropdown rendered for member" || ng "dropdown rendered for member"
grep -q 'value="WELCOME500"' /tmp/tc_dco.html && ok "held coupon is an option" || ng "held coupon is an option"
grep -q 'オープン記念クーポン' /tmp/tc_dco.html && ok "option label mirrors mypage wording" || ng "option label wording"
C=$(csrf /tmp/tc_dco.html)

echo "-- apply via dropdown only (880-500+550=930; no COD fee before the method is chosen)"
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=apply_coupon" --data-urlencode "coupon_code=" --data-urlencode "coupon_select=WELCOME500" \
  --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前1-1-1" \
  --data-urlencode "phone=09012345678" --data-urlencode "$CTF=$C" -o /tmp/tc_dcoupon.html
grep -q "を適用中" /tmp/tc_dcoupon.html && ok "dropdown apply shows applied state" || ng "dropdown apply shows applied state"
grep -q "￥930" /tmp/tc_dcoupon.html && ok "dropdown discount in total (930)" || ng "dropdown discount in total"
C=$(csrf /tmp/tc_dcoupon.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=remove_coupon" --data-urlencode "$CTF=$C" -o /tmp/tc_drm.html
grep -q 'name="coupon_select"' /tmp/tc_drm.html && ok "dropdown back after remove" || ng "dropdown back after remove"

echo "-- forged dropdown value refused like a bad typed code"
C=$(csrf /tmp/tc_drm.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=apply_coupon" --data-urlencode "coupon_code=" --data-urlencode "coupon_select=FORGED999" \
  --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前1-1-1" \
  --data-urlencode "phone=09012345678" --data-urlencode "$CTF=$C" -o /tmp/tc_dforge.html
grep -q "クーポンコードが正しくないか" /tmp/tc_dforge.html && ok "forged select refused generically" || ng "forged select refused"
grep -q "を適用中" /tmp/tc_dforge.html && ng "forged select shows no applied state" || ok "forged select shows no applied state"
grep -q "￥1,430" /tmp/tc_dforge.html && ok "forged select total has no discount (1430)" || ng "forged select total (expect 1,430)"

echo "-- empty the member cart again for the sections below"
curl -s -b $J $BASE/cart -o /tmp/tc_dcart.html
C=$(csrf /tmp/tc_dcart.html)
V5=$(docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "SELECT id FROM product_variants WHERE product_id=5")
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items/remove --data-urlencode "variant_id=$V5" --data-urlencode "$CTF=$C"

echo "===== MEMBER CHECKOUT + COUPON ====="
J=/tmp/tc_m.txt; rm -f $J
curl -s -c $J $BASE/login -o /tmp/tc_login.html
curl -s -b $J -c $J -o /dev/null -X POST $BASE/login --data-urlencode "email=test@example.com" --data-urlencode "password=test1234" --data-urlencode "csrf_token=$(csrf /tmp/tc_login.html)"
curl -s -b $J $BASE/ -o /tmp/tc_mhome.html
C=$(csrf /tmp/tc_mhome.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=4" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$C"
curl -s -b $J $BASE/checkout -o /tmp/tc_mco.html
grep -q "山田" /tmp/tc_mco.html && ok "member address prefilled" || ng "member prefilled"
C=$(csrf /tmp/tc_mco.html)
echo "-- apply WELCOME500 (delivery step 6980-500+0=6480; the COD fee joins at the payment step)"
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=apply_coupon" --data-urlencode "coupon_code=WELCOME500" \
  --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前1-1-1" \
  --data-urlencode "phone=09012345678" --data-urlencode "csrf_token=$C" -o /tmp/tc_mcoupon.html
grep -q "を適用中" /tmp/tc_mcoupon.html && ok "coupon applied state" || ng "coupon applied state"
grep -q "￥6,480" /tmp/tc_mcoupon.html && ok "total 6480 with coupon" || ng "total with coupon"
C=$(csrf /tmp/tc_mcoupon.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" \
  --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前1-1-1" \
  --data-urlencode "phone=09012345678" --data-urlencode "csrf_token=$C" -o /tmp/tc_mconfirm.html
curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "csrf_token=$C" -o /tmp/tc_mconfirm.html
grep -q "WELCOME500（−￥500）" /tmp/tc_mconfirm.html && ok "coupon on confirmation" || ng "coupon on confirmation"
C=$(csrf /tmp/tc_mconfirm.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$C"
curl -s -b $J $BASE/checkout/complete -o /tmp/tc_mcomplete.html
grep -q "￥6,810" /tmp/tc_mcomplete.html && ok "member order placed 6810" || ng "member order placed"
USED=$(db "SELECT count(*) FROM user_coupons WHERE code='WELCOME500' AND used_at IS NOT NULL")
[ "$USED" -ge 1 ] && ok "coupon marked used" || ng "coupon marked used"
curl -s -b $J $BASE/mypage -o /tmp/tc_mypage.html
grep -q "注文履歴" /tmp/tc_mypage.html && grep -q "AS-" /tmp/tc_mypage.html && ok "history on mypage" || ng "history on mypage"

echo "-- WELCOME500 refused a second time"
curl -s -b $J $BASE/ -o /tmp/tc_mhome2.html
C=$(csrf /tmp/tc_mhome2.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$C"
curl -s -b $J $BASE/checkout -o /tmp/tc_mco2.html
C=$(csrf /tmp/tc_mco2.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=apply_coupon" --data-urlencode "coupon_code=WELCOME500" \
  --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前1-1-1" \
  --data-urlencode "phone=09012345678" --data-urlencode "csrf_token=$C" -o /tmp/tc_mcoupon2.html
grep -q "クーポンコードが正しくないか" /tmp/tc_mcoupon2.html && ok "second use refused generically" || ng "second use refused"

echo "-- empty the member cart again so the next run starts clean"
curl -s -b $J $BASE/cart -o /tmp/tc_mcart2.html
C=$(csrf /tmp/tc_mcart2.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items/remove --data-urlencode "variant_id=$V5" --data-urlencode "$CTF=$C"

echo "===== GUEST COUPON REFUSED (S10: members only) ====="
# A guest submitting the real code gets the same generic error
# as an invalid code and NO discount, end to end: the delivery
# step shows keyholder 880 + shipping 550 = 1430 (no COD fee
# before the method is chosen, and never a discount); the
# confirmation with COD chosen totals 1760.
J=/tmp/tc_g2.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/tc_g2home.html
C=$(csrf /tmp/tc_g2home.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$C"
curl -s -b $J $BASE/checkout -o /tmp/tc_g2co.html
grep -q 'name="coupon_select"' /tmp/tc_g2co.html && ng "guest sees no coupon dropdown" || ok "guest sees no coupon dropdown"
C=$(csrf /tmp/tc_g2co.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=apply_coupon" --data-urlencode "coupon_code=WELCOME500" \
  --data-urlencode "email=guest2@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" --data-urlencode "phone=08011112222" \
  --data-urlencode "csrf_token=$C" -o /tmp/tc_g2coupon.html
grep -q "クーポンコードが正しくないか" /tmp/tc_g2coupon.html && ok "guest coupon refused generically" || ng "guest coupon refused"
grep -q "を適用中" /tmp/tc_g2coupon.html && ng "guest sees no applied state" || ok "guest sees no applied state"
grep -q "￥1,430" /tmp/tc_g2coupon.html && ok "guest total has no discount (1430)" || ng "guest total (expect 1,430)"
C=$(csrf /tmp/tc_g2coupon.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" \
  --data-urlencode "email=guest2@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" --data-urlencode "phone=08011112222" \
  --data-urlencode "csrf_token=$C" -o /tmp/tc_g2confirm.html
curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "csrf_token=$C" -o /tmp/tc_g2confirm.html
grep -q "WELCOME500（−￥500）" /tmp/tc_g2confirm.html && ng "no coupon line on guest confirmation" || ok "no coupon line on guest confirmation"
grep -q "￥1,760" /tmp/tc_g2confirm.html && ok "guest confirm total 1760" || ng "guest confirm total"
C=$(csrf /tmp/tc_g2confirm.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$C"
curl -s -b $J $BASE/checkout/complete -o /tmp/tc_g2complete.html
grep -q "￥1,760" /tmp/tc_g2complete.html && ok "guest order placed at 1760" || ng "guest order total"
GDISC=$(db "SELECT discount_yen FROM orders WHERE user_id IS NULL ORDER BY id DESC LIMIT 1")
[ "$GDISC" = "0" ] && ok "guest order discount_yen = 0 in DB" || ng "guest order discount $GDISC"

echo "===== COD FEE FOLLOWS THE PAYMENT SELECTION ====="
# Owner decision 2026-11-22 (the EC-CUBE/Amazon pattern):
# totals shown before the payment method is chosen exclude
# payment-method fees. Keyholder 880 + shipping 550 = 1430
# on the delivery step with NO fee row; choosing COD adds
# the 330 fee (1760, itemised on the confirmation);
# switching to a card method removes it again (1430).
# Placement math is untouched -- the guest sections above
# already placed COD orders at 1760 end to end.
J=/tmp/tc_f.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/tc_fhome.html
C=$(csrf /tmp/tc_fhome.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CTF=$C"
curl -s -b $J $BASE/checkout -o /tmp/tc_fco.html
grep -q "￥1,430" /tmp/tc_fco.html && ok "delivery total excludes the COD fee (1430)" || ng "delivery total (expect 1,430)"
grep -q "代引き手数料" /tmp/tc_fco.html && ng "no COD fee row on the delivery step" || ok "no COD fee row on the delivery step"
C=$(csrf /tmp/tc_fco.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" \
  --data-urlencode "email=fee@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" --data-urlencode "phone=08011112222" \
  --data-urlencode "$CTF=$C" -o /dev/null
curl -s -b $J $BASE/checkout/payment -o /tmp/tc_fpay.html
C=$(csrf /tmp/tc_fpay.html)
curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "$CTF=$C" -o /tmp/tc_fcod.html
grep -q "￥1,760" /tmp/tc_fcod.html && grep -q "代引き手数料" /tmp/tc_fcod.html \
  && ok "choosing COD adds the fee, itemised (1760)" || ng "COD confirm (expect 1,760 + fee row)"
C=$(csrf /tmp/tc_fcod.html)
curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=credit_card" --data-urlencode "$CTF=$C" -o /tmp/tc_fcard.html
grep -q "￥1,430" /tmp/tc_fcard.html && ok "switching to card removes the fee (1430)" || ng "card confirm (expect 1,430)"
grep -q "代引き手数料" /tmp/tc_fcard.html && ng "no COD fee row for a card order" || ok "no COD fee row for a card order"
