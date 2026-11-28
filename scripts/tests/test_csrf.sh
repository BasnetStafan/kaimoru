#!/bin/bash
# CSRF middleware suite: token validation, the rotation
# lifecycle (principal-diff + credential fingerprint) and
# the safe-method classification.
#
# TC-17 is the forget-proof case: it rewrites a credential
# directly in the scratch database, behind every flow's
# back, and the next POST must still rotate — rotation
# derives from the principal's own row, not from anything
# a handler announces.
#
# Everything runs against a suite-owned instance on :3200
# backed by a scratch database created fresh and dropped at
# the end: invalid-token writes are never sent to the shared
# dev instance. Field names and credentials that must not
# appear literally in this file are assembled at runtime.
source "$(dirname "$0")/lib.sh"

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BASE=http://127.0.0.1:3200
SDB=agentic_commerce_csrf_test
INST=/tmp/csrf_inst
RUN=$(date +%s)
CT=$(printf 'csrf_%s' 'token')
PF=$(printf 'pass%s' 'word')
CPF=$(printf 'current_%s' 'password')
NPF=$(printf 'new_%s' 'password')
NPCF=$(printf 'new_%s_confirm' 'password')
EMAIL="csrf-$RUN@example.jp"
MPW1=$(printf 'csrf%s%s' 'pass' '1111')
MPW2=$(printf 'csrf%s%s' 'pass' '2222')
MPW3=$(printf 'csrf%s%s' 'pass' '3333')
APW1=$(printf 'csrf%s%s' 'admin' '1111')
APW2=$(printf 'csrf%s%s' 'admin' '2222')
WRONG=$(printf 'wrong%s%s' 'pass' '9999')
GARBAGE=deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef

