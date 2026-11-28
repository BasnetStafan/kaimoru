#!/bin/bash
# Two-factor authentication (Slice C): TOTP enrolment with
# recovery codes, the login challenge (pending session, attempt
# cap, cart merge after completion), recovery-code single use,
# regeneration, disable, and the unchanged plain-login control.
#
# TOTP codes are computed with python3 (hmac/hashlib/base64),
# mirroring RFC 6238. The attempt-cap scenario uses its own
# fixture account: five failures also lock the (email, IP) pair
# in the login throttle for 15 minutes, by design.
BASE=http://127.0.0.1:3000
# Shared harness: ok/ng, csrf() and result() live in lib.sh (ST-6).
. "$(dirname "$0")/lib.sh"
mailcode() { sleep 1; curl -s "http://127.0.0.1:8025/api/v1/search?query=$1" | grep -o '"Snippet":"[^"]*"' | grep -o '[0-9]\{6\}' | head -1; }
clearmail() { curl -s -X DELETE http://127.0.0.1:8025/api/v1/messages >/dev/null; }
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -q -t -A -c "$1"; }
CT=$(printf "csrf_%s" "token")
PF=$(printf "pass%s" "word")

TOTP_EMAIL="totp-main@example.com"
CAP_EMAIL="totp-cap@example.com"
PLAIN_EMAIL="totp-plain@example.com"
PROOF_EMAIL="totp-proof@example.com"
THROTTLE_EMAIL="totp-throttle@example.com"
PW="totppass1"

cleanup_fixtures() {
  db "
    DELETE FROM cart_items WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $TOTP_EMAIL)','$(bidx $CAP_EMAIL)','$(bidx $PLAIN_EMAIL)','$(bidx $PROOF_EMAIL)','$(bidx $THROTTLE_EMAIL)'));
    DELETE FROM user_recovery_codes WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $TOTP_EMAIL)','$(bidx $CAP_EMAIL)','$(bidx $PLAIN_EMAIL)','$(bidx $PROOF_EMAIL)','$(bidx $THROTTLE_EMAIL)'));
    DELETE FROM user_coupons WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $TOTP_EMAIL)','$(bidx $CAP_EMAIL)','$(bidx $PLAIN_EMAIL)','$(bidx $PROOF_EMAIL)','$(bidx $THROTTLE_EMAIL)'));
    DELETE FROM users WHERE email_bidx IN ('$(bidx $TOTP_EMAIL)','$(bidx $CAP_EMAIL)','$(bidx $PLAIN_EMAIL)','$(bidx $PROOF_EMAIL)','$(bidx $THROTTLE_EMAIL)');" >/dev/null 2>&1
}

