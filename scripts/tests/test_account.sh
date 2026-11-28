#!/bin/bash
# Account self-service flows (V1 perfection pass SF-1/SF-3/SF-4/SF-5):
# password reset with anti-enumeration, session revocation on
# password change and on reset, and the verified email change.
#
# Uses suite-owned fixture accounts, created through the real
# sign-up flow at the start (leftovers from an earlier run are
# deleted first). Like test_admin.sh, this suite is not free to
# rerun endlessly against one app process: reset requests feed
# the in-memory start throttle (5 per address per hour), so a
# fresh app process is the clean slate, as in the ordered gate.
BASE=http://127.0.0.1:3000
# Shared harness: ok/ng, csrf() and result() live in lib.sh (ST-6).
. "$(dirname "$0")/lib.sh"
mailcode() { sleep 1; curl -s "http://127.0.0.1:8025/api/v1/search?query=$1" | grep -o '"Snippet":"[^"]*"' | grep -o '[0-9]\{6\}' | head -1; }
clearmail() { curl -s -X DELETE http://127.0.0.1:8025/api/v1/messages >/dev/null; }
sid() { awk -v n="id" '$6==n {print $7}' "$1" | tail -1; }
rowexists() { docker exec agentic-db psql -U agentic -d agentic_commerce -At -c "SELECT count(*) FROM tower_sessions.session WHERE id = '$1';"; }

RESET_EMAIL="acct-reset@example.com"
SESS_EMAIL="acct-sess@example.com"
MAIL_EMAIL="acct-mail@example.com"
MAIL_NEW="acct-mail-new@example.com"
ORD_EMAIL="acct-orders@example.com"
CPN_EMAIL="acct-coupon@example.com"
CPNN_EMAIL="acct-coupon-none@example.com"
PW="acctpass1"

cleanup_fixtures() {
  docker exec agentic-db psql -U agentic -d agentic_commerce -q -c "
    DELETE FROM webhook_events WHERE payment_id IN (SELECT id FROM payments WHERE order_id IN (SELECT id FROM orders WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $ORD_EMAIL)','$(bidx $CPN_EMAIL)'))));
    DELETE FROM coupon_redemptions WHERE order_id IN (SELECT id FROM orders WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $ORD_EMAIL)','$(bidx $CPN_EMAIL)'))) OR user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $ORD_EMAIL)','$(bidx $CPN_EMAIL)'));
    DELETE FROM payments WHERE order_id IN (SELECT id FROM orders WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $ORD_EMAIL)','$(bidx $CPN_EMAIL)')));
    DELETE FROM order_items WHERE order_id IN (SELECT id FROM orders WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $ORD_EMAIL)','$(bidx $CPN_EMAIL)')));
    DELETE FROM orders WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $ORD_EMAIL)','$(bidx $CPN_EMAIL)'));
    DELETE FROM user_coupons WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $RESET_EMAIL)','$(bidx $SESS_EMAIL)','$(bidx $MAIL_EMAIL)','$(bidx $MAIL_NEW)','$(bidx $ORD_EMAIL)','$(bidx $CPN_EMAIL)','$(bidx $CPNN_EMAIL)'));
    DELETE FROM password_resets WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $RESET_EMAIL)','$(bidx $SESS_EMAIL)','$(bidx $MAIL_EMAIL)','$(bidx $MAIL_NEW)','$(bidx $ORD_EMAIL)','$(bidx $CPN_EMAIL)','$(bidx $CPNN_EMAIL)'));
    DELETE FROM email_changes WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $RESET_EMAIL)','$(bidx $SESS_EMAIL)','$(bidx $MAIL_EMAIL)','$(bidx $MAIL_NEW)','$(bidx $ORD_EMAIL)','$(bidx $CPN_EMAIL)','$(bidx $CPNN_EMAIL)'));
    DELETE FROM users WHERE email_bidx IN ('$(bidx $RESET_EMAIL)','$(bidx $SESS_EMAIL)','$(bidx $MAIL_EMAIL)','$(bidx $MAIL_NEW)','$(bidx $ORD_EMAIL)','$(bidx $CPN_EMAIL)','$(bidx $CPNN_EMAIL)');" >/dev/null 2>&1
}

