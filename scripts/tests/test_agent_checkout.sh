#!/bin/bash
# The agent checkout on KOMOJU — the
# in-chat settlement. The agent stocks the cart; the owner
# settles in the thread itself: policy agreement, delivery
# address, payment method, final confirmation, and the pay
# click that re-binds the approval to the final state and
# places the order through the storefront's ordinary KOMOJU
# machinery (COD completes in place).
#
# Like test_payments.sh this suite owns the app lifecycle:
# it boots the app against the stub provider so KOMOJU
# session creation is countable, then restarts the app with
# the normal development configuration, as it found it.
# Every actor is a fixture: a fresh guest jar per case, plus
# the fixture member; the owner's account is only ever counted,
# never touched. The suite drives the step posts WITHOUT
# htmx and asserts each step from a fresh GET /agent — the
# reload-resume behaviour is the thing under test, so the
# suite never trusts a fragment response. Re-runnable: each
# case builds its own session.
BASE=http://127.0.0.1:3000
STUB=http://127.0.0.1:18099
. "$(dirname "$0")/lib.sh"

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

# Assembled from fragments, never written literally with a
# value — the same care the other suites take.
CT=$(printf 'csrf_%s' 'token')
PF=$(printf 'pass%s' 'word')
XH=$(printf 'x-csrf-%s' 'token')
MPW=$(printf "test%s" "1234")
WHSEC=$(printf "whsec_%s" "suite_secret")
KOMOJU_SK=$(printf "sk_test_%s" "suite")
APW=$(printf "Password%s" "1234")
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
# the owner's account is resolved by blind index, not a hardcoded id:
# the id changes whenever the dev DB is rebuilt.
# Fixture: the pre-seeded owner/member account the suite inspects.
# Override with OWNER_EMAIL when running against another seed.
OWNER_EMAIL="${OWNER_EMAIL:-owner-fixture@example.com}"
OWNER_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx "$OWNER_EMAIL")'")
basnet_counts() { db "SELECT (SELECT COUNT(*) FROM orders WHERE user_id = $OWNER_ID) || '/' || (SELECT COUNT(*) FROM reviews WHERE user_id = $OWNER_ID) || '/' || (SELECT COUNT(*) FROM user_coupons WHERE user_id = $OWNER_ID)"; }
TEST_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx test@example.com)'")
INVARIANT_START=$(basnet_counts)
PUBLIC_START=$(db "SELECT COUNT(*) FROM products WHERE status = 'public'")

eval "$(grep -E '^(DB_PASSWORD|APP_DB_PASSWORD|VERIFICATION_CODE_SECRET)=' .env | sed 's/^/export /')"
export ADMIN_EMAIL=admin@example.com
export ADMIN_PASSWORD="$APW"

echo "===== SETUP: build, stub, app against the stub ====="
docker compose up -d >/dev/null 2>&1
for i in $(seq 1 30); do
  docker exec agentic-db pg_isready -U agentic -d agentic_commerce >/dev/null 2>&1 && break
  sleep 1
done
cargo build -p server 2>/tmp/s65_build.log | tail -1
[ -x target/debug/server ] && ok "server builds" || { ng "server builds"; tail -5 /tmp/s65_build.log; exit 1; }

fuser -k 18099/tcp 2>/dev/null; sleep 1
python3 scripts/stubs/komoju_stub.py >/tmp/s65_stub.log 2>&1 &
STUB_PID=$!
sleep 1
curl -s $STUB/__sessions | grep -q '"count": 0' && ok "stub provider up" || ng "stub provider up"
stub_count() { curl -s $STUB/__sessions | grep -o '"count": [0-9]*' | grep -o '[0-9]*'; }

S65CFG=/tmp/s65_config.toml
sed -e 's/^komoju_enabled = false/komoju_enabled = true/' \
    -e 's/^payment_sweep_interval_secs = 600/payment_sweep_interval_secs = 2/' \
    config.toml > $S65CFG
grep -q '^komoju_enabled = true' $S65CFG && ok "settlement test config written" || ng "settlement test config written"

