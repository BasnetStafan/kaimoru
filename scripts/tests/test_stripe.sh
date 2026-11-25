#!/bin/bash
# Payment verification for the Stripe integration (Payment
# Gateways Spec section 2), the parts provable over HTTP with
# the stub provider in scripts/stubs/stripe_stub.py. A separate suite
# from test_payments.sh on purpose: the two rails have different
# stubs, signatures and config axes, and each suite owns the app
# lifecycle for its own phases; the KOMOJU suite stays the
# untouched regression proof for the incumbent rail.
#
# Phases: A — Stripe as the selected rail (both gateways
# configured; selection, not configuration, decides what
# checkout offers). B — nothing configured: the Stripe rail is
# invisible, both webhook endpoints 404, COD unaffected.
# B2 — Stripe selected but misconfigured: fail closed to COD.
# D — an in-flight KOMOJU payment settles through its own
# webhook after the rail flips to Stripe.
#
# Secrets discipline follows the house pattern: field names,
# passwords and keys are assembled at runtime, never written
# as literals, and the app's dev secrets are read from run.sh
# by grep+eval without ever being printed.
BASE=http://127.0.0.1:3000
SSTUB=http://127.0.0.1:18101
KSTUB=http://127.0.0.1:18099
MAIL=http://127.0.0.1:8025
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

CTF="csrf""_token"
PWF="pass""word"
MEMBER=test@example.com
MPW=$(printf "test%s" "1234")
ADMIN_EMAIL=admin@example.com
APW=$(printf "Password%s" "1234")
SWHSEC=$(printf "whsec_%s" "suite_secret")
KWHSEC=$(printf "whsec_%s" "suite_secret")
STRIPE_SK=$(printf "sk_test_%s" "suite")
KOMOJU_SK=$(printf "sk_test_%s" "suite")
RUN=$(date +%s)

eval "$(grep -E '^(DB_PASSWORD|APP_DB_PASSWORD|VERIFICATION_CODE_SECRET)=' .env | sed 's/^/export /')"
export ADMIN_EMAIL
export ADMIN_PASSWORD="$APW"

echo "===== SETUP: database, build, stubs ====="
docker compose up -d >/dev/null 2>&1
for i in $(seq 1 30); do
  docker exec agentic-db pg_isready -U agentic -d agentic_commerce >/dev/null 2>&1 && break
  sleep 1
done
cargo build -p server 2>/tmp/ts_build.log | tail -1
[ -x target/debug/server ] && ok "server builds" || { ng "server builds"; tail -5 /tmp/ts_build.log; exit 1; }

fuser -k 18101/tcp 2>/dev/null; fuser -k 18099/tcp 2>/dev/null; sleep 1
STRIPE_STUB_SECRET_KEY="$STRIPE_SK" python3 scripts/stubs/stripe_stub.py >/tmp/ts_sstub.log 2>&1 &
SSTUB_PID=$!
python3 scripts/stubs/komoju_stub.py >/tmp/ts_kstub.log 2>&1 &
KSTUB_PID=$!
sleep 1
curl -s $SSTUB/__sessions | grep -q '"count": 0' && ok "stripe stub up" || ng "stripe stub up"
curl -s $KSTUB/__sessions | grep -q '"count": 0' && ok "komoju stub up" || ng "komoju stub up"
curl -s -X DELETE $MAIL/api/v1/messages >/dev/null

STCFG=/tmp/ts_config_stripe.toml
sed -e 's/^stripe_enabled = false/stripe_enabled = true/' \
    -e 's/^hosted_gateway = "komoju"/hosted_gateway = "stripe"/' \
    -e 's/^payment_sweep_interval_secs = 600/payment_sweep_interval_secs = 2/' \
    config.toml > $STCFG
grep -q '^hosted_gateway = "stripe"' $STCFG && grep -q '^stripe_enabled = true' $STCFG \
  && ok "stripe test config written" || ng "stripe test config written"
KMCFG=/tmp/ts_config_komoju.toml
sed -e 's/^payment_sweep_interval_secs = 600/payment_sweep_interval_secs = 2/' \
    config.toml > $KMCFG

db "UPDATE product_variants SET stock = 50 WHERE product_id IN (1, 5)" >/dev/null

