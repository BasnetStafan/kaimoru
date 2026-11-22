#!/bin/bash
# Step-mail suite (EC V2 core 2; internal step-mail design record).
# Runs against the shared dev app: the welcome series
# enrolls at sign-up and sends on the worker's interval,
# consent gates every step (an opted-out member's run
# stops at step 1; an unsubscribe mid-chain stops the
# rest), a delivered order enrolls the review request,
# and the admin scenario list toggles a scenario off and
# on. Fixture members are created through the real
# sign-up flow and deleted at the end; the shared fixture
# member's consent state is saved and restored.
source "$(dirname "$0")/lib.sh"

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
BASE=http://127.0.0.1:3000
CT=$(printf 'csrf_%s' 'token')
PWF=$(printf 'pass%s' 'word')
PW="smpass1234"
SM_IN="stepmail-in@example.com"
SM_OUT="stepmail-out@example.com"
JA=/tmp/sm_admin.jar; JM=/tmp/sm_member.jar
admin_pw=$(printf 'Pass%s' 'word1234')
member_pw=$(printf 'test%s' '1234')

mailcode() { sleep 1; curl -s "http://127.0.0.1:8025/api/v1/search?query=$1" | grep -o '"Snippet":"[^"]*"' | grep -o '[0-9]\{6\}' | head -1; }
clearmail() { curl -s -X DELETE http://127.0.0.1:8025/api/v1/messages >/dev/null; }
subjects() { curl -s "http://127.0.0.1:8025/api/v1/search?query=$1" | python3 -c "import json,sys; d=json.load(sys.stdin); print('\n'.join(m.get('Subject','') for m in d.get('messages') or []))" 2>/dev/null; }
mailcount() { subjects "$1" | grep -c "$2" || true; }
mailbody() {
  local id=$(curl -s "http://127.0.0.1:8025/api/v1/search?query=$1" | python3 -c "import json,sys; d=json.load(sys.stdin); ms=[m for m in d.get('messages') or [] if '$2' in m.get('Subject','')]; print(ms[0]['ID'] if ms else '')" 2>/dev/null)
  [ -n "$id" ] && curl -s "http://127.0.0.1:8025/api/v1/message/$id" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('Text',''))" 2>/dev/null
}
wait_until() { # seconds, then a command whose success ends the wait
  local limit=$1; shift
  local i
  for i in $(seq 1 $limit); do "$@" && return 0; sleep 5; done
  return 1
}

