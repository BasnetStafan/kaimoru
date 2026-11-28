#!/bin/bash
# Payment verification for the KOMOJU integration (Payments Spec
# TC-01..TC-17, the parts provable over HTTP with the stub
# provider in scripts/stubs/komoju_stub.py).
#
# Unlike the other suites this one owns the app lifecycle: it
# starts the app three times — KOMOJU enabled against the stub,
# KOMOJU disabled (the default config), and KOMOJU enabled but
# misconfigured — and finishes by restarting the app with the
# normal development configuration, as it found it.
#
# Secrets discipline follows the house pattern: field names and
# passwords are assembled at runtime (variables, printf), never
# written as literals, and the app's dev secrets are read from
# run.sh by grep+eval without ever being printed.
BASE=http://127.0.0.1:3000
STUB=http://127.0.0.1:18099
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
WHSEC=$(printf "whsec_%s" "suite_secret")
RUN=$(date +%s)
KOMOJU_SK=$(printf "sk_test_%s" "suite")

eval "$(grep -E '^(DB_PASSWORD|APP_DB_PASSWORD|VERIFICATION_CODE_SECRET)=' .env | sed 's/^/export /')"
export ADMIN_EMAIL
export ADMIN_PASSWORD="$APW"

echo "===== SETUP: database, build, stub ====="
docker compose up -d >/dev/null 2>&1
for i in $(seq 1 30); do
  docker exec agentic-db pg_isready -U agentic -d agentic_commerce >/dev/null 2>&1 && break
  sleep 1
done
cargo build -p server 2>/tmp/tp_build.log | tail -1
[ -x target/debug/server ] && ok "server builds" || { ng "server builds"; tail -5 /tmp/tp_build.log; exit 1; }

fuser -k 18099/tcp 2>/dev/null; sleep 1
python3 scripts/stubs/komoju_stub.py >/tmp/tp_stub.log 2>&1 &
STUB_PID=$!
sleep 1
curl -s $STUB/__sessions | grep -q '"count": 0' && ok "stub provider up" || ng "stub provider up"

PAYCFG=/tmp/tp_config.toml
sed -e 's/^komoju_enabled = false/komoju_enabled = true/' \
    -e 's/^payment_sweep_interval_secs = 600/payment_sweep_interval_secs = 2/' \
    config.toml > $PAYCFG
grep -q '^komoju_enabled = true' $PAYCFG && ok "payments test config written" || ng "payments test config written"

# The suite places real orders against products 1 (now
# multi-variant) and 5: top their stock up so placement
# never starves. The stock assertions inside compare
# before/after within the run, so the level is free.
db "UPDATE product_variants SET stock = 50 WHERE product_id IN (1, 5)" >/dev/null

APPLOG=/tmp/tp_app.log
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
start_payments_app() {
  start_app env CONFIG_PATH=$PAYCFG \
    KOMOJU_SECRET_KEY="$KOMOJU_SK" KOMOJU_WEBHOOK_SECRET="$WHSEC" \
    PUBLIC_BASE_URL=http://127.0.0.1:3000 KOMOJU_API_BASE_URL=$STUB
}

# Signs a webhook body exactly as KOMOJU would (hex HMAC-SHA256
# over the raw bytes) and posts it; prints the HTTP status.
sign() { python3 -c 'import hmac,hashlib,sys;print(hmac.new(sys.argv[1].encode(),sys.argv[2].encode(),hashlib.sha256).hexdigest())' "$WHSEC" "$1"; }
webhook() {
  local sig; sig=$(sign "$1")
  curl -s -o /tmp/tp_wh.html -w "%{http_code}" -X POST $BASE/webhooks/komoju \
    -H "Content-Type: application/json" -H "X-Komoju-Signature: $sig" \
    -H "X-Komoju-ID: $2" --data-binary "$1"
}
# Builds a webhook body whose data is the stub's current payment
# object: webhook_body EVENT_ID TYPE PAYMENT_ID
webhook_body() {
  python3 - "$1" "$2" "$3" <<'PYEOF'
import json, sys, urllib.request
payment = json.load(urllib.request.urlopen("http://127.0.0.1:18099/api/v1/payments/" + sys.argv[3]))
print(json.dumps({"id": sys.argv[1], "type": sys.argv[2], "data": payment}))
PYEOF
}
# Polls a SQL query until it returns the expected string (30s).
wait_db() {
  for i in $(seq 1 15); do
    [ "$(db "$1")" = "$2" ] && return 0
    sleep 2
  done
  return 1
}
# Polls Mailpit search results for a needle (20s).
wait_mail() {
  for i in $(seq 1 10); do
    curl -s "$MAIL/api/v1/search?query=$1" | grep -q "$2" && return 0
    sleep 2
  done
  return 1
}


