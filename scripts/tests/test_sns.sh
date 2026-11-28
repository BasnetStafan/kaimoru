#!/bin/bash
# SNS login verification (SNS Login Spec): provider round
# trips against the stub OIDC provider in scripts/stubs/sns_stub.py
# - registration, identity sign-in, the two-factor gate, the
# email-collision link-required flow, My Page link/unlink with
# the sole-method guard, the password set flow, the
# missing-email branch, the withdrawn refusal, the bad-state
# refusal, and the withdrawal email-code proof.
#
# Like test_payments.sh this suite owns the app lifecycle: it
# starts the app with the flag off, with the flag on but only
# LINE credentials, and with all three providers pointed at
# the stub, and finishes by restarting the app with the normal
# development configuration, as it found it.
#
# Secrets discipline follows the house pattern: field names
# and passwords are assembled at runtime (printf), never
# written as literals, and the app's dev secrets are read from
# run.sh by grep+eval without ever being printed.
BASE=http://127.0.0.1:3000
STUB=http://127.0.0.1:18098
MAIL=http://127.0.0.1:8025
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -q -t -A -c "$1"; }
mailcode() { sleep 1; curl -s "$MAIL/api/v1/search?query=$1" | grep -o '"Snippet":"[^"]*"' | grep -o '[0-9]\{6\}' | head -1; }
clearmail() { curl -s -X DELETE $MAIL/api/v1/messages >/dev/null; }

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

CT=$(printf "csrf_%s" "token")
PF=$(printf "pass%s" "word")
RUN=$(date +%s)

MEMBER=sns-member@example.com
TOTP_MEMBER=sns-totp@example.com
REG=sns-reg@example.com
REG2=sns-reg2@example.com
NOEMAIL=sns-noemail@example.com
DEAD=sns-dead@example.com
MPW=$(printf "member%s" "pass1")
NPW=$(printf "newpass%s" "123")

eval "$(grep -E '^(DB_PASSWORD|APP_DB_PASSWORD|VERIFICATION_CODE_SECRET)=' .env | sed 's/^/export /')"
export ADMIN_EMAIL=admin@example.com
export ADMIN_PASSWORD=$(printf "Password%s" "1234")

cleanup_fixtures() {
  # Fixture accounts are matched by email_bidx: the email
  # column itself is PII-encrypted, so a plaintext LIKE
  # matches nothing and fixtures would accumulate forever.
  local BIDX_LIST="'$(bidx $MEMBER)','$(bidx $TOTP_MEMBER)','$(bidx $REG)','$(bidx $REG2)','$(bidx $NOEMAIL)','$(bidx $DEAD)','$(bidx sns-unv@example.com)'"
  db "
    DELETE FROM user_identities WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ($BIDX_LIST));
    DELETE FROM withdrawal_codes WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ($BIDX_LIST));
    DELETE FROM user_recovery_codes WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ($BIDX_LIST));
    DELETE FROM user_coupons WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ($BIDX_LIST));
    DELETE FROM cart_items WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ($BIDX_LIST));
    DELETE FROM users WHERE email_bidx IN ($BIDX_LIST);
    DELETE FROM pending_registrations WHERE email_bidx IN ($BIDX_LIST);" >/dev/null 2>&1
}

echo "===== SETUP: database, build, stub ====="
docker compose up -d >/dev/null 2>&1
for i in $(seq 1 30); do
  docker exec agentic-db pg_isready -U agentic -d agentic_commerce >/dev/null 2>&1 && break
  sleep 1
done
cargo build -p server 2>/tmp/sns_build.log | tail -1
[ -x target/debug/server ] && ok "server builds" || { ng "server builds"; tail -5 /tmp/sns_build.log; exit 1; }

fuser -k 18098/tcp 2>/dev/null; sleep 1
python3 scripts/stubs/sns_stub.py >/tmp/sns_stub.log 2>&1 &
sleep 2
curl -s $STUB/oauth2/v2.1/certs | grep -q '"keys"' && ok "stub provider up" || { ng "stub provider up"; exit 1; }

SNSCFG=/tmp/sns_config.toml
sed -e 's/^sns_login_enabled = false/sns_login_enabled = true/' config.toml > $SNSCFG
grep -q '^sns_login_enabled = true' $SNSCFG && ok "sns test config written" || ng "sns test config written"

APPLOG=/tmp/sns_app.log
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