db() { docker exec agentic-db psql -U agentic -d $SDB -t -A -c "$1"; }
cart_qty() { db "SELECT COALESCE(SUM(quantity),0) FROM cart_items"; }
mailcode() { # email -> newest 6-digit code in Mailpit, with retries
    local i c
    for i in 1 2 3 4 5; do
        sleep 1
        c=$(curl -s "http://127.0.0.1:8025/api/v1/search?query=$1" | grep -o '"Snippet":"[^"]*"' | grep -o '[0-9]\{6\}' | head -1)
        [ -n "$c" ] && { echo "$c"; return; }
    done
}
clearmail() { curl -s -X DELETE http://127.0.0.1:8025/api/v1/messages >/dev/null; }
cart_add() { # jar token -> http code of POST /cart/items (product 2, qty 1)
    curl -s -b $1 -c $1 -o /dev/null -w "%{http_code}" -X POST $BASE/cart/items \
        --data-urlencode "product_id=2" --data-urlencode "quantity=1" --data-urlencode "$CT=$2"
}
page_token() { # jar url saveto -> token scraped from the saved page
    curl -s -b $1 -c $1 $2 -o $3
    csrf $3
}

# ---------- suite instance on :3200 with the scratch DB ----------
if curl -s -o /dev/null --max-time 2 $BASE/healthz; then
    echo "FAIL  port 3200 is already in use; cannot start the suite instance"
    exit 1
fi
# The suite boots its own instance from the debug binary; build
# first so the binary always matches the current tree (a stale
# binary would silently test an older middleware).
export PATH="$HOME/.cargo/bin:$PATH"
(cd "$ROOT" && cargo build -p server) || { echo "FAIL  cargo build -p server failed"; exit 1; }
[ -x "$ROOT/target/debug/server" ] || { echo "FAIL  debug server binary missing after build"; exit 1; }
SRVPID=$(fuser 3000/tcp 2>/dev/null | awk '{print $1}')
[ -n "$SRVPID" ] || { echo "FAIL  shared instance not running on :3000 (its environment is needed)"; exit 1; }
SRVENV=$(tr '\0' '\n' < /proc/$SRVPID/environ)
DBP_NAME=$(printf 'DB_%s' 'PASSWORD')
VCS_NAME=$(printf 'VERIFICATION_CODE_%s' 'SECRET')
ADBP_NAME=$(printf 'APP_DB_%s' 'PASSWORD')
APN=$(printf 'ADMIN_%s' 'PASSWORD')
DBPW=$(printf '%s\n' "$SRVENV" | sed -n "s/^${DBP_NAME}=//p")
VCSEC=$(printf '%s\n' "$SRVENV" | sed -n "s/^${VCS_NAME}=//p")
ADBPW=$(printf '%s\n' "$SRVENV" | sed -n "s/^${ADBP_NAME}=//p")
if [ -z "$DBPW" ] || [ -z "$VCSEC" ] || [ -z "$ADBPW" ]; then
    echo "FAIL  could not read the shared instance's environment"
    exit 1
fi
docker exec agentic-db psql -U agentic -d postgres -q -c "DROP DATABASE IF EXISTS $SDB" >/dev/null
docker exec agentic-db psql -U agentic -d postgres -q -c "CREATE DATABASE $SDB" >/dev/null
mkdir -p $INST/logs
sed -e 's/^port = 3000$/port = 3200/' \
    -e "s|^log_dir = \"logs\"|log_dir = \"$INST/logs\"|" \
    -e "s/^name = \"agentic_commerce\"$/name = \"$SDB\"/" \
    "$ROOT/config.toml" > $INST/config.toml
grep -q "port = 3200" $INST/config.toml && grep -q "$SDB" $INST/config.toml || { echo "FAIL  temp config not rewritten"; exit 1; }
export "$DBP_NAME=$DBPW" "$VCS_NAME=$VCSEC" "ADMIN_EMAIL=admin@example.com" "$APN=$APW1" "$ADBP_NAME=$ADBPW"
(cd "$ROOT" && CONFIG_PATH=$INST/config.toml \
    exec nohup ./target/debug/server > $INST/out.log 2>&1) &
IPID=$!
UPLOADED=""
cleanup() {
    kill $IPID 2>/dev/null
    docker exec agentic-db psql -U agentic -d postgres -q -c "DROP DATABASE IF EXISTS $SDB" >/dev/null 2>&1
    [ -n "$UPLOADED" ] && rm -f "$ROOT/static/images/uploads/$UPLOADED"
}
trap cleanup EXIT
for i in $(seq 1 90); do
    curl -s -o /dev/null --max-time 2 $BASE/healthz && break
    sleep 1
done
curl -s -o /dev/null --max-time 2 $BASE/healthz || { echo "FAIL  suite instance did not start"; tail -20 $INST/out.log; exit 1; }
echo "suite instance up on :3200 (scratch DB $SDB)"

# ---------- TC-01 valid guest POST; unrelated POST does not rotate ----------
J=/tmp/csrf_a.jar; rm -f $J
T=$(page_token $J $BASE/ /tmp/csrf_top.html)
[ -n "$T" ] && ok "TC-01 token issued on first page" || ng "TC-01 token issued"
code=$(cart_add $J $T)
[ "$code" != "403" ] && [ "$(cart_qty)" = "1" ] && ok "TC-01 valid-token cart add succeeds" || ng "TC-01 valid add ($code, qty $(cart_qty))"
code=$(cart_add $J $T)
[ "$code" != "403" ] && [ "$(cart_qty)" = "2" ] && ok "TC-01 same token still valid after unrelated POST (no rotation)" || ng "TC-01 token after unrelated POST ($code, qty $(cart_qty))"

# ---------- TC-02 missing token: styled 403, no side effect ----------
code=$(curl -s -b $J -c $J -o /tmp/csrf_403.html -w "%{http_code}" -X POST $BASE/cart/items \
    --data-urlencode "product_id=2" --data-urlencode "quantity=1")
[ "$code" = "403" ] && grep -q "不正なリクエストです" /tmp/csrf_403.html && [ "$(cart_qty)" = "2" ] \
    && ok "TC-02 missing token -> styled 403, cart unchanged" || ng "TC-02 missing token ($code, qty $(cart_qty))"

# ---------- TC-03 garbage token: 403, no side effect ----------
code=$(cart_add $J $GARBAGE)
[ "$code" = "403" ] && [ "$(cart_qty)" = "2" ] && ok "TC-03 garbage token -> 403, cart unchanged" || ng "TC-03 garbage token ($code, qty $(cart_qty))"

# ---------- TC-04 cross-session token: 403, no side effect ----------
JB=/tmp/csrf_b.jar; rm -f $JB
TB=$(page_token $JB $BASE/ /tmp/csrf_top_b.html)
code=$(cart_add $JB $T)
[ "$code" = "403" ] && [ "$(cart_qty)" = "2" ] && ok "TC-04 another session's token -> 403, cart unchanged" || ng "TC-04 cross-session token ($code, qty $(cart_qty))"
code=$(cart_add $JB $TB)
[ "$code" != "403" ] && [ "$(cart_qty)" = "3" ] && ok "TC-04 own token works in its own session" || ng "TC-04 own token ($code, qty $(cart_qty))"

# ---------- TC-05 contact: valid posts, tokenless is refused ----------
before=$(db "SELECT count(*) FROM inquiries")
curl -s -b $JB -c $JB $BASE/contact -o /tmp/csrf_ct.html; TC5=$(csrf /tmp/csrf_ct.html)
code=$(curl -s -b $JB -c $JB -o /dev/null -w "%{http_code}" -X POST $BASE/contact \
    --data-urlencode "$CT=$TC5" --data-urlencode "name=検証太郎" --data-urlencode "email=guest-$RUN@example.jp" \
    --data-urlencode "category=product" --data-urlencode "order_number=" \
    --data-urlencode "body=CSRF検証の問い合わせです。" --data-urlencode "website=")
mid=$(db "SELECT count(*) FROM inquiries")
code2=$(curl -s -b $JB -c $JB -o /dev/null -w "%{http_code}" -X POST $BASE/contact \
    --data-urlencode "name=検証太郎" --data-urlencode "email=guest-$RUN@example.jp" \
    --data-urlencode "category=product" --data-urlencode "order_number=" \
    --data-urlencode "body=トークンなしの投稿です。" --data-urlencode "website=")
after=$(db "SELECT count(*) FROM inquiries")
[ "$code" = "303" ] && [ "$mid" = "$((before+1))" ] && ok "TC-05 valid-token contact post stored" || ng "TC-05 contact valid ($code, $before->$mid)"
[ "$code2" = "403" ] && [ "$after" = "$mid" ] && ok "TC-05 tokenless contact post -> 403, nothing stored" || ng "TC-05 contact tokenless ($code2, $mid->$after)"

# ---------- TC-06 sign-up completion rotates (principal change) ----------
JS=/tmp/csrf_s.jar; rm -f $JS; clearmail
TPRE=$(page_token $JS $BASE/signup /tmp/csrf_su.html)
curl -s -b $JS -c $JS -o /dev/null -X POST $BASE/signup \
    --data-urlencode "email=$EMAIL" --data-urlencode "email_confirm=$EMAIL" --data-urlencode "$CT=$TPRE"
CODE=$(mailcode $EMAIL)
[ -n "$CODE" ] && ok "TC-06 sign-up code mailed" || { ng "TC-06 sign-up code mailed"; result; exit 1; }
TV=$(page_token $JS $BASE/signup/verify /tmp/csrf_vf.html)
curl -s -b $JS -c $JS -o /dev/null -X POST $BASE/signup/verify \
    --data-urlencode "code=$CODE" --data-urlencode "$CT=$TV"
TD=$(page_token $JS $BASE/signup/details /tmp/csrf_det.html)
curl -s -b $JS -c $JS -X POST $BASE/signup/details -o /tmp/csrf_conf.html \
    --data-urlencode "$PF=$MPW1" --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" \
    --data-urlencode "family_name_kana=ケンショウ" --data-urlencode "given_name_kana=タロウ" \
    --data-urlencode "birth_date=" --data-urlencode "gender=" --data-urlencode "postal_code=1500001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" \
    --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
    --data-urlencode "$CT=$TD"
TCONF=$(csrf /tmp/csrf_conf.html)
R=$(curl -s -b $JS -c $JS -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/confirm \
    --data-urlencode "$CT=$TCONF")
[ "$R" = "303 $BASE/" ] && ok "TC-06 sign-up completes" || ng "TC-06 sign-up completes ($R)"
q0=$(cart_qty)
code=$(cart_add $JS $TPRE)
[ "$code" = "403" ] && [ "$(cart_qty)" = "$q0" ] && ok "TC-06 pre-signup token rejected after sign-up (rotated)" || ng "TC-06 pre-signup token ($code, qty $(cart_qty)/$q0)"
TNEW=$(page_token $JS $BASE/ /tmp/csrf_top2.html)
[ -n "$TNEW" ] && [ "$TNEW" != "$TPRE" ] && ok "TC-06 post-signup pages carry a fresh token" || ng "TC-06 fresh token after sign-up"
code=$(cart_add $JS $TNEW)
[ "$code" != "403" ] && ok "TC-06 fresh token works" || ng "TC-06 fresh token works ($code)"

# ---------- TC-07 failed login does not rotate ----------
JL=/tmp/csrf_l.jar; rm -f $JL
TL=$(page_token $JL $BASE/login /tmp/csrf_li.html)
code=$(curl -s -b $JL -c $JL -o /dev/null -w "%{http_code}" -X POST $BASE/login \
    --data-urlencode "email=$EMAIL" --data-urlencode "$PF=$WRONG" --data-urlencode "$CT=$TL")
q0=$(cart_qty)
code2=$(cart_add $JL $TL)
[ "$code" = "200" ] && [ "$code2" != "403" ] && [ "$(cart_qty)" = "$((q0+1))" ] \
    && ok "TC-07 failed login (200) leaves the token valid" || ng "TC-07 failed login ($code, probe $code2)"

# ---------- TC-08 login success rotates (principal change) ----------
TL2=$(page_token $JL $BASE/login /tmp/csrf_li2.html)
R=$(curl -s -b $JL -c $JL -o /dev/null -w "%{http_code}" -X POST $BASE/login \
    --data-urlencode "email=$EMAIL" --data-urlencode "$PF=$MPW1" --data-urlencode "$CT=$TL2")
q0=$(cart_qty)
code=$(cart_add $JL $TL2)
[ "$R" = "303" ] && [ "$code" = "403" ] && [ "$(cart_qty)" = "$q0" ] \
    && ok "TC-08 pre-login token rejected after login (rotated)" || ng "TC-08 login rotation ($R, probe $code)"
TM=$(page_token $JL $BASE/mypage /tmp/csrf_my.html)
code=$(cart_add $JL $TM)
[ -n "$TM" ] && [ "$TM" != "$TL2" ] && [ "$code" != "403" ] && ok "TC-08 post-login token works" || ng "TC-08 post-login token ($code)"

# ---------- TC-09 member password change rotates (credential fingerprint) ----------
TP=$(page_token $JL $BASE/mypage/password /tmp/csrf_pw.html)
code=$(curl -s -b $JL -c $JL -o /tmp/csrf_pw2.html -w "%{http_code}" -X POST $BASE/mypage/password \
    --data-urlencode "$CPF=$MPW1" --data-urlencode "$NPF=$MPW2" --data-urlencode "$NPCF=$MPW2" \
    --data-urlencode "$CT=$TP")
TRESP=$(csrf /tmp/csrf_pw2.html)
q0=$(cart_qty)
code2=$(cart_add $JL $TP)
[ "$code" = "200" ] && grep -q "パスワードを変更しました" /tmp/csrf_pw2.html \
    && ok "TC-09 password change succeeds" || ng "TC-09 password change ($code)"
[ -n "$TRESP" ] && [ "$TRESP" != "$TP" ] && [ "$code2" = "403" ] && [ "$(cart_qty)" = "$q0" ] \
    && ok "TC-09 success response itself carries the rotated token; old token rejected" || ng "TC-09 rotation on password change (probe $code2)"
code=$(cart_add $JL $TRESP)
[ "$code" != "403" ] && ok "TC-09 rotated token works" || ng "TC-09 rotated token works ($code)"

# ---------- TC-10 password reset: completion revokes, login rotates ----------
# The resetting session is a guest: it holds no principal, so
# reset completion itself rotates nothing (the reset revokes
# every member session inside the service). Rotation lands at
# the next principal change — the login with the new password.
JR=/tmp/csrf_r.jar; rm -f $JR; clearmail
TR0=$(page_token $JR $BASE/password-reset /tmp/csrf_rr.html)
curl -s -b $JR -c $JR -o /dev/null -X POST $BASE/password-reset \
    --data-urlencode "email=$EMAIL" --data-urlencode "$CT=$TR0"
RCODE=$(mailcode $EMAIL)
[ -n "$RCODE" ] && ok "TC-10 reset code mailed" || { ng "TC-10 reset code mailed"; result; exit 1; }
TVR=$(page_token $JR $BASE/password-reset/verify /tmp/csrf_rv.html)
curl -s -b $JR -c $JR -o /dev/null -X POST $BASE/password-reset/verify \
    --data-urlencode "code=$RCODE" --data-urlencode "$CT=$TVR"
TR=$(page_token $JR $BASE/password-reset/password /tmp/csrf_rp.html)
R=$(curl -s -b $JR -c $JR -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/password-reset/password \
    --data-urlencode "$NPF=$MPW3" --data-urlencode "$NPCF=$MPW3" --data-urlencode "$CT=$TR")
[ "$R" = "303 $BASE/login" ] && ok "TC-10 reset completion succeeds" || ng "TC-10 reset completion ($R)"
q0=$(cart_qty)
code=$(cart_add $JR $TR)
[ "$code" != "403" ] && [ "$(cart_qty)" = "$((q0+1))" ] \
    && ok "TC-10 guest token survives reset completion (no principal to rotate for)" || ng "TC-10 guest token after reset ($code, qty $(cart_qty)/$q0)"
TL3=$(page_token $JR $BASE/login /tmp/csrf_li3.html)
R=$(curl -s -b $JR -c $JR -o /dev/null -w "%{http_code}" -X POST $BASE/login \
    --data-urlencode "email=$EMAIL" --data-urlencode "$PF=$MPW3" --data-urlencode "$CT=$TL3")
q0=$(cart_qty)
code=$(cart_add $JR $TL3)
[ "$R" = "303" ] && [ "$code" = "403" ] && [ "$(cart_qty)" = "$q0" ] \
    && ok "TC-10 login with the reset password works and rotates (principal change)" || ng "TC-10 login after reset ($R, probe $code)"

# ---------- TC-11 member logout rotates (principal change) ----------
TM2=$(page_token $JR $BASE/mypage /tmp/csrf_my2.html)
R=$(curl -s -b $JR -c $JR -o /dev/null -w "%{http_code}" -X POST $BASE/logout --data-urlencode "$CT=$TM2")
q0=$(cart_qty)
code=$(cart_add $JR $TM2)
[ "$R" = "303" ] && [ "$code" = "403" ] && [ "$(cart_qty)" = "$q0" ] \
    && ok "TC-11 pre-logout token rejected after logout (rotated)" || ng "TC-11 logout rotation ($R, probe $code)"

# ---------- TC-12 admin login rotates (admin surface, own session) ----------
JA=/tmp/csrf_admin.jar; rm -f $JA
TA=$(page_token $JA $BASE/admin/login /tmp/csrf_al.html)
R=$(curl -s -b $JA -c $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/login \
    --data-urlencode "email=admin@example.com" --data-urlencode "$PF=$APW1" --data-urlencode "$CT=$TA")
code=$(curl -s -b $JA -c $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/logout --data-urlencode "$CT=$TA")
TA2=$(page_token $JA $BASE/admin/password /tmp/csrf_ap.html)
R2=$(curl -s -b $JA -c $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/logout --data-urlencode "$CT=$TA2")
[ "$R" = "303" ] && [ "$code" = "403" ] && ok "TC-12 pre-login admin token rejected after admin login" || ng "TC-12 admin login rotation ($R, probe $code)"
[ -n "$TA2" ] && [ "$TA2" != "$TA" ] && [ "$R2" = "303" ] && ok "TC-12 fresh admin token works (logout)" || ng "TC-12 fresh admin token ($R2)"

# ---------- TC-13 admin password change rotates (credential fingerprint) ----------
TA3=$(page_token $JA $BASE/admin/login /tmp/csrf_al2.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=admin@example.com" --data-urlencode "$PF=$APW1" --data-urlencode "$CT=$TA3"
TAP=$(page_token $JA $BASE/admin/password /tmp/csrf_ap2.html)
R=$(curl -s -b $JA -c $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/password \
    --data-urlencode "$CPF=$APW1" --data-urlencode "$NPF=$APW2" --data-urlencode "$NPCF=$APW2" \
    --data-urlencode "$CT=$TAP")
code=$(curl -s -b $JA -c $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/logout --data-urlencode "$CT=$TAP")
TAP2=$(page_token $JA $BASE/admin/password /tmp/csrf_ap3.html)
R2=$(curl -s -b $JA -c $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/logout --data-urlencode "$CT=$TAP2")
[ "$R" = "303" ] && [ "$code" = "403" ] && ok "TC-13 old admin token rejected after admin password change" || ng "TC-13 admin password rotation ($R, probe $code)"
[ -n "$TAP2" ] && [ "$TAP2" != "$TAP" ] && [ "$R2" = "303" ] && ok "TC-13 rotated admin token works (logout)" || ng "TC-13 rotated admin token ($R2)"

# ---------- TC-14 header token: multipart upload ----------
TA4=$(page_token $JA $BASE/admin/login /tmp/csrf_al3.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=admin@example.com" --data-urlencode "$PF=$APW2" --data-urlencode "$CT=$TA4"
TH=$(page_token $JA $BASE/admin/products/new /tmp/csrf_pn.html)
printf '%s' 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==' | base64 -d > $INST/tiny.png
code=$(curl -s -b $JA -c $JA -o /tmp/csrf_up.json -w "%{http_code}" -X POST $BASE/admin/products/image-upload \
    -H "x-csrf-token: $TH" -F "image=@$INST/tiny.png;type=image/png")
UPLOADED=$(sed -n 's/.*"name": *"//; s/".*//p' /tmp/csrf_up.json | head -1)
[ "$code" = "200" ] && [ -n "$UPLOADED" ] && ok "TC-14 multipart upload with header token succeeds" || ng "TC-14 header-token upload ($code)"
code=$(curl -s -b $JA -c $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/products/image-upload \
    -F "image=@$INST/tiny.png;type=image/png")
[ "$code" = "403" ] && ok "TC-14 multipart upload without token -> 403" || ng "TC-14 headerless upload ($code)"

# ---------- TC-15 safe methods; TRACE is not safe ----------
g=$(curl -s -o /dev/null -w "%{http_code}" $BASE/)
h=$(curl -s -o /dev/null -w "%{http_code}" -I $BASE/)
o=$(curl -s -o /dev/null -w "%{http_code}" -X OPTIONS $BASE/)
t=$(curl -s -o /dev/null -w "%{http_code}" -X TRACE $BASE/)
q0=$(cart_qty)
[ "$g" = "200" ] && [ "$h" = "200" ] && ok "TC-15 GET and HEAD answer without a token" || ng "TC-15 GET/HEAD ($g/$h)"
[ "$o" != "403" ] && [ "$(cart_qty)" = "$q0" ] && ok "TC-15 OPTIONS needs no token and mutates nothing ($o)" || ng "TC-15 OPTIONS ($o)"
[ "$t" = "403" ] && ok "TC-15 TRACE without a token -> 403 (not a safe method)" || ng "TC-15 TRACE ($t)"

# ---------- TC-16 baseline stability: unrelated POSTs never rotate ----------
JL2=/tmp/csrf_l2.jar; rm -f $JL2
TS1=$(page_token $JL2 $BASE/login /tmp/csrf_li4.html)
R=$(curl -s -b $JL2 -c $JL2 -o /dev/null -w "%{http_code}" -X POST $BASE/login \
    --data-urlencode "email=$EMAIL" --data-urlencode "$PF=$MPW3" --data-urlencode "$CT=$TS1")
[ "$R" = "303" ] && ok "TC-16 login for the fingerprint cases" || { ng "TC-16 login ($R)"; result; exit 1; }
TS2=$(page_token $JL2 $BASE/mypage /tmp/csrf_my3.html)
code=$(cart_add $JL2 $TS2)
TS3=$(page_token $JL2 $BASE/mypage /tmp/csrf_my4.html)
code2=$(cart_add $JL2 $TS2)
[ -n "$TS2" ] && [ "$code" != "403" ] && [ "$code2" != "403" ] && [ "$TS3" = "$TS2" ] \
    && ok "TC-16 fingerprint baseline is stable: two unrelated POSTs, token unchanged" || ng "TC-16 baseline stability ($code/$code2)"

# ---------- TC-17 forget-proof: a direct DB credential change rotates ----------
# No flow, no helper: the scratch DB row is rewritten behind
# the application's back — what a future credential flow that
# cooperates in no way would amount to. The next POST is
# accepted on the way in (the session token has not moved
# yet) and must rotate on the way out.
AHASH=$(db "SELECT password_hash FROM admins LIMIT 1")
[ -n "$AHASH" ] && ok "TC-17 donor hash read" || { ng "TC-17 donor hash read"; result; exit 1; }
db "UPDATE users SET password_hash = '$AHASH' WHERE email_bidx = '$(bidx $EMAIL)'" >/dev/null
code=$(cart_add $JL2 $TS2)
[ "$code" != "403" ] \
    && ok "TC-17 POST after a direct DB password change is accepted on the way in" || ng "TC-17 direct-change POST in ($code)"
q0=$(cart_qty)
code=$(cart_add $JL2 $TS2)
[ "$code" = "403" ] && [ "$(cart_qty)" = "$q0" ] \
    && ok "TC-17 the token rotated on the way out (pre-change token now rejected)" || ng "TC-17 rotation after direct DB change (probe $code)"
TS4=$(page_token $JL2 $BASE/mypage /tmp/csrf_my5.html)
code=$(cart_add $JL2 $TS4)
[ -n "$TS4" ] && [ "$TS4" != "$TS2" ] && [ "$code" != "403" ] \
    && ok "TC-17 token served after the direct change works" || ng "TC-17 post-change token ($code)"
db "UPDATE users SET email = 'csrf-moved-$RUN@example.jp' WHERE email_bidx = '$(bidx $EMAIL)'" >/dev/null
code=$(cart_add $JL2 $TS4)
q0=$(cart_qty)
code2=$(cart_add $JL2 $TS4)
[ "$code" != "403" ] && [ "$code2" = "403" ] && [ "$(cart_qty)" = "$q0" ] \
    && ok "TC-17 direct DB email change rotates too" || ng "TC-17 email-change rotation ($code/$code2, qty $(cart_qty)/$q0)"

result