# Adds a product to the cart the way the current storefront
# requires: cart_add JAR PRODUCT QTY CSRF. Multi-variant
# products need a variant_id, resolved to the best-stocked
# variant; single-variant products resolve to their one
# variant, which the endpoint also accepts explicitly.
cart_add() {
  local jar=$1 pid=$2 qty=$3 c=$4
  local vid; vid=$(db "SELECT id FROM product_variants WHERE product_id=$pid AND stock >= $qty ORDER BY stock DESC, id LIMIT 1")
  if [ -n "$vid" ]; then
    curl -s -b "$jar" -c "$jar" -o /dev/null -X POST $BASE/cart/items \
      --data-urlencode "product_id=$pid" --data-urlencode "variant_id=$vid" \
      --data-urlencode "quantity=$qty" --data-urlencode "$CTF=$c"
  else
    curl -s -b "$jar" -c "$jar" -o /dev/null -X POST $BASE/cart/items \
      --data-urlencode "product_id=$pid" --data-urlencode "quantity=$qty" \
      --data-urlencode "$CTF=$c"
  fi
}

# Places one guest order: guest_order JAR PRODUCT QTY METHOD
# Sets OID (order id) and LOC (the confirm POST's Location).
guest_order() {
  local jar=$1 pid=$2 qty=$3 method=$4
  rm -f "$jar"
  curl -s -c "$jar" $BASE/ -o /tmp/tp_home.html
  local c; c=$(csrf /tmp/tp_home.html)
  cart_add "$jar" $pid $qty "$c"
  curl -s -b "$jar" -c "$jar" $BASE/checkout -o /tmp/tp_co.html
  c=$(csrf /tmp/tp_co.html)
  curl -s -b "$jar" -c "$jar" -X POST $BASE/checkout \
    --data-urlencode "intent=confirm" --data-urlencode "payment_method=$method" \
    --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "family_name_kana=サトウ" --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
    --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" \
    --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$c" -o /tmp/tp_confirm.html
  curl -s -b "$jar" -c "$jar" -X POST $BASE/checkout/payment --data-urlencode "payment_method=$method" --data-urlencode "$CTF=$c" -o /tmp/tp_confirm.html
  c=$(csrf /tmp/tp_confirm.html)
  curl -s -b "$jar" -c "$jar" -D /tmp/tp_hdrs.txt -o /dev/null -X POST $BASE/checkout/confirm \
    --data-urlencode "$CTF=$c"
  LOC=$(grep -i '^location:' /tmp/tp_hdrs.txt | tr -d '\r' | awk '{print $2}')
  OID=$(db "SELECT max(id) FROM orders")
}
payrow() { db "SELECT $1 FROM payments WHERE order_id=$OID"; }

echo "===== PHASE A: KOMOJU enabled (stub provider) ====="
start_payments_app && ok "payments app starts" || ng "payments app starts"

