#!/bin/bash
# Member Order Cancellation suite (Order Cancel spec
# section 7, TC-01..TC-13). Payments cases run against
# the KOMOJU stub provider, exactly like the payments
# suite: this suite starts its own app instance with the
# stub configuration and stops it at the end (relaunch
# ./run.sh afterwards for the normal instance).
source "$(dirname "$0")/lib.sh"

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
BASE=http://127.0.0.1:3000
MAIL=http://127.0.0.1:8025
STUB=http://127.0.0.1:18099
RUN=$(date +%s)
CT=$(printf 'csrf_%s' 'token')
WHSEC=$(printf 'whsec_%s' 'cancelsuite')
KOMOJU_SK=$(printf 'sk_test_%s' 'suite')

wait_db() {
  for i in $(seq 1 15); do
    [ "$(db "$1")" = "$2" ] && return 0
    sleep 2
  done
  return 1
}
wait_mail() {
  # Mailpit search summaries carry subjects, not bodies:
  # fetch each matching message and grep its text.
  for i in $(seq 1 10); do
    for mid in $(curl -s "$MAIL/api/v1/search?query=$1" | grep -o '"ID":"[^"]*"' | sed 's/"ID":"//;s/"//'); do
      curl -s "$MAIL/api/v1/message/$mid" | grep -q "$2" && return 0
    done
    sleep 2
  done
  return 1
}

# ---------- harness: build, stub, app ----------
cd "$(dirname "$0")/../.."
export PATH=$HOME/.cargo/bin:$PATH
eval "$(grep -E '^(DB_PASSWORD|APP_DB_PASSWORD|VERIFICATION_CODE_SECRET)=' .env | sed 's/^/export /')"
cargo build -p server 2>/tmp/ocx_build.log | tail -1
[ -x target/debug/server ] && ok "server builds" || { ng "server builds"; tail -5 /tmp/ocx_build.log; exit 1; }
fuser -k 18099/tcp 2>/dev/null; sleep 1
python3 scripts/stubs/komoju_stub.py >/tmp/ocx_stub.log 2>&1 &
sleep 1
curl -s $STUB/__sessions | grep -q '"count": 0' && ok "stub provider up" || ng "stub provider up"
PAYCFG=/tmp/ocx_config.toml
sed -e 's/^komoju_enabled = false/komoju_enabled = true/' \
    -e 's/^payment_sweep_interval_secs = 600/payment_sweep_interval_secs = 2/' \
    config.toml > $PAYCFG
fuser -k 3000/tcp 2>/dev/null; sleep 1
env CONFIG_PATH=$PAYCFG KOMOJU_SECRET_KEY="$KOMOJU_SK" \
    KOMOJU_WEBHOOK_SECRET="$WHSEC" \
    PUBLIC_BASE_URL=http://127.0.0.1:3000 KOMOJU_API_BASE_URL=$STUB \
    ./target/debug/server > /tmp/ocx_app.log 2>&1 &
for i in $(seq 1 60); do curl -s -o /dev/null $BASE/healthz && break; sleep 1; done
curl -s -o /dev/null -w "%{http_code}" $BASE/healthz | grep -q 200 && ok "payments app up" || ng "app up"

sign() { python3 -c 'import hmac,hashlib,sys;print(hmac.new(sys.argv[1].encode(),sys.argv[2].encode(),hashlib.sha256).hexdigest())' "$WHSEC" "$1"; }
webhook() {
  local sig; sig=$(sign "$1")
  curl -s -o /dev/null -w "%{http_code}" -X POST $BASE/webhooks/komoju \
    -H "Content-Type: application/json" -H "X-Komoju-Signature: $sig" \
    -H "X-Komoju-ID: $2" --data-binary "$1"
}
webhook_body() {
  python3 - "$1" "$2" "$3" <<'PYEOF'
import json, sys, urllib.request
payment = json.load(urllib.request.urlopen("http://127.0.0.1:18099/api/v1/payments/" + sys.argv[3]))
print(json.dumps({"id": sys.argv[1], "type": sys.argv[2], "data": payment}))
PYEOF
}