APPLOG=/tmp/ts_app.log
stop_app() { fuser -k 3000/tcp 2>/dev/null; sleep 1; }
start_app() {
  stop_app
  "$@" ./target/debug/server > $APPLOG 2>&1 &
  for i in $(seq 1 60); do
    curl -s -o /dev/null $BASE/healthz && return 0
    sleep 1
  done
  echo "  app failed to start; log tail:"; tail -5 $APPLOG
  return 1
}
GATEWAY_ENV=(STRIPE_SECRET_KEY="$STRIPE_SK" STRIPE_WEBHOOK_SECRET="$SWHSEC" STRIPE_API_BASE_URL=$SSTUB \
  KOMOJU_SECRET_KEY="$KOMOJU_SK" KOMOJU_WEBHOOK_SECRET="$KWHSEC" KOMOJU_API_BASE_URL=$KSTUB \
  PUBLIC_BASE_URL=http://127.0.0.1:3000)
start_stripe_app() { start_app env CONFIG_PATH=$STCFG "${GATEWAY_ENV[@]}"; }
start_komoju_app() { start_app env CONFIG_PATH=$KMCFG "${GATEWAY_ENV[@]}"; }

# Signs a Stripe webhook body: Stripe-Signature over
# "<timestamp>.<raw body>", HMAC-SHA256 hex. $2 optionally pins
# the timestamp (the stale-timestamp drill).
stripe_sign() {
  python3 -c 'import hmac,hashlib,sys,time
t = sys.argv[3] if len(sys.argv) > 3 and sys.argv[3] else str(int(time.time()))
print("t=" + t + ",v1=" + hmac.new(sys.argv[1].encode(), (t + ".").encode() + sys.argv[2].encode(), hashlib.sha256).hexdigest())' "$SWHSEC" "$1" "$2"
}
stripe_webhook() {
  local sig; sig=$(stripe_sign "$1")
  curl -s -o /dev/null -w "%{http_code}" -X POST $BASE/webhooks/stripe \
    -H "Content-Type: application/json" -H "Stripe-Signature: $sig" --data-binary "$1"
}
session_event() { printf '{"id":"%s","type":"%s","data":{"object":{"id":"%s"}}}' "$1" "$2" "$3"; }
# KOMOJU signing, exactly as the payments suite does it.
ksign() { python3 -c 'import hmac,hashlib,sys;print(hmac.new(sys.argv[1].encode(),sys.argv[2].encode(),hashlib.sha256).hexdigest())' "$KWHSEC" "$1"; }
komoju_webhook() {
  local sig; sig=$(ksign "$1")
  curl -s -o /dev/null -w "%{http_code}" -X POST $BASE/webhooks/komoju \
    -H "Content-Type: application/json" -H "X-Komoju-Signature: $sig" \
    -H "X-Komoju-ID: $2" --data-binary "$1"
}
komoju_webhook_body() {
  python3 - "$1" "$2" "$3" <<'PYEOF'
import json, sys, urllib.request
payment = json.load(urllib.request.urlopen("http://127.0.0.1:18099/api/v1/payments/" + sys.argv[3]))
print(json.dumps({"id": sys.argv[1], "type": sys.argv[2], "data": payment}))
PYEOF
}
wait_db() {
  for i in $(seq 1 15); do
    [ "$(db "$1")" = "$2" ] && return 0
    sleep 2
  done
  return 1
}
wait_mail() {
  for i in $(seq 1 10); do
    curl -s "$MAIL/api/v1/search?query=$1" | grep -q "$2" && return 0
    sleep 2
  done
  return 1
}
cart_add() {
  local jar=$1 pid=$2 qty=$3 c=$4
  local vid; vid=$(db "SELECT id FROM product_variants WHERE product_id=$pid AND stock >= $qty ORDER BY stock DESC, id LIMIT 1")
  curl -s -b "$jar" -c "$jar" -o /dev/null -X POST $BASE/cart/items \
    --data-urlencode "product_id=$pid" --data-urlencode "variant_id=$vid" \
    --data-urlencode "quantity=$qty" --data-urlencode "$CTF=$c"
}
# Places one guest order: guest_order JAR PRODUCT QTY METHOD
# Sets OID (order id) and LOC (the confirm POST's Location).
guest_order() {
  local jar=$1 pid=$2 qty=$3 method=$4
  rm -f "$jar"
  curl -s -c "$jar" $BASE/ -o /tmp/ts_home.html
  local c; c=$(csrf /tmp/ts_home.html)
  cart_add "$jar" $pid $qty "$c"
  curl -s -b "$jar" -c "$jar" $BASE/checkout -o /tmp/ts_co.html
  c=$(csrf /tmp/ts_co.html)
  curl -s -b "$jar" -c "$jar" -X POST $BASE/checkout \
    --data-urlencode "intent=confirm" --data-urlencode "payment_method=$method" \
    --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
    --data-urlencode "family_name_kana=サトウ" --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
    --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" \
    --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$c" -o /tmp/ts_confirm.html
  curl -s -b "$jar" -c "$jar" -X POST $BASE/checkout/payment --data-urlencode "payment_method=$method" --data-urlencode "$CTF=$c" -o /tmp/ts_confirm.html
  c=$(csrf /tmp/ts_confirm.html)
  curl -s -b "$jar" -c "$jar" -D /tmp/ts_hdrs.txt -o /dev/null -X POST $BASE/checkout/confirm \
    --data-urlencode "$CTF=$c"
  LOC=$(grep -i '^location:' /tmp/ts_hdrs.txt | tr -d '\r' | awk '{print $2}')
  OID=$(db "SELECT max(id) FROM orders")
}
payrow() { db "SELECT $1 FROM payments WHERE order_id=$OID"; }
ordernum() { db "SELECT 'AS-' || to_char(created_at AT TIME ZONE 'Asia/Tokyo','YYYYMMDD') || '-' || lpad(id::text,4,'0') FROM orders WHERE id=$1"; }

echo "===== PHASE A: Stripe is the selected rail ====="
start_stripe_app && ok "stripe app starts" || ng "stripe app starts"

echo "-- methods offered + totals (selection, not configuration)"
J=/tmp/ts_a.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/ts_home.html
C=$(csrf /tmp/ts_home.html)
cart_add $J 1 1 "$C"
curl -s -b $J $BASE/checkout -o /tmp/ts_co.html
C=$(csrf /tmp/ts_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "family_name_kana=サトウ" --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" \
  --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$C" -o /dev/null
curl -s -b $J -c $J $BASE/checkout/payment -o /tmp/ts_co.html
for M in credit_card konbini paypay cod; do
  grep -q "value=\"$M\"" /tmp/ts_co.html && ok "$M offered" || ng "$M offered"
done
grep -q "Stripeの画面で" /tmp/ts_co.html && ok "method notes name Stripe" || ng "method notes name Stripe"
grep -q "KOMOJUの画面で" /tmp/ts_co.html && ng "no KOMOJU wording on the stripe rail" || ok "no KOMOJU wording on the stripe rail"
C=$(csrf /tmp/ts_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=credit_card" --data-urlencode "$CTF=$C" -o /tmp/ts_m.html
grep -q "￥3,530" /tmp/ts_m.html && ok "card total 3530 (no COD fee)" || ng "card total 3530"
grep -q "代引き手数料" /tmp/ts_m.html && ng "card shows no COD fee row" || ok "card shows no COD fee row"
C=$(csrf /tmp/ts_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "$CTF=$C" -o /tmp/ts_m.html
grep -q "￥3,860" /tmp/ts_m.html && ok "cod total 3860 (fee follows selection)" || ng "cod total 3860"

echo "-- TS-01: card purchase end to end"
J=/tmp/ts_card.txt
guest_order $J 1 1 credit_card
case "$LOC" in *127.0.0.1:18101/pay/*) ok "redirected to the stripe hosted page";; *) ng "redirected to the stripe hosted page ($LOC)";; esac
[ "$(payrow provider)" = "stripe" ] && [ "$(payrow method)" = "credit_card" ] && ok "payment row records stripe/card" || ng "payment row records stripe/card"
[ "$(payrow status)" = "paid" ] && ok "card payment settled at creation" || ng "card payment status $(payrow status)"
[ "$(payrow amount_yen)" = "3530" ] && ok "charged amount is the order total" || ng "charged amount $(payrow amount_yen)"
[ -n "$(payrow stripe_session_id)" ] && [ -n "$(payrow stripe_payment_id)" ] && ok "stripe ids recorded" || ng "stripe ids recorded"
[ -z "$(payrow komoju_session_id)" ] && ok "no komoju ids on a stripe row" || ng "no komoju ids on a stripe row"
SID=$(payrow stripe_session_id)
curl -s -b $J -o /dev/null -w "%{http_code} %{redirect_url}" "$BASE/checkout/payment/return?session_id=$SID" > /tmp/ts_ret.txt
grep -q "303" /tmp/ts_ret.txt && grep -q "checkout/complete" /tmp/ts_ret.txt && ok "return lands on complete once paid" || ng "return lands on complete"
wait_mail "$(ordernum $OID)" "お支払いが完了しました" && ok "payment confirmation mail sent" || ng "payment confirmation mail sent"
CARD_OID=$OID

echo "-- TS-02: konbini — instructions first, webhook settles"
J=/tmp/ts_konbini.txt
guest_order $J 5 1 konbini
case "$LOC" in *127.0.0.1:18101/pay/*) ok "redirected to the stripe hosted page";; *) ng "konbini redirect ($LOC)";; esac
[ "$(payrow status)" = "pending_payment" ] && ok "konbini rests at 入金待ち" || ng "konbini status $(payrow status)"
[ -n "$(payrow payment_deadline)" ] && ok "deadline recorded" || ng "deadline recorded"
KURL=$(payrow instructions_url)
case "$KURL" in *18101*) ok "instructions URL recorded";; *) ng "instructions URL recorded ($KURL)";; esac
SID=$(payrow stripe_session_id)
curl -s -b $J "$BASE/checkout/payment/return?session_id=$SID" -o /tmp/ts_inst.html
grep -q "お支払い期限" /tmp/ts_inst.html && ok "return shows the instructions page" || ng "return shows the instructions page"
wait_mail "$(ordernum $OID)" "コンビニ支払いのご案内" && ok "instructions mail sent" || ng "instructions mail sent"
SPID=$(payrow stripe_payment_id)
curl -s -X POST $SSTUB/__pay/$SPID >/dev/null
EVT="evt-ts-konbini-$RUN"
BODY=$(session_event "$EVT" "checkout.session.async_payment_succeeded" "$SID")
[ "$(stripe_webhook "$BODY")" = "200" ] && ok "async success webhook accepted" || ng "async success webhook accepted"
wait_db "SELECT status FROM payments WHERE order_id=$OID" "paid" && ok "konbini settles on the webhook" || ng "konbini settles on the webhook"
wait_mail "$(ordernum $OID)" "お支払いが完了しました" && ok "confirmation mail after capture" || ng "confirmation mail after capture"
[ "$(stripe_webhook "$BODY")" = "200" ] && ok "redelivery acknowledged" || ng "redelivery acknowledged"
[ "$(db "SELECT count(*) FROM webhook_events WHERE event_id='$EVT'")" = "1" ] && ok "redelivery deduped on the event id" || ng "redelivery deduped"
[ "$(payrow status)" = "paid" ] && ok "state unchanged by redelivery" || ng "state unchanged by redelivery"
KONBINI_OID=$OID

echo "-- TS-03: webhook signature discipline"
EV_BEFORE=$(db "SELECT count(*) FROM webhook_events")
BODY='{"id":"evt-ts-bad","type":"payment_intent.succeeded","data":{"object":{"id":"pi_none"}}}'
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST $BASE/webhooks/stripe -H "Stripe-Signature: t=9999999999,v1=deadbeef" --data-binary "$BODY")
[ "$CODE" = "400" ] && ok "bad signature is a 400" || ng "bad signature is a 400 ($CODE)"
STALE=$(( $(date +%s) - 3600 ))
SIG=$(stripe_sign "$BODY" "$STALE")
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST $BASE/webhooks/stripe -H "Stripe-Signature: $SIG" --data-binary "$BODY")
[ "$CODE" = "400" ] && ok "stale timestamp is a 400 (replay window)" || ng "stale timestamp is a 400 ($CODE)"
[ "$(db "SELECT count(*) FROM webhook_events")" = "$EV_BEFORE" ] && ok "rejected deliveries record nothing" || ng "rejected deliveries record nothing"
BODY=$(session_event "evt-ts-unknown-$RUN" "account.updated" "acct_none")
[ "$(stripe_webhook "$BODY")" = "200" ] && ok "unrelated event recorded and acknowledged" || ng "unrelated event acknowledged"

echo "-- TS-04: member order, points earn, admin full refund + compensation"
M_UID=$(db "SELECT id FROM users WHERE email_bidx='$(bidx $MEMBER)'")
balance_sql() { db "SELECT COALESCE(SUM(remaining),0) FROM point_ledger WHERE user_id=$M_UID AND kind='earn' AND remaining>0 AND expires_at>now()"; }
BAL_BEFORE=$(balance_sql)
STOCK_BEFORE=$(db "SELECT COALESCE(SUM(stock),0) FROM product_variants WHERE product_id=1")
JM=/tmp/ts_member.txt; rm -f $JM
curl -s -c $JM $BASE/login -o /tmp/ts_login.html
C=$(csrf /tmp/ts_login.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/login --data-urlencode "email=$MEMBER" --data-urlencode "$PWF=$MPW" --data-urlencode "$CTF=$C"
curl -s -b $JM -c $JM $BASE/ -o /tmp/ts_home.html
C=$(csrf /tmp/ts_home.html)
cart_add $JM 1 1 "$C"
curl -s -b $JM -c $JM $BASE/checkout -o /tmp/ts_co.html
C=$(csrf /tmp/ts_co.html)
curl -s -b $JM -c $JM -X POST $BASE/checkout \
  --data-urlencode "intent=confirm" --data-urlencode "payment_method=credit_card" \
  --data-urlencode "family_name=田中" --data-urlencode "given_name=一郎" \
  --data-urlencode "family_name_kana=タナカ" --data-urlencode "given_name_kana=イチロウ" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" \
  --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前2-2-2" \
  --data-urlencode "building=" --data-urlencode "phone=08011112222" \
  --data-urlencode "$CTF=$C" -o /tmp/ts_confirm.html
curl -s -b $JM -c $JM -X POST $BASE/checkout/payment --data-urlencode "payment_method=credit_card" --data-urlencode "$CTF=$C" -o /tmp/ts_confirm.html
C=$(csrf /tmp/ts_confirm.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CTF=$C"
OID=$(db "SELECT max(id) FROM orders")
MEMBER_OID=$OID
[ "$(payrow provider)" = "stripe" ] && [ "$(payrow status)" = "paid" ] && ok "member stripe payment paid" || ng "member stripe payment ($(payrow status))"
JA=/tmp/ts_admin.txt; rm -f $JA
curl -s -c $JA $BASE/admin/login -o /tmp/ts_alogin.html
C=$(csrf /tmp/ts_alogin.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/login --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "$PWF=$APW" --data-urlencode "$CTF=$C"
# The points programme is off in the dev shop's standing
# settings: enable it through the admin form for this section
# (the form post also refreshes the settings cache), and
# restore the originals afterwards, the test_points pattern.
ORIG_POINTS=$(db "SELECT points_enabled || '|' || points_earn_percent || '|' || points_expiry_days FROM shop_settings WHERE id=1")
curl -s -b $JA $BASE/admin/settings/shop -o /tmp/ts_set.html
C=$(csrf /tmp/ts_set.html)
TICKET=$(grep -o 'name="form_ticket" value="[^"]*"' /tmp/ts_set.html | head -1 | sed 's/.*value="//;s/"//')
curl -s -b $JA -o /dev/null -X POST $BASE/admin/settings/shop/points   --data-urlencode "$CTF=$C" --data-urlencode "form_ticket=$TICKET"   --data-urlencode "points_enabled=on"   --data-urlencode "points_earn_percent=1" --data-urlencode "points_expiry_days=180"
curl -s -b $JA $BASE/admin/orders/$MEMBER_OID -o /tmp/ts_detail.html
grep -q "決済情報" /tmp/ts_detail.html && ok "admin detail shows the payment block" || ng "admin detail shows the payment block"
grep -q "Stripe" /tmp/ts_detail.html && ok "admin detail names Stripe" || ng "admin detail names Stripe"
grep -q "Stripeと同期する" /tmp/ts_detail.html && ok "sync button names Stripe" || ng "sync button names Stripe"
grep -q "action=\"/admin/orders/$MEMBER_OID/refund\"" /tmp/ts_detail.html && ok "refund form offered for a paid stripe card order" || ng "refund form offered"
for STEP in preparing shipped; do
  CUR=$(db "SELECT status FROM orders WHERE id=$MEMBER_OID")
  curl -s -b $JA $BASE/admin/orders/$MEMBER_OID -o /tmp/ts_detail.html
  C=$(csrf /tmp/ts_detail.html)
  if [ "$STEP" = "shipped" ]; then
    curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$MEMBER_OID/status --data-urlencode "status=$STEP" --data-urlencode "expected=$CUR" --data-urlencode "carrier=sagawa" --data-urlencode "tracking_number=123456789012" --data-urlencode "$CTF=$C"
  else
    curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$MEMBER_OID/status --data-urlencode "status=$STEP" --data-urlencode "expected=$CUR" --data-urlencode "$CTF=$C"
  fi
done
[ "$(db "SELECT status FROM orders WHERE id=$MEMBER_OID")" = "shipped" ] && ok "member order shipped" || ng "member order shipped"
wait_db "SELECT COALESCE(SUM(remaining),0) FROM point_ledger WHERE user_id=$M_UID AND kind='earn' AND remaining>0 AND expires_at>now()" "$((BAL_BEFORE + 29))" \
  && ok "points earned at shipment (29 on 2980)" || ng "points earned at shipment (balance $(balance_sql), before $BAL_BEFORE)"
curl -s -b $JA $BASE/admin/orders/$MEMBER_OID -o /tmp/ts_detail.html
C=$(csrf /tmp/ts_detail.html)
curl -s -b $JA -c $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/orders/$MEMBER_OID/refund --data-urlencode "amount_yen=" --data-urlencode "$CTF=$C" > /tmp/ts_refund_code.txt
grep -q "303" /tmp/ts_refund_code.txt && ok "admin refund accepted" || ng "admin refund accepted"
OID=$MEMBER_OID
wait_db "SELECT status FROM payments WHERE order_id=$OID" "refunded" && ok "stripe full refund settles as refunded" || ng "stripe full refund settles"
[ "$(payrow amount_refunded_yen)" = "3530" ] && ok "refunded total recorded" || ng "refunded total recorded"
wait_db "SELECT compensation_pending FROM payments WHERE order_id=$OID" "f" && ok "compensation completed (flag cleared)" || ng "compensation completed"
STOCK_NOW=$(db "SELECT COALESCE(SUM(stock),0) FROM product_variants WHERE product_id=1")
[ "$STOCK_NOW" = "$STOCK_BEFORE" ] && ok "stock restored by the refund compensation" || ng "stock restored ($STOCK_BEFORE->$STOCK_NOW)"
wait_db "SELECT COALESCE(SUM(remaining),0) FROM point_ledger WHERE user_id=$M_UID AND kind='earn' AND remaining>0 AND expires_at>now()" "$BAL_BEFORE" \
  && ok "earned points reversed by the compensation" || ng "points reversed (balance $(balance_sql), before $BAL_BEFORE)"
[ "$(db "SELECT count(*) FROM admin_audit_logs WHERE action='payment.refund_requested' AND target_id=$OID")" -ge 1 ] && ok "refund audited" || ng "refund audited"
OE=$(echo "$ORIG_POINTS" | cut -d'|' -f1); OP=$(echo "$ORIG_POINTS" | cut -d'|' -f2); OD=$(echo "$ORIG_POINTS" | cut -d'|' -f3)
[ "$OE" = "t" ] && OE=true || OE=false
db "UPDATE shop_settings SET points_enabled=$OE, points_earn_percent=$OP, points_expiry_days=$OD WHERE id=1" >/dev/null

echo "-- TS-05: konbini refund stays dashboard-directed on stripe too"
curl -s -b $JA $BASE/admin/orders/$KONBINI_OID -o /tmp/ts_detail.html
grep -q "Stripeダッシュボード" /tmp/ts_detail.html && ok "konbini refund names the Stripe dashboard" || ng "konbini refund names the Stripe dashboard"
grep -q "action=\"/admin/orders/$KONBINI_OID/refund\"" /tmp/ts_detail.html && ng "no in-admin konbini refund form" || ok "no in-admin konbini refund form"

echo "-- TS-06: partial refund"
J=/tmp/ts_partial.txt
guest_order $J 5 1 credit_card
PARTIAL_OID=$OID
curl -s -b $JA $BASE/admin/orders/$PARTIAL_OID -o /tmp/ts_detail.html
C=$(csrf /tmp/ts_detail.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/orders/$PARTIAL_OID/refund --data-urlencode "amount_yen=100" --data-urlencode "$CTF=$C"
OID=$PARTIAL_OID
wait_db "SELECT status FROM payments WHERE order_id=$OID" "partially_refunded" && ok "partial refund settles" || ng "partial refund settles"
[ "$(payrow amount_refunded_yen)" = "100" ] && ok "partial amount recorded" || ng "partial amount recorded"


echo "-- TS-07: the shipping guard covers stripe too"
J=/tmp/ts_guard.txt
guest_order $J 5 1 konbini
GUARD_OID=$OID
curl -s -b $JA $BASE/admin/orders/$GUARD_OID -o /tmp/ts_detail.html
C=$(csrf /tmp/ts_detail.html)
curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$GUARD_OID/status --data-urlencode "status=preparing" --data-urlencode "expected=received" --data-urlencode "$CTF=$C"
curl -s -b $JA -X POST $BASE/admin/orders/$GUARD_OID/status --data-urlencode "status=shipped" --data-urlencode "expected=preparing" --data-urlencode "carrier=sagawa" --data-urlencode "tracking_number=123456789012" --data-urlencode "$CTF=$C" -o /tmp/ts_guard.html
grep -q "入金が確認できていないため発送できません" /tmp/ts_guard.html && ok "unpaid stripe konbini order cannot ship" || ng "unpaid stripe konbini order cannot ship"
[ "$(db "SELECT status FROM orders WHERE id=$GUARD_OID")" = "preparing" ] && ok "stripe order stayed preparing" || ng "stripe order stayed preparing"
SPID=$(payrow stripe_payment_id)
curl -s -X POST $SSTUB/__pay/$SPID >/dev/null
BODY=$(session_event "evt-ts-guard-$RUN" "checkout.session.async_payment_succeeded" "$(payrow stripe_session_id)")
[ "$(stripe_webhook "$BODY")" = "200" ] && ok "guard order payment webhook accepted" || ng "guard order payment webhook accepted"
wait_db "SELECT status FROM payments WHERE order_id=$GUARD_OID" "paid" && ok "guard order settled" || ng "guard order settled"
curl -s -b $JA $BASE/admin/orders/$GUARD_OID -o /tmp/ts_detail.html
C=$(csrf /tmp/ts_detail.html)
curl -s -b $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/orders/$GUARD_OID/status --data-urlencode "status=shipped" --data-urlencode "expected=preparing" --data-urlencode "carrier=sagawa" --data-urlencode "tracking_number=123456789012" --data-urlencode "$CTF=$C" > /tmp/ts_ship_code.txt
grep -q "303" /tmp/ts_ship_code.txt && ok "settled stripe order ships" || ng "settled stripe order ships"

echo "-- TS-08: member cancel refunds a captured stripe card payment"
curl -s -b $JM -c $JM $BASE/ -o /tmp/ts_home.html
C=$(csrf /tmp/ts_home.html)
cart_add $JM 1 1 "$C"
curl -s -b $JM -c $JM $BASE/checkout -o /tmp/ts_co.html
C=$(csrf /tmp/ts_co.html)
curl -s -b $JM -c $JM -X POST $BASE/checkout \
  --data-urlencode "intent=confirm" --data-urlencode "payment_method=credit_card" \
  --data-urlencode "family_name=田中" --data-urlencode "given_name=一郎" \
  --data-urlencode "family_name_kana=タナカ" --data-urlencode "given_name_kana=イチロウ" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" \
  --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前2-2-2" \
  --data-urlencode "building=" --data-urlencode "phone=08011112222" \
  --data-urlencode "$CTF=$C" -o /tmp/ts_confirm.html
curl -s -b $JM -c $JM -X POST $BASE/checkout/payment --data-urlencode "payment_method=credit_card" --data-urlencode "$CTF=$C" -o /tmp/ts_confirm.html
C=$(csrf /tmp/ts_confirm.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CTF=$C"
OID=$(db "SELECT max(id) FROM orders")
CANCEL_OID=$OID
[ "$(payrow provider)" = "stripe" ] && [ "$(payrow status)" = "paid" ] && ok "member stripe card payment captured" || ng "member stripe card payment captured ($(payrow status))"
curl -s -b $JM $BASE/mypage/orders/$CANCEL_OID/cancel -o /tmp/ts_cx.html
grep -q "ご利用のお支払い方法へ返金します" /tmp/ts_cx.html && ok "cancel page promises the gateway refund" || ng "cancel page promises the gateway refund"
! grep -q "返金はありません" /tmp/ts_cx.html && ok "cancel page does not claim no refund" || ng "cancel page does not claim no refund"
C=$(csrf /tmp/ts_cx.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$CANCEL_OID/cancel --data-urlencode "$CTF=$C"
[ "$(db "SELECT status FROM orders WHERE id=$CANCEL_OID")" = "cancelled" ] && ok "member cancel committed" || ng "member cancel committed"
OID=$CANCEL_OID
wait_db "SELECT status FROM payments WHERE order_id=$OID" "refunded" && ok "captured stripe payment auto-refunded" || ng "captured stripe payment auto-refunded"
[ "$(payrow amount_refunded_yen)" = "$(payrow amount_yen)" ] && ok "refund is for the full captured amount" || ng "refund is for the full captured amount"
curl -s -b $JM $BASE/mypage/orders/$CANCEL_OID -o /tmp/ts_cx_detail.html
grep -q "お支払い済みの金額は返金済みです" /tmp/ts_cx_detail.html && ok "cancelled detail states the refund" || ng "cancelled detail states the refund"

echo "-- TS-09: member cancel of a paid stripe konbini order flags the Stripe dashboard"
curl -s -b $JM -c $JM $BASE/ -o /tmp/ts_home.html
C=$(csrf /tmp/ts_home.html)
cart_add $JM 5 1 "$C"
curl -s -b $JM -c $JM $BASE/checkout -o /tmp/ts_co.html
C=$(csrf /tmp/ts_co.html)
curl -s -b $JM -c $JM -X POST $BASE/checkout \
  --data-urlencode "intent=confirm" --data-urlencode "payment_method=konbini" \
  --data-urlencode "family_name=田中" --data-urlencode "given_name=一郎" \
  --data-urlencode "family_name_kana=タナカ" --data-urlencode "given_name_kana=イチロウ" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" \
  --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前2-2-2" \
  --data-urlencode "building=" --data-urlencode "phone=08011112222" \
  --data-urlencode "$CTF=$C" -o /tmp/ts_confirm.html
curl -s -b $JM -c $JM -X POST $BASE/checkout/payment --data-urlencode "payment_method=konbini" --data-urlencode "$CTF=$C" -o /tmp/ts_confirm.html
C=$(csrf /tmp/ts_confirm.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CTF=$C"
OID=$(db "SELECT max(id) FROM orders")
KCAN_OID=$OID
SPID=$(payrow stripe_payment_id)
curl -s -X POST $SSTUB/__pay/$SPID >/dev/null
BODY=$(session_event "evt-ts-kcancel-$RUN" "checkout.session.async_payment_succeeded" "$(payrow stripe_session_id)")
[ "$(stripe_webhook "$BODY")" = "200" ] && ok "konbini cancel fixture webhook accepted" || ng "konbini cancel fixture webhook accepted"
wait_db "SELECT status FROM payments WHERE order_id=$KCAN_OID" "paid" && ok "konbini cancel fixture paid" || ng "konbini cancel fixture paid"
curl -s -b $JM $BASE/mypage/orders/$KCAN_OID/cancel -o /tmp/ts_kcx.html
grep -q "コンビニ決済はオンラインで自動返金できません" /tmp/ts_kcx.html && ok "cancel page states the manual konbini refund" || ng "cancel page states the manual konbini refund"
C=$(csrf /tmp/ts_kcx.html)
curl -s -b $JM -o /dev/null -X POST $BASE/mypage/orders/$KCAN_OID/cancel --data-urlencode "$CTF=$C"
[ "$(db "SELECT status FROM orders WHERE id=$KCAN_OID")" = "cancelled" ] && ok "konbini member cancel committed" || ng "konbini member cancel committed"
FLAG=$(db "SELECT COALESCE(flag_note,'') FROM payments WHERE order_id=$KCAN_OID")
case "$FLAG" in *Stripeダッシュボード*) ok "flag names the Stripe dashboard";; *) ng "flag names the Stripe dashboard ($FLAG)";; esac
case "$FLAG" in *KOMOJU*) ng "flag does not name KOMOJU";; *) ok "flag does not name KOMOJU";; esac
[ "$(db "SELECT status FROM payments WHERE order_id=$KCAN_OID")" = "paid" ] && ok "konbini money stays captured for the manual refund" || ng "konbini money stays captured for the manual refund"

echo "-- TS-10: a failed stripe refund names Stripe in the operator message"
J=/tmp/ts_rfail.txt
guest_order $J 5 1 credit_card
RFAIL_OID=$OID
curl -s -X POST $SSTUB/__control -H 'Content-Type: application/json' -d '{"fail_refund":true}' >/dev/null
curl -s -b $JA $BASE/admin/orders/$RFAIL_OID -o /tmp/ts_detail.html
C=$(csrf /tmp/ts_detail.html)
curl -s -b $JA -X POST $BASE/admin/orders/$RFAIL_OID/refund --data-urlencode "amount_yen=" --data-urlencode "$CTF=$C" -o /tmp/ts_rfail.html
curl -s -X POST $SSTUB/__control -H 'Content-Type: application/json' -d '{"fail_refund":false}' >/dev/null
grep -q "Stripeで返金を受け付けられませんでした" /tmp/ts_rfail.html && ok "refund failure names Stripe" || ng "refund failure names Stripe"
[ "$(db "SELECT status FROM payments WHERE order_id=$RFAIL_OID")" = "paid" ] && ok "failed refund leaves the payment paid" || ng "failed refund leaves the payment paid"

echo "===== PHASE B: nothing configured — the rail is invisible ====="
start_app env && ok "plain app starts" || ng "plain app starts"
J=/tmp/ts_b.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/ts_home.html
C=$(csrf /tmp/ts_home.html)
cart_add $J 5 1 "$C"
curl -s -b $J $BASE/checkout -o /tmp/ts_co.html
C=$(csrf /tmp/ts_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "family_name_kana=サトウ" --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" \
  --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$C" -o /dev/null
curl -s -b $J -c $J $BASE/checkout/payment -o /tmp/ts_co.html
grep -q 'name="payment_method"' /tmp/ts_co.html && ng "no method choice when unconfigured" || ok "no method choice when unconfigured"
grep -q "代金引換" /tmp/ts_co.html && ok "static COD box as before" || ng "static COD box as before"
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST $BASE/webhooks/stripe --data-binary '{}')
[ "$CODE" = "404" ] && ok "stripe webhook absent when unconfigured" || ng "stripe webhook absent ($CODE)"
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST $BASE/webhooks/komoju --data-binary '{}')
[ "$CODE" = "404" ] && ok "komoju webhook absent when unconfigured" || ng "komoju webhook absent ($CODE)"
guest_order $J 5 1 cod
[ "$LOC" = "/checkout/complete" ] && ok "COD checkout unaffected" || ng "COD checkout unaffected"

echo "===== PHASE B2: stripe selected but misconfigured — fail closed ====="
start_app env CONFIG_PATH=$STCFG PUBLIC_BASE_URL=http://127.0.0.1:3000 STRIPE_WEBHOOK_SECRET="$SWHSEC" \
  && ok "misconfigured app still boots" || ng "misconfigured app still boots"
grep -q "cash on delivery only" $APPLOG && ok "misconfiguration logged once, clearly" || ng "misconfiguration logged once, clearly"
J=/tmp/ts_b2.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/ts_home.html
C=$(csrf /tmp/ts_home.html)
cart_add $J 5 1 "$C"
curl -s -b $J $BASE/checkout -o /tmp/ts_co.html
C=$(csrf /tmp/ts_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "family_name_kana=サトウ" --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" \
  --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$C" -o /dev/null
curl -s -b $J -c $J $BASE/checkout/payment -o /tmp/ts_co.html
grep -q 'name="payment_method"' /tmp/ts_co.html && ng "misconfigured stripe checkout is COD-only (no silent KOMOJU fallback)" || ok "misconfigured stripe checkout is COD-only (no silent KOMOJU fallback)"

echo "===== PHASE D: an in-flight KOMOJU payment survives the rail flip ====="
start_komoju_app && ok "komoju app starts" || ng "komoju app starts"
J=/tmp/ts_d.txt
guest_order $J 5 1 konbini
[ "$(payrow provider)" = "komoju" ] && [ "$(payrow status)" = "pending_payment" ] && ok "komoju konbini opened on the komoju rail" || ng "komoju konbini opened ($(payrow provider)/$(payrow status))"
INFLIGHT_OID=$OID
KPID=$(payrow komoju_payment_id)
curl -s -X POST $KSTUB/__pay/$KPID >/dev/null
start_stripe_app && ok "app restarts on the stripe rail" || ng "app restarts on the stripe rail"
J=/tmp/ts_d2.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/ts_home.html
C=$(csrf /tmp/ts_home.html)
cart_add $J 1 1 "$C"
curl -s -b $J $BASE/checkout -o /tmp/ts_co.html
C=$(csrf /tmp/ts_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "family_name_kana=サトウ" --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" \
  --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$C" -o /dev/null
curl -s -b $J -c $J $BASE/checkout/payment -o /tmp/ts_co.html
grep -q "Stripeの画面で" /tmp/ts_co.html && ok "checkout now speaks Stripe" || ng "checkout now speaks Stripe"
J=/tmp/ts_d3.txt
guest_order $J 1 1 credit_card
[ "$(payrow provider)" = "stripe" ] && ok "new orders record the stripe rail" || ng "new orders record the stripe rail ($(payrow provider))"
KBODY=$(komoju_webhook_body "evt-ts-inflight-$RUN" "payment.captured" "$KPID")
[ "$(komoju_webhook "$KBODY" "dlv-ts-$RUN")" = "200" ] && ok "in-flight komoju webhook accepted on the stripe rail" || ng "in-flight komoju webhook accepted"
wait_db "SELECT status FROM payments WHERE order_id=$INFLIGHT_OID" "paid" && ok "in-flight komoju payment settles after the flip" || ng "in-flight komoju payment settles after the flip"

echo "===== CLEANUP: restore the normal app ====="
kill $SSTUB_PID $KSTUB_PID 2>/dev/null
start_app env && ok "normal app restored" || ng "normal app restored"
result