# Creates one fixture account through the real sign-up flow.
create_account() {
  local jar=/tmp/acct_su_$$.txt email=$1
  rm -f $jar; clearmail
  curl -s -c $jar $BASE/signup -o /tmp/acct_su.html; local t=$(csrf /tmp/acct_su.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup --data-urlencode "email=$email" --data-urlencode "email_confirm=$email" --data-urlencode "csrf_token=$t"
  local code=$(mailcode $email)
  [ -n "$code" ] || return 1
  curl -s -b $jar -c $jar $BASE/signup/verify -o /tmp/acct_vf.html; t=$(csrf /tmp/acct_vf.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup/verify --data-urlencode "code=$code" --data-urlencode "csrf_token=$t"
  curl -s -b $jar -c $jar $BASE/signup/details -o /tmp/acct_det.html; t=$(csrf /tmp/acct_det.html)
  curl -s -b $jar -c $jar -X POST $BASE/signup/details -o /tmp/acct_conf.html \
    --data-urlencode "password=$PW" --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" \
    --data-urlencode "family_name_kana=ケンショウ" --data-urlencode "given_name_kana=タロウ" \
    --data-urlencode "birth_date=" --data-urlencode "gender=" --data-urlencode "postal_code=1500001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" \
    --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
    --data-urlencode "csrf_token=$t"
  t=$(csrf /tmp/acct_conf.html)
  local body=$(curl -s -b $jar -c $jar -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/confirm --data-urlencode "csrf_token=$t")
  rm -f $jar
  [ "$body" = "303 $BASE/" ]
}

# Logs in with a fresh jar; echoes nothing, leaves the jar filled.
login_as() {
  local jar=$1 email=$2 password=$3
  rm -f $jar
  curl -s -c $jar $BASE/login -o /tmp/acct_login.html; local t=$(csrf /tmp/acct_login.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/login --data-urlencode "email=$email" --data-urlencode "password=$password" --data-urlencode "csrf_token=$t"
}

cleanup_fixtures
echo "-- fixture accounts"
create_account $RESET_EMAIL && ok "reset fixture created" || { ng "reset fixture created"; echo "RESULT: blocked"; exit 1; }
create_account $SESS_EMAIL && ok "session fixture created" || { ng "session fixture created"; echo "RESULT: blocked"; exit 1; }
create_account $MAIL_EMAIL && ok "email fixture created" || { ng "email fixture created"; echo "RESULT: blocked"; exit 1; }

echo "-- login page links the reset flow"
curl -s $BASE/login -o /tmp/acct_li.html
grep -q 'href="/password-reset"' /tmp/acct_li.html && ok "forgot-password link on /login" || ng "forgot-password link on /login"

echo "-- SF-3: password change revokes the other session, keeps the current one"
JA=/tmp/acct_a.txt; JB=/tmp/acct_b.txt
login_as $JA $SESS_EMAIL $PW
login_as $JB $SESS_EMAIL $PW
SIDA=$(sid $JA); SIDB=$(sid $JB)
[ -n "$SIDA" ] && [ -n "$SIDB" ] && [ "$SIDA" != "$SIDB" ] && ok "two distinct sessions" || ng "two distinct sessions"
[ "$(rowexists $SIDA)" = "1" ] && [ "$(rowexists $SIDB)" = "1" ] && ok "both session rows stored" || ng "both session rows stored"
curl -s -b $JA -c $JA $BASE/mypage/password -o /tmp/acct_pw.html; T=$(csrf /tmp/acct_pw.html)
curl -s -b $JA -c $JA -X POST $BASE/mypage/password -o /tmp/acct_pw2.html \
  --data-urlencode "current_password=$PW" --data-urlencode "new_password=acctpass2" \
  --data-urlencode "new_password_confirm=acctpass2" --data-urlencode "csrf_token=$T"
grep -q "パスワードを変更しました" /tmp/acct_pw2.html && ok "password change accepted" || ng "password change accepted"
[ "$(rowexists $SIDB)" = "0" ] && ok "other session row deleted" || ng "other session row deleted"
[ "$(rowexists $SIDA)" = "1" ] && ok "current session row kept" || ng "current session row kept"
R=$(curl -s -b $JB -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/mypage)
[ "$R" = "303 $BASE/login?next=/mypage" ] && ok "revoked session bounced to login" || ng "revoked session bounced to login ($R)"
curl -s -b $JA $BASE/mypage -o /dev/null -w "%{http_code}" | grep -q 200 && ok "current session still works" || ng "current session still works"

echo "-- SF-1: reset happy path, and completion revokes a live session"
JC=/tmp/acct_c.txt; JD=/tmp/acct_d.txt
login_as $JC $RESET_EMAIL $PW
SIDC=$(sid $JC)
[ "$(rowexists $SIDC)" = "1" ] && ok "live session before reset" || ng "live session before reset"
rm -f $JD; clearmail
curl -s -c $JD $BASE/password-reset -o /tmp/acct_rr.html; T=$(csrf /tmp/acct_rr.html)
R=$(curl -s -b $JD -c $JD -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/password-reset --data-urlencode "email=$RESET_EMAIL" --data-urlencode "csrf_token=$T")
[ "$R" = "303 $BASE/password-reset/verify" ] && ok "reset request redirects to code step" || ng "reset request redirects ($R)"
RCODE=$(mailcode $RESET_EMAIL); [ -n "$RCODE" ] && ok "reset code mailed" || { ng "reset code mailed"; echo "RESULT: blocked"; exit 1; }
WRONG=000000; [ "$WRONG" = "$RCODE" ] && WRONG=111111
curl -s -b $JD -c $JD $BASE/password-reset/verify -o /tmp/acct_rv.html; T=$(csrf /tmp/acct_rv.html)
curl -s -b $JD -c $JD -X POST $BASE/password-reset/verify -o /tmp/acct_rv2.html --data-urlencode "code=$WRONG" --data-urlencode "csrf_token=$T"
grep -q "認証コードが正しくありません" /tmp/acct_rv2.html && ok "wrong reset code rejected" || ng "wrong reset code rejected"
T=$(csrf /tmp/acct_rv2.html)
R=$(curl -s -b $JD -c $JD -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/password-reset/verify --data-urlencode "code=$RCODE" --data-urlencode "csrf_token=$T")
[ "$R" = "303 $BASE/password-reset/password" ] && ok "right code advances to password step" || ng "right code advances ($R)"
curl -s -b $JD -c $JD $BASE/password-reset/password -o /tmp/acct_rp.html; T=$(csrf /tmp/acct_rp.html)
R=$(curl -s -b $JD -c $JD -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/password-reset/password --data-urlencode "new_password=acctpass9" --data-urlencode "new_password_confirm=acctpass9" --data-urlencode "csrf_token=$T")
[ "$R" = "303 $BASE/login" ] && ok "reset completion redirects to login" || ng "reset completion redirects ($R)"
curl -s -b $JD -c $JD $BASE/login -o /tmp/acct_li2.html
grep -q "パスワードを再設定しました" /tmp/acct_li2.html && ok "completion flash on login page" || ng "completion flash on login page"
[ "$(rowexists $SIDC)" = "0" ] && ok "reset deleted the live session row" || ng "reset deleted the live session row"
R=$(curl -s -b $JC -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/mypage)
[ "$R" = "303 $BASE/login?next=/mypage" ] && ok "pre-reset session bounced" || ng "pre-reset session bounced ($R)"
JO=/tmp/acct_o.txt
curl -s -c $JO $BASE/login -o /tmp/acct_login.html; T=$(csrf /tmp/acct_login.html)
curl -s -b $JO -c $JO -X POST $BASE/login -o /tmp/acct_old.html --data-urlencode "email=$RESET_EMAIL" --data-urlencode "password=$PW" --data-urlencode "csrf_token=$T"
grep -q "メールアドレスまたはパスワードが正しくありません" /tmp/acct_old.html && ok "old password refused after reset" || ng "old password refused after reset"
JN=/tmp/acct_n.txt; login_as $JN $RESET_EMAIL acctpass9
curl -s -b $JN $BASE/mypage -o /dev/null -w "%{http_code}" | grep -q 200 && ok "new password logs in after reset" || ng "new password logs in after reset"

echo "-- SF-1 anti-enumeration: unknown address is indistinguishable"
JE=/tmp/acct_e.txt; JF=/tmp/acct_f.txt; rm -f $JE $JF; clearmail
curl -s -c $JE $BASE/password-reset -o /tmp/acct_rr.html; T=$(csrf /tmp/acct_rr.html)
RU=$(curl -s -b $JE -c $JE -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/password-reset --data-urlencode "email=nobody-xyz@example.com" --data-urlencode "csrf_token=$T")
curl -s -b $JE -c $JE $BASE/password-reset/verify -o /tmp/acct_vu.html
curl -s -c $JF $BASE/password-reset -o /tmp/acct_rr.html; T=$(csrf /tmp/acct_rr.html)
RK=$(curl -s -b $JF -c $JF -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/password-reset --data-urlencode "email=$RESET_EMAIL" --data-urlencode "csrf_token=$T")
curl -s -b $JF -c $JF $BASE/password-reset/verify -o /tmp/acct_vk.html
[ "$RU" = "$RK" ] && ok "request responses identical" || ng "request responses differ ($RU vs $RK)"
sed -E 's/name="csrf_token" value="[^"]*"/name="csrf_token" value="X"/' /tmp/acct_vu.html > /tmp/acct_vu.n
sed -E 's/name="csrf_token" value="[^"]*"/name="csrf_token" value="X"/' /tmp/acct_vk.html > /tmp/acct_vk.n
diff -q /tmp/acct_vu.n /tmp/acct_vk.n >/dev/null && ok "code pages identical (modulo csrf token)" || ng "code pages differ"
NROWS=$(docker exec agentic-db psql -U agentic -d agentic_commerce -At -c "SELECT count(*) FROM password_resets WHERE email = 'nobody-xyz@example.com';")
[ "$NROWS" = "0" ] && ok "no reset row for unknown address" || ng "reset row for unknown address ($NROWS)"
curl -s "http://127.0.0.1:8025/api/v1/search?query=nobody-xyz@example.com" | grep -q '"messages":\[\]' && ok "no mail sent for unknown address" || ng "mail sent for unknown address"
T=$(csrf /tmp/acct_vu.html)
curl -s -b $JE -c $JE -X POST $BASE/password-reset/verify -o /tmp/acct_vu2.html --data-urlencode "code=123456" --data-urlencode "csrf_token=$T"
grep -q "認証コードが正しくありません" /tmp/acct_vu2.html && ok "code submit without a challenge gets the generic error" || ng "code submit without a challenge"
docker exec agentic-db psql -U agentic -d agentic_commerce -q -c "DELETE FROM password_resets WHERE email = '$RESET_EMAIL';" >/dev/null 2>&1

echo "-- SF-4: email change end to end"
JG=/tmp/acct_g.txt; login_as $JG $MAIL_EMAIL $PW
curl -s -b $JG -c $JG $BASE/mypage -o /tmp/acct_mp0.html
grep -q "メールアドレス変更" /tmp/acct_mp0.html && ok "mypage links the email change" || ng "mypage links the email change"
curl -s -b $JG -c $JG $BASE/mypage/email -o /tmp/acct_em.html; T=$(csrf /tmp/acct_em.html)
curl -s -b $JG -c $JG -X POST $BASE/mypage/email -o /tmp/acct_em2.html --data-urlencode "current_password=wrongpass1" --data-urlencode "new_email=$MAIL_NEW" --data-urlencode "csrf_token=$T"
grep -q "現在のパスワードが正しくありません" /tmp/acct_em2.html && ok "wrong current password refused" || ng "wrong current password refused"
T=$(csrf /tmp/acct_em2.html)
curl -s -b $JG -c $JG -X POST $BASE/mypage/email -o /tmp/acct_em3.html --data-urlencode "current_password=$PW" --data-urlencode "new_email=test@example.com" --data-urlencode "csrf_token=$T"
grep -q "このメールアドレスは登録済みです" /tmp/acct_em3.html && ok "already-registered address refused" || ng "already-registered address refused"
clearmail
T=$(csrf /tmp/acct_em3.html)
R=$(curl -s -b $JG -c $JG -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/mypage/email --data-urlencode "current_password=$PW" --data-urlencode "new_email=$MAIL_NEW" --data-urlencode "csrf_token=$T")
[ "$R" = "303 $BASE/mypage/email/verify" ] && ok "change start redirects to code step" || ng "change start redirects ($R)"
ECODE=$(mailcode $MAIL_NEW); [ -n "$ECODE" ] && ok "code mailed to the NEW address" || { ng "code mailed to the NEW address"; echo "RESULT: blocked"; exit 1; }
curl -s -b $JG -c $JG $BASE/mypage/email/verify -o /tmp/acct_ev.html; T=$(csrf /tmp/acct_ev.html)
WRONG=000000; [ "$WRONG" = "$ECODE" ] && WRONG=111111
curl -s -b $JG -c $JG -X POST $BASE/mypage/email/verify -o /tmp/acct_ev2.html --data-urlencode "code=$WRONG" --data-urlencode "csrf_token=$T"
grep -q "認証コードが正しくありません" /tmp/acct_ev2.html && ok "wrong change code rejected" || ng "wrong change code rejected"
T=$(csrf /tmp/acct_ev2.html)
curl -s -b $JG -c $JG -X POST $BASE/mypage/email/verify -o /tmp/acct_ev3.html --data-urlencode "code=$ECODE" --data-urlencode "csrf_token=$T"
grep -q "メールアドレスを変更しました" /tmp/acct_ev3.html && ok "change completed" || ng "change completed"
curl -s -b $JG -c $JG $BASE/mypage -o /tmp/acct_mp.html
grep -q "$MAIL_NEW" /tmp/acct_mp.html && ok "still logged in, mypage shows new address" || ng "still logged in, mypage shows new address"
UID_BEFORE=$(docker exec agentic-db psql -U agentic -d agentic_commerce -At -c "SELECT id FROM users WHERE email_bidx = '$(bidx $MAIL_NEW)';")
COUPONS=$(docker exec agentic-db psql -U agentic -d agentic_commerce -At -c "SELECT count(*) FROM user_coupons WHERE user_id = $UID_BEFORE;")
[ "$COUPONS" = "1" ] && ok "coupons still attached to the same user_id" || ng "coupons still attached ($COUPONS)"
JOLD=/tmp/acct_old.txt
curl -s -c $JOLD $BASE/login -o /tmp/acct_login.html; T=$(csrf /tmp/acct_login.html)
curl -s -b $JOLD -c $JOLD -X POST $BASE/login -o /tmp/acct_old2.html --data-urlencode "email=$MAIL_EMAIL" --data-urlencode "password=$PW" --data-urlencode "csrf_token=$T"
grep -q "メールアドレスまたはパスワードが正しくありません" /tmp/acct_old2.html && ok "login with old address refused" || ng "login with old address refused"
JNEW=/tmp/acct_new.txt; login_as $JNEW $MAIL_NEW $PW
curl -s -b $JNEW $BASE/mypage -o /dev/null -w "%{http_code}" | grep -q 200 && ok "login with new address works" || ng "login with new address works"

echo "-- SF-5: mypage order history paginates at ten, newest first"
create_account $ORD_EMAIL && ok "orders fixture created" || { ng "orders fixture created"; echo "RESULT: blocked"; exit 1; }
ORD_UID=$(docker exec agentic-db psql -U agentic -d agentic_commerce -At -c "SELECT id FROM users WHERE email_bidx = '$(bidx $ORD_EMAIL)';")
for i in $(seq 0 10); do
  docker exec agentic-db psql -U agentic -d agentic_commerce -q -c "INSERT INTO orders (user_id, email, family_name, given_name, postal_code, prefecture, city, address_line, building, phone, subtotal_yen, discount_yen, shipping_yen, cod_fee_yen, total_yen, status, created_at) VALUES ($ORD_UID, '$ORD_EMAIL', '検証', '太郎', '1500001', '東京都', '渋谷区', '1-1-1', NULL, '09012345678', 1000, 0, 550, 330, 1880, 'received', now() - interval '$i hours');" >/dev/null
done
TOTAL_ORD=$(docker exec agentic-db psql -U agentic -d agentic_commerce -At -c "SELECT count(*) FROM orders WHERE user_id = $ORD_UID;")
[ "$TOTAL_ORD" = "11" ] && ok "eleven fixture orders inserted" || ng "eleven fixture orders inserted ($TOTAL_ORD)"
NEWEST=$(docker exec agentic-db psql -U agentic -d agentic_commerce -At -c "SELECT 'AS-' || to_char(created_at AT TIME ZONE 'Asia/Tokyo', 'YYYYMMDD') || '-' || LPAD(id::text, 4, '0') FROM orders WHERE user_id = $ORD_UID ORDER BY created_at DESC, id DESC LIMIT 1;")
OLDEST=$(docker exec agentic-db psql -U agentic -d agentic_commerce -At -c "SELECT 'AS-' || to_char(created_at AT TIME ZONE 'Asia/Tokyo', 'YYYYMMDD') || '-' || LPAD(id::text, 4, '0') FROM orders WHERE user_id = $ORD_UID ORDER BY created_at ASC, id ASC LIMIT 1;")
JH=/tmp/acct_h.txt; login_as $JH $ORD_EMAIL $PW
curl -s -b $JH $BASE/mypage/orders -o /tmp/acct_h1.html
ROWS1=$(grep -c 'class="order-number"' /tmp/acct_h1.html)
[ "$ROWS1" = "10" ] && grep -q "$NEWEST" /tmp/acct_h1.html && ! grep -q "$OLDEST" /tmp/acct_h1.html && grep -q 'href="/mypage/orders?page=2"' /tmp/acct_h1.html \
  && ok "history page 1 shows the newest ten with a next link" || ng "history page 1 shows the newest ten (rows=$ROWS1)"
curl -s -b $JH "$BASE/mypage/orders?page=2" -o /tmp/acct_h2.html
ROWS2=$(grep -c 'class="order-number"' /tmp/acct_h2.html)
[ "$ROWS2" = "1" ] && grep -q "$OLDEST" /tmp/acct_h2.html && grep -q 'href="/mypage/orders?page=1"' /tmp/acct_h2.html \
  && ok "history page 2 shows the oldest order with a previous link" || ng "history page 2 shows the oldest order (rows=$ROWS2)"

echo "-- SF-6: mypage lists the member's coupons, held then used"
create_account $CPN_EMAIL && ok "coupon fixture created" || { ng "coupon fixture created"; echo "RESULT: blocked"; exit 1; }
JCP=/tmp/acct_cp.txt; login_as $JCP $CPN_EMAIL $PW
curl -s -b $JCP $BASE/mypage/coupons -o /tmp/acct_cp1.html
grep -q "<h1>クーポン一覧</h1>" /tmp/acct_cp1.html && grep -q "WELCOME500" /tmp/acct_cp1.html && grep -q "未使用" /tmp/acct_cp1.html && grep -q "￥500割引" /tmp/acct_cp1.html \
  && ok "held WELCOME500 listed as unused with its discount" || ng "held WELCOME500 listed as unused with its discount"
# Use it at checkout: keyholder 880 - 500 + shipping 550 + COD 330.
curl -s -b $JCP $BASE/ -o /tmp/acct_cp_home.html; T=$(csrf /tmp/acct_cp_home.html)
curl -s -b $JCP -c $JCP -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$T"
curl -s -b $JCP $BASE/checkout -o /tmp/acct_cp_co.html; T=$(csrf /tmp/acct_cp_co.html)
curl -s -b $JCP -c $JCP -X POST $BASE/checkout --data-urlencode "intent=apply_coupon" --data-urlencode "coupon_code=WELCOME500" \
  --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" \
  --data-urlencode "phone=09012345678" --data-urlencode "csrf_token=$T" -o /tmp/acct_cp_ap.html
grep -q "を適用中" /tmp/acct_cp_ap.html && ok "coupon applies at the fixture checkout" || ng "coupon applies at the fixture checkout"
T=$(csrf /tmp/acct_cp_ap.html)
curl -s -b $JCP -c $JCP -X POST $BASE/checkout --data-urlencode "intent=confirm" \
  --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" \
  --data-urlencode "phone=09012345678" --data-urlencode "csrf_token=$T" -o /tmp/acct_cp_cf.html
curl -s -b $JCP -c $JCP -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "csrf_token=$T" -o /tmp/acct_cp_cf.html
T=$(csrf /tmp/acct_cp_cf.html)
curl -s -b $JCP -c $JCP -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$T"
CPN_UID=$(docker exec agentic-db psql -U agentic -d agentic_commerce -At -c "SELECT id FROM users WHERE email_bidx = '$(bidx $CPN_EMAIL)';")
CPN_OID=$(docker exec agentic-db psql -U agentic -d agentic_commerce -At -c "SELECT MAX(id) FROM orders WHERE user_id = $CPN_UID;")
curl -s -b $JCP $BASE/mypage/coupons -o /tmp/acct_cp2.html
grep -q "使用済み" /tmp/acct_cp2.html && grep -q "href=\"/mypage/orders/$CPN_OID\"" /tmp/acct_cp2.html \
  && ok "used coupon shows its state and links the using order" || ng "used coupon shows its state and links the using order"
curl -s -b $JCP $BASE/mypage/orders/$CPN_OID -o /tmp/acct_cp3.html
grep -q "ご注文内容" /tmp/acct_cp3.html && ok "the linked order detail opens" || ng "the linked order detail opens"

echo "-- SF-6: a member with no coupons sees the empty state"
create_account $CPNN_EMAIL && ok "no-coupon fixture created" || { ng "no-coupon fixture created"; echo "RESULT: blocked"; exit 1; }
docker exec agentic-db psql -U agentic -d agentic_commerce -q -c "DELETE FROM user_coupons WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $CPNN_EMAIL)');" >/dev/null
JCN=/tmp/acct_cn.txt; login_as $JCN $CPNN_EMAIL $PW
curl -s -b $JCN $BASE/mypage/coupons -o /tmp/acct_cn1.html
grep -q "現在お持ちのクーポンはありません" /tmp/acct_cn1.html && ! grep -q "mypage-coupon-code" /tmp/acct_cn1.html \
  && ok "empty state when the member holds no coupons" || ng "empty state when the member holds no coupons"

result