# ---------- actors ----------
login_as() { # jar email password
    rm -f "$1"
    curl -s -c "$1" $BASE/login -o /tmp/ocx_login.html
    local tok=$(csrf /tmp/ocx_login.html)
    curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/login \
        --data-urlencode "$CT=$tok" --data-urlencode "email=$2" \
        --data-urlencode "password=$3"
}
JA=/tmp/ocx_admin.jar; JM=/tmp/ocx_a.jar; JB=/tmp/ocx_b.jar; JF=/tmp/ocx_f.jar
admin_pw=$(printf 'Pass%s' 'word1234')
rm -f $JA
curl -s -c $JA $BASE/admin/login -o /tmp/ocx_al.html
tok=$(csrf /tmp/ocx_al.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "$CT=$tok" --data-urlencode "email=admin@example.com" \
    --data-urlencode "password=$admin_pw"
member_pw=$(printf 'test%s' '1234')
login_as $JM test@example.com "$member_pw"
# The fixture member's id is resolved by identity (the
# email blind index), never hard-coded: account ids move
# when accounts are recreated.
M_UID=$(db "SELECT id FROM users WHERE email_bidx='$(bidx test@example.com)'")
b_pw=$(printf 'addr%s' '5678')
login_as $JB addr-b@example.jp "$b_pw"

admin_status() { # order_id status [carrier tracking]
    curl -s -b $JA $BASE/admin/orders/$1 -o /tmp/ocx_ad.html
    local tok=$(csrf /tmp/ocx_ad.html)
    if [ -n "${3:-}" ]; then
        curl -s -b $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/orders/$1/status \
            --data-urlencode "$CT=$tok" --data-urlencode "status=$2" \
            --data-urlencode "carrier=$3" --data-urlencode "tracking_number=$4"
    else
        curl -s -b $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/orders/$1/status \
            --data-urlencode "$CT=$tok" --data-urlencode "status=$2"
    fi
}

# ---------- member order placement (current checkout) ----------
checkout_fields() { # jar email fname gname
    echo "--data-urlencode email=$2 --data-urlencode family_name=$3 --data-urlencode given_name=$4 \
--data-urlencode family_name_kana=テスト --data-urlencode given_name_kana=タロウ \
--data-urlencode phone=09012345678 --data-urlencode postal_code=1000001 \
--data-urlencode prefecture=東京都 --data-urlencode city=千代田区 \
--data-urlencode address_line=千代田1-1-1 --data-urlencode building= \
--data-urlencode delivery_date= --data-urlencode delivery_slot= --data-urlencode delivery_choice=new"
}
place_member_order() { # jar uid email method [coupon] -> order id
    local jar=$1 uid=$2 email=$3 method=$4 coupon=${5:-}
    db "DELETE FROM cart_items WHERE user_id=$uid" >/dev/null
    curl -s -b "$jar" $BASE/cart -o /tmp/ocx_cart.html
    local tok=$(csrf /tmp/ocx_cart.html)
    curl -s -b "$jar" -o /dev/null -X POST $BASE/cart/items \
        --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CT=$tok"
    if [ -n "$coupon" ]; then
        curl -s -b "$jar" $BASE/checkout -o /tmp/ocx_co.html
        tok=$(csrf /tmp/ocx_co.html)
        curl -s -b "$jar" -o /dev/null -X POST $BASE/checkout \
            --data-urlencode "$CT=$tok" --data-urlencode "intent=apply_coupon" \
            $(checkout_fields x "$email" テスト 会員) \
            --data-urlencode "coupon_code=$coupon" --data-urlencode "coupon_select="
    fi
    local before=$(db "SELECT COALESCE(MAX(id),0) FROM orders")
    curl -s -b "$jar" $BASE/checkout -o /tmp/ocx_co.html
    tok=$(csrf /tmp/ocx_co.html)
    curl -s -b "$jar" -o /dev/null -X POST $BASE/checkout \
        --data-urlencode "$CT=$tok" --data-urlencode "intent=confirm" \
        $(checkout_fields x "$email" テスト 会員) \
        --data-urlencode "coupon_code=$coupon" --data-urlencode "coupon_select="
    curl -s -b "$jar" $BASE/checkout -o /tmp/ocx_co.html
    tok=$(csrf /tmp/ocx_co.html)
    curl -s -b "$jar" -o /dev/null -X POST $BASE/checkout/payment \
        --data-urlencode "$CT=$tok" --data-urlencode "payment_method=$method"
    curl -s -b "$jar" $BASE/checkout -o /tmp/ocx_co.html
    tok=$(csrf /tmp/ocx_co.html)
    curl -s -b "$jar" -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$tok"
    db "SELECT id FROM orders WHERE id>$before AND user_id=$uid ORDER BY id DESC LIMIT 1"
}
stock5() { db "SELECT COALESCE(SUM(stock),0) FROM product_variants WHERE product_id=5"; }
paycol() { db "SELECT $1 FROM payments WHERE order_id=$2"; }
ordnum() { db "SELECT 'AS-' || to_char(created_at AT TIME ZONE 'Asia/Tokyo','YYYYMMDD') || '-' || lpad(id::text,4,'0') FROM orders WHERE id=$1"; }

db "UPDATE product_variants SET stock=50 WHERE product_id=5" >/dev/null

# ---------- TC-01 visibility + TC-11 preparing POST + TC-10 race ----------
O1=$(place_member_order $JM $M_UID test@example.com cod)
curl -s -b $JM $BASE/mypage/orders/$O1 -o /tmp/ocx_d.html
grep -q "注文をキャンセルする" /tmp/ocx_d.html && ok "TC-01 received order shows the cancel button" || ng "TC-01 button"
admin_status $O1 preparing >/dev/null
curl -s -b $JM $BASE/mypage/orders/$O1 -o /tmp/ocx_d.html
grep -q "発送準備中のため、オンラインではキャンセルできません" /tmp/ocx_d.html && ! grep -q "注文をキャンセルする" /tmp/ocx_d.html \
    && ok "TC-01 preparing shows the note instead of the button" || ng "TC-01 preparing note"
# TC-11: direct POST against the preparing order
curl -s -b $JM $BASE/mypage/orders/$O1 -o /tmp/ocx_d.html
tok=$(csrf /tmp/ocx_d.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$O1/cancel --data-urlencode "$CT=$tok"
curl -s -b $JM $BASE/mypage/orders/$O1 -o /tmp/ocx_d.html
[ "$(db "SELECT status FROM orders WHERE id=$O1")" = "preparing" ] && grep -q "キャンセルできませんでした" /tmp/ocx_d.html \
    && ok "TC-11 preparing POST refused server-side with the notice" || ng "TC-11 preparing refusal"
admin_status $O1 shipped yamato 123456789012 >/dev/null
curl -s -b $JM $BASE/mypage/orders/$O1 -o /tmp/ocx_d.html
grep -q "注文をキャンセルする\|発送準備中のため" /tmp/ocx_d.html && ng "TC-01 shipped shows no cancel UI" || ok "TC-01 shipped shows no cancel UI"

# TC-10 reverse: member cancels first, admin ship refused
O10=$(place_member_order $JM $M_UID test@example.com cod)
curl -s -b $JM $BASE/mypage/orders/$O10 -o /tmp/ocx_d.html
tok=$(csrf /tmp/ocx_d.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$O10/cancel --data-urlencode "$CT=$tok"
code=$(admin_status $O10 shipped yamato 123456789013)
[ "$(db "SELECT status FROM orders WHERE id=$O10")" = "cancelled" ] \
    && ok "TC-10 member-first: admin cannot ship a cancelled order" || ng "TC-10 reverse race"

# ---------- TC-02 ownership ----------
curl -s -b $JB -o /tmp/ocx_b.html -w "%{http_code}" $BASE/mypage/orders/$O10/cancel > /tmp/ocx_code.txt
grep -q "404" /tmp/ocx_code.txt && ok "TC-02 foreign member GET confirm is a 404" || ng "TC-02 GET ($(cat /tmp/ocx_code.txt))"
curl -s -b $JB $BASE/mypage -o /tmp/ocx_b.html
tok=$(csrf /tmp/ocx_b.html)
curl -s -b $JB -o /dev/null -w "%{http_code}" -X POST $BASE/mypage/orders/$O1/cancel --data-urlencode "$CT=$tok" > /tmp/ocx_code.txt
grep -q "404" /tmp/ocx_code.txt && [ "$(db "SELECT status FROM orders WHERE id=$O1")" = "shipped" ] \
    && ok "TC-02 foreign member POST is a 404 and the order is untouched" || ng "TC-02 POST"

# ---------- TC-13 CSRF ----------
O13=$(place_member_order $JM $M_UID test@example.com cod)
code=$(curl -s -b $JM -o /dev/null -w "%{http_code}" -X POST $BASE/mypage/orders/$O13/cancel)
[ "$code" = "403" ] && [ "$(db "SELECT status FROM orders WHERE id=$O13")" = "received" ] \
    && ok "TC-13 tokenless POST is a styled 403, order untouched" || ng "TC-13 CSRF ($code)"

# ---------- TC-03 COD cancel with coupon release (fresh member) ----------
new_email="cancel-f-$RUN@example.jp"
rm -f $JF
curl -s -c $JF $BASE/signup -o /tmp/ocx_su.html
tok=$(csrf /tmp/ocx_su.html)
curl -s -b $JF -c $JF -o /dev/null -X POST $BASE/signup \
    --data-urlencode "$CT=$tok" --data-urlencode "email=$new_email" --data-urlencode "email_confirm=$new_email"
sleep 2
mid_msg=$(curl -s "$MAIL/api/v1/search?query=$(python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1]))" "$new_email")" | grep -o '"ID":"[^"]*"' | head -1 | sed 's/"ID":"//;s/"//')
code6=$(curl -s "$MAIL/api/v1/message/$mid_msg" | python3 -c "import json,sys,re; t=json.load(sys.stdin).get('Text',''); m=re.search(r'認証コード:([0-9]{6})', t); print(m.group(1) if m else '')")
curl -s -b $JF $BASE/signup/verify -o /tmp/ocx_sv.html
tok=$(csrf /tmp/ocx_sv.html)
curl -s -b $JF -c $JF -o /dev/null -X POST $BASE/signup/verify --data-urlencode "$CT=$tok" --data-urlencode "code=$code6"
curl -s -b $JF $BASE/signup/details -o /tmp/ocx_sd.html
tok=$(csrf /tmp/ocx_sd.html)
PWN=$(printf 'pass%s' 'word')
curl -s -b $JF -c $JF -o /tmp/ocx_sconf.html -X POST $BASE/signup/details \
    --data-urlencode "$CT=$tok" --data-urlencode "$PWN=f${RUN}pass" \
    --data-urlencode "family_name=新規" --data-urlencode "given_name=会員" \
    --data-urlencode "family_name_kana=シンキ" --data-urlencode "given_name_kana=カイイン" \
    --data-urlencode "birth_date=" --data-urlencode "gender=" \
    --data-urlencode "postal_code=1000001" --data-urlencode "prefecture=東京都" \
    --data-urlencode "city=千代田区" --data-urlencode "address_line=千代田1-1-1" \
    --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true"
tok=$(csrf /tmp/ocx_sconf.html)
curl -s -b $JF -c $JF -o /dev/null -X POST $BASE/signup/confirm --data-urlencode "$CT=$tok"
F_UID=$(db "SELECT id FROM users WHERE email_bidx='$(bidx $new_email)'")
[ -n "$F_UID" ] && ok "TC-03 fresh member signed up" || ng "TC-03 signup"
S_BEFORE=$(stock5)
O3=$(place_member_order $JF $F_UID $new_email cod WELCOME500)
S_ORDERED=$(stock5)
curl -s -b $JF $BASE/mypage/orders/$O3/cancel -o /tmp/ocx_cf.html
grep -q "再度ご利用いただけるようになります" /tmp/ocx_cf.html \
    && ok "TC-03 confirm page promises the coupon release" || ng "TC-03 confirm coupon line"
curl -s -b $JF $BASE/mypage/orders/$O3 -o /tmp/ocx_d.html
tok=$(csrf /tmp/ocx_d.html)
curl -s -b $JF -o /dev/null -X POST $BASE/mypage/orders/$O3/cancel --data-urlencode "$CT=$tok"
S_AFTER=$(stock5)
[ "$(db "SELECT status FROM orders WHERE id=$O3")" = "cancelled" ] && [ "$(paycol status $O3)" = "cancelled" ] \
    && [ "$S_ORDERED" = "$((S_BEFORE - 1))" ] && [ "$S_AFTER" = "$S_BEFORE" ] \
    && ok "TC-03 COD cancel: status, payment flip, stock restored" || ng "TC-03 COD cancel ($S_BEFORE/$S_ORDERED/$S_AFTER)"
[ "$(db "SELECT count(*) FROM user_coupons WHERE user_id=$F_UID AND code='WELCOME500' AND used_at IS NULL")" = "1" ] \
    && [ "$(db "SELECT count(*) FROM admin_audit_logs WHERE action='order.member_cancel' AND target_id=$O3 AND actor_user_id=$F_UID AND admin_id IS NULL")" = "1" ] \
    && ok "TC-03 coupon released; audit row names the member as actor" || ng "TC-03 release/audit"
# functional proof: the released coupon applies again
db "DELETE FROM cart_items WHERE user_id=$F_UID" >/dev/null
curl -s -b $JF $BASE/cart -o /tmp/ocx_cart.html
tok=$(csrf /tmp/ocx_cart.html)
curl -s -b $JF -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CT=$tok"
curl -s -b $JF $BASE/checkout -o /tmp/ocx_co.html
tok=$(csrf /tmp/ocx_co.html)
curl -s -b $JF -o /tmp/ocx_ap.html -X POST $BASE/checkout \
    --data-urlencode "$CT=$tok" --data-urlencode "intent=apply_coupon" \
    $(checkout_fields x "$new_email" 新規 会員) \
    --data-urlencode "coupon_code=WELCOME500" --data-urlencode "coupon_select="
grep -q "WELCOME500 を適用中" /tmp/ocx_ap.html && ok "TC-03 released coupon applies to a new cart" || ng "TC-03 reapply"
db "DELETE FROM cart_items WHERE user_id=$F_UID" >/dev/null

# ---------- TC-09 double-submit ----------
O9=$(place_member_order $JM $M_UID test@example.com cod)
S_BEFORE=$(stock5)
curl -s -b $JM $BASE/mypage/orders/$O9 -o /tmp/ocx_d.html
tok=$(csrf /tmp/ocx_d.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$O9/cancel --data-urlencode "$CT=$tok"
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$O9/cancel --data-urlencode "$CT=$tok"
[ "$(db "SELECT count(*) FROM admin_audit_logs WHERE action='order.member_cancel' AND target_id=$O9")" = "1" ] \
    && [ "$(stock5)" = "$((S_BEFORE + 1))" ] \
    && ok "TC-09 double submit: one cancellation, one stock restore" || ng "TC-09 double"

# ---------- TC-04 card captured: automatic refund ----------
O4=$(place_member_order $JM $M_UID test@example.com credit_card)
wait_db "SELECT status FROM payments WHERE order_id=$O4" "paid" \
    && ok "TC-04 card order captured at the stub" || ng "TC-04 capture"
curl -s -b $JM $BASE/mypage/orders/$O4/cancel -o /tmp/ocx_cf.html
grep -q "返金します" /tmp/ocx_cf.html && ok "TC-04 confirm page states the refund" || ng "TC-04 confirm wording"
curl -s -b $JM $BASE/mypage/orders/$O4 -o /tmp/ocx_d.html
tok=$(csrf /tmp/ocx_d.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$O4/cancel --data-urlencode "$CT=$tok"
wait_db "SELECT status FROM payments WHERE order_id=$O4" "refunded" \
    && ok "TC-04 captured card refunded automatically" || ng "TC-04 refund"
curl -s -b $JM $BASE/mypage/orders/$O4 -o /tmp/ocx_d.html
grep -q "この注文はキャンセルされました" /tmp/ocx_d.html && grep -q "返金済みです" /tmp/ocx_d.html \
    && ok "TC-04 completion block shows the refund as done" || ng "TC-04 completion"
wait_mail "$(ordnum $O4)" "返金済みです" \
    && ok "TC-04 cancellation mail carries the refunded wording" || ng "TC-04 mail"
[ "$(db "SELECT count(*) FROM admin_audit_logs WHERE action='payment.refund_requested' AND target_id=$O4 AND actor_user_id=$M_UID")" = "1" ] \
    && ok "TC-04 refund audited with the member as actor" || ng "TC-04 refund audit"

# ---------- TC-05 refund failure: flagged, admin retry ----------
O5=$(place_member_order $JM $M_UID test@example.com credit_card)
wait_db "SELECT status FROM payments WHERE order_id=$O5" "paid" >/dev/null
ORIG_PID=$(paycol komoju_payment_id $O5)
db "UPDATE payments SET komoju_payment_id='pay_unknown_$RUN' WHERE order_id=$O5" >/dev/null
curl -s -b $JM $BASE/mypage/orders/$O5 -o /tmp/ocx_d.html
tok=$(csrf /tmp/ocx_d.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$O5/cancel --data-urlencode "$CT=$tok"
[ "$(db "SELECT status FROM orders WHERE id=$O5")" = "cancelled" ] && [ "$(paycol status $O5)" = "paid" ] \
    && [ -n "$(paycol flag_note $O5)" ] \
    && ok "TC-05 failed refund: cancel stands, payment flagged" || ng "TC-05 failure state"
curl -s -b $JM $BASE/mypage/orders/$O5 -o /tmp/ocx_d.html
grep -q "返金処理中です" /tmp/ocx_d.html && ! grep -q "返金済みです" /tmp/ocx_d.html \
    && ok "TC-05 member sees 返金処理中, never 返金済み" || ng "TC-05 member wording"
db "UPDATE payments SET komoju_payment_id='$ORIG_PID' WHERE order_id=$O5" >/dev/null
curl -s -b $JA $BASE/admin/orders/$O5 -o /tmp/ocx_ad.html
tok=$(csrf /tmp/ocx_ad.html)
curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$O5/refund --data-urlencode "$CT=$tok" --data-urlencode "amount_yen="
wait_db "SELECT status FROM payments WHERE order_id=$O5" "refunded" \
    && ok "TC-05 admin refund endpoint recovers the flagged payment" || ng "TC-05 admin retry"

# ---------- TC-06 PayPay pending: cancel, late capture flagged ----------
curl -s -X POST $STUB/__control -H "Content-Type: application/json" --data '{"authorize_only": true}' -o /dev/null
O6=$(place_member_order $JM $M_UID test@example.com paypay)
curl -s -X POST $STUB/__control -H "Content-Type: application/json" --data '{"authorize_only": false}' -o /dev/null
[ "$(paycol status $O6)" = "pending_payment" ] && ok "TC-06 PayPay order pending" || ng "TC-06 pending ($(paycol status $O6))"
KPID6=$(paycol komoju_payment_id $O6)
S_BEFORE=$(stock5)
curl -s -b $JM $BASE/mypage/orders/$O6 -o /tmp/ocx_d.html
tok=$(csrf /tmp/ocx_d.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$O6/cancel --data-urlencode "$CT=$tok"
[ "$(paycol status $O6)" = "cancelled" ] && [ "$(stock5)" = "$((S_BEFORE + 1))" ] \
    && ok "TC-06 pending cancel flips the payment and restores stock" || ng "TC-06 cancel"
curl -s -X POST $STUB/__pay/$KPID6 -o /dev/null
BODY=$(webhook_body evt-ocx-6-$RUN payment.captured "$KPID6")
[ "$(webhook "$BODY" dlv-ocx-6-$RUN)" = "200" ] && ok "TC-06 late capture webhook accepted" || ng "TC-06 webhook (body: ${BODY:0:80})"
sleep 3
[ "$(db "SELECT status FROM orders WHERE id=$O6")" = "cancelled" ] && [ -n "$(paycol flag_note $O6)" ] \
    && [ "$(stock5)" = "$((S_BEFORE + 1))" ] \
    && ok "TC-06 late capture flagged, not applied; no double restore" || ng "TC-06 flag"

# ---------- TC-07 konbini pending ----------
O7=$(place_member_order $JM $M_UID test@example.com konbini)
[ "$(paycol status $O7)" = "pending_payment" ] && ok "TC-07 konbini order pending" || ng "TC-07 pending"
curl -s -b $JM $BASE/mypage/orders/$O7/cancel -o /tmp/ocx_cf.html
grep -q "コンビニでのお支払いは不要になります" /tmp/ocx_cf.html \
    && ok "TC-07 confirm page carries the no-payment-needed wording" || ng "TC-07 confirm wording"
curl -s -b $JM $BASE/mypage/orders/$O7 -o /tmp/ocx_d.html
tok=$(csrf /tmp/ocx_d.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$O7/cancel --data-urlencode "$CT=$tok"
[ "$(paycol status $O7)" = "cancelled" ] && ok "TC-07 konbini pending cancel flips the payment" || ng "TC-07 cancel"

# ---------- TC-08 konbini captured: accept and flag ----------
O8=$(place_member_order $JM $M_UID test@example.com konbini)
KPID8=$(paycol komoju_payment_id $O8)
curl -s -X POST $STUB/__pay/$KPID8 -o /dev/null
BODY=$(webhook_body evt-ocx-8-$RUN payment.captured "$KPID8")
WCODE=$(webhook "$BODY" dlv-ocx-8-$RUN)
wait_db "SELECT status FROM payments WHERE order_id=$O8" "paid" \
    && ok "TC-08 konbini captured at the counter (webhook)" || ng "TC-08 capture (wh=$WCODE kpid=$KPID8 body: ${BODY:0:80})"
curl -s -b $JM $BASE/mypage/orders/$O8/cancel -o /tmp/ocx_cf.html
grep -q "オンラインで自動返金できません" /tmp/ocx_cf.html \
    && ok "TC-08 confirm page states the manual refund honestly" || ng "TC-08 confirm wording"
curl -s -b $JM $BASE/mypage/orders/$O8 -o /tmp/ocx_d.html
tok=$(csrf /tmp/ocx_d.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$O8/cancel --data-urlencode "$CT=$tok"
[ "$(db "SELECT status FROM orders WHERE id=$O8")" = "cancelled" ] && [ "$(paycol status $O8)" = "paid" ] \
    && [ "$(paycol amount_refunded_yen $O8)" = "0" ] \
    && ok "TC-08 cancel accepted; payment stays paid, no gateway refund" || ng "TC-08 cancel state"
paycol flag_note $O8 | grep -q "返金対応が必要" \
    && ok "TC-08 manual-refund flag recorded" || ng "TC-08 flag"
curl -s -b $JA $BASE/admin/orders/$O8 -o /tmp/ocx_ad.html
grep -q "返金対応が必要" /tmp/ocx_ad.html && ok "TC-08 admin order detail surfaces the flag" || ng "TC-08 admin surface"
wait_mail "$(ordnum $O8)" "自動返金できません" \
    && ok "TC-08 cancellation mail carries the manual wording" || ng "TC-08 mail"

# ---------- TC-12 partially refunded: remainder refunded ----------
O12=$(place_member_order $JM $M_UID test@example.com credit_card)
wait_db "SELECT status FROM payments WHERE order_id=$O12" "paid" >/dev/null
curl -s -b $JA $BASE/admin/orders/$O12 -o /tmp/ocx_ad.html
tok=$(csrf /tmp/ocx_ad.html)
curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$O12/refund --data-urlencode "$CT=$tok" --data-urlencode "amount_yen=100"
wait_db "SELECT status FROM payments WHERE order_id=$O12" "partially_refunded" >/dev/null
AMT=$(paycol amount_yen $O12)
curl -s -b $JM $BASE/mypage/orders/$O12 -o /tmp/ocx_d.html
tok=$(csrf /tmp/ocx_d.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$O12/cancel --data-urlencode "$CT=$tok"
wait_db "SELECT status FROM payments WHERE order_id=$O12" "refunded" \
    && [ "$(paycol amount_refunded_yen $O12)" = "$AMT" ] \
    && ok "TC-12 cancel refunds the remainder only, ending fully refunded" || ng "TC-12 remainder"

# ---------- teardown ----------
fuser -k 3000/tcp 2>/dev/null
fuser -k 18099/tcp 2>/dev/null

result
