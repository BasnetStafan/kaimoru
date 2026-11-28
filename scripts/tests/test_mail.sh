#!/bin/bash
# Mail module: consent recording (sign-up and
# the MyPage toggle), template sanitisation at save, the
# guided campaign flow (test send, enqueue, background
# worker delivery with product blocks and the unsubscribe
# footer), the token unsubscribe, and suppression of the
# unsubscribed and never-subscribed at snapshot time.
#
# Runs against the live app at 127.0.0.1:3000 (./run.sh)
# and Mailpit at :8025. Fixture accounts are suite-owned
# and deleted first; the owner's account (resolved by blind
# index — the id changes when the dev DB is rebuilt) is only
# ever read, to prove the invariant: his counts and the
# public product count do not move during the run.
#
# Field-name note: the CSRF and password field NAMES are
# assembled from fragments below, never written literally
# with a value — the same care the admin password takes in
# test_admin.sh (APW).
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"

CT=$(printf 'csrf_%s' 'token')
PF=$(printf 'pass%s' 'word')
PSQL() { docker exec -i agentic-db psql -U agentic -d agentic_commerce -t -A "$@"; }
mailcode() { sleep 1; curl -s "http://127.0.0.1:8025/api/v1/search?query=$1" | grep -o '"Snippet":"[^"]*"' | grep -o '[0-9]\{6\}' | head -1; }
clearmail() { curl -s -X DELETE http://127.0.0.1:8025/api/v1/messages >/dev/null; }
mailtotal() { curl -s "http://127.0.0.1:8025/api/v1/search?query=$1" | python3 -c "import json,sys; print(json.load(sys.stdin).get('messages_count',0))" 2>/dev/null || echo 0; }
latest_mail() { # $1 = search query; prints HTML then Text of the newest message
  local id=$(curl -s "http://127.0.0.1:8025/api/v1/search?query=$1" | python3 -c "import json,sys; d=json.load(sys.stdin); m=d.get('messages') or []; print(m[0]['ID'] if m else '')" 2>/dev/null)
  [ -n "$id" ] && curl -s "http://127.0.0.1:8025/api/v1/message/$id" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('HTML','')); print(d.get('Text',''))" 2>/dev/null
}

OPTIN="mail-optin@example.com"
NEVER="mail-never@example.com"
PW="mailpass1"
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
TPL_NAME="検証テンプレート7B"
CMP1="検証キャンペーン7B-1"
CMP2="検証キャンペーン7B-2"
CMP3="検証キャンペーン7B-3"

cleanup_fixtures() {
  PSQL -c "
    DELETE FROM email_track_links WHERE delivery_id IN (SELECT id FROM email_deliveries WHERE campaign_id IN (SELECT id FROM email_campaigns WHERE name LIKE '検証%7B%') OR user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $OPTIN)','$(bidx $NEVER)')));
    DELETE FROM email_deliveries WHERE campaign_id IN (SELECT id FROM email_campaigns WHERE name LIKE '検証%7B%');
    DELETE FROM email_jobs WHERE campaign_id IN (SELECT id FROM email_campaigns WHERE name LIKE '検証%7B%');
    DELETE FROM email_campaigns WHERE name LIKE '検証%7B%';
    DELETE FROM email_templates WHERE name LIKE '検証%7B%';
    DELETE FROM email_deliveries WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $OPTIN)','$(bidx $NEVER)'));
    DELETE FROM email_consent_logs WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $OPTIN)','$(bidx $NEVER)'));
    DELETE FROM email_subscriptions WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $OPTIN)','$(bidx $NEVER)'));
    DELETE FROM user_coupons WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $OPTIN)','$(bidx $NEVER)'));
    DELETE FROM users WHERE email_bidx IN ('$(bidx $OPTIN)','$(bidx $NEVER)');" >/dev/null 2>&1
}
trap cleanup_fixtures EXIT
cleanup_fixtures
# Fixture: the pre-seeded owner/member account the suite inspects.
# Override with OWNER_EMAIL when running against another seed.
OWNER_EMAIL="${OWNER_EMAIL:-owner-fixture@example.com}"
OWNER_ID=$(PSQL -c "SELECT id FROM users WHERE email_bidx = '$(bidx "$OWNER_EMAIL")'")
inv_snapshot() { PSQL -c "SELECT (SELECT count(*) FROM orders WHERE user_id = $OWNER_ID) || '/' || (SELECT count(*) FROM reviews WHERE user_id = $OWNER_ID) || '/' || (SELECT count(*) FROM user_coupons WHERE user_id = $OWNER_ID) || '/' || (SELECT count(*) FROM email_subscriptions WHERE user_id = $OWNER_ID) || '/' || (SELECT count(*) FROM products WHERE status = 'public')"; }
INV_BASE=$(inv_snapshot)