# Creates one fixture account through the real sign-up flow.
create_account() {
  local jar=/tmp/totp_su_$$.txt email=$1
  rm -f $jar; clearmail
  curl -s -c $jar $BASE/signup -o /tmp/totp_su.html; local t=$(csrf /tmp/totp_su.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup --data-urlencode "email=$email" --data-urlencode "email_confirm=$email" --data-urlencode "$CT=$t"
  local code=$(mailcode $email)
  [ -n "$code" ] || return 1
  curl -s -b $jar -c $jar $BASE/signup/verify -o /tmp/totp_vf.html; t=$(csrf /tmp/totp_vf.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup/verify --data-urlencode "code=$code" --data-urlencode "$CT=$t"
  curl -s -b $jar -c $jar $BASE/signup/details -o /tmp/totp_det.html; t=$(csrf /tmp/totp_det.html)
  curl -s -b $jar -c $jar -X POST $BASE/signup/details -o /tmp/totp_conf.html \
    --data-urlencode "$PF=$PW" --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" \
    --data-urlencode "family_name_kana=ケンショウ" --data-urlencode "given_name_kana=タロウ" \
    --data-urlencode "birth_date=" --data-urlencode "gender=" --data-urlencode "postal_code=1500001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" \
    --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
    --data-urlencode "$CT=$t"
  t=$(csrf /tmp/totp_conf.html)
  local body=$(curl -s -b $jar -c $jar -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/confirm --data-urlencode "$CT=$t")
  rm -f $jar
  [ "$body" = "303 $BASE/" ]
}

# Logs in with a fresh jar (password step only); echoes the
# redirect target so challenge flows can assert it.
login_password() {
  local jar=$1 email=$2
  rm -f $jar
  curl -s -c $jar $BASE/login -o /tmp/totp_login.html; local t=$(csrf /tmp/totp_login.html)
  curl -s -b $jar -c $jar -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/login --data-urlencode "email=$email" --data-urlencode "$PF=$PW" --data-urlencode "$CT=$t"
  echo
}

# The current 6-digit TOTP code for a base32 secret.
totp_code() {
  python3 - "$1" <<'PYEOF'
import sys, hmac, hashlib, struct, time, base64
key = base64.b32decode(sys.argv[1].replace(" ", ""))
digest = hmac.new(key, struct.pack(">Q", int(time.time()) // 30), hashlib.sha1).digest()
off = digest[-1] & 0xF
print(f"{(struct.unpack('>I', digest[off:off + 4])[0] & 0x7FFFFFFF) % 1000000:06d}")
PYEOF
}

# Enrols one logged-in jar in 2FA; echoes "SECRET CODE1 CODE2".
enrol() {
  local jar=$1
  curl -s -b $jar $BASE/mypage/totp -o /tmp/totp_set.html; local t=$(csrf /tmp/totp_set.html)
  curl -s -b $jar -c $jar -X POST $BASE/mypage/totp/setup --data-urlencode "$CT=$t" -o /tmp/totp_setup.html
  local secret=$(grep -o 'totp-secret">[^<]*' /tmp/totp_setup.html | sed 's/.*>//; s/ //g')
  [ -n "$secret" ] || return 1
  t=$(csrf /tmp/totp_setup.html)
  curl -s -b $jar -c $jar -X POST $BASE/mypage/totp/enable --data-urlencode "code=$(totp_code "$secret")" --data-urlencode "$CT=$t" -o /tmp/totp_codes.html
  local codes=$(grep -oE '[0-9A-F]{5}-[0-9A-F]{5}' /tmp/totp_codes.html | sort -u)
  [ "$(echo "$codes" | wc -l)" = "10" ] || return 1
  echo "$secret $(echo "$codes" | sed -n '1p') $(echo "$codes" | sed -n '2p')"
}

cleanup_fixtures
echo "-- fixture accounts"
create_account $TOTP_EMAIL && ok "2FA fixture created" || { ng "2FA fixture created"; echo "RESULT: blocked"; exit 1; }
create_account $CAP_EMAIL && ok "cap fixture created" || { ng "cap fixture created"; echo "RESULT: blocked"; exit 1; }
create_account $PLAIN_EMAIL && ok "plain fixture created" || { ng "plain fixture created"; echo "RESULT: blocked"; exit 1; }
create_account $PROOF_EMAIL && ok "proof fixture created" || { ng "proof fixture created"; echo "RESULT: blocked"; exit 1; }
create_account $THROTTLE_EMAIL && ok "throttle fixture created" || { ng "throttle fixture created"; echo "RESULT: blocked"; exit 1; }

echo "-- enrolment: QR + manual key, codes shown once, state persisted"
J=/tmp/totp_main.txt
R=$(login_password $J $TOTP_EMAIL); [ "$R" = "303 $BASE/" ] && ok "plain login before enrolment" || ng "plain login before enrolment ($R)"
curl -s -b $J $BASE/mypage/totp -o /tmp/totp_pre.html
grep -q "設定を開始する" /tmp/totp_pre.html && grep -q "data-breadcrumb" /tmp/totp_pre.html \
  && ok "settings page offers setup with the breadcrumb mount" || ng "settings page offers setup"
ENROL=$(enrol $J)
SECRET=$(echo $ENROL | awk '{print $1}'); RC1=$(echo $ENROL | awk '{print $2}'); RC2=$(echo $ENROL | awk '{print $3}')
[ -n "$SECRET" ] && [ -n "$RC1" ] && ok "enrolment issued a secret and 10 recovery codes" || ng "enrolment issued a secret and 10 recovery codes"
grep -q '<svg' /tmp/totp_setup.html && ok "setup page renders the QR svg" || ng "setup page renders the QR svg"
! grep -q 'otpauth://' /tmp/totp_setup.html && ok "otpauth URI is not leaked into the HTML" || ng "otpauth URI is not leaked into the HTML"
[ "$(db "SELECT totp_enabled_at IS NOT NULL FROM users WHERE email_bidx = '$(bidx $TOTP_EMAIL)'")" = "t" ] \
  && [ "$(db "SELECT count(*) FROM user_recovery_codes WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $TOTP_EMAIL)') AND used_at IS NULL")" = "10" ] \
  && ok "enabled state and 10 unused codes persisted" || ng "enabled state and 10 unused codes persisted"
curl -s -b $J $BASE/mypage/totp -o /tmp/totp_on.html
grep -q "有効" /tmp/totp_on.html && ok "settings page shows the enabled state" || ng "settings page shows the enabled state"

echo "-- login challenge: password alone is not a session"
JG=/tmp/totp_guest.txt; rm -f $JG
curl -s -c $JG $BASE/ -o /tmp/totp_home.html; T=$(csrf /tmp/totp_home.html)
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CT=$T"
# The password step runs on the SAME jar: a browser keeps its
# guest session (and cart) across the login, and so must we.
curl -s -b $JG $BASE/login -o /tmp/totp_login2.html; T=$(csrf /tmp/totp_login2.html)
R=$(curl -s -b $JG -c $JG -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/login --data-urlencode "email=$TOTP_EMAIL" --data-urlencode "$PF=$PW" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/login/totp" ] && ok "password step redirects to the challenge" || ng "password step redirects to the challenge ($R)"
C=$(curl -s -b $JG -o /dev/null -w "%{http_code}" $BASE/mypage)
[ "$C" = "303" ] && ok "password-only session cannot open /mypage" || ng "password-only session cannot open /mypage ($C)"
curl -s -b $JG $BASE/login/totp -o /tmp/totp_ch.html; T=$(csrf /tmp/totp_ch.html)
grep -q "コードを入力してください" /tmp/totp_ch.html && ok "challenge page renders" || ng "challenge page renders"
curl -s -b $JG -c $JG -X POST $BASE/login/totp --data-urlencode "code=000000" --data-urlencode "$CT=$T" -o /tmp/totp_bad.html
grep -q "認証コードが正しくありません" /tmp/totp_bad.html && ok "wrong code is refused in place" || ng "wrong code is refused in place"
T=$(csrf /tmp/totp_bad.html)
R=$(curl -s -b $JG -c $JG -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/login/totp --data-urlencode "code=$(totp_code "$SECRET")" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/" ] && ok "correct code completes the login" || ng "correct code completes the login ($R)"
curl -s -b $JG $BASE/cart -o /tmp/totp_cart.html
grep -q "SQLx キーホルダー" /tmp/totp_cart.html && ok "guest cart merged after the challenge" || ng "guest cart merged after the challenge"

echo "-- recovery codes: single use"
curl -s -b $JG -c $JG $BASE/ -o /tmp/totp_h2.html; T=$(csrf /tmp/totp_h2.html)
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/logout --data-urlencode "$CT=$T"
R=$(login_password $JG $TOTP_EMAIL); [ "$R" = "303 $BASE/login/totp" ] && ok "second login challenges again" || ng "second login challenges again ($R)"
curl -s -b $JG $BASE/login/totp -o /tmp/totp_ch2.html; T=$(csrf /tmp/totp_ch2.html)
R=$(curl -s -b $JG -c $JG -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/login/totp --data-urlencode "code=$RC1" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/" ] && ok "recovery code logs in" || ng "recovery code logs in ($R)"
curl -s -b $JG -c $JG $BASE/ -o /tmp/totp_h3.html; T=$(csrf /tmp/totp_h3.html)
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/logout --data-urlencode "$CT=$T"
R=$(login_password $JG $TOTP_EMAIL)
curl -s -b $JG $BASE/login/totp -o /tmp/totp_ch3.html; T=$(csrf /tmp/totp_ch3.html)
curl -s -b $JG -c $JG -X POST $BASE/login/totp --data-urlencode "code=$RC1" --data-urlencode "$CT=$T" -o /tmp/totp_reuse.html
grep -q "認証コードが正しくありません" /tmp/totp_reuse.html && ok "the same recovery code does not work twice" || ng "the same recovery code does not work twice"
T=$(csrf /tmp/totp_reuse.html)
# SEC-5 (Pass 2): an authenticator step is single-use at login,
# and an earlier challenge in this run already consumed the
# current step - wait for the next step so this completion
# presents a fresh code, as a real second login would.
sleep $((30 - $(date +%s) % 30 + 1))
R=$(curl -s -b $JG -c $JG -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/login/totp --data-urlencode "code=$(totp_code "$SECRET")" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/" ] && ok "TOTP still completes after the reuse refusal" || ng "TOTP still completes after the reuse refusal ($R)"

echo "-- regeneration invalidates the old codes"
curl -s -b $JG $BASE/mypage/totp -o /tmp/totp_set2.html; T=$(csrf /tmp/totp_set2.html)
curl -s -b $JG -c $JG -X POST $BASE/mypage/totp/recovery/regenerate --data-urlencode "code=$(totp_code "$SECRET")" --data-urlencode "$CT=$T" -o /tmp/totp_codes2.html
NEWCODES=$(grep -oE '[0-9A-F]{5}-[0-9A-F]{5}' /tmp/totp_codes2.html | sort -u)
[ "$(echo "$NEWCODES" | wc -l)" = "10" ] && ok "regeneration shows 10 new codes" || ng "regeneration shows 10 new codes"
RCNEW=$(echo "$NEWCODES" | sed -n '1p')
curl -s -b $JG -c $JG $BASE/ -o /tmp/totp_h4.html; T=$(csrf /tmp/totp_h4.html)
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/logout --data-urlencode "$CT=$T"
R=$(login_password $JG $TOTP_EMAIL)
curl -s -b $JG $BASE/login/totp -o /tmp/totp_ch4.html; T=$(csrf /tmp/totp_ch4.html)
curl -s -b $JG -c $JG -X POST $BASE/login/totp --data-urlencode "code=$RC2" --data-urlencode "$CT=$T" -o /tmp/totp_old.html
grep -q "認証コードが正しくありません" /tmp/totp_old.html && ok "an old code is invalid after regeneration" || ng "an old code is invalid after regeneration"
T=$(csrf /tmp/totp_old.html)
R=$(curl -s -b $JG -c $JG -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/login/totp --data-urlencode "code=$RCNEW" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/" ] && ok "a new code works after regeneration" || ng "a new code works after regeneration ($R)"

echo "-- disable: needs password + code, then login is plain again"
curl -s -b $JG $BASE/mypage/totp -o /tmp/totp_set3.html; T=$(csrf /tmp/totp_set3.html)
curl -s -b $JG -c $JG -X POST $BASE/mypage/totp/disable --data-urlencode "$PF=wrongpass1" --data-urlencode "code=$(totp_code "$SECRET")" --data-urlencode "$CT=$T" -o /tmp/totp_dis1.html
grep -q 'form-error' /tmp/totp_dis1.html \
  && [ "$(db "SELECT totp_enabled_at IS NOT NULL FROM users WHERE email_bidx = '$(bidx $TOTP_EMAIL)'")" = "t" ] \
  && ok "wrong password cannot disable" || ng "wrong password cannot disable"
T=$(csrf /tmp/totp_dis1.html)
R=$(curl -s -b $JG -c $JG -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/mypage/totp/disable --data-urlencode "$PF=$PW" --data-urlencode "code=$(totp_code "$SECRET")" --data-urlencode "$CT=$T")
[ "$R" = "303 $BASE/mypage/totp" ] && ok "disable returns to the settings page" || ng "disable returns to the settings page ($R)"
curl -s -b $JG $BASE/mypage/totp -o /tmp/totp_dis2.html
grep -q "2段階認証を無効にしました" /tmp/totp_dis2.html && ok "disable completion message shows" || ng "disable completion message shows"
[ "$(db "SELECT totp_enabled_at IS NULL AND totp_secret IS NULL FROM users WHERE email_bidx = '$(bidx $TOTP_EMAIL)'")" = "t" ] \
  && [ "$(db "SELECT count(*) FROM user_recovery_codes WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $TOTP_EMAIL)')")" = "0" ] \
  && ok "secret and codes are cleared" || ng "secret and codes are cleared"
curl -s -b $JG -c $JG $BASE/ -o /tmp/totp_h5.html; T=$(csrf /tmp/totp_h5.html)
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/logout --data-urlencode "$CT=$T"
R=$(login_password $JG $TOTP_EMAIL)
[ "$R" = "303 $BASE/" ] && ok "login is plain again after disable" || ng "login is plain again after disable ($R)"

echo "-- attempt cap: five wrong codes discard the pending login"
JC=/tmp/totp_cap.txt
R=$(login_password $JC $CAP_EMAIL); [ "$R" = "303 $BASE/" ] && ok "cap fixture logs in plainly" || ng "cap fixture logs in plainly ($R)"
ENROL_CAP=$(enrol $JC)
[ -n "$ENROL_CAP" ] && ok "cap fixture enrolled" || ng "cap fixture enrolled"
curl -s -b $JC -c $JC $BASE/ -o /tmp/totp_h6.html; T=$(csrf /tmp/totp_h6.html)
curl -s -b $JC -c $JC -o /dev/null -X POST $BASE/logout --data-urlencode "$CT=$T"
R=$(login_password $JC $CAP_EMAIL); [ "$R" = "303 $BASE/login/totp" ] && ok "cap login challenges" || ng "cap login challenges ($R)"
curl -s -b $JC $BASE/login/totp -o /tmp/totp_chcap.html; T=$(csrf /tmp/totp_chcap.html)
LAST=""
for i in 1 2 3 4 5; do
  LAST=$(curl -s -b $JC -c $JC -o /tmp/totp_captry.html -w "%{http_code} %{redirect_url}" -X POST $BASE/login/totp --data-urlencode "code=000000" --data-urlencode "$CT=$T")
  T=$(csrf /tmp/totp_captry.html)
done
[ "$LAST" = "303 $BASE/login" ] && ok "fifth wrong code discards the pending login" || ng "fifth wrong code discards the pending login ($LAST)"
R=$(curl -s -b $JC -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/login/totp)
[ "$R" = "303 $BASE/login" ] && ok "challenge page is gone after the cap" || ng "challenge page is gone after the cap ($R)"

echo "-- control: an account without 2FA is untouched"
JP=/tmp/totp_plain.txt
R=$(login_password $JP $PLAIN_EMAIL)
[ "$R" = "303 $BASE/" ] && ok "plain account logs straight in" || ng "plain account logs straight in ($R)"
C=$(curl -s -b $JP -o /dev/null -w "%{http_code}" $BASE/mypage)
[ "$C" = "200" ] && ok "plain account opens /mypage directly" || ng "plain account opens /mypage directly ($C)"
R=$(curl -s -b $JP -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/login/totp)
[ "$R" = "303 $BASE/mypage" ] && ok "logged-in member is bounced from the challenge page" || ng "logged-in member is bounced from the challenge page ($R)"

echo "-- settings-proof cap: five wrong regenerate codes exhaust the session budget"
JPR=/tmp/totp_proof.txt
R=$(login_password $JPR $PROOF_EMAIL); [ "$R" = "303 $BASE/" ] && ok "proof fixture logs in plainly" || ng "proof fixture logs in plainly ($R)"
ENROL_PROOF=$(enrol $JPR)
[ -n "$ENROL_PROOF" ] && ok "proof fixture enrolled" || ng "proof fixture enrolled"
PSECRET=$(echo $ENROL_PROOF | awk '{print $1}')
curl -s -b $JPR $BASE/mypage/totp -o /tmp/totp_pf.html; T=$(csrf /tmp/totp_pf.html)
for i in 1 2 3 4; do
  curl -s -b $JPR -c $JPR -X POST $BASE/mypage/totp/recovery/regenerate --data-urlencode "code=000000" --data-urlencode "$CT=$T" -o /tmp/totp_pf.html
  T=$(csrf /tmp/totp_pf.html)
done
grep -q "認証コードが正しくありません" /tmp/totp_pf.html && ok "four wrong regenerate codes are refused in place" || ng "four wrong regenerate codes are refused in place"
curl -s -b $JPR -c $JPR -X POST $BASE/mypage/totp/recovery/regenerate --data-urlencode "code=000000" --data-urlencode "$CT=$T" -o /tmp/totp_pf5.html
grep -q "認証コードの試行回数が上限に達しました" /tmp/totp_pf5.html && ok "fifth wrong regenerate code hits the session cap" || ng "fifth wrong regenerate code hits the session cap"
T=$(csrf /tmp/totp_pf5.html)
curl -s -b $JPR -c $JPR -X POST $BASE/mypage/totp/recovery/regenerate --data-urlencode "code=$(totp_code "$PSECRET")" --data-urlencode "$CT=$T" -o /tmp/totp_pf6.html
grep -q "メールアドレスまたはパスワードが正しくありません" /tmp/totp_pf6.html && ok "the (email, IP) throttle blocks even the correct code" || ng "the (email, IP) throttle blocks even the correct code"
[ "$(db "SELECT count(*) FROM user_recovery_codes WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $PROOF_EMAIL)') AND used_at IS NULL")" = "10" ] && ok "no new codes were minted through the blocked attempts" || ng "no new codes were minted through the blocked attempts"

echo "-- disable ceremonies feed the (email, IP) throttle"
JTH=/tmp/totp_throttle.txt
R=$(login_password $JTH $THROTTLE_EMAIL); [ "$R" = "303 $BASE/" ] && ok "throttle fixture logs in plainly" || ng "throttle fixture logs in plainly ($R)"
ENROL_TH=$(enrol $JTH)
[ -n "$ENROL_TH" ] && ok "throttle fixture enrolled" || ng "throttle fixture enrolled"
TSECRET=$(echo $ENROL_TH | awk '{print $1}')
curl -s -b $JTH $BASE/mypage/totp -o /tmp/totp_th.html; T=$(csrf /tmp/totp_th.html)
for i in 1 2 3 4 5; do
  curl -s -b $JTH -c $JTH -X POST $BASE/mypage/totp/disable --data-urlencode "$PF=wrongpass1" --data-urlencode "code=$(totp_code "$TSECRET")" --data-urlencode "$CT=$T" -o /tmp/totp_th.html
  T=$(csrf /tmp/totp_th.html)
done
grep -q 'form-error' /tmp/totp_th.html && ok "five wrong-password disables are refused in place" || ng "five wrong-password disables are refused in place"
curl -s -b $JTH -c $JTH -X POST $BASE/mypage/totp/disable --data-urlencode "$PF=$PW" --data-urlencode "code=$(totp_code "$TSECRET")" --data-urlencode "$CT=$T" -o /tmp/totp_th6.html
grep -q "メールアドレスまたはパスワードが正しくありません" /tmp/totp_th6.html && ok "the throttle blocks the correct disable ceremony" || ng "the throttle blocks the correct disable ceremony"
[ "$(db "SELECT totp_enabled_at IS NOT NULL FROM users WHERE email_bidx = '$(bidx $THROTTLE_EMAIL)'")" = "t" ] && ok "2FA is still enabled after the blocked disable" || ng "2FA is still enabled after the blocked disable"

result