echo "-- TC-15 (enabled): checkout offers all four methods"
J=/tmp/tp_a.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/tp_home.html
C=$(csrf /tmp/tp_home.html)
cart_add $J 1 1 "$C"
curl -s -b $J $BASE/checkout -o /tmp/tp_co.html
C=$(csrf /tmp/tp_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "family_name_kana=サトウ" --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$C" -o /dev/null
curl -s -b $J -c $J $BASE/checkout/payment -o /tmp/tp_co.html
grep -q 'value="credit_card"' /tmp/tp_co.html && ok "card offered" || ng "card offered"
grep -q 'value="konbini"' /tmp/tp_co.html && ok "konbini offered" || ng "konbini offered"
grep -q 'value="paypay"' /tmp/tp_co.html && ok "paypay offered" || ng "paypay offered"
grep -q 'value="cod"' /tmp/tp_co.html && ok "cod offered" || ng "cod offered"

echo "-- TC-11: the totals matrix (P1 2980 + shipping 550; COD adds 330)"
for M in credit_card konbini paypay; do
  C=$(csrf /tmp/tp_co.html)
  curl -s -b $J -c $J -X POST $BASE/checkout \
    --data-urlencode "intent=confirm" --data-urlencode "payment_method=$M" \
    --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "family_name_kana=サトウ" --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
    --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" \
    --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$C" -o /tmp/tp_m.html
  curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=$M" --data-urlencode "$CTF=$C" -o /tmp/tp_m.html
  grep -q "￥3,530" /tmp/tp_m.html && ok "$M total 3530 (no COD fee)" || ng "$M total 3530"
  grep -q "代引き手数料" /tmp/tp_m.html && ng "$M shows no COD fee row" || ok "$M shows no COD fee row"
done
C=$(csrf /tmp/tp_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout \
  --data-urlencode "intent=confirm" --data-urlencode "payment_method=cod" \
  --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" \
  --data-urlencode "given_name=花" --data-urlencode "family_name_kana=サトウ" \
  --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" \
  --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$C" -o /tmp/tp_m.html
curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "$CTF=$C" -o /tmp/tp_m.html
grep -q "￥3,860" /tmp/tp_m.html && ok "cod total 3860 (fee in)" || ng "cod total 3860"
grep -q "代引き手数料" /tmp/tp_m.html && ok "cod shows the fee row" || ng "cod shows the fee row"

echo "-- TC-01: card purchase end to end"
J=/tmp/tp_card.txt
guest_order $J 1 1 credit_card
case "$LOC" in *127.0.0.1:18099/pay/*) ok "redirected to the hosted page";; *) ng "redirected to the hosted page ($LOC)";; esac
[ "$(payrow status)" = "paid" ] && ok "card payment settled at creation (capture auto)" || ng "card payment status $(payrow status)"
[ "$(payrow provider)" = "komoju" ] && [ "$(payrow method)" = "credit_card" ] && ok "payment row records komoju/card" || ng "payment row records komoju/card"
[ "$(payrow amount_yen)" = "3530" ] && ok "charged amount is the KOMOJU total (no fee)" || ng "charged amount $(payrow amount_yen)"
SID=$(payrow komoju_session_id)
[ -n "$SID" ] && ok "session id recorded" || ng "session id recorded"
ORDERNUM=$(db "SELECT 'AS-' || to_char(created_at AT TIME ZONE 'Asia/Tokyo','YYYYMMDD') || '-' || lpad(id::text,4,'0') FROM orders WHERE id=$OID")
curl -s -b $J -o /dev/null -w "%{http_code} %{redirect_url}" "$BASE/checkout/payment/return?session_id=$SID" > /tmp/tp_ret.txt
grep -q "303" /tmp/tp_ret.txt && grep -q "checkout/complete" /tmp/tp_ret.txt && ok "return lands on complete once paid" || ng "return lands on complete"
curl -s -b $J $BASE/checkout/complete -o /tmp/tp_complete.html
grep -q "お支払い済み" /tmp/tp_complete.html && ok "complete page says paid" || ng "complete page says paid"
[ -n "$(payrow confirmation_mail_sent_at)" ] && ok "confirmation claim recorded" || ng "confirmation claim recorded"
wait_mail "$ORDERNUM" "お支払いが完了しました" && ok "payment confirmation mail sent" || ng "payment confirmation mail sent"
CARD_OID=$OID

echo "-- TC-02: konbini — instructions first, capture settles"
J=/tmp/tp_konbini.txt
guest_order $J 5 1 konbini
case "$LOC" in *127.0.0.1:18099/pay/*) ok "redirected to the hosted page";; *) ng "konbini redirect ($LOC)";; esac
[ "$(payrow status)" = "pending_payment" ] && ok "konbini rests at 入金待ち" || ng "konbini status $(payrow status)"
[ -n "$(payrow payment_deadline)" ] && ok "deadline recorded" || ng "deadline recorded"
KURL=$(payrow instructions_url)
[ -n "$KURL" ] && ok "instructions URL recorded" || ng "instructions URL recorded"
SID=$(payrow komoju_session_id)
curl -s -b $J "$BASE/checkout/payment/return?session_id=$SID" -o /tmp/tp_inst.html
grep -q "お支払い期限" /tmp/tp_inst.html && ok "return shows the instructions page" || ng "return shows the instructions page"
grep -q "instructions/" /tmp/tp_inst.html && ok "instructions page links the hosted instructions" || ng "instructions link"
ORDERNUM=$(db "SELECT 'AS-' || to_char(created_at AT TIME ZONE 'Asia/Tokyo','YYYYMMDD') || '-' || lpad(id::text,4,'0') FROM orders WHERE id=$OID")
wait_mail "$ORDERNUM" "コンビニ支払いのご案内" && ok "instructions mail sent" || ng "instructions mail sent"
curl -s "$MAIL/api/v1/search?query=$ORDERNUM" | grep -q "お支払いが完了しました" && ng "no confirmation mail before capture" || ok "no confirmation mail before capture"
KPID=$(payrow komoju_payment_id)
curl -s -X POST $STUB/__pay/$KPID >/dev/null
BODY=$(webhook_body evt-tp-konbini-$RUN payment.captured "$KPID")
[ "$(webhook "$BODY" dlv-tp-$RUN)" = "200" ] && ok "capture webhook accepted" || ng "capture webhook accepted"
wait_db "SELECT status FROM payments WHERE order_id=$OID" "paid" && ok "konbini settles on capture" || ng "konbini settles on capture"
[ -n "$(payrow captured_at)" ] && ok "capture instant recorded" || ng "capture instant recorded"
wait_mail "$ORDERNUM" "お支払いが完了しました" && ok "confirmation mail after capture" || ng "confirmation mail after capture"
curl -s -b $J -o /dev/null -w "%{http_code} %{redirect_url}" "$BASE/checkout/payment/return?session_id=$SID" > /tmp/tp_ret.txt
grep -q "checkout/complete" /tmp/tp_ret.txt && ok "return after capture lands on complete" || ng "return after capture lands on complete"
KONBINI_OID=$OID

echo "-- TC-06: webhook signature discipline"
EV_BEFORE=$(db "SELECT count(*) FROM webhook_events")
BODY='{"id":"evt-tp-bad","type":"payment.captured","data":{}}'
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST $BASE/webhooks/komoju -H "X-Komoju-Signature: deadbeef" --data-binary "$BODY")
[ "$CODE" = "400" ] && ok "bad signature is a 400" || ng "bad signature is a 400 ($CODE)"
# The A3 body re-encoded with different spacing, carrying the
# ORIGINAL signature: same JSON, different bytes, must fail.
BODY2=$(webhook_body evt-tp-konbini-$RUN payment.captured "$KPID")
SIG2=$(sign "$BODY2")
PRETTY=$(printf '%s' "$BODY2" | python3 -c 'import json,sys; print(json.dumps(json.loads(sys.stdin.read()), indent=2))')
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST $BASE/webhooks/komoju -H "X-Komoju-Signature: $SIG2" --data-binary "$PRETTY")
[ "$CODE" = "400" ] && ok "re-encoded body fails verification" || ng "re-encoded body fails verification ($CODE)"
[ "$(db "SELECT count(*) FROM webhook_events")" = "$EV_BEFORE" ] && ok "rejected deliveries record nothing" || ng "rejected deliveries record nothing"
BODY="{\"id\":\"evt-tp-ping-$RUN\",\"type\":\"ping\",\"data\":{}}"
[ "$(webhook "$BODY" dlv-ping-$RUN)" = "200" ] && ok "ping acknowledged" || ng "ping acknowledged"
[ "$(db "SELECT count(*) FROM webhook_events")" = "$((EV_BEFORE+1))" ] && ok "ping recorded in the ledger" || ng "ping recorded"
[ "$(webhook "$BODY2" dlv-tp-DIFFERENT-$RUN)" = "200" ] && ok "redelivery acknowledged" || ng "redelivery acknowledged"
[ "$(db "SELECT count(*) FROM webhook_events WHERE event_id='evt-tp-konbini-$RUN'")" = "1" ] && ok "redelivery deduped on the body event id" || ng "redelivery deduped"
[ "$(payrow status)" = "paid" ] && ok "state unchanged by redelivery" || ng "state unchanged by redelivery"

echo "-- TC-06b: redelivery of a recorded-but-unapplied event applies it"
# A first delivery whose apply step failed (it answers 500 and
# the provider retries) leaves the ledger row recorded with
# applied=false. Simulate exactly that row for a fresh konbini
# payment, then deliver the same event again: it must apply,
# not be acknowledged as a duplicate.
J=/tmp/tp_retry.txt
guest_order $J 5 1 konbini
[ "$(payrow status)" = "pending_payment" ] && ok "retry fixture rests at 入金待ち" || ng "retry fixture rests ($(payrow status))"
RPID=$(payrow komoju_payment_id)
curl -s -X POST $STUB/__pay/$RPID >/dev/null
RBODY=$(webhook_body evt-tp-retry-$RUN payment.captured "$RPID")
printf "INSERT INTO webhook_events (event_id, delivery_id, event_type, raw_body) VALUES ('evt-tp-retry-%s', 'dlv-failed-%s', 'payment.captured', '%s'::jsonb);\n" "$RUN" "$RUN" "$RBODY" | docker exec -i agentic-db psql -U agentic -d agentic_commerce -q >/dev/null
[ "$(db "SELECT applied FROM webhook_events WHERE event_id='evt-tp-retry-$RUN'")" = "f" ] && ok "failed first delivery simulated (recorded, unapplied)" || ng "failed first delivery simulated"
[ "$(webhook "$RBODY" dlv-retry-$RUN)" = "200" ] && ok "redelivery of unapplied event accepted" || ng "redelivery of unapplied event accepted"
wait_db "SELECT status FROM payments WHERE order_id=$OID" "paid" && ok "unapplied event applies on redelivery" || ng "unapplied event applies on redelivery"
[ "$(db "SELECT applied FROM webhook_events WHERE event_id='evt-tp-retry-$RUN'")" = "t" ] && ok "ledger row stamped applied after redelivery" || ng "ledger row stamped applied"
[ "$(webhook "$RBODY" dlv-retry2-$RUN)" = "200" ] && ok "second redelivery acknowledged" || ng "second redelivery acknowledged"
[ "$(payrow status)" = "paid" ] && ok "state unchanged by second redelivery" || ng "state unchanged by second redelivery"

echo "-- TC-09: double submit + return scoping"
J=/tmp/tp_ds.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/tp_home.html
C=$(csrf /tmp/tp_home.html)
cart_add $J 1 1 "$C"
curl -s -b $J -c $J $BASE/checkout -o /tmp/tp_co.html
C=$(csrf /tmp/tp_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout \
  --data-urlencode "intent=confirm" --data-urlencode "payment_method=credit_card" \
  --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" \
  --data-urlencode "given_name=花" --data-urlencode "family_name_kana=サトウ" \
  --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" \
  --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$C" -o /tmp/tp_confirm.html
curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=credit_card" --data-urlencode "$CTF=$C" -o /tmp/tp_confirm.html
C=$(csrf /tmp/tp_confirm.html)
ORD_BEFORE=$(db "SELECT count(*) FROM orders")
SES_BEFORE=$(curl -s $STUB/__sessions | sed 's/[^0-9]//g')
curl -s -b $J -c $J -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CTF=$C"
curl -s -b $J -c $J -D /tmp/tp_hdrs.txt -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CTF=$C"
[ "$(db "SELECT count(*) FROM orders")" = "$((ORD_BEFORE+1))" ] && ok "double submit places one order" || ng "double submit places one order"
SES_AFTER=$(curl -s $STUB/__sessions | sed 's/[^0-9]//g')
[ "$SES_AFTER" = "$((SES_BEFORE+1))" ] && ok "double submit creates one session" || ng "double submit creates one session"
grep -qi "checkout/complete" /tmp/tp_hdrs.txt && ok "second submit lands on complete" || ng "second submit lands on complete"
OID=$(db "SELECT max(id) FROM orders")
SID=$(payrow komoju_session_id)
curl -s "$BASE/checkout/payment/return?session_id=$SID" -o /tmp/tp_scope.html
grep -q "お支払いの確認中です" /tmp/tp_scope.html && ok "foreign return gets the generic page" || ng "foreign return gets the generic page"
ORDERNUM=$(db "SELECT 'AS-' || to_char(created_at AT TIME ZONE 'Asia/Tokyo','YYYYMMDD') || '-' || lpad(id::text,4,'0') FROM orders WHERE id=$OID")
grep -q "$ORDERNUM" /tmp/tp_scope.html && ng "generic page leaks no order number" || ok "generic page leaks no order number"

echo "-- TC-12: konbini expiry releases stock and the once-only coupon"
db "DELETE FROM user_coupons WHERE user_id=(SELECT id FROM users WHERE email_bidx='$(bidx $MEMBER)') AND code='WELCOME500'" >/dev/null
# The once-per-customer cap counts the redemption ledger, not
# the held copies: reset it too, or a member who redeemed
# WELCOME500 in any earlier run can never apply it again and
# TC-12 fails deterministically.
db "DELETE FROM coupon_redemptions WHERE user_id=(SELECT id FROM users WHERE email_bidx='$(bidx $MEMBER)') AND code='WELCOME500'" >/dev/null
db "INSERT INTO user_coupons (user_id, code) SELECT id, 'WELCOME500' FROM users WHERE email_bidx='$(bidx $MEMBER)'" >/dev/null
J=/tmp/tp_member.txt; rm -f $J
curl -s -c $J $BASE/login -o /tmp/tp_login.html
C=$(csrf /tmp/tp_login.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/login --data-urlencode "email=$MEMBER" --data-urlencode "$PWF=$MPW" --data-urlencode "$CTF=$C"
curl -s -b $J -c $J $BASE/ -o /tmp/tp_home.html
C=$(csrf /tmp/tp_home.html)
cart_add $J 5 1 "$C"
curl -s -b $J -c $J $BASE/checkout -o /tmp/tp_co.html
C=$(csrf /tmp/tp_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout \
  --data-urlencode "intent=apply_coupon" --data-urlencode "coupon_code=WELCOME500" \
  --data-urlencode "family_name=田中" --data-urlencode "given_name=一郎" \
  --data-urlencode "family_name_kana=タナカ" --data-urlencode "given_name_kana=イチロウ" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" \
  --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前2-2-2" \
  --data-urlencode "building=" --data-urlencode "phone=08011112222" \
  --data-urlencode "$CTF=$C" -o /tmp/tp_coupon.html
grep -q "WELCOME500" /tmp/tp_coupon.html && ok "coupon applied" || ng "coupon applied"
C=$(csrf /tmp/tp_coupon.html)
curl -s -b $J -c $J -X POST $BASE/checkout \
  --data-urlencode "intent=confirm" --data-urlencode "payment_method=konbini" \
  --data-urlencode "family_name=田中" --data-urlencode "given_name=一郎" \
  --data-urlencode "family_name_kana=タナカ" --data-urlencode "given_name_kana=イチロウ" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" \
  --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前2-2-2" \
  --data-urlencode "building=" --data-urlencode "phone=08011112222" \
  --data-urlencode "$CTF=$C" -o /tmp/tp_confirm.html
curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=konbini" --data-urlencode "$CTF=$C" -o /tmp/tp_confirm.html
STOCK_BEFORE=$(db "SELECT COALESCE(SUM(stock), 0) FROM product_variants WHERE product_id=5")
C=$(csrf /tmp/tp_confirm.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CTF=$C"
OID=$(db "SELECT max(id) FROM orders")
[ "$(payrow amount_yen)" = "930" ] && ok "konbini amount is coupon-discounted (1430-500)" || ng "konbini amount $(payrow amount_yen)"
[ -n "$(db "SELECT used_at FROM user_coupons WHERE user_id=(SELECT id FROM users WHERE email_bidx='$(bidx $MEMBER)') AND code='WELCOME500'")" ] && ok "coupon consumed at placement" || ng "coupon consumed at placement"
KPID2=$(payrow komoju_payment_id)
db "UPDATE payments SET payment_deadline = now() - interval '1 hour' WHERE order_id=$OID" >/dev/null
wait_db "SELECT status FROM payments WHERE order_id=$OID" "expired" && ok "sweep expires the unpaid konbini payment" || ng "sweep expires the unpaid konbini payment"
[ "$(db "SELECT status FROM orders WHERE id=$OID")" = "cancelled" ] && ok "order cancelled by expiry" || ng "order cancelled by expiry"
STOCK_NOW=$(db "SELECT COALESCE(SUM(stock), 0) FROM product_variants WHERE product_id=5")
[ "$STOCK_NOW" = "$STOCK_BEFORE" ] && ok "stock released on expiry" || ng "stock released on expiry ($STOCK_BEFORE->$STOCK_NOW)"
[ -z "$(db "SELECT used_at FROM user_coupons WHERE user_id=(SELECT id FROM users WHERE email_bidx='$(bidx $MEMBER)') AND code='WELCOME500'")" ] && ok "coupon released on expiry" || ng "coupon released on expiry"
EXP_OID=$OID

echo "-- TC-14: capture after expiry is flagged, never applied"
curl -s -X POST $STUB/__pay/$KPID2 >/dev/null
BODY=$(webhook_body evt-tp-late-$RUN payment.captured "$KPID2")
[ "$(webhook "$BODY" dlv-late-$RUN)" = "200" ] && ok "late capture webhook acknowledged" || ng "late capture webhook acknowledged"
OID=$EXP_OID
[ "$(payrow status)" = "expired" ] && ok "late capture does not resurrect the payment" || ng "late capture does not resurrect ($(payrow status))"
[ -n "$(payrow flag_note)" ] && ok "late capture flagged for a human" || ng "late capture flagged"
[ "$(db "SELECT status FROM orders WHERE id=$OID")" = "cancelled" ] && ok "order stays cancelled" || ng "order stays cancelled"

echo "-- TC-13: abandoned card payment dies at the 24h TTL"
curl -s -X POST $STUB/__control -H "Content-Type: application/json" -d '{"authorize_only": true}' >/dev/null
STOCK_BEFORE=$(db "SELECT COALESCE(SUM(stock), 0) FROM product_variants WHERE product_id=1")
J=/tmp/tp_ttl.txt
guest_order $J 1 1 credit_card
curl -s -X POST $STUB/__control -H "Content-Type: application/json" -d '{"authorize_only": false}' >/dev/null
[ "$(payrow status)" = "pending_payment" ] && ok "abandoned payment rests pending" || ng "abandoned payment rests ($(payrow status))"
db "UPDATE payments SET created_at = now() - interval '25 hours' WHERE order_id=$OID" >/dev/null
wait_db "SELECT status FROM orders WHERE id=$OID" "cancelled" && ok "TTL cancels the abandoned order" || ng "TTL cancels the abandoned order"
[ "$(payrow status)" = "cancelled" ] && ok "provider void applied" || ng "provider void applied ($(payrow status))"
STOCK_NOW=$(db "SELECT COALESCE(SUM(stock), 0) FROM product_variants WHERE product_id=1")
[ "$STOCK_NOW" = "$STOCK_BEFORE" ] && ok "stock released on TTL expiry" || ng "stock released on TTL expiry"

echo "-- TC-07/08: admin refunds (card full + partial; konbini is dashboard-only)"
JA=/tmp/tp_admin.txt; rm -f $JA
curl -s -c $JA $BASE/admin/login -o /tmp/tp_alogin.html
C=$(csrf /tmp/tp_alogin.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/login --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "$PWF=$APW" --data-urlencode "$CTF=$C"
curl -s -b $JA $BASE/admin/orders/$CARD_OID -o /tmp/tp_detail.html
grep -q "決済情報" /tmp/tp_detail.html && ok "admin detail shows the payment block" || ng "admin detail shows the payment block"
grep -q "action=\"/admin/orders/$CARD_OID/refund\"" /tmp/tp_detail.html && ok "refund form offered for a paid card order" || ng "refund form offered"
grep -q "チャージバック" /tmp/tp_detail.html && ok "chargeback-aware refund copy present" || ng "chargeback-aware refund copy"
C=$(csrf /tmp/tp_detail.html)
curl -s -b $JA -c $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/orders/$CARD_OID/refund --data-urlencode "amount_yen=" --data-urlencode "$CTF=$C" > /tmp/tp_refund_code.txt
grep -q "303" /tmp/tp_refund_code.txt && ok "refund accepted" || ng "refund accepted"
OID=$CARD_OID
wait_db "SELECT status FROM payments WHERE order_id=$OID" "refunded" && ok "full refund settles as refunded" || ng "full refund settles"
[ "$(payrow amount_refunded_yen)" = "3530" ] && ok "refunded total recorded" || ng "refunded total recorded"
[ "$(db "SELECT count(*) FROM admin_audit_logs WHERE action='payment.refund_requested' AND target_id=$OID")" -ge 1 ] && ok "refund audited" || ng "refund audited"
J=/tmp/tp_partial.txt
guest_order $J 5 1 credit_card
PARTIAL_OID=$OID
curl -s -b $JA $BASE/admin/orders/$PARTIAL_OID -o /tmp/tp_detail.html
C=$(csrf /tmp/tp_detail.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/orders/$PARTIAL_OID/refund --data-urlencode "amount_yen=100" --data-urlencode "$CTF=$C"
OID=$PARTIAL_OID
wait_db "SELECT status FROM payments WHERE order_id=$OID" "partially_refunded" && ok "partial refund settles as partially_refunded" || ng "partial refund settles"
[ "$(payrow amount_refunded_yen)" = "100" ] && ok "partial amount recorded" || ng "partial amount recorded"
curl -s -b $JA $BASE/admin/orders/$KONBINI_OID -o /tmp/tp_detail.html
grep -q "KOMOJUダッシュボード" /tmp/tp_detail.html && ok "konbini refund is dashboard-directed" || ng "konbini refund is dashboard-directed"
grep -q "action=\"/admin/orders/$KONBINI_OID/refund\"" /tmp/tp_detail.html && ng "no in-admin konbini refund form" || ok "no in-admin konbini refund form"

echo "-- TC-17: the shipping guard"
J=/tmp/tp_guard.txt
guest_order $J 5 1 konbini
GUARD_OID=$OID
# The flow is forward-only (received → preparing → shipped):
# walk both orders to preparing first, where shipping becomes
# the legal next step the guard exists for.
curl -s -b $JA $BASE/admin/orders/$GUARD_OID -o /tmp/tp_detail.html
C=$(csrf /tmp/tp_detail.html)
curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$GUARD_OID/status --data-urlencode "status=preparing" --data-urlencode "expected=received" --data-urlencode "$CTF=$C"
curl -s -b $JA -X POST $BASE/admin/orders/$GUARD_OID/status --data-urlencode "status=shipped" --data-urlencode "expected=preparing" --data-urlencode "carrier=sagawa" --data-urlencode "tracking_number=123456789012" --data-urlencode "$CTF=$C" -o /tmp/tp_guard.html
grep -q "入金が確認できていないため発送できません" /tmp/tp_guard.html && ok "unpaid konbini order cannot ship" || ng "unpaid konbini order cannot ship"
[ "$(db "SELECT status FROM orders WHERE id=$GUARD_OID")" = "preparing" ] && ok "order stayed preparing" || ng "order stayed preparing"
curl -s -b $JA $BASE/admin/orders/$PARTIAL_OID -o /tmp/tp_detail.html
C=$(csrf /tmp/tp_detail.html)
curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$PARTIAL_OID/status --data-urlencode "status=preparing" --data-urlencode "expected=received" --data-urlencode "$CTF=$C"
curl -s -b $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/orders/$PARTIAL_OID/status --data-urlencode "status=shipped" --data-urlencode "expected=preparing" --data-urlencode "carrier=sagawa" --data-urlencode "tracking_number=123456789012" --data-urlencode "$CTF=$C" > /tmp/tp_ship_code.txt
grep -q "303" /tmp/tp_ship_code.txt && ok "settled order ships" || ng "settled order ships"
[ "$(db "SELECT status FROM orders WHERE id=$PARTIAL_OID")" = "shipped" ] && ok "shipped state stored" || ng "shipped state stored"

echo "-- TC-16: cash on delivery in the payments world"
J=/tmp/tp_cod.txt
guest_order $J 1 1 cod
COD_OID=$OID
[ "$LOC" = "/checkout/complete" ] && ok "COD completes directly, as always" || ng "COD completes directly ($LOC)"
[ "$(payrow provider)" = "cod" ] && [ "$(payrow status)" = "pending_payment" ] && [ "$(payrow amount_yen)" = "3860" ] && ok "COD payment row (fee included)" || ng "COD payment row"
curl -s -b $J $BASE/checkout/complete -o /tmp/tp_complete.html
grep -q "代金引換・お届け時に現金でお支払いください" /tmp/tp_complete.html && ok "COD complete page wording unchanged" || ng "COD complete page wording unchanged"
for NEXT in preparing shipped delivered; do
  CUR=$(db "SELECT status FROM orders WHERE id=$COD_OID")
  curl -s -b $JA $BASE/admin/orders/$COD_OID -o /tmp/tp_detail.html
  C=$(csrf /tmp/tp_detail.html)
  if [ "$NEXT" = "shipped" ]; then
    curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$COD_OID/status --data-urlencode "status=$NEXT" --data-urlencode "expected=$CUR" --data-urlencode "carrier=sagawa" --data-urlencode "tracking_number=123456789012" --data-urlencode "$CTF=$C"
  else
    curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$COD_OID/status --data-urlencode "status=$NEXT" --data-urlencode "expected=$CUR" --data-urlencode "$CTF=$C"
  fi
done
[ "$(db "SELECT status FROM orders WHERE id=$COD_OID")" = "delivered" ] && ok "COD order delivered" || ng "COD order delivered"
OID=$COD_OID
[ "$(payrow status)" = "paid" ] && ok "COD payment settles at the door" || ng "COD payment settles at the door"
J=/tmp/tp_cod2.txt
guest_order $J 5 1 cod
COD2_OID=$OID
curl -s -b $JA $BASE/admin/orders/$COD2_OID -o /tmp/tp_detail.html
C=$(csrf /tmp/tp_detail.html)
curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$COD2_OID/cancel --data-urlencode "expected=received" --data-urlencode "$CTF=$C"
OID=$COD2_OID
[ "$(payrow status)" = "cancelled" ] && ok "cancelled COD order flips its payment row" || ng "cancelled COD payment flip"

echo "-- TC-03: provider outage at session creation"
curl -s -X POST $STUB/__control -H "Content-Type: application/json" -d '{"fail_create": true}' >/dev/null
ORD_BEFORE=$(db "SELECT count(*) FROM orders")
J=/tmp/tp_fail.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/tp_home.html
C=$(csrf /tmp/tp_home.html)
cart_add $J 1 1 "$C"
curl -s -b $J -c $J $BASE/checkout -o /tmp/tp_co.html
C=$(csrf /tmp/tp_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout \
  --data-urlencode "intent=confirm" --data-urlencode "payment_method=credit_card" \
  --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" \
  --data-urlencode "given_name=花" --data-urlencode "family_name_kana=サトウ" \
  --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" \
  --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$C" -o /tmp/tp_confirm.html
curl -s -b $J -c $J -X POST $BASE/checkout/payment --data-urlencode "payment_method=credit_card" --data-urlencode "$CTF=$C" -o /tmp/tp_confirm.html
C=$(csrf /tmp/tp_confirm.html)
curl -s -b $J -c $J -X POST $BASE/checkout/confirm --data-urlencode "$CTF=$C" -o /tmp/tp_fail.html
grep -q "お支払いを開始できませんでした" /tmp/tp_fail.html && ok "outage renders the payment error page" || ng "outage renders the payment error page"
[ "$(db "SELECT count(*) FROM orders")" = "$((ORD_BEFORE+1))" ] && ok "the order stands for recovery" || ng "the order stands for recovery"
OID=$(db "SELECT max(id) FROM orders")
[ "$(payrow status)" = "pending_payment" ] && [ -z "$(payrow komoju_session_id)" ] && ok "payment rests pending without a session" || ng "payment rests pending without a session"
curl -s -X POST $STUB/__control -H "Content-Type: application/json" -d '{"fail_create": false}' >/dev/null

echo "===== PHASE B: KOMOJU disabled (default config) ====="
start_app env && ok "plain app starts" || ng "plain app starts"
J=/tmp/tp_b.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/tp_home.html
C=$(csrf /tmp/tp_home.html)
cart_add $J 5 1 "$C"
curl -s -b $J $BASE/checkout -o /tmp/tp_co.html
C=$(csrf /tmp/tp_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "family_name_kana=サトウ" --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$C" -o /dev/null
curl -s -b $J -c $J $BASE/checkout/payment -o /tmp/tp_co.html
grep -q 'name="payment_method"' /tmp/tp_co.html && ng "no method choice when disabled (TC-15)" || ok "no method choice when disabled (TC-15)"
grep -q "代金引換" /tmp/tp_co.html && ok "static COD box as before" || ng "static COD box as before"
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST $BASE/webhooks/komoju --data-binary '{}')
[ "$CODE" = "404" ] && ok "webhook endpoint absent when unconfigured" || ng "webhook endpoint absent ($CODE)"
guest_order $J 5 1 cod
[ "$LOC" = "/checkout/complete" ] && ok "COD checkout unaffected" || ng "COD checkout unaffected"

echo "===== PHASE C: enabled but misconfigured — fail closed ====="
start_app env CONFIG_PATH=$PAYCFG PUBLIC_BASE_URL=http://127.0.0.1:3000 KOMOJU_WEBHOOK_SECRET="$WHSEC" \
  && ok "misconfigured app still boots" || ng "misconfigured app still boots"
grep -q "cash on delivery only" $APPLOG && ok "misconfiguration logged once, clearly" || ng "misconfiguration logged once, clearly"
J=/tmp/tp_c.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/tp_home.html
C=$(csrf /tmp/tp_home.html)
cart_add $J 5 1 "$C"
curl -s -b $J $BASE/checkout -o /tmp/tp_co.html
C=$(csrf /tmp/tp_co.html)
curl -s -b $J -c $J -X POST $BASE/checkout --data-urlencode "intent=confirm" --data-urlencode "email=guest@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花" \
  --data-urlencode "family_name_kana=サトウ" --data-urlencode "given_name_kana=ハナ" --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" --data-urlencode "phone=08011112222" --data-urlencode "$CTF=$C" -o /dev/null
curl -s -b $J -c $J $BASE/checkout/payment -o /tmp/tp_co.html
grep -q 'name="payment_method"' /tmp/tp_co.html && ng "misconfigured checkout is COD-only" || ok "misconfigured checkout is COD-only"

echo "===== CLEANUP: restore the normal app ====="
kill $STUB_PID 2>/dev/null
start_app env && ok "normal app restored" || ng "normal app restored"
result