APPLOG=/tmp/s65_app.log
stop_app() { fuser -k 3000/tcp 2>/dev/null; sleep 1; }
start_app() {
  stop_app
  env CONFIG_PATH=$S65CFG \
    KOMOJU_SECRET_KEY="$KOMOJU_SK" KOMOJU_WEBHOOK_SECRET="$WHSEC" \
    PUBLIC_BASE_URL=http://127.0.0.1:3000 KOMOJU_API_BASE_URL=$STUB \
    ./target/debug/server > $APPLOG 2>&1 &
  for i in $(seq 1 60); do
    curl -s -o /dev/null $BASE/healthz && return 0
    sleep 1
  done
  echo "  app failed to start; log tail:"; tail -5 $APPLOG
  return 1
}
start_app && ok "app up against the stub" || { ng "app up against the stub"; exit 1; }

# --- shared helpers -------------------------------------------------

# Establishes the jar's session (the first GET mints it, so
# tokens are only ever scraped from later pages).
warm() { curl -s -c "$1" $BASE/ -o /dev/null; curl -s -b "$1" -c "$1" $BASE/agent -o /dev/null; }

# Runs the agent once in the jar at the given budget, through
# the chat (the brief form was retired on 2026-11-07; this
# suite stocked its carts through it). The all-categories
# brief lands the same three picks the form's unrestricted
# run did (its sections derive every expected total from the
# approval snapshot, never from the brief); a clarifying
# reply is answered affirmatively once. The last response lands in
# /tmp/s65_run_$$.html.
run_agent() { # jar budget
  curl -s -b "$1" -c "$1" $BASE/agent -o /tmp/s65_form_$$.html
  local t=$(csrf /tmp/s65_form_$$.html)
  curl -s -b "$1" -c "$1" -X POST $BASE/agent/chat -o /tmp/s65_run_$$.html \
    --data-urlencode "$CT=$t" --data-urlencode "message=全カテゴリーから探してください。予算は$2円以内です"
  if ! grep -q '総合スコア' /tmp/s65_run_$$.html; then
    curl -s -b "$1" -c "$1" $BASE/agent -o /tmp/s65_form_$$.html
    local t2=$(csrf /tmp/s65_form_$$.html)
    curl -s -b "$1" -c "$1" -X POST $BASE/agent/chat -o /tmp/s65_run_$$.html \
      --data-urlencode "$CT=$t2" --data-urlencode "message=はい"
  fi
  # Design rule (2026-11-09): a chat search only PROPOSES.
  # This helper's job is to stock the cart for settlement,
  # so it finishes with the shopper's apply-all action.
  curl -s -b "$1" -c "$1" $BASE/agent -o /tmp/s65_form_$$.html
  local t3=$(csrf /tmp/s65_form_$$.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/agent/apply-all \
    --data-urlencode "$CT=$t3"
}

# The guest session token a run just stocked, by watermark.
session_since() { db "SELECT session_id FROM cart_items WHERE id > $1 AND added_by_agent ORDER BY id DESC LIMIT 1"; }

# A fresh full-page GET /agent into /tmp/s65_page_$$.html:
# the settlement state asserted from a reload, never from a
# step post's own answer.
get_agent() { curl -s -b "$1" -c "$1" $BASE/agent -o /tmp/s65_page_$$.html; }

# The settlement card's grand total on the current page,
# digits only: the in-place payment total and the
# confirmation total are compared as rendered.
card_total() { awk '/agent-approval-card/,0' /tmp/s65_page_$$.html | grep -o '<dt>合計</dt><dd>[^<]*' | head -1 | grep -o '[0-9,]*' | tr -d ','; }

# One settlement step post (plain, no htmx): prints
# "code redirect_url".
step() { # jar path [extra --data-urlencode args...]
  local jar=$1 path=$2; shift 2
  get_agent "$jar"
  local t=$(csrf /tmp/s65_page_$$.html)
  curl -s -b "$jar" -c "$jar" -o /dev/null -w "%{http_code} %{redirect_url}" -X POST "$BASE$path" \
    --data-urlencode "$CT=$t" "$@"
}

# The fixture guest address, posted to the address step.
guest_address() { # jar email
  step "$1" /agent/settle/address \
    --data-urlencode "email=$2" --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" \
    --data-urlencode "postal_code=1000001" --data-urlencode "prefecture=東京都" \
    --data-urlencode "city=千代田区" --data-urlencode "address_line=千代田1-1-1" \
    --data-urlencode "building=" --data-urlencode "phone=09012345678" >/dev/null
}

# Walks a jar from the offer to the confirmation step with
# the fixture guest address and the given method slug. The
# method post only selects (the card re-prices in place);
# the proceed post is what advances to the confirmation.
walk_to_confirm() { # jar email method
  step "$1" /agent/settle/start >/dev/null
  step "$1" /agent/settle/agree >/dev/null
  guest_address "$1" "$2"
  step "$1" /agent/settle/method --data-urlencode "payment_method=$3" >/dev/null
  step "$1" /agent/settle/proceed >/dev/null
}

orders_count() { db "SELECT COUNT(*) FROM orders"; }
approval_field() { db "SELECT $2 FROM agent_approvals WHERE session_id = '$1' ORDER BY id DESC LIMIT 1"; }

# Posts the delivery step of the NORMAL checkout with the
# fixture address (human-flow guard cases).
deliver() { # jar email
  curl -s -b "$1" -c "$1" $BASE/checkout -o /tmp/s65_co_$$.html
  local t=$(csrf /tmp/s65_co_$$.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/checkout \
    --data-urlencode "$CT=$t" \
    --data-urlencode "email=$2" --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" \
    --data-urlencode "postal_code=1000001" --data-urlencode "prefecture=東京都" \
    --data-urlencode "city=千代田区" --data-urlencode "address_line=千代田1-1-1" \
    --data-urlencode "building=" --data-urlencode "phone=09012345678" \
    --data-urlencode "delivery_date=" --data-urlencode "delivery_slot=none"
}

echo "-- no settlement: pay and step posts advance nothing, no session, no order"
J1=$(mktemp); warm $J1
run_agent $J1 10000
get_agent $J1
grep -q 'この内容で注文しますか？' /tmp/s65_page_$$.html && ok "the offer follows the run" || ng "the offer follows the run"
S0=$(stub_count); O0=$(orders_count)
OUT=$(step $J1 /agent/settle/pay)
[ "$OUT" = "303 $BASE/agent" ] && ok "pay without a settlement returns to the thread" || ng "pay without a settlement returns to the thread ($OUT)"
step $J1 /agent/settle/method --data-urlencode "payment_method=credit_card" >/dev/null
get_agent $J1
grep -q 'ご注文にあたっての確認' /tmp/s65_page_$$.html && ng "a method post alone opens no settlement" || ok "a method post alone opens no settlement"
[ "$(stub_count)" = "$S0" ] && ok "no KOMOJU session without a settlement" || ng "no KOMOJU session without a settlement"
[ "$(orders_count)" = "$O0" ] && ok "no order without a settlement" || ng "no order without a settlement"

echo "-- dismiss: the offer hides for this cart and returns when the cart changes"
J2=$(mktemp); warm $J2
run_agent $J2 10000
step $J2 /agent/settle/continue >/dev/null
get_agent $J2
grep -q 'この内容で注文しますか？' /tmp/s65_page_$$.html && ng "continue-shopping dismisses the offer" || ok "continue-shopping dismisses the offer"
run_agent $J2 2000
get_agent $J2
grep -q 'この内容で注文しますか？' /tmp/s65_page_$$.html && ok "a fresh run brings the offer back" || ng "a fresh run brings the offer back"

echo "-- guest card settlement: policy gate, steps resume from reloads, pay redirects to KOMOJU"
J3=$(mktemp); warm $J3
W=$(db "SELECT COALESCE(MAX(id),0) FROM cart_items")
run_agent $J3 10000
SES=$(session_since $W)
[ -n "$SES" ] && ok "guest run stocked agent lines" || ng "guest run stocked agent lines"
step $J3 /agent/settle/start >/dev/null
get_agent $J3
grep -q 'ご注文にあたっての確認' /tmp/s65_page_$$.html && ok "start opens the policy step (seen on reload)" || ng "start opens the policy step (seen on reload)"
step $J3 /agent/settle/method --data-urlencode "payment_method=credit_card" >/dev/null
get_agent $J3
grep -q 'ご注文にあたっての確認' /tmp/s65_page_$$.html && ok "the policy gate blocks progression until agreed" || ng "the policy gate blocks progression until agreed"
grep -q 'お届け先の指定' /tmp/s65_page_$$.html && ng "no address step before agreement" || ok "no address step before agreement"
step $J3 /agent/settle/agree >/dev/null
get_agent $J3
grep -q 'お届け先の指定' /tmp/s65_page_$$.html && ok "agreement advances to the address step" || ng "agreement advances to the address step"
guest_address $J3 guest3@example.com
get_agent $J3
grep -q 'お支払い方法の選択' /tmp/s65_page_$$.html && ok "a valid address advances to the payment step" || ng "a valid address advances to the payment step"
step $J3 /agent/settle/proceed >/dev/null
get_agent $J3
grep -q 'お支払い方法の選択' /tmp/s65_page_$$.html && ok "proceed with no method chosen stays on the payment step" || ng "proceed with no method chosen stays on the payment step"
grep -q '注文内容の最終確認' /tmp/s65_page_$$.html && ng "no confirmation before a method is chosen" || ok "no confirmation before a method is chosen"
step $J3 /agent/settle/method --data-urlencode "payment_method=credit_card" >/dev/null
get_agent $J3
grep -q 'お支払い方法の選択' /tmp/s65_page_$$.html && ok "selecting a method stays on the payment step" || ng "selecting a method stays on the payment step"
grep -q '注文内容の最終確認' /tmp/s65_page_$$.html && ng "the method alone no longer advances" || ok "the method alone no longer advances"
grep -q '<dt>代引き手数料</dt>' /tmp/s65_page_$$.html && ng "the card method carries no COD fee in place" || ok "the card method carries no COD fee in place"
PAY_TOTAL=$(card_total)
step $J3 /agent/settle/proceed >/dev/null
get_agent $J3
grep -q '注文内容の最終確認' /tmp/s65_page_$$.html && ok "proceed advances to the final confirmation" || ng "proceed advances to the final confirmation"
grep -q 'クレジットカード' /tmp/s65_page_$$.html && ok "the confirmation names the method" || ng "the confirmation names the method"
[ "$(card_total)" = "$PAY_TOTAL" ] && ok "the confirmation total matches the payment card's in-place total" || ng "the confirmation total matches the payment card's in-place total"
S0=$(stub_count); O0=$(orders_count)
OUT=$(step $J3 /agent/settle/pay)
case "$OUT" in "303 $STUB"*) ok "pay redirects to the hosted payment page";; *) ng "pay redirects to the hosted payment page ($OUT)";; esac
[ "$(stub_count)" = "$((S0+1))" ] && ok "exactly one KOMOJU session was created" || ng "exactly one KOMOJU session was created"
[ "$(orders_count)" = "$((O0+1))" ] && ok "exactly one order was placed" || ng "exactly one order was placed"
ORDER_ID=$(approval_field $SES consumed_order_id)
[ -n "$ORDER_ID" ] && ok "the approval consumed onto an order" || ng "the approval consumed onto an order"
[ "$(approval_field $SES status)" = "consumed" ] && ok "the approval rests consumed" || ng "the approval rests consumed"
FINAL_TOTAL=$(approval_field $SES "snapshot->>'total_yen'")
[ "$(db "SELECT total_yen FROM orders WHERE id = $ORDER_ID")" = "$FINAL_TOTAL" ] && ok "order total equals the final agreed snapshot" || ng "order total equals the final agreed snapshot"
[ "$(db "SELECT provider || '/' || method || '/' || status || '/' || amount_yen FROM payments WHERE order_id = $ORDER_ID")" = "komoju/credit_card/paid/$FINAL_TOTAL" ] \
  && ok "payment row is the KOMOJU card payment at the agreed amount" || ng "payment row is the KOMOJU card payment at the agreed amount"