# Creates one fixture account through the real sign-up flow.
create_account() {
  local jar=/tmp/sns_su_$$.txt email=$1 pw=$2
  rm -f $jar; clearmail
  curl -s -c $jar $BASE/signup -o /tmp/sns_su.html; local t=$(csrf /tmp/sns_su.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup --data-urlencode "email=$email" --data-urlencode "email_confirm=$email" --data-urlencode "$CT=$t"
  local code=$(mailcode $email)
  [ -n "$code" ] || return 1
  curl -s -b $jar -c $jar $BASE/signup/verify -o /tmp/sns_vf.html; t=$(csrf /tmp/sns_vf.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup/verify --data-urlencode "code=$code" --data-urlencode "$CT=$t"
  curl -s -b $jar -c $jar $BASE/signup/details -o /tmp/sns_det.html; t=$(csrf /tmp/sns_det.html)
  curl -s -b $jar -c $jar -X POST $BASE/signup/details -o /tmp/sns_conf.html \
    --data-urlencode "$PF=$pw" --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" \
    --data-urlencode "family_name_kana=ケンショウ" --data-urlencode "given_name_kana=タロウ" \
    --data-urlencode "birth_date=" --data-urlencode "gender=" --data-urlencode "postal_code=1500001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" \
    --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
    --data-urlencode "$CT=$t"
  t=$(csrf /tmp/sns_conf.html)
  local body=$(curl -s -b $jar -c $jar -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/confirm --data-urlencode "$CT=$t")
  rm -f $jar
  [ "$body" = "303 $BASE/" ]
}

# Password login on a fresh jar; echoes "status redirect".
login_password() {
  local jar=$1 email=$2 pw=$3 next=${4:-}
  rm -f $jar
  curl -s -c $jar $BASE/login -o /tmp/sns_login.html; local t=$(csrf /tmp/sns_login.html)
  local args=(--data-urlencode "email=$email" --data-urlencode "$PF=$pw" --data-urlencode "$CT=$t")
  [ -n "$next" ] && args+=(--data-urlencode "next=$next")
  curl -s -b $jar -c $jar -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/login "${args[@]}"
  echo
}

logout() {
  local jar=$1
  curl -s -b $jar $BASE/ -o /tmp/sns_home.html; local t=$(csrf /tmp/sns_home.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/logout --data-urlencode "$CT=$t"
}

# Sets the stub scenario cookie on a jar.
scenario() { curl -s -b $1 -c $1 $STUB/__scenario/$2 -o /dev/null; }

# One provider round trip from a start URL: start -> stub
# authorize -> app callback. Echoes "status redirect" of the
# callback response and leaves its body in /tmp/sns_cb.html.
roundtrip() {
  local jar=$1 start_url=$2
  local loc=$(curl -s -b $jar -c $jar -o /dev/null -w "%{redirect_url}" "$start_url")
  [ -n "$loc" ] || { echo "0 NOSTART"; return; }
  local cb=$(curl -s -b $jar -c $jar -o /dev/null -w "%{redirect_url}" "$loc")
  [ -n "$cb" ] || { echo "0 NOAUTHORIZE"; return; }
  curl -s -b $jar -c $jar -o /tmp/sns_cb.html -w "%{http_code} %{redirect_url}" "$cb"
  echo
}

totp_code() {
  python3 - "$1" <<'PYEOF'
import sys, hmac, hashlib, struct, time, base64
key = base64.b32decode(sys.argv[1].replace(" ", ""))
digest = hmac.new(key, struct.pack(">Q", int(time.time()) // 30), hashlib.sha1).digest()
off = digest[-1] & 0xF
print(f"{(struct.unpack('>I', digest[off:off + 4])[0] & 0x7FFFFFFF) % 1000000:06d}")
PYEOF
}

echo "===== PHASE 0: flag off (production-facing default) ====="
start_app env || { ng "app starts (flag off)"; exit 1; }
curl -s $BASE/login -o /tmp/sns_p0_login.html
grep -q 'sns-button' /tmp/sns_p0_login.html && ng "flag off: no SNS block on /login" || ok "flag off: no SNS block on /login"
curl -s $BASE/signup -o /tmp/sns_p0_signup.html
grep -q 'sns-button' /tmp/sns_p0_signup.html && ng "flag off: no SNS block on /signup" || ok "flag off: no SNS block on /signup"
CODE=$(curl -s -o /dev/null -w '%{http_code}' $BASE/auth/line/start)
[ "$CODE" = "404" ] && ok "flag off: /auth/line/start is 404" || ng "flag off: /auth/line/start is 404 ($CODE)"

echo "===== PHASE 1: flag on, LINE credentials only ====="
start_app env CONFIG_PATH=$SNSCFG PUBLIC_BASE_URL=$BASE \
  LINE_CHANNEL_ID=stub-line-id LINE_CHANNEL_SECRET=stub-line-secret \
  || { ng "app starts (LINE only)"; exit 1; }
curl -s $BASE/login -o /tmp/sns_p1_login.html
grep -q 'LINEでログイン' /tmp/sns_p1_login.html && ok "LINE-only: LINE button renders" || ng "LINE-only: LINE button renders"
grep -q 'Googleでログイン' /tmp/sns_p1_login.html && ng "LINE-only: Google button hidden" || ok "LINE-only: Google button hidden"
grep -q 'SNSログインでは、メールアドレスを取得し' /tmp/sns_p1_login.html && ok "LINE-only: email-use note renders" || ng "LINE-only: email-use note renders"
CODE=$(curl -s -o /dev/null -w '%{http_code}' $BASE/auth/google/start)
[ "$CODE" = "404" ] && ok "LINE-only: /auth/google/start is 404" || ng "LINE-only: /auth/google/start is 404 ($CODE)"
CODE=$(curl -s -o /dev/null -w '%{http_code}' $BASE/auth/yahoo/start)
[ "$CODE" = "404" ] && ok "LINE-only: /auth/yahoo/start is 404" || ng "LINE-only: /auth/yahoo/start is 404 ($CODE)"
LOC=$(curl -s -o /dev/null -w '%{redirect_url}' $BASE/auth/line/start)
case "$LOC" in "https://access.line.me/oauth2/v2.1/authorize"*) ok "LINE-only: LINE start redirects to the production provider";; *) ng "LINE-only: LINE start redirects to the production provider ($LOC)";; esac

echo "===== PHASE 2: all providers against the stub ====="
cleanup_fixtures
start_app env CONFIG_PATH=$SNSCFG PUBLIC_BASE_URL=$BASE \
  LINE_CHANNEL_ID=stub-line-id LINE_CHANNEL_SECRET=stub-line-secret \
  GOOGLE_CLIENT_ID=stub-google-id GOOGLE_CLIENT_SECRET=stub-google-secret \
  YAHOO_CLIENT_ID=stub-yahoo-id YAHOO_CLIENT_SECRET=stub-yahoo-secret \
  SNS_LINE_BASE_URL=$STUB SNS_GOOGLE_BASE_URL=$STUB SNS_YAHOO_BASE_URL=$STUB \
  || { ng "app starts (all providers)"; exit 1; }

create_account $MEMBER "$MPW" && ok "password member fixture created" || { ng "password member fixture created"; exit 1; }
create_account $TOTP_MEMBER "$MPW" && ok "TOTP member fixture created" || { ng "TOTP member fixture created"; exit 1; }

# -- registration happy path (LINE) --------------------------
echo "-- registration: verified email, no existing account"
J=/tmp/sns_reg.txt; rm -f $J
scenario $J reg
R=$(roundtrip $J "$BASE/auth/line/start")
[ "$R" = "303 $BASE/signup/sns/details" ] && ok "registration: callback lands on SNS details" || ng "registration: callback lands on SNS details ($R)"
curl -s $STUB/__last | grep -q 'code_challenge_method.*S256' && ok "authorize request carries PKCE S256" || ng "authorize request carries PKCE S256"
curl -s $STUB/__last | grep -q 'scope.*openid' && ok "authorize request carries the openid scope" || ng "authorize request carries the openid scope"
curl -s -b $J $BASE/signup/sns/details -o /tmp/sns_det.html
grep -q '会員情報の入力' /tmp/sns_det.html && ok "registration: details page renders" || ng "registration: details page renders"
grep -q 'name="password"' /tmp/sns_det.html && ng "registration: no password field in SNS mode" || ok "registration: no password field in SNS mode"
grep -q "value=\"$REG\"" /tmp/sns_det.html && ok "registration: provider email shown read-only" || ng "registration: provider email shown read-only"
T=$(csrf /tmp/sns_det.html)
curl -s -b $J -c $J -X POST $BASE/signup/sns/details -o /tmp/sns_conf.html \
  --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" \
  --data-urlencode "family_name_kana=ケンショウ" --data-urlencode "given_name_kana=タロウ" \
  --data-urlencode "birth_date=" --data-urlencode "gender=" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" \
  --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
  --data-urlencode "$CT=$T"
grep -q 'action="/signup/sns/confirm"' /tmp/sns_conf.html && ok "registration: confirm posts to the SNS endpoint" || ng "registration: confirm posts to the SNS endpoint"
T=$(csrf /tmp/sns_conf.html)
R=$(curl -s -b $J -c $J -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/sns/confirm --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/" ] && ok "registration: confirm logs the member in" || ng "registration: confirm logs the member in ($R)"
[ "$(db "SELECT has_password FROM users WHERE email_bidx = '$(bidx $REG)'")" = "f" ] && ok "registration: account is passwordless" || ng "registration: account is passwordless"
[ "$(db "SELECT count(*) FROM user_identities WHERE provider = 'line' AND provider_user_id = 'stub-sub-reg' AND user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $REG)')")" = "1" ] \
  && ok "registration: identity row linked" || ng "registration: identity row linked"
[ "$(db "SELECT count(*) FROM user_coupons WHERE code = 'WELCOME500' AND user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $REG)')")" = "1" ] \
  && ok "registration: WELCOME500 granted" || ng "registration: WELCOME500 granted"
CODE=$(curl -s -b $J -o /dev/null -w '%{http_code}' $BASE/mypage)
[ "$CODE" = "200" ] && ok "registration: session reaches mypage" || ng "registration: session reaches mypage ($CODE)"

# -- identity sign-in ----------------------------------------
echo "-- sign-in: known identity goes straight in"
J2=/tmp/sns_reg2.txt; rm -f $J2
scenario $J2 reg
R=$(roundtrip $J2 "$BASE/auth/line/start")
[ "$R" = "303 $BASE/" ] && ok "sign-in: identity login completes" || ng "sign-in: identity login completes ($R)"
CODE=$(curl -s -b $J2 -o /dev/null -w '%{http_code}' $BASE/mypage)
[ "$CODE" = "200" ] && ok "sign-in: session reaches mypage" || ng "sign-in: session reaches mypage ($CODE)"

# -- second provider (Google) --------------------------------
echo "-- registration: a second provider (Google)"
J3=/tmp/sns_reg3.txt; rm -f $J3
scenario $J3 reg2
R=$(roundtrip $J3 "$BASE/auth/google/start")
[ "$R" = "303 $BASE/signup/sns/details" ] && ok "google: callback lands on SNS details" || ng "google: callback lands on SNS details ($R)"
curl -s -b $J3 $BASE/signup/sns/details -o /tmp/sns_det3.html; T=$(csrf /tmp/sns_det3.html)
curl -s -b $J3 -c $J3 -X POST $BASE/signup/sns/details -o /tmp/sns_conf3.html \
  --data-urlencode "family_name=検証" --data-urlencode "given_name=花子" \
  --data-urlencode "family_name_kana=ケンショウ" --data-urlencode "given_name_kana=ハナコ" \
  --data-urlencode "birth_date=" --data-urlencode "gender=" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" \
  --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
  --data-urlencode "$CT=$T"
T=$(csrf /tmp/sns_conf3.html)
R=$(curl -s -b $J3 -c $J3 -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/sns/confirm --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/" ] && ok "google: registration completes" || ng "google: registration completes ($R)"
[ "$(db "SELECT count(*) FROM user_identities WHERE provider = 'google' AND provider_user_id = 'stub-sub-reg2'")" = "1" ] \
  && ok "google: identity row linked" || ng "google: identity row linked"

# -- email collision: link-required --------------------------
echo "-- collision: verified email of an existing account"
J4=/tmp/sns_col.txt; rm -f $J4
scenario $J4 collision
LOC=$(curl -s -b $J4 -c $J4 -o /dev/null -w "%{redirect_url}" "$BASE/auth/line/start")
CB=$(curl -s -b $J4 -c $J4 -o /dev/null -w "%{redirect_url}" "$LOC")
CODE=$(curl -s -b $J4 -c $J4 -o /tmp/sns_col.html -w '%{http_code}' "$CB")
[ "$CODE" = "200" ] && grep -q 'パスワードでログインすると連携できます' /tmp/sns_col.html \
  && ok "collision: link-required page renders" || ng "collision: link-required page renders ($CODE)"
CODE=$(curl -s -b $J4 -o /dev/null -w '%{http_code}' $BASE/mypage)
[ "$CODE" = "303" ] && ok "collision: nobody is logged in" || ng "collision: nobody is logged in ($CODE)"
[ "$(db "SELECT count(*) FROM user_identities WHERE provider_user_id = 'stub-sub-collision'")" = "0" ] \
  && ok "collision: nothing was created" || ng "collision: nothing was created"
# The password login stays in the SAME jar: the link-pending
# marker lives in that session, like the real user's browser.
curl -s -b $J4 -c $J4 $BASE/login -o /tmp/sns_login4.html; T=$(csrf /tmp/sns_login4.html)
R=$(curl -s -b $J4 -c $J4 -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/login --data-urlencode "email=$MEMBER" --data-urlencode "$PF=$MPW" --data-urlencode "next=/mypage/sns" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/mypage/sns" ] && ok "collision: password login returns to SNS settings" || ng "collision: password login returns to SNS settings ($R)"
curl -s -b $J4 $BASE/mypage/sns -o /tmp/sns_set4.html
grep -q '連携が完了していません' /tmp/sns_set4.html && ok "collision: link-pending banner shows" || ng "collision: link-pending banner shows"
T=$(csrf /tmp/sns_set4.html)
R=$(curl -s -b $J4 -c $J4 -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/mypage/sns/link-pending --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/mypage/sns" ] && ok "collision: link-pending confirms" || ng "collision: link-pending confirms ($R)"
[ "$(db "SELECT count(*) FROM user_identities WHERE provider = 'line' AND provider_user_id = 'stub-sub-collision' AND user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $MEMBER)')")" = "1" ] \
  && ok "collision: identity now linked to the member" || ng "collision: identity now linked to the member"
logout $J4
J5=/tmp/sns_col2.txt; rm -f $J5
scenario $J5 collision
R=$(roundtrip $J5 "$BASE/auth/line/start")
[ "$R" = "303 $BASE/" ] && ok "collision: after linking, SNS login is direct" || ng "collision: after linking, SNS login is direct ($R)"

# -- unlink (password account) -------------------------------
echo "-- unlink: a password account can unlink"
curl -s -b $J4 $BASE/login -o /dev/null 2>/dev/null
R=$(login_password $J4 $MEMBER "$MPW")
curl -s -b $J4 $BASE/mypage/sns -o /tmp/sns_set4b.html; T=$(csrf /tmp/sns_set4b.html)
R=$(curl -s -b $J4 -c $J4 -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/mypage/sns/line/unlink --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/mypage/sns" ] && ok "unlink: posts back to settings" || ng "unlink: posts back to settings ($R)"
[ "$(db "SELECT count(*) FROM user_identities WHERE provider_user_id = 'stub-sub-collision'")" = "0" ] \
  && ok "unlink: identity removed" || ng "unlink: identity removed"

# -- sole-method guard + password set ------------------------
echo "-- sole-method guard, then the password set flow"
# $J still holds the SNS-registered (passwordless) session.
curl -s -b $J $BASE/mypage/sns -o /tmp/sns_set1.html; T=$(csrf /tmp/sns_set1.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/mypage/sns/line/unlink --data-urlencode "$CT=$T"
curl -s -b $J $BASE/mypage/sns -o /tmp/sns_set1b.html
grep -q 'パスワードを設定すると解除できます' /tmp/sns_set1b.html && ok "guard: sole-method unlink refused with the reason" || ng "guard: sole-method unlink refused with the reason"
[ "$(db "SELECT count(*) FROM user_identities WHERE provider_user_id = 'stub-sub-reg'")" = "1" ] \
  && ok "guard: identity still linked" || ng "guard: identity still linked"
curl -s -b $J $BASE/mypage/password -o /tmp/sns_pw.html
grep -q 'パスワードの設定' /tmp/sns_pw.html && ok "password set: set-mode heading renders" || ng "password set: set-mode heading renders"
grep -q 'name="current_password"' /tmp/sns_pw.html && ng "password set: no current-password field" || ok "password set: no current-password field"
T=$(csrf /tmp/sns_pw.html)
curl -s -b $J -c $J -X POST $BASE/mypage/password -o /tmp/sns_pw2.html \
  --data-urlencode "new_password=$NPW" --data-urlencode "new_password_confirm=$NPW" --data-urlencode "$CT=$T"
grep -q 'パスワードを設定しました' /tmp/sns_pw2.html && ok "password set: success notice" || ng "password set: success notice"
[ "$(db "SELECT has_password FROM users WHERE email_bidx = '$(bidx $REG)'")" = "t" ] && ok "password set: flag flipped" || ng "password set: flag flipped"
logout $J
R=$(login_password /tmp/sns_regpw.txt $REG "$NPW")
[ "$R" = "303 $BASE/" ] && ok "password set: password login now works" || ng "password set: password login now works ($R)"
# With a password, unlinking the only identity is allowed.
J6=/tmp/sns_regpw.txt
curl -s -b $J6 $BASE/mypage/sns -o /tmp/sns_set6.html; T=$(csrf /tmp/sns_set6.html)
curl -s -b $J6 -c $J6 -o /dev/null -X POST $BASE/mypage/sns/line/unlink --data-urlencode "$CT=$T"
[ "$(db "SELECT count(*) FROM user_identities WHERE provider_user_id = 'stub-sub-reg'")" = "0" ] \
  && ok "password set: unlink allowed afterwards" || ng "password set: unlink allowed afterwards"

# -- link from My Page + the two-factor gate -----------------
echo "-- link from My Page, then SNS login meets the TOTP challenge"
JT=/tmp/sns_totp.txt
R=$(login_password $JT $TOTP_MEMBER "$MPW")
curl -s -b $JT $BASE/mypage/totp -o /tmp/sns_totp_pre.html; T=$(csrf /tmp/sns_totp_pre.html)
curl -s -b $JT -c $JT -X POST $BASE/mypage/totp/setup --data-urlencode "$CT=$T" -o /tmp/sns_totp_setup.html
TOTP_SECRET=$(grep -o 'totp-secret">[^<]*' /tmp/sns_totp_setup.html | sed 's/.*>//; s/ //g')
[ -n "$TOTP_SECRET" ] && ok "totp: enrolment secret issued" || ng "totp: enrolment secret issued"
T=$(csrf /tmp/sns_totp_setup.html)
curl -s -b $JT -c $JT -X POST $BASE/mypage/totp/enable --data-urlencode "code=$(totp_code "$TOTP_SECRET")" --data-urlencode "$CT=$T" -o /tmp/sns_totp_codes.html
grep -qE '[0-9A-F]{5}-[0-9A-F]{5}' /tmp/sns_totp_codes.html && ok "totp: enrolment completes" || ng "totp: enrolment completes"
scenario $JT totplink
R=$(roundtrip $JT "$BASE/auth/line/start?link=1&next=/mypage/sns")
[ "$R" = "303 $BASE/mypage/sns" ] && ok "link: My Page link flow returns to settings" || ng "link: My Page link flow returns to settings ($R)"
curl -s -b $JT $BASE/mypage/sns -o /tmp/sns_setT.html
grep -q '連携済み' /tmp/sns_setT.html && ok "link: settings show the link" || ng "link: settings show the link"
[ "$(db "SELECT count(*) FROM user_identities WHERE provider_user_id = 'stub-sub-totp' AND user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $TOTP_MEMBER)')")" = "1" ] \
  && ok "link: identity row inserted" || ng "link: identity row inserted"
logout $JT
JT2=/tmp/sns_totp2.txt; rm -f $JT2
scenario $JT2 totplink
R=$(roundtrip $JT2 "$BASE/auth/line/start")
[ "$R" = "303 $BASE/login/totp" ] && ok "totp gate: SNS login parks the challenge" || ng "totp gate: SNS login parks the challenge ($R)"
CODE=$(curl -s -b $JT2 -o /dev/null -w '%{http_code}' $BASE/mypage)
[ "$CODE" = "303" ] && ok "totp gate: no session before the challenge" || ng "totp gate: no session before the challenge ($CODE)"
curl -s -b $JT2 $BASE/login/totp -o /tmp/sns_chal.html; T=$(csrf /tmp/sns_chal.html)
R=$(curl -s -b $JT2 -c $JT2 -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/login/totp --data-urlencode "code=$(totp_code "$TOTP_SECRET")" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/" ] && ok "totp gate: challenge completes the SNS login" || ng "totp gate: challenge completes the SNS login ($R)"
CODE=$(curl -s -b $JT2 -o /dev/null -w '%{http_code}' $BASE/mypage)
[ "$CODE" = "200" ] && ok "totp gate: session reaches mypage" || ng "totp gate: session reaches mypage ($CODE)"

# -- missing email: the code branch --------------------------
echo "-- missing email: code verification, then registration"
JN=/tmp/sns_noemail.txt; rm -f $JN
scenario $JN noemail
R=$(roundtrip $JN "$BASE/auth/line/start")
[ "$R" = "303 $BASE/signup/sns/email" ] && ok "noemail: callback asks for an email" || ng "noemail: callback asks for an email ($R)"
curl -s -b $JN $BASE/signup/sns/email -o /tmp/sns_em.html
grep -q 'メールアドレスの入力' /tmp/sns_em.html && ok "noemail: email page renders" || ng "noemail: email page renders"
clearmail
T=$(csrf /tmp/sns_em.html)
R=$(curl -s -b $JN -c $JN -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/sns/email --data-urlencode "email=$NOEMAIL" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/signup/verify" ] && ok "noemail: code sent via the shared machinery" || ng "noemail: code sent via the shared machinery ($R)"
VCODE=$(mailcode $NOEMAIL)
[ -n "$VCODE" ] && ok "noemail: code arrived in Mailpit" || ng "noemail: code arrived in Mailpit"
curl -s -b $JN $BASE/signup/verify -o /tmp/sns_vf2.html; T=$(csrf /tmp/sns_vf2.html)
R=$(curl -s -b $JN -c $JN -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/verify --data-urlencode "code=$VCODE" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/signup/sns/details" ] && ok "noemail: verify returns to the SNS details step" || ng "noemail: verify returns to the SNS details step ($R)"
curl -s -b $JN $BASE/signup/sns/details -o /tmp/sns_detN.html; T=$(csrf /tmp/sns_detN.html)
curl -s -b $JN -c $JN -X POST $BASE/signup/sns/details -o /tmp/sns_confN.html \
  --data-urlencode "family_name=検証" --data-urlencode "given_name=三郎" \
  --data-urlencode "family_name_kana=ケンショウ" --data-urlencode "given_name_kana=サブロウ" \
  --data-urlencode "birth_date=" --data-urlencode "gender=" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" \
  --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
  --data-urlencode "$CT=$T"
T=$(csrf /tmp/sns_confN.html)
R=$(curl -s -b $JN -c $JN -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/sns/confirm --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/" ] && ok "noemail: registration completes" || ng "noemail: registration completes ($R)"
[ "$(db "SELECT count(*) FROM user_identities WHERE provider = 'line' AND provider_user_id = 'stub-sub-noemail' AND provider_email = '$NOEMAIL'")" = "1" ] \
  && ok "noemail: identity linked with the verified email" || ng "noemail: identity linked with the verified email"

# -- unverified email: also the code branch ------------------
echo "-- unverified email: treated as no usable email"
JU=/tmp/sns_unv.txt; rm -f $JU
scenario $JU unvemail
R=$(roundtrip $JU "$BASE/auth/line/start")
[ "$R" = "303 $BASE/signup/sns/email" ] && ok "unvemail: callback asks for an email" || ng "unvemail: callback asks for an email ($R)"
curl -s -b $JU $BASE/signup/sns/email -o /tmp/sns_emU.html
grep -q 'value="sns-unv@example.com"' /tmp/sns_emU.html && ok "unvemail: claimed email prefilled" || ng "unvemail: claimed email prefilled"

# -- withdrawn identity: refused ------------------------------
echo "-- withdrawn account: identity sign-in refused"
db "INSERT INTO users (email, email_bidx, password_hash, family_name, given_name, family_name_kana, given_name_kana, phone, postal_code, prefecture, city, address_line, mail_magazine, withdrawn_at) VALUES ('$DEAD', '$(bidx $DEAD)', 'x', '退会', '済', 'タイカイ', 'スミ', '09012345678', '1500001', '東京都', '渋谷区', '1-1-1', false, now())" >/dev/null
DEAD_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx $DEAD)'")
db "INSERT INTO user_identities (user_id, provider, provider_user_id, provider_email) VALUES ($DEAD_ID, 'line', 'stub-sub-dead', '$DEAD')" >/dev/null
JD=/tmp/sns_dead.txt; rm -f $JD
scenario $JD dead
R=$(roundtrip $JD "$BASE/auth/line/start")
[ "$R" = "303 $BASE/login" ] && ok "withdrawn: sign-in refused to the login page" || ng "withdrawn: sign-in refused to the login page ($R)"
CODE=$(curl -s -b $JD -o /dev/null -w '%{http_code}' $BASE/mypage)
[ "$CODE" = "303" ] && ok "withdrawn: no session" || ng "withdrawn: no session ($CODE)"

# -- bad state: refused generically ---------------------------
echo "-- bad state: generic failure, no session"
JB=/tmp/sns_bad.txt; rm -f $JB
scenario $JB reg
LOC=$(curl -s -b $JB -c $JB -o /dev/null -w "%{redirect_url}" "$BASE/auth/line/start")
CB=$(curl -s -b $JB -c $JB -o /dev/null -w "%{redirect_url}" "$LOC")
BADCB=$(echo "$CB" | sed 's/state=[^&]*/state=FORGED/')
R=$(curl -s -b $JB -c $JB -o /dev/null -w "%{http_code} %{redirect_url}" "$BADCB")
[ "$R" = "303 $BASE/login" ] && ok "bad state: refused to the login page" || ng "bad state: refused to the login page ($R)"
curl -s -b $JB $BASE/login -o /tmp/sns_badlogin.html
grep -q 'SNSログインに失敗しました' /tmp/sns_badlogin.html && ok "bad state: generic failure flash" || ng "bad state: generic failure flash"
CODE=$(curl -s -b $JB -o /dev/null -w '%{http_code}' $BASE/mypage)
[ "$CODE" = "303" ] && ok "bad state: no session" || ng "bad state: no session ($CODE)"

# -- hub row ---------------------------------------------------
echo "-- hub: the SNS row is on the account settings group"
curl -s -b $JT2 $BASE/mypage -o /tmp/sns_hub.html
grep -q 'href="/mypage/sns"' /tmp/sns_hub.html && grep -q 'SNS連携' /tmp/sns_hub.html \
  && ok "hub: SNS連携 row renders" || ng "hub: SNS連携 row renders"

# -- withdrawal by email code (passwordless account) ---------
echo "-- withdrawal: the email code proof replaces the password"
# $JN is the passwordless noemail account, still logged in.
NOEMAIL_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx $NOEMAIL)'")
curl -s -b $JN $BASE/mypage/withdraw -o /tmp/sns_wd.html
grep -q '認証コードを送信する' /tmp/sns_wd.html && ok "withdraw: code-proof ceremony renders" || ng "withdraw: code-proof ceremony renders"
grep -q 'name="password"' /tmp/sns_wd.html && ng "withdraw: no password field for passwordless accounts" || ok "withdraw: no password field for passwordless accounts"
clearmail
T=$(csrf /tmp/sns_wd.html)
R=$(curl -s -b $JN -c $JN -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/mypage/withdraw --data-urlencode "confirm=true" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/mypage/withdraw/verify" ] && ok "withdraw: first POST starts the code challenge" || ng "withdraw: first POST starts the code challenge ($R)"
WCODE=$(mailcode $NOEMAIL)
[ -n "$WCODE" ] && ok "withdraw: code arrived in Mailpit" || ng "withdraw: code arrived in Mailpit"
curl -s -b $JN $BASE/mypage/withdraw/verify -o /tmp/sns_wdv.html; T=$(csrf /tmp/sns_wdv.html)
R=$(curl -s -b $JN -c $JN -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/mypage/withdraw/verify --data-urlencode "code=$WCODE" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/mypage/withdraw" ] && ok "withdraw: verified code returns to the ceremony" || ng "withdraw: verified code returns to the ceremony ($R)"
curl -s -b $JN $BASE/mypage/withdraw -o /tmp/sns_wd2.html
grep -q '本人確認が完了しました' /tmp/sns_wd2.html && ok "withdraw: ceremony shows the verified state" || ng "withdraw: ceremony shows the verified state"
T=$(csrf /tmp/sns_wd2.html)
R=$(curl -s -b $JN -c $JN -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/mypage/withdraw --data-urlencode "confirm=true" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/withdrawn" ] && ok "withdraw: ceremony completes" || ng "withdraw: ceremony completes ($R)"
[ -n "$(db "SELECT withdrawn_at FROM users WHERE id = $NOEMAIL_ID")" ] && [ "$(db "SELECT email_bidx FROM users WHERE id = $NOEMAIL_ID")" = "$(bidx "deleted-$NOEMAIL_ID@invalid.example")" ] && ok "withdraw: account is a tombstone" || ng "withdraw: account is a tombstone"
[ "$(db "SELECT count(*) FROM user_identities WHERE provider_user_id = 'stub-sub-noemail'")" = "0" ] \
  && ok "withdraw: identities scrubbed" || ng "withdraw: identities scrubbed"

echo "===== TEARDOWN: fixtures out, normal app restored ====="
cleanup_fixtures
fuser -k 18098/tcp 2>/dev/null
start_app env && ok "app restored with the normal config" || ng "app restored with the normal config"
result