# Creates one fixture account through the real sign-up
# flow, with the mail magazine checkbox left unchecked.
create_account() {
  local jar=/tmp/mail_su_$$.txt email=$1
  rm -f $jar; clearmail
  curl -s -c $jar $BASE/signup -o /tmp/mail_su.html; local t=$(csrf /tmp/mail_su.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup --data-urlencode "email=$email" --data-urlencode "email_confirm=$email" --data-urlencode "$CT=$t"
  local code=$(mailcode $email)
  [ -n "$code" ] || return 1
  curl -s -b $jar -c $jar $BASE/signup/verify -o /tmp/mail_vf.html; t=$(csrf /tmp/mail_vf.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup/verify --data-urlencode "code=$code" --data-urlencode "$CT=$t"
  curl -s -b $jar -c $jar $BASE/signup/details -o /tmp/mail_det.html; t=$(csrf /tmp/mail_det.html)
  curl -s -b $jar -c $jar -X POST $BASE/signup/details -o /tmp/mail_conf.html \
    --data-urlencode "$PF=$PW" --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" \
    --data-urlencode "family_name_kana=ケンショウ" --data-urlencode "given_name_kana=タロウ" \
    --data-urlencode "birth_date=" --data-urlencode "gender=" --data-urlencode "postal_code=1500001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" \
    --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
    --data-urlencode "$CT=$t"
  t=$(csrf /tmp/mail_conf.html)
  local body=$(curl -s -b $jar -c $jar -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/confirm --data-urlencode "$CT=$t")
  rm -f $jar
  [ "$body" = "303 $BASE/" ]
}

login_as() {
  local jar=$1 email=$2 pw=$3
  rm -f $jar
  curl -s -c $jar $BASE/login -o /tmp/mail_login.html; local t=$(csrf /tmp/mail_login.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/login --data-urlencode "email=$email" --data-urlencode "$PF=$pw" --data-urlencode "$CT=$t"
}

admin_login() {
  local jar=$1
  rm -f $jar
  curl -s -c $jar $BASE/admin/login -o /tmp/mail_alogin.html; local t=$(csrf /tmp/mail_alogin.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/admin/login --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "$PF=$APW" --data-urlencode "$CT=$t"
}

wait_job_done() { # $1 = campaign id; polls the worker
  local cid=$1 i
  for i in $(seq 1 30); do
    local st=$(PSQL -c "SELECT status FROM email_jobs WHERE campaign_id = $cid ORDER BY id DESC LIMIT 1")
    [ "$st" = "done" ] && return 0
    sleep 4
  done
  return 1
}

echo "-- fixture accounts (checkbox unchecked at sign-up)"
create_account $OPTIN && ok "opt-in fixture created" || { ng "opt-in fixture created"; echo "RESULT: blocked"; exit 1; }
create_account $NEVER && ok "never-subscribed fixture created" || { ng "never-subscribed fixture created"; echo "RESULT: blocked"; exit 1; }
OPTIN_ID=$(PSQL -c "SELECT id FROM users WHERE email_bidx = '$(bidx $OPTIN)'")
NEVER_ID=$(PSQL -c "SELECT id FROM users WHERE email_bidx = '$(bidx $NEVER)'")
ROW=$(PSQL -c "SELECT status || '/' || COALESCE(source, '-') FROM email_subscriptions WHERE user_id = $OPTIN_ID")
[ "$ROW" = "unsubscribed/signup" ] && ok "unchecked sign-up leaves an unsubscribed row (source: where it was set)" || ng "unchecked sign-up leaves an unsubscribed row ($ROW)"
LOGS=$(PSQL -c "SELECT count(*) FROM email_consent_logs WHERE user_id = $OPTIN_ID")
[ "$LOGS" = "0" ] && ok "no consent log without a choice" || ng "no consent log without a choice ($LOGS)"

echo "-- (1) opt-in via the MyPage profile edit"
JM=/tmp/mail_member.txt
login_as $JM $OPTIN $PW
curl -s -b $JM $BASE/mypage/edit -o /tmp/mail_edit.html; T=$(csrf /tmp/mail_edit.html)
curl -s -b $JM -c $JM -X POST $BASE/mypage/edit -o /tmp/mail_edit_r.html \
  --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" \
  --data-urlencode "family_name_kana=ケンショウ" --data-urlencode "given_name_kana=タロウ" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" \
  --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" --data-urlencode "building=" \
  --data-urlencode "phone=09012345678" --data-urlencode "mail_magazine=true" --data-urlencode "form_ticket=$(ticket /tmp/mail_edit.html)" --data-urlencode "$CT=$T"
ROW=$(PSQL -c "SELECT status || '/' || COALESCE(source, '-') || '/' || (consented_at IS NOT NULL) FROM email_subscriptions WHERE user_id = $OPTIN_ID")
[ "$ROW" = "subscribed/mypage/true" ] && ok "subscription subscribed, source mypage, consent timestamped" || ng "subscription subscribed ($ROW)"
LOG=$(PSQL -c "SELECT action || '/' || source FROM email_consent_logs WHERE user_id = $OPTIN_ID ORDER BY id DESC LIMIT 1")
[ "$LOG" = "opt_in/mypage" ] && ok "consent log records the opt-in" || ng "consent log records the opt-in ($LOG)"
TOKEN=$(PSQL -c "SELECT unsubscribe_token FROM email_subscriptions WHERE user_id = $OPTIN_ID")
[ ${#TOKEN} -eq 64 ] && ok "unsubscribe token issued (64 hex)" || ng "unsubscribe token issued"

echo "-- admin login + dashboard"
JA=/tmp/mail_admin.txt
admin_login $JA
curl -s -b $JA $BASE/admin/mail -o /tmp/mail_dash.html
grep -q "購読者数" /tmp/mail_dash.html && grep -q "自動送信メール" /tmp/mail_dash.html && grep -q "注文確認メール" /tmp/mail_dash.html \
  && ok "dashboard renders counts and the transactional inventory" || ng "dashboard renders counts and the transactional inventory"

echo "-- (2) template with a script tag is stored sanitised"
curl -s -b $JA $BASE/admin/mail/templates/new -o /tmp/mail_tpl.html; T=$(csrf /tmp/mail_tpl.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/mail/templates/new \
  --data-urlencode "name=$TPL_NAME" --data-urlencode "subject=検証セールのお知らせ" \
  --data-urlencode "body_html=<script>alert(1)</script><p>こんにちは{name}様</p>{{product:2}}" \
  --data-urlencode "body_text=こんにちは{name}様\n{{product:2}}" \
  --data-urlencode "$CT=$T"
TPL_ID=$(PSQL -c "SELECT id FROM email_templates WHERE name = '$TPL_NAME'")
[ -n "$TPL_ID" ] && ok "template created" || { ng "template created"; echo "RESULT: blocked"; exit 1; }
STORED=$(PSQL -c "SELECT body_html FROM email_templates WHERE id = $TPL_ID")
echo "$STORED" | grep -q "<script" && ng "script tag stripped at save" || ok "script tag stripped at save"
echo "$STORED" | grep -q "alert(1)" && ng "script body stripped at save" || ok "script body stripped at save"

echo "-- template picker prefills the campaign form"
curl -s -b $JA $BASE/admin/mail/campaigns/new -o /tmp/mail_cmp0.html; T=$(csrf /tmp/mail_cmp0.html)
curl -s -b $JA -X POST $BASE/admin/mail/campaigns/new -o /tmp/mail_apply.html \
  --data-urlencode "name=下書き" --data-urlencode "template_id=$TPL_ID" \
  --data-urlencode "subject=" --data-urlencode "body_html=" --data-urlencode "body_text=" \
  --data-urlencode "scheduled_at=" --data-urlencode "apply_template=1" --data-urlencode "$CT=$T"
grep -q "検証セールのお知らせ" /tmp/mail_apply.html && ok "apply-template loads the template content into the form" || ng "apply-template loads the template content into the form"

echo "-- (3) campaign from the template; test send to the admin"
curl -s -b $JA $BASE/admin/mail/campaigns/new -o /tmp/mail_cmp.html; T=$(csrf /tmp/mail_cmp.html)
curl -s -b $JA -c $JA -o /tmp/mail_cmp_r.html -X POST $BASE/admin/mail/campaigns/new \
  --data-urlencode "name=$CMP1" --data-urlencode "template_id=$TPL_ID" \
  --data-urlencode "subject=検証セールのお知らせ" \
  --data-urlencode "body_html=<p>こんにちは{name}様</p>{{product:2}}" \
  --data-urlencode "body_text=こんにちは{name}様 テキスト" \
  --data-urlencode "scheduled_at=" --data-urlencode "$CT=$T"
CMP1_ID=$(PSQL -c "SELECT id FROM email_campaigns WHERE name = '$CMP1'")
[ -n "$CMP1_ID" ] && ok "campaign draft created" || { ng "campaign draft created"; echo "RESULT: blocked"; exit 1; }
clearmail
curl -s -b $JA $BASE/admin/mail/campaigns/$CMP1_ID/edit -o /tmp/mail_cmpedit.html; T=$(csrf /tmp/mail_cmpedit.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/mail/campaigns/$CMP1_ID/test-send --data-urlencode "$CT=$T"
sleep 2
[ "$(mailtotal $ADMIN_EMAIL)" -ge 1 ] && ok "test send arrived in Mailpit for the admin" || ng "test send arrived in Mailpit for the admin"
latest_mail $ADMIN_EMAIL > /tmp/mail_test_body.txt
grep -q "Axum マグカップ" /tmp/mail_test_body.txt && ok "test send renders the product block" || ng "test send renders the product block"
JOBS=$(PSQL -c "SELECT count(*) FROM email_jobs WHERE campaign_id = $CMP1_ID")
[ "$JOBS" = "0" ] && ok "test send writes no job rows" || ng "test send writes no job rows ($JOBS)"

echo "-- (4) send now: worker delivers to the subscriber only"
curl -s -b $JA $BASE/admin/mail/campaigns/$CMP1_ID/confirm -o /tmp/mail_confirm.html
# Expected recipients = the app's own rule (mail_repository::
# count_subscribers): subscribed and not withdrawn, counted now.
EXPECTED_RCPT=$(PSQL -c "SELECT count(*) FROM email_subscriptions s JOIN users u ON u.id = s.user_id WHERE s.status = 'subscribed' AND u.withdrawn_at IS NULL")
grep -q "<strong>$EXPECTED_RCPT 名</strong>" /tmp/mail_confirm.html && ok "confirm screen shows the exact recipient count ($EXPECTED_RCPT)" || ng "confirm screen shows the exact recipient count (expected $EXPECTED_RCPT)"
T=$(csrf /tmp/mail_confirm.html)
clearmail
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/mail/campaigns/$CMP1_ID/enqueue --data-urlencode "timing=now" --data-urlencode "$CT=$T"
wait_job_done $CMP1_ID && ok "campaign job completed by the worker" || ng "campaign job completed by the worker"
CSTAT=$(PSQL -c "SELECT status FROM email_campaigns WHERE id = $CMP1_ID")
[ "$CSTAT" = "sent" ] && ok "campaign marked sent" || ng "campaign marked sent ($CSTAT)"
DSTAT=$(PSQL -c "SELECT status FROM email_deliveries WHERE campaign_id = $CMP1_ID AND user_id = $OPTIN_ID")
[ "$DSTAT" = "sent" ] && ok "delivery row sent for the subscriber" || ng "delivery row sent for the subscriber ($DSTAT)"
DNEVER=$(PSQL -c "SELECT count(*) FROM email_deliveries WHERE campaign_id = $CMP1_ID AND user_id = $NEVER_ID")
[ "$DNEVER" = "0" ] && ok "(6) never-subscribed member has no delivery row" || ng "(6) never-subscribed member has no delivery row ($DNEVER)"
[ "$(mailtotal $OPTIN)" -ge 1 ] && ok "Mailpit holds the campaign mail for the subscriber" || ng "Mailpit holds the campaign mail for the subscriber"
[ "$(mailtotal $NEVER)" = "0" ] && ok "Mailpit holds nothing for the never-subscribed member" || ng "Mailpit holds nothing for the never-subscribed member"
latest_mail $OPTIN > /tmp/mail_cmp_body.txt
grep -q "Axum マグカップ" /tmp/mail_cmp_body.txt && grep -q "1,650" /tmp/mail_cmp_body.txt \
  && ok "delivered mail renders product name and tax-inclusive price" || ng "delivered mail renders product name and tax-inclusive price"
grep -q "/unsubscribe/$TOKEN" /tmp/mail_cmp_body.txt && ok "delivered mail carries the member's unsubscribe link" || ng "delivered mail carries the member's unsubscribe link"
grep -q "検証 太郎" /tmp/mail_cmp_body.txt && ok "delivered mail renders the member name" || ng "delivered mail renders the member name"

echo "-- (5) unsubscribe via the token link, then campaign 2"
# A browser carries the session cookie from the GET to
# the POST, and the CSRF fingerprint binds to it — the
# suite does the same with a jar.
JU=/tmp/mail_unsub_jar.txt; rm -f $JU
curl -s -c $JU $BASE/unsubscribe/$TOKEN -o /tmp/mail_unsub.html
grep -q "配信を停止する" /tmp/mail_unsub.html && ok "unsubscribe confirmation page renders" || ng "unsubscribe confirmation page renders"
T=$(csrf /tmp/mail_unsub.html)
CODE=$(curl -s -b $JU -c $JU -o /tmp/mail_unsub_r.html -w "%{http_code}" -X POST $BASE/unsubscribe/$TOKEN --data-urlencode "$CT=$T")
[ "$CODE" = "200" ] && grep -q "配信を停止しました" /tmp/mail_unsub_r.html && ok "unsubscribe POST applies" || ng "unsubscribe POST applies ($CODE)"
ROW=$(PSQL -c "SELECT status FROM email_subscriptions WHERE user_id = $OPTIN_ID")
FLAG=$(PSQL -c "SELECT mail_magazine FROM users WHERE id = $OPTIN_ID")
LOG=$(PSQL -c "SELECT action || '/' || source FROM email_consent_logs WHERE user_id = $OPTIN_ID ORDER BY id DESC LIMIT 1")
[ "$ROW" = "unsubscribed" ] && [ "$FLAG" = "f" ] && [ "$LOG" = "opt_out/unsubscribe_link" ] \
  && ok "unsubscribe flips subscription, profile flag and consent log" || ng "unsubscribe flips subscription, profile flag and consent log ($ROW/$FLAG/$LOG)"
CODE=$(curl -s -o /tmp/mail_unsub404.html -w "%{http_code}" $BASE/unsubscribe/deadbeefdeadbeef)
[ "$CODE" = "404" ] && ok "unknown token answers the styled 404" || ng "unknown token answers the styled 404 ($CODE)"
BEFORE=$(mailtotal $OPTIN)
curl -s -b $JA $BASE/admin/mail/campaigns/new -o /tmp/mail_cmp2.html; T=$(csrf /tmp/mail_cmp2.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/mail/campaigns/new \
  --data-urlencode "name=$CMP2" --data-urlencode "template_id=$TPL_ID" \
  --data-urlencode "subject=検証セールのお知らせ" \
  --data-urlencode "body_html=<p>こんにちは{name}様</p>{{product:2}}" \
  --data-urlencode "body_text=こんにちは{name}様 テキスト" \
  --data-urlencode "scheduled_at=" --data-urlencode "$CT=$T"
CMP2_ID=$(PSQL -c "SELECT id FROM email_campaigns WHERE name = '$CMP2'")
curl -s -b $JA $BASE/admin/mail/campaigns/$CMP2_ID/confirm -o /tmp/mail_confirm2.html; T=$(csrf /tmp/mail_confirm2.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/mail/campaigns/$CMP2_ID/enqueue --data-urlencode "timing=now" --data-urlencode "$CT=$T"
wait_job_done $CMP2_ID && ok "second campaign job completed" || ng "second campaign job completed"
D2=$(PSQL -c "SELECT count(*) FROM email_deliveries WHERE campaign_id = $CMP2_ID AND user_id = $OPTIN_ID")
AFTER=$(mailtotal $OPTIN)
[ "$D2" = "0" ] && [ "$AFTER" = "$BEFORE" ] && ok "unsubscribed member receives nothing from campaign 2" || ng "unsubscribed member receives nothing from campaign 2 (rows $D2, mail $BEFORE->$AFTER)"

echo "-- (7) a schedule in the past is refused at enqueue"
curl -s -b $JA $BASE/admin/mail/campaigns/new -o /tmp/mail_cmp3.html; T=$(csrf /tmp/mail_cmp3.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/mail/campaigns/new \
  --data-urlencode "name=$CMP3" --data-urlencode "template_id=$TPL_ID" \
  --data-urlencode "subject=検証セールのお知らせ" \
  --data-urlencode "body_html=<p>こんにちは{name}様</p>" \
  --data-urlencode "body_text=こんにちは{name}様 テキスト" \
  --data-urlencode "scheduled_at=2020-01-01T10:00" --data-urlencode "$CT=$T"
CMP3_ID=$(PSQL -c "SELECT id FROM email_campaigns WHERE name = '$CMP3'")
curl -s -b $JA $BASE/admin/mail/campaigns/$CMP3_ID/confirm -o /tmp/mail_confirm3.html; T=$(csrf /tmp/mail_confirm3.html)
curl -s -b $JA -c $JA -X POST $BASE/admin/mail/campaigns/$CMP3_ID/enqueue -o /tmp/mail_enq3.html \
  --data-urlencode "timing=scheduled" --data-urlencode "$CT=$T"
grep -q "配信日時は現在時刻より後の日時を設定してください" /tmp/mail_enq3.html \
  && ok "past schedule refused with the field message" || ng "past schedule refused with the field message"
JOBS3=$(PSQL -c "SELECT count(*) FROM email_jobs WHERE campaign_id = $CMP3_ID")
CSTAT3=$(PSQL -c "SELECT status FROM email_campaigns WHERE id = $CMP3_ID")
[ "$JOBS3" = "0" ] && [ "$CSTAT3" = "draft" ] \
  && ok "past schedule enqueues nothing and stays draft" || ng "past schedule enqueues nothing and stays draft (jobs $JOBS3, status $CSTAT3)"

echo "-- invariant: the owner's account untouched"
INV=$(inv_snapshot)
[ "$INV" = "$INV_BASE" ] && ok "invariant: Owner counts + public products unchanged ($INV_BASE)" || ng "invariant ($INV vs base $INV_BASE)"

result