[ "$(approval_field $SES "snapshot->>'payment_method'")" = "credit_card" ] && ok "the final snapshot records the method" || ng "the final snapshot records the method"
[ "$(approval_field $SES fingerprint)" != "$(approval_field $SES "snapshot->>'lines_fingerprint'")" ] \
  && ok "the headline fingerprint was re-bound at pay" || ng "the headline fingerprint was re-bound at pay"
[ "$(approval_field $SES "snapshot->'delivery'->>'email'")" = "guest3@example.com" ] && ok "the final snapshot records the address" || ng "the final snapshot records the address"

echo "-- double pay: the second click lands on the same completion, nothing new is created"
S0=$(stub_count); O0=$(orders_count)
OUT=$(step $J3 /agent/settle/pay)
[ "$OUT" = "303 $BASE/checkout/complete" ] && ok "second pay lands on the completion page" || ng "second pay lands on the completion page ($OUT)"
[ "$(stub_count)" = "$S0" ] && ok "double pay created no second session" || ng "double pay created no second session"
[ "$(orders_count)" = "$O0" ] && ok "double pay placed no second order" || ng "double pay placed no second order"

echo "-- guest COD settlement: the fee enters the agreed total, placement completes in place"
J4=$(mktemp); warm $J4
W=$(db "SELECT COALESCE(MAX(id),0) FROM cart_items")
run_agent $J4 10000
SES=$(session_since $W)
walk_to_confirm $J4 guest4@example.com cod
get_agent $J4
grep -q '代引き手数料' /tmp/s65_page_$$.html && ok "the COD confirmation shows the fee line" || ng "the COD confirmation shows the fee line"
BASE_TOTAL=$(approval_field $SES "snapshot->>'total_yen'")
S0=$(stub_count)
OUT=$(step $J4 /agent/settle/pay)
[ "$OUT" = "303 $BASE/checkout/complete" ] && ok "COD pay completes in place" || ng "COD pay completes in place ($OUT)"
[ "$(stub_count)" = "$S0" ] && ok "COD created no KOMOJU session" || ng "COD created no KOMOJU session"
ORDER_ID=$(approval_field $SES consumed_order_id)
[ "$(db "SELECT total_yen FROM orders WHERE id = $ORDER_ID")" = "$((BASE_TOTAL+330))" ] \
  && ok "the COD order total is the agreed total plus the fee" || ng "the COD order total is the agreed total plus the fee"