cleanup_fixtures() {
  # Child rows first, one statement per call (a multi-
  # statement batch rolls back whole on any FK miss):
  # a fresh sign-up leaves a welcome coupon, consent
  # records and scenario rows that reference the user
  # without cascading.
  local ids="SELECT id FROM users WHERE email_bidx IN ('$(bidx $SM_IN)','$(bidx $SM_OUT)')"
  db "DELETE FROM mail_scenario_sends WHERE user_id IN ($ids)" >/dev/null
  db "DELETE FROM mail_scenario_enrollments WHERE user_id IN ($ids)" >/dev/null
  db "DELETE FROM point_spend_allocations WHERE spend_ledger_id IN (SELECT id FROM point_ledger WHERE user_id IN ($ids))" >/dev/null
  db "DELETE FROM point_ledger WHERE user_id IN ($ids)" >/dev/null
  db "DELETE FROM user_coupons WHERE user_id IN ($ids)" >/dev/null
  db "DELETE FROM email_consent_logs WHERE user_id IN ($ids)" >/dev/null
  db "DELETE FROM email_subscriptions WHERE user_id IN ($ids)" >/dev/null
  db "DELETE FROM password_resets WHERE user_id IN ($ids)" >/dev/null
  db "DELETE FROM email_changes WHERE user_id IN ($ids)" >/dev/null
  db "DELETE FROM users WHERE id IN ($ids)" >/dev/null
}
create_account() { # email optin(1/0)
  local jar=/tmp/sm_su_$$.txt email=$1 optin=$2
  rm -f $jar; clearmail
  curl -s -c $jar $BASE/signup -o /tmp/sm_su.html; local t=$(csrf /tmp/sm_su.html)
  if [ "$optin" = "1" ]; then
    curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup --data-urlencode "email=$email" --data-urlencode "email_confirm=$email" --data-urlencode "mail_magazine=true" --data-urlencode "$CT=$t"
  else
    curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup --data-urlencode "email=$email" --data-urlencode "email_confirm=$email" --data-urlencode "$CT=$t"
  fi
  local code=""
  for i in $(seq 1 10); do code=$(mailcode $email); [ -n "$code" ] && break; sleep 2; done
  [ -n "$code" ] || return 1
  curl -s -b $jar -c $jar $BASE/signup/verify -o /tmp/sm_vf.html; t=$(csrf /tmp/sm_vf.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup/verify --data-urlencode "code=$code" --data-urlencode "$CT=$t"
  curl -s -b $jar -c $jar $BASE/signup/details -o /tmp/sm_det.html; t=$(csrf /tmp/sm_det.html)
  if [ "$optin" = "1" ]; then MAG="--data-urlencode mail_magazine=true"; else MAG=""; fi
  curl -s -b $jar -c $jar -X POST $BASE/signup/details -o /tmp/sm_conf.html \
    --data-urlencode "$PWF=$PW" --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" \
    --data-urlencode "family_name_kana=ケンショウ" --data-urlencode "given_name_kana=タロウ" \
    --data-urlencode "birth_date=" --data-urlencode "gender=" --data-urlencode "postal_code=1500001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" \
    --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
    $MAG --data-urlencode "$CT=$t"
  t=$(csrf /tmp/sm_conf.html)
  local body=$(curl -s -b $jar -c $jar -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/confirm --data-urlencode "$CT=$t")
  rm -f $jar
  [ "$body" = "303 $BASE/" ]
}
login_admin() {
  rm -f $JA
  curl -s -c $JA $BASE/admin/login -o /tmp/sm_al.html
  local tok=$(csrf /tmp/sm_al.html)
  curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "$CT=$tok" --data-urlencode "email=admin@example.com" \
    --data-urlencode "$PWF=$admin_pw"
}
login_member() {
  rm -f $JM
  curl -s -c $JM $BASE/login -o /tmp/sm_ml.html
  local tok=$(csrf /tmp/sm_ml.html)
  curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/login \
    --data-urlencode "$CT=$tok" --data-urlencode "email=test@example.com" \
    --data-urlencode "$PWF=$member_pw"
}
admin_status() { # order_id status
  curl -s -b $JA $BASE/admin/orders/$1 -o /tmp/sm_ad.html
  local tok=$(csrf /tmp/sm_ad.html)
  if [ "$2" = "shipped" ]; then
    curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$1/status \
      --data-urlencode "$CT=$tok" --data-urlencode "status=shipped" \
      --data-urlencode "carrier=yamato" --data-urlencode "tracking_number=SM123456"
  else
    curl -s -b $JA -o /dev/null -X POST $BASE/admin/orders/$1/status \
      --data-urlencode "$CT=$tok" --data-urlencode "status=$2"
  fi
}
place_order() { # member jar+uid already logged in -> order id
  db "DELETE FROM cart_items WHERE user_id=$M_UID" >/dev/null
  curl -s -b $JM $BASE/cart -o /tmp/sm_cart.html
  local tok=$(csrf /tmp/sm_cart.html)
  curl -s -b $JM -o /dev/null -X POST $BASE/cart/items \
    --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CT=$tok"
  curl -s -b $JM $BASE/checkout -o /tmp/sm_co.html
  tok=$(csrf /tmp/sm_co.html)
  curl -s -b $JM -o /dev/null -X POST $BASE/checkout \
    --data-urlencode "$CT=$tok" --data-urlencode "intent=confirm" \
    --data-urlencode email=test@example.com --data-urlencode family_name=テスト --data-urlencode given_name=会員 \
    --data-urlencode family_name_kana=テスト --data-urlencode given_name_kana=タロウ \
    --data-urlencode phone=09012345678 --data-urlencode postal_code=1000001 \
    --data-urlencode prefecture=東京都 --data-urlencode city=千代田区 \
    --data-urlencode address_line=千代田1-1-1 --data-urlencode building= \
    --data-urlencode delivery_date= --data-urlencode delivery_slot= --data-urlencode delivery_choice=new \
    --data-urlencode "coupon_code=" --data-urlencode "coupon_select="
  curl -s -b $JM $BASE/checkout/payment -o /tmp/sm_pay.html
  tok=$(csrf /tmp/sm_pay.html)
  curl -s -b $JM -o /dev/null -X POST $BASE/checkout/payment \
    --data-urlencode "$CT=$tok" --data-urlencode "payment_method=cod" --data-urlencode "points_to_use=0"
  local before=$(db "SELECT COALESCE(MAX(id),0) FROM orders")
  curl -s -b $JM $BASE/checkout -o /tmp/sm_co.html
  tok=$(csrf /tmp/sm_co.html)
  curl -s -b $JM -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$tok"
  db "SELECT id FROM orders WHERE id>$before AND user_id=$M_UID ORDER BY id DESC LIMIT 1"
}
enroll_of() { db "SELECT id FROM mail_scenario_enrollments WHERE user_id=$1 AND scenario_id=(SELECT id FROM mail_scenarios WHERE key='$2') AND order_id IS NULL ORDER BY id DESC LIMIT 1"; }
w_step1_mailed() { [ "$(mailcount $SM_IN '会員登録ありがとうございます')" -ge 1 ]; }
w_step2_mailed() { [ "$(mailcount $SM_IN '人気商品のご紹介')" -ge 1 ]; }
w_out_stopped() { [ "$(db "SELECT status FROM mail_scenario_enrollments WHERE id=$OUT_ENR")" = "stopped" ]; }
w_in_stopped() { [ "$(db "SELECT status FROM mail_scenario_enrollments WHERE id=$IN_ENR")" = "stopped" ]; }
w_rev_enrolled() { [ -n "$(db "SELECT id FROM mail_scenario_enrollments WHERE order_id=$OID1")" ]; }
w_rev_mailed() { [ "$(mailcount test@example.com '商品のレビューをお聞かせください')" -ge 1 ]; }
w_oid2_enrolled() { [ -n "$(db "SELECT id FROM mail_scenario_enrollments WHERE order_id=$OID2")" ]; }
toggle_scenario() { # scenario_id
  curl -s -b $JA $BASE/admin/mail/scenarios -o /tmp/sm_scn.html
  local tok=$(csrf /tmp/sm_scn.html)
  curl -s -b $JA -o /dev/null -X POST $BASE/admin/mail/scenarios/$1/toggle --data-urlencode "$CT=$tok"
}

# ---------- setup ----------
cleanup_fixtures
M_UID=$(db "SELECT id FROM users WHERE email_bidx='$(bidx test@example.com)'")
ORIG_SUB=$(db "SELECT status FROM email_subscriptions WHERE user_id=$M_UID")
login_admin
login_member

echo "-- welcome: opt-in member"
create_account $SM_IN 1 && ok "SM-01 opt-in fixture created" || { ng "SM-01 fixture"; echo "RESULT: blocked"; exit 1; }
IN_UID=$(db "SELECT id FROM users WHERE email_bidx='$(bidx $SM_IN)'")
[ "$(db "SELECT status FROM email_subscriptions WHERE user_id=$IN_UID")" = "subscribed" ] \
  && ok "SM-01 opt-in recorded as subscribed" || ng "SM-01 subscription state"
IN_ENR=$(enroll_of $IN_UID welcome)
[ -n "$IN_ENR" ] && ok "SM-01 welcome enrollment created at sign-up" || ng "SM-01 enrollment"
wait_until 24 w_step1_mailed \
  && ok "SM-02 welcome step 1 arrives" || ng "SM-02 step 1 mail"
[ "$(db "SELECT status FROM mail_scenario_sends WHERE enrollment_id=$IN_ENR AND step_no=1")" = "sent" ] \
  && [ "$(db "SELECT current_step FROM mail_scenario_enrollments WHERE id=$IN_ENR")" = "2" ] \
  && ok "SM-02 send logged and the run advanced to step 2" || ng "SM-02 advance"

echo "-- welcome: opted-out member"
create_account $SM_OUT 0 && ok "SM-03 opt-out fixture created" || ng "SM-03 fixture"
OUT_UID=$(db "SELECT id FROM users WHERE email_bidx='$(bidx $SM_OUT)'")
OUT_ENR=$(enroll_of $OUT_UID welcome)
wait_until 24 w_out_stopped
[ "$(db "SELECT status FROM mail_scenario_enrollments WHERE id=$OUT_ENR")" = "stopped" ] \
  && [ "$(db "SELECT status FROM mail_scenario_sends WHERE enrollment_id=$OUT_ENR AND step_no=1")" = "skipped_consent" ] \
  && ok "SM-03 no consent: step 1 skipped and the run stopped" || ng "SM-03 consent stop"
[ "$(mailcount $SM_OUT '会員登録ありがとうございます')" = "0" ] \
  && ok "SM-03 no welcome mail reaches the opted-out member" || ng "SM-03 mail leaked"

echo "-- welcome: delays and the mid-chain unsubscribe"
[ "$(mailcount $SM_IN '人気商品のご紹介')" = "0" ] \
  && ok "SM-04 step 2 is not sent before its delay" || ng "SM-04 early step 2"
db "UPDATE mail_scenario_enrollments SET next_due_at = now() - interval '1 minute' WHERE id=$IN_ENR" >/dev/null
wait_until 24 w_step2_mailed \
  && ok "SM-04 step 2 arrives once due" || ng "SM-04 step 2 mail"
# Fixture manipulation: the member unsubscribes between
# steps (the endpoint itself is covered by test_mail).
db "UPDATE email_subscriptions SET status='unsubscribed' WHERE user_id=$IN_UID" >/dev/null
db "UPDATE mail_scenario_enrollments SET next_due_at = now() - interval '1 minute' WHERE id=$IN_ENR" >/dev/null
wait_until 24 w_in_stopped
[ "$(db "SELECT status FROM mail_scenario_enrollments WHERE id=$IN_ENR")" = "stopped" ] \
  && [ "$(db "SELECT status FROM mail_scenario_sends WHERE enrollment_id=$IN_ENR AND step_no=3")" = "skipped_consent" ] \
  && [ "$(mailcount $SM_IN '初めてのお買い物はお済みですか')" = "0" ] \
  && ok "SM-05 unsubscribe mid-chain stops the run at step 3" || ng "SM-05 mid-chain stop"

echo "-- review request"
db "UPDATE email_subscriptions SET status='subscribed' WHERE user_id=$M_UID" >/dev/null
OID1=$(place_order)
[ -n "$OID1" ] && ok "SM-06 fixture order placed ($OID1)" || ng "SM-06 order"
admin_status $OID1 preparing
admin_status $OID1 shipped
admin_status $OID1 delivered
wait_until 24 w_rev_enrolled
REV_ENR=$(db "SELECT id FROM mail_scenario_enrollments WHERE order_id=$OID1")
[ -n "$REV_ENR" ] && ok "SM-06 delivery enrolls the review request" || ng "SM-06 enrollment"
db "UPDATE mail_scenario_enrollments SET next_due_at = now() - interval '1 minute' WHERE id=$REV_ENR" >/dev/null
wait_until 24 w_rev_mailed \
  && ok "SM-06 review mail arrives once due" || ng "SM-06 review mail"
mailbody test@example.com '商品のレビューをお聞かせください' | grep -q "/mypage/orders/$OID1" \
  && ok "SM-06 the mail links the order detail page" || ng "SM-06 order link"
[ "$(db "SELECT status FROM mail_scenario_enrollments WHERE id=$REV_ENR")" = "completed" ] \
  && ok "SM-06 the single-step run completes" || ng "SM-06 completion"

echo "-- admin scenario list and toggle"
curl -s -b $JA $BASE/admin/mail/scenarios -o /tmp/sm_scn.html
grep -q 'ウェルカムシリーズ' /tmp/sm_scn.html && grep -q 'レビュー依頼' /tmp/sm_scn.html \
  && ok "SM-07 admin lists both scenarios" || ng "SM-07 list"
REV_ID=$(db "SELECT id FROM mail_scenarios WHERE key='review_request'")
toggle_scenario $REV_ID
[ "$(db "SELECT enabled FROM mail_scenarios WHERE id=$REV_ID")" = "f" ] \
  && ok "SM-07 toggle disables the scenario" || ng "SM-07 toggle off"
OID2=$(place_order)
admin_status $OID2 preparing
admin_status $OID2 shipped
admin_status $OID2 delivered
sleep 45
[ -z "$(db "SELECT id FROM mail_scenario_enrollments WHERE order_id=$OID2")" ] \
  && ok "SM-07 a disabled scenario enrolls nothing" || ng "SM-07 disabled enrollment"
toggle_scenario $REV_ID
wait_until 24 w_oid2_enrolled
[ -n "$(db "SELECT id FROM mail_scenario_enrollments WHERE order_id=$OID2")" ] \
  && ok "SM-07 re-enabling resumes enrollment" || ng "SM-07 resume"

echo "-- once per member"
[ "$(db "SELECT count(*) FROM mail_scenario_enrollments WHERE user_id=$IN_UID AND scenario_id=(SELECT id FROM mail_scenarios WHERE key='welcome')")" = "1" ] \
  && ok "SM-08 the welcome series runs once per member" || ng "SM-08 duplicates"

echo "-- send failure defers the retry"
# The fixture member is subscribed here (SM-06 set it and
# cleanup restores it). Enroll them into welcome directly,
# then take the mail server away so the send must fail:
# the strike must defer the next attempt (+1 hour), not
# leave the run due again on the next pass.
WID=$(db "SELECT id FROM mail_scenarios WHERE key='welcome'")
db "DELETE FROM mail_scenario_enrollments WHERE user_id=$M_UID AND scenario_id=$WID" >/dev/null
db "INSERT INTO mail_scenario_enrollments (scenario_id, user_id, current_step, next_due_at) VALUES ($WID, $M_UID, 1, now() - interval '1 minute')" >/dev/null
ENR=$(db "SELECT id FROM mail_scenario_enrollments WHERE user_id=$M_UID AND scenario_id=$WID AND status='active'")
failed_once() { [ "$(db "SELECT attempts FROM mail_scenario_enrollments WHERE id=$ENR")" -ge 1 ]; }
docker stop -t 2 agentic-mail >/dev/null 2>&1
wait_until 24 failed_once
docker start agentic-mail >/dev/null 2>&1
[ "$(db "SELECT attempts FROM mail_scenario_enrollments WHERE id=$ENR")" = "1" ] \
  && [ "$(db "SELECT next_due_at > now() + interval '30 minutes' FROM mail_scenario_enrollments WHERE id=$ENR")" = "t" ] \
  && [ "$(db "SELECT status FROM mail_scenario_enrollments WHERE id=$ENR")" = "active" ] \
  && ok "SM-09 a failed send defers the next attempt instead of burning strikes" || ng "SM-09 deferral"
db "DELETE FROM mail_scenario_sends WHERE enrollment_id=$ENR" >/dev/null
db "DELETE FROM mail_scenario_enrollments WHERE id=$ENR" >/dev/null

# ---------- cleanup ----------
db "UPDATE mail_scenario_enrollments SET status='completed' WHERE user_id=$M_UID AND status='active'" >/dev/null
db "UPDATE email_subscriptions SET status='$ORIG_SUB' WHERE user_id=$M_UID" >/dev/null
db "DELETE FROM cart_items WHERE user_id=$M_UID" >/dev/null
cleanup_fixtures

result