[ "$(db "SELECT provider || '/' || method || '/' || amount_yen FROM payments WHERE order_id = $ORDER_ID")" = "cod/cod/$((BASE_TOTAL+330))" ] \
  && ok "payment row is the COD payment at the agreed amount" || ng "payment row is the COD payment at the agreed amount"
[ "$(db "SELECT cod_fee_yen FROM orders WHERE id = $ORDER_ID")" = "330" ] && ok "the order carries the COD fee" || ng "the order carries the COD fee"

echo "-- method switch: COD shows the fee, switching to PayPay takes it back out"
J5=$(mktemp); warm $J5
W=$(db "SELECT COALESCE(MAX(id),0) FROM cart_items")
run_agent $J5 10000
SES=$(session_since $W)
step $J5 /agent/settle/start >/dev/null
step $J5 /agent/settle/agree >/dev/null
guest_address $J5 guest5@example.com
step $J5 /agent/settle/method --data-urlencode "payment_method=cod" >/dev/null
get_agent $J5
grep -q 'お支払い方法の選択' /tmp/s65_page_$$.html && ok "choosing COD stays on the payment step" || ng "choosing COD stays on the payment step"
grep -q '<dt>代引き手数料</dt>' /tmp/s65_page_$$.html && ok "the payment card shows the fee in place for COD" || ng "the payment card shows the fee in place for COD"
COD_TOTAL=$(card_total)
step $J5 /agent/settle/proceed >/dev/null
get_agent $J5
grep -q '代引き手数料' /tmp/s65_page_$$.html && ok "COD confirmation carries the fee" || ng "COD confirmation carries the fee"
[ "$(card_total)" = "$COD_TOTAL" ] && ok "the COD confirmation total matches the payment card" || ng "the COD confirmation total matches the payment card"
step $J5 /agent/settle/back --data-urlencode "to=payment" >/dev/null
get_agent $J5
grep -q 'お支払い方法の選択' /tmp/s65_page_$$.html && ok "back returns to the payment step" || ng "back returns to the payment step"
step $J5 /agent/settle/method --data-urlencode "payment_method=paypay" >/dev/null
get_agent $J5
grep -q '<dt>代引き手数料</dt>' /tmp/s65_page_$$.html && ng "the payment card drops the fee for PayPay" || ok "the payment card drops the fee for PayPay"
[ "$((COD_TOTAL - $(card_total)))" = "330" ] && ok "switching off COD removes exactly the fee in place" || ng "switching off COD removes exactly the fee in place"
step $J5 /agent/settle/proceed >/dev/null
get_agent $J5
grep -q '代引き手数料' /tmp/s65_page_$$.html && ng "the PayPay confirmation drops the fee" || ok "the PayPay confirmation drops the fee"
BASE_TOTAL=$(approval_field $SES "snapshot->>'total_yen'")
OUT=$(step $J5 /agent/settle/pay)
case "$OUT" in "303 $STUB"*) ok "PayPay pay redirects to the hosted page";; *) ng "PayPay pay redirects to the hosted page ($OUT)";; esac
ORDER_ID=$(approval_field $SES consumed_order_id)
[ "$(db "SELECT total_yen FROM orders WHERE id = $ORDER_ID")" = "$BASE_TOTAL" ] \
  && ok "the PayPay order total is the un-fee'd agreed total" || ng "the PayPay order total is the un-fee'd agreed total"
[ "$(approval_field $SES "snapshot->>'payment_method'")" = "paypay" ] && ok "the final snapshot records PayPay" || ng "the final snapshot records PayPay"

echo "-- cart change mid-settlement: the walk dies, the offer explains, pay creates nothing"
J6=$(mktemp); warm $J6
W=$(db "SELECT COALESCE(MAX(id),0) FROM cart_items")
run_agent $J6 10000
SES=$(session_since $W)
step $J6 /agent/settle/start >/dev/null
step $J6 /agent/settle/agree >/dev/null
VICTIM=$(db "SELECT variant_id FROM cart_items WHERE session_id = '$SES' AND added_by_agent ORDER BY variant_id LIMIT 1")
curl -s -b $J6 -c $J6 $BASE/cart -o /tmp/s65_cart_$$.html
T=$(csrf /tmp/s65_cart_$$.html)
curl -s -b $J6 -c $J6 -o /dev/null -X POST $BASE/cart/items/remove -H "$XH: $T" \
  --data-urlencode "variant_id=$VICTIM"
get_agent $J6
grep -q 'この内容で注文しますか？' /tmp/s65_page_$$.html && ok "the offer returns after the change" || ng "the offer returns after the change"
grep -q 'カートの内容が変わったため' /tmp/s65_page_$$.html && ok "the thread explains the voided settlement" || ng "the thread explains the voided settlement"
[ "$(approval_field $SES status)" = "voided" ] && ok "the changed approval is voided" || ng "the changed approval is voided"
S0=$(stub_count); O0=$(orders_count)
step $J6 /agent/settle/pay >/dev/null
[ "$(stub_count)" = "$S0" ] && [ "$(orders_count)" = "$O0" ] && ok "pay after the change creates nothing" || ng "pay after the change creates nothing"

echo "-- expiry at pay: refused with the explanation, the record rests expired"
J7=$(mktemp); warm $J7
W=$(db "SELECT COALESCE(MAX(id),0) FROM cart_items")
run_agent $J7 10000
SES=$(session_since $W)
walk_to_confirm $J7 guest7@example.com credit_card
db "UPDATE agent_approvals SET expires_at = now() - interval '1 minute' WHERE session_id = '$SES' AND status = 'pending'" >/dev/null
S0=$(stub_count); O0=$(orders_count)
OUT=$(step $J7 /agent/settle/pay)
[ "$OUT" = "303 $BASE/agent" ] && ok "expired pay returns to the thread" || ng "expired pay returns to the thread ($OUT)"
get_agent $J7
grep -q '有効期限' /tmp/s65_page_$$.html && ok "the thread explains the expiry" || ng "the thread explains the expiry"
[ "$(approval_field $SES status)" = "expired" ] && ok "the lapsed approval rests expired" || ng "the lapsed approval rests expired"
[ "$(stub_count)" = "$S0" ] && [ "$(orders_count)" = "$O0" ] && ok "expired pay created nothing" || ng "expired pay created nothing"

echo "-- guest address validation: a missing city stays on the step with its error"
J8=$(mktemp); warm $J8
run_agent $J8 10000
step $J8 /agent/settle/start >/dev/null
step $J8 /agent/settle/agree >/dev/null
step $J8 /agent/settle/address \
  --data-urlencode "email=guest8@example.com" --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" \
  --data-urlencode "postal_code=1000001" --data-urlencode "prefecture=東京都" \
  --data-urlencode "city=" --data-urlencode "address_line=千代田1-1-1" \
  --data-urlencode "building=" --data-urlencode "phone=09012345678" >/dev/null
get_agent $J8
grep -q 'お届け先の指定' /tmp/s65_page_$$.html && ok "an invalid address stays on the address step" || ng "an invalid address stays on the address step"
grep -q '市区町村を入力してください' /tmp/s65_page_$$.html && ok "the field error is shown" || ng "the field error is shown"
grep -q 'お支払い方法の選択' /tmp/s65_page_$$.html && ng "no progression on an invalid address" || ok "no progression on an invalid address"

echo "-- member parity: the saved default is the one-tap pick, konbini settles via the stub"
db "DELETE FROM cart_items WHERE user_id = $TEST_ID; DELETE FROM agent_approvals WHERE user_id = $TEST_ID" >/dev/null
db "DELETE FROM member_addresses WHERE user_id = $TEST_ID" >/dev/null
db "INSERT INTO member_addresses (user_id, family_name, given_name, postal_code, prefecture, city, address_line, building, phone, is_default) VALUES ($TEST_ID, '検証', '花子', '1000001', '東京都', '千代田区', '千代田2-2-2', '', '08011112222', true)" >/dev/null
ADDR_ID=$(db "SELECT id FROM member_addresses WHERE user_id = $TEST_ID AND address_line = '千代田2-2-2'")
JM=$(mktemp)
curl -s -c $JM $BASE/login -o /tmp/s65_login_$$.html
T=$(csrf /tmp/s65_login_$$.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/login \
  --data-urlencode "email=test@example.com" --data-urlencode "$PF=$MPW" --data-urlencode "$CT=$T"
run_agent $JM 10000
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE user_id = $TEST_ID AND added_by_agent")" -ge 1 ] \
  && ok "member run stocked agent lines" || ng "member run stocked agent lines"
step $JM /agent/settle/start >/dev/null
step $JM /agent/settle/agree >/dev/null
get_agent $JM
grep -q 'この住所でOK' /tmp/s65_page_$$.html && ok "the address step offers the saved default" || ng "the address step offers the saved default"
grep -q '千代田2-2-2' /tmp/s65_page_$$.html && ok "the saved default is shown" || ng "the saved default is shown"
step $JM /agent/settle/address --data-urlencode "delivery_choice=$ADDR_ID" >/dev/null
get_agent $JM
grep -q 'お支払い方法の選択' /tmp/s65_page_$$.html && ok "the one-tap pick advances to the payment step" || ng "the one-tap pick advances to the payment step"
step $JM /agent/settle/method --data-urlencode "payment_method=konbini" >/dev/null
step $JM /agent/settle/proceed >/dev/null
S0=$(stub_count)
OUT=$(step $JM /agent/settle/pay)
case "$OUT" in "303 $STUB"*) ok "member konbini pay redirects to the hosted page";; *) ng "member konbini pay redirects to the hosted page ($OUT)";; esac
[ "$(stub_count)" = "$((S0+1))" ] && ok "member settlement created one session" || ng "member settlement created one session"
MORDER=$(db "SELECT consumed_order_id FROM agent_approvals WHERE user_id = $TEST_ID ORDER BY id DESC LIMIT 1")
[ "$(pii_dec "$(db "SELECT city FROM orders WHERE id = $MORDER")")/$(pii_dec "$(db "SELECT address_line FROM orders WHERE id = $MORDER")")" = "千代田区/千代田2-2-2" ] \
  && ok "the order delivers to the saved address" || ng "the order delivers to the saved address"
[ "$(db "SELECT method FROM payments WHERE order_id = $MORDER")" = "konbini" ] && ok "the member payment is konbini" || ng "the member payment is konbini"
db "DELETE FROM member_addresses WHERE id = $ADDR_ID" >/dev/null

echo "-- human flow untouched: coupons live, a forged dummy slug settles nothing"
J9=$(mktemp); warm $J9
run_agent $J9 10000
step $J9 /agent/settle/start >/dev/null
deliver $J9 guest9@example.com
grep -q 'name="coupon_code"' /tmp/s65_co_$$.html && ok "the delivery page keeps the coupon field during a settlement" || ng "the delivery page keeps the coupon field during a settlement"
curl -s -b $J9 -c $J9 $BASE/checkout/payment -o /tmp/s65_pay_$$.html
grep -q 'name="payment_method"' /tmp/s65_pay_$$.html && ok "the human payment step offers its methods" || ng "the human payment step offers its methods"
grep -q 'name="agent_token"' /tmp/s65_pay_$$.html && ng "no dummy token field anywhere" || ok "no dummy token field anywhere"
T=$(csrf /tmp/s65_pay_$$.html)
O0=$(orders_count)
curl -s -b $J9 -c $J9 -X POST $BASE/checkout/payment -o /tmp/s65_forge_$$.html \
  --data-urlencode "$CT=$T" --data-urlencode "payment_method=agent_test"
grep -q 'エージェント決済（テスト）' /tmp/s65_forge_$$.html && ng "forged dummy slug never reaches the dummy" || ok "forged dummy slug never reaches the dummy"
grep -q '代金引換' /tmp/s65_forge_$$.html && ok "forged slug lands on the default method" || ng "forged slug lands on the default method"
[ "$(orders_count)" = "$O0" ] && ok "no order from the forged attempt" || ng "no order from the forged attempt"

echo "-- the invariant: the owner's data and the public catalog are untouched"
[ "$(basnet_counts)" = "$INVARIANT_START" ] && ok "Owner counts unchanged ($INVARIANT_START)" || ng "Owner counts unchanged ($INVARIANT_START -> $(basnet_counts))"
[ "$(db "SELECT COUNT(*) FROM products WHERE status = 'public'")" = "$PUBLIC_START" ] && ok "public products unchanged ($PUBLIC_START)" || ng "public products unchanged"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE user_id = 32 AND added_by_agent")" = "0" ] && ok "no agent lines in the owner's cart" || ng "no agent lines in the owner's cart"

echo "===== TEARDOWN: restore the development app ====="
kill $STUB_PID 2>/dev/null
stop_app
nohup ./run.sh >/tmp/s65_runsh.log 2>&1 &
for i in $(seq 1 90); do
  curl -s -o /dev/null $BASE/healthz && break
  sleep 1
done
curl -s -o /dev/null $BASE/healthz && ok "development app restored" || ng "development app restored"
result
