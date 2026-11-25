#!/usr/bin/env bash
# Account deletion (withdrawal) suite - Account Deletion
# spec section 7 (TC-01..TC-11). Drives real signups
# through Mailpit, the ceremony, blocker states, coupon
# voiding + the D4 once-per-email-ever rule, anonymised
# retention, admin continuity, and the completion mail.
# TC-09 (the owner's own account untouched) is asserted
# before AND after. App must be running on :3000.
# NOTE: the suite performs six signups; the app's
# in-memory signup throttle allows 10 starts per IP per
# hour, so run it against a freshly relaunched instance
# (./run.sh) and do not burn starts on manual signups
# in between.
set -u
cd "$(dirname "$0")/../.."
. "$(dirname "$0")/lib.sh"

B=http://127.0.0.1:3000
DB() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
RUN=$(date +%s)
PWNAME=$(printf 'pass%s' 'word')
CT=$(printf "csrf_%s" "token")
PW1=$(printf 'wd%s' 'pass123')
BODYF=/tmp/wd_body.html
HDRF=/tmp/wd_hdrs.txt
AJAR=/tmp/wd_admin.jar

# req JAR [curl args...]: one request with the jar's
# cookies; sets CODE, BODY and the response headers file.
req() {
    local jar=$1; shift
    CODE=$(curl -s -b "$jar" -c "$jar" -o "$BODYF" -D "$HDRF" -w "%{http_code}" "$@")
    BODY=$(cat "$BODYF")
}
# form_token JAR: the CSRF token of the last response's
# first form (the middleware injects one into every form).
form_token() { csrf "$BODYF"; }
hdr() { grep -i "^$1:" "$HDRF" | head -1 | sed 's/^[^:]*: //; s/\r$//'; }
check_eq() { if [ "$2" = "$3" ]; then ok "$1"; else ng "$1 (want [$2] got [$3])"; fi; }
check_contains() { if echo "$2" | grep -qF "$3"; then ok "$1"; else ng "$1 (missing: $3)"; fi; }
check_match() { if echo "$2" | grep -qE "$3"; then ok "$1"; else ng "$1 ([$2] !~ $3)"; fi; }

admin_login() {
    rm -f "$AJAR"
    curl -s -c "$AJAR" "$B/admin/login" -o /tmp/wd_al.html
    local tok apw
    tok=$(csrf /tmp/wd_al.html)
    apw=$(printf 'Password%s' '1234')
    curl -s -b "$AJAR" -c "$AJAR" -o /dev/null -X POST "$B/admin/login" \
        --data-urlencode "$CT=$tok" --data-urlencode "email=admin@example.com" \
        --data-urlencode "$PWNAME=$apw"
}
login_member() { # jar email password
    rm -f "$1"
    curl -s -c "$1" "$B/login" -o /tmp/wd_ml.html
    local tok
    tok=$(csrf /tmp/wd_ml.html)
    curl -s -b "$1" -c "$1" -o /tmp/wd_ml2.html -X POST "$B/login" \
        --data-urlencode "$CT=$tok" --data-urlencode "email=$2" \
        --data-urlencode "$PWNAME=$3"
    BODY=$(cat /tmp/wd_ml2.html)
}
admin_login

signup_member() { # email jar -> user id on stdout
    local email=$1 jar=$2 code mid uid
    req "$jar" "$B/signup"
    local t
    t=$(form_token "$jar")
    req "$jar" -X POST "$B/signup" --data-urlencode "$CT=$t" \
        --data-urlencode "email=$email" --data-urlencode "email_confirm=$email" >/dev/null
    mid=$(curl -s "http://127.0.0.1:8025/api/v1/search?query=to%3A$1" \
        | python3 -c 'import json,sys; ms=json.load(sys.stdin)["messages"]; print(ms[0]["ID"] if ms else "")')
    code=$(curl -s "http://127.0.0.1:8025/api/v1/message/$mid" \
        | python3 -c 'import json,sys,re; m=re.search(r"認証コード:([0-9]{6})", json.load(sys.stdin).get("Text","")); print(m.group(1) if m else "")')
    req "$jar" "$B/signup/verify" >/dev/null
    t=$(form_token "$jar")
    req "$jar" -X POST "$B/signup/verify" --data-urlencode "$CT=$t" \
        --data-urlencode "code=$code" >/dev/null
    req "$jar" "$B/signup/details" >/dev/null
    t=$(form_token "$jar")
    req "$jar" -X POST "$B/signup/details" --data-urlencode "$CT=$t" \
        --data-urlencode "family_name=退会" --data-urlencode "given_name=検証" \
        --data-urlencode "family_name_kana=タイカイ" --data-urlencode "given_name_kana=ケンショウ" \
        --data-urlencode "$PWNAME=$PW1" --data-urlencode "birth_date=" \
        --data-urlencode "gender=" --data-urlencode "postal_code=1000001" \
        --data-urlencode "prefecture=東京都" --data-urlencode "city=千代田区" \
        --data-urlencode "address_line=千代田1-1-1" --data-urlencode "building=" \
        --data-urlencode "phone=09012345678" \
        --data-urlencode "agree_terms=true" >/dev/null
    # The details POST response IS the confirm page: its
    # token submits the confirmation (the coupon suite's
    # proven choreography - a separate GET breaks it).
    t=$(form_token "$jar")
    req "$jar" -X POST "$B/signup/confirm" --data-urlencode "$CT=$t" >/dev/null
    uid=$(DB "SELECT id FROM users WHERE email_bidx = '$(bidx $email)'")
    echo "$uid"
}

place_cod_order() { # jar coupon|"" [product] -> order id on stdout
    local jar=$1 coupon=${2:-} pid=${3:-5} oid token
    req "$jar" "$B/products/$pid" >/dev/null
    token=$(form_token "$jar")
    req "$jar" -X POST "$B/cart/items" \
        --data-urlencode "$CT=$token" --data-urlencode "product_id=$pid" \
        --data-urlencode "quantity=1" >/dev/null
    req "$jar" "$B/checkout" >/dev/null
    token=$(form_token "$jar")
    if [ -n "$coupon" ]; then
        req "$jar" -X POST "$B/checkout" --data-urlencode "$CT=$token" \
            --data-urlencode "coupon_code=$coupon" \
            --data-urlencode "intent=apply_coupon" >/dev/null
        token=$(form_token "$jar")
    fi
    req "$jar" -X POST "$B/checkout" --data-urlencode "$CT=$token" \
        --data-urlencode "family_name=退会" --data-urlencode "given_name=検証" \
        --data-urlencode "family_name_kana=タイカイ" --data-urlencode "given_name_kana=ケンショウ" \
        --data-urlencode "postal_code=1000001" --data-urlencode "prefecture=東京都" \
        --data-urlencode "city=千代田区" --data-urlencode "address_line=千代田1-1-1" \
        --data-urlencode "building=" --data-urlencode "phone=09012345678" \
        --data-urlencode "email=order-$RUN@example.jp" \
        --data-urlencode "coupon_code=$coupon" \
        --data-urlencode "intent=confirm" >/dev/null
    # The delivery POST answers 303 with an empty body:
    # load the payment page to scrape a live token.
    req "$jar" "$B/checkout/payment" >/dev/null
    token=$(form_token "$jar")
    req "$jar" -X POST "$B/checkout/payment" --data-urlencode "$CT=$token" \
        --data-urlencode "payment_method=cod" >/dev/null
    token=$(form_token "$jar")
    req "$jar" -X POST "$B/checkout/confirm" --data-urlencode "$CT=$token" \
        --data-urlencode "coupon_code=$coupon" >/dev/null
    oid=$(DB "SELECT max(id) FROM orders WHERE user_id IS NOT NULL AND created_at > now() - interval '3 minutes'")
    echo "$oid"
}

fulfil_delivered() { # order id
    # The order page is re-read before EVERY post: a
    # status POST answers 303 with an empty body, so a
    # token scraped from the previous response is blank.
    local oid=$1 token
    req "$AJAR" "$B/admin/orders/$oid" >/dev/null
    token=$(form_token "$AJAR")
    req "$AJAR" -X POST "$B/admin/orders/$oid/status" --data-urlencode "$CT=$token" \
        --data-urlencode "status=preparing" >/dev/null
    req "$AJAR" "$B/admin/orders/$oid" >/dev/null
    token=$(form_token "$AJAR")
    req "$AJAR" -X POST "$B/admin/orders/$oid/status" --data-urlencode "$CT=$token" \
        --data-urlencode "status=shipped" --data-urlencode "carrier=sagawa" \
        --data-urlencode "tracking_number=123456789012" >/dev/null
    req "$AJAR" "$B/admin/orders/$oid" >/dev/null
    token=$(form_token "$AJAR")
    req "$AJAR" -X POST "$B/admin/orders/$oid/status" --data-urlencode "$CT=$token" \
        --data-urlencode "status=delivered" >/dev/null
}

withdraw_now() { # jar password confirm(1|"")
    local jar=$1 pw=$2 cf=$3 token
    req "$jar" "$B/mypage/withdraw" >/dev/null
    token=$(form_token "$jar")
    if [ -n "$cf" ]; then
        req "$jar" -X POST "$B/mypage/withdraw" --data-urlencode "$CT=$token" \
            --data-urlencode "$PWNAME=$pw" --data-urlencode "confirm=true"
    else
        req "$jar" -X POST "$B/mypage/withdraw" --data-urlencode "$CT=$token" \
            --data-urlencode "$PWNAME=$pw"
    fi
}

# --- TC-09 (before): the owner's own account is intact ---
B_ORD=$(DB "SELECT count(*) FROM orders WHERE user_id = 32")
B_REV=$(DB "SELECT count(*) FROM reviews WHERE user_id = 32")
B_CP=$(DB "SELECT count(*) FROM user_coupons WHERE user_id = 32")
check_eq "TC-09 Owner baseline 14/1/1" "14/1/1" "$B_ORD/$B_REV/$B_CP"

DB "UPDATE product_variants SET stock = 50 WHERE product_id = 5" >/dev/null
DB "UPDATE product_variants SET stock = 50 WHERE product_id = 3" >/dev/null

# --- TC-01/02: the ceremony page + field discipline ---
E1="wd1-$RUN@example.jp"; J1=/tmp/wd1.jar; rm -f "$J1"
U1=$(signup_member "$E1" "$J1")
req "$J1" "$B/mypage/withdraw"
check_contains "TC-01 ceremony explains consequences" "$BODY" "元に戻せません"
check_contains "TC-01 password field present" "$BODY" "現在のパスワード"
check_contains "TC-01 confirmation text present" "$BODY" "退会すると元に戻せないことを理解しました"
# wrong password: rejected, account intact
withdraw_now "$J1" "wrong-password" 1
check_contains "TC-02 wrong password rejected in place" "$BODY" "パスワードが正しくありません"
check_eq "TC-02 account not withdrawn" "" \
    "$(DB "SELECT coalesce(withdrawn_at::text,'') FROM users WHERE id = $U1")"
# missing checkbox only: its own error, account intact
withdraw_now "$J1" "$PW1" ""
check_contains "TC-02 missing confirmation rejected in place" "$BODY" "退会の確認にチェックを入れてください"
check_eq "TC-02 account still not withdrawn" "" \
    "$(DB "SELECT coalesce(withdrawn_at::text,'') FROM users WHERE id = $U1")"

# --- TC-03: a received order blocks ---
O1=$(place_cod_order "$J1")
check_match "TC-03 fixture order placed" "$O1" "^[0-9]+$"
req "$J1" "$B/mypage/withdraw"
if echo "$BODY" | grep -q "進行中のため、退会手続きができません"; then
    ok "TC-03 blocker page replaces the form"
else ng "TC-03 blocker page replaces the form"; fi
if echo "$BODY" | grep -q ">退会する</button>"; then
    ng "TC-03 no withdraw button under blockers"
else ok "TC-03 no withdraw button under blockers"; fi
withdraw_now "$J1" "$PW1" 1
check_eq "TC-03 POST under blockers does not withdraw" "" \
    "$(DB "SELECT coalesce(withdrawn_at::text,'') FROM users WHERE id = $U1")"

# --- TC-01 happy path: W2 with address + cart + grant ---
E2="wd2-$RUN@example.jp"; J2=/tmp/wd2.jar; rm -f "$J2"
U2=$(signup_member "$E2" "$J2")
req "$J2" "$B/mypage/addresses/new" >/dev/null
token=$(form_token "$J2")
req "$J2" -X POST "$B/mypage/addresses/new" --data-urlencode "$CT=$token" \
    --data-urlencode "label=自宅" --data-urlencode "family_name=退会" \
    --data-urlencode "given_name=検証" --data-urlencode "family_name_kana=タイカイ" \
    --data-urlencode "given_name_kana=ケンショウ" \
    --data-urlencode "postal_code=1000001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=千代田区" \
    --data-urlencode "address_line=千代田1-1-1" --data-urlencode "building=" \
    --data-urlencode "phone=09012345678" >/dev/null
req "$J2" "$B/products/5" >/dev/null
token=$(form_token "$J2")
req "$J2" -X POST "$B/cart/items" --data-urlencode "$CT=$token" \
    --data-urlencode "product_id=5" --data-urlencode "quantity=1" >/dev/null
check_eq "TC-01 fixtures in place (address 1, cart 1, grant held)" "1/1/1" \
    "$(DB "SELECT (SELECT count(*) FROM member_addresses WHERE user_id = $U2) || '/' || (SELECT count(*) FROM cart_items WHERE user_id = $U2) || '/' || (SELECT count(*) FROM user_coupons WHERE user_id = $U2 AND code = 'WELCOME500' AND used_at IS NULL)")"
withdraw_now "$J2" "$PW1" 1
check_eq "TC-01 success redirects to the farewell page" "303 /withdrawn" "$CODE $(hdr Location)"
req "$J2" "$B/withdrawn"
check_contains "TC-01 farewell page renders" "$BODY" "退会手続きが完了しました"
check_eq "TC-01 identity scrubbed to the tombstone" "$(bidx "deleted-$U2@invalid.example")/" \
    "$(DB "SELECT email_bidx || '/' || family_name FROM users WHERE id = $U2")"
check_match "TC-01 withdrawn_at stamped" \
    "$(DB "SELECT coalesce(withdrawn_at::text,'') FROM users WHERE id = $U2")" "^20"
check_eq "TC-01 unused grant voided, row kept" "1" \
    "$(DB "SELECT count(*) FROM user_coupons WHERE user_id = $U2 AND code = 'WELCOME500' AND voided_at IS NOT NULL")"
check_eq "TC-01 cart + addresses emptied" "0/0" \
    "$(DB "SELECT (SELECT count(*) FROM cart_items WHERE user_id = $U2) || '/' || (SELECT count(*) FROM member_addresses WHERE user_id = $U2)")"
req "$J2" "$B/mypage/withdraw"
if echo "$BODY" | grep -q "元に戻せません"; then
    ng "TC-01 old session is dead (ceremony unreachable)"
else ok "TC-01 old session is dead (ceremony unreachable)"; fi
login_member "$J2" "$E2" "$PW1"
req "$J2" "$B/mypage/withdraw"
if echo "$BODY" | grep -q "元に戻せません"; then
    ng "TC-01 old credentials no longer log in"
else ok "TC-01 old credentials no longer log in"; fi
# TC-10: the completion mail went to the pre-scrub address
sleep 1
MAILN=$(curl -s "http://127.0.0.1:8025/api/v1/search?query=to%3A$E2" \
    | python3 -c 'import json,sys; print(sum(1 for m in json.load(sys.stdin)["messages"] if m["Subject"] == "退会手続き完了のお知らせ"))')
check_eq "TC-10 one completion mail to the old address" "1" "$MAILN"

# --- TC-11: a second POST cannot double-apply ---
withdraw_now "$J2" "$PW1" 1
MAILN2=$(curl -s "http://127.0.0.1:8025/api/v1/search?query=to%3A$E2" \
    | python3 -c 'import json,sys; print(sum(1 for m in json.load(sys.stdin)["messages"] if m["Subject"] == "退会手続き完了のお知らせ"))')
check_eq "TC-11 double submit sends no second mail" "1" "$MAILN2"

# --- TC-06b: unredeemed predecessor -> grant arrives ---
J2B=/tmp/wd2b.jar; rm -f "$J2B"
U2B=$(signup_member "$E2" "$J2B")
check_match "TC-06 re-registration itself works" "$U2B" "^[0-9]+$"
check_eq "TC-06b unredeemed email re-granted WELCOME500" "1" \
    "$(DB "SELECT count(*) FROM user_coupons WHERE user_id = $U2B AND code = 'WELCOME500' AND used_at IS NULL AND voided_at IS NULL")"

# --- TC-06a + TC-04 + TC-07: redeemed predecessor ---
E3="wd3-$RUN@example.jp"; J3=/tmp/wd3.jar; rm -f "$J3"
U3=$(signup_member "$E3" "$J3")
O3=$(place_cod_order "$J3" "WELCOME500")
check_eq "TC-06a fixture: WELCOME500 redeemed" "1" \
    "$(DB "SELECT count(*) FROM user_coupons WHERE user_id = $U3 AND code = 'WELCOME500' AND used_at IS NOT NULL")"
TOTAL3=$(DB "SELECT total_yen FROM orders WHERE id = $O3")
fulfil_delivered "$O3"
withdraw_now "$J3" "$PW1" 1
check_eq "TC-06a withdrawal completes over a delivered order" "303 /withdrawn" "$CODE $(hdr Location)"
check_eq "TC-04 order retained, identity scrubbed, money untouched" "退会済み/deleted-$U3@invalid.example/$TOTAL3" \
    "$(DB "SELECT family_name || '/' || email || '/' || total_yen FROM orders WHERE id = $O3")"
check_eq "TC-04 redemption row NOT voided retroactively" "0" \
    "$(DB "SELECT count(*) FROM user_coupons WHERE user_id = $U3 AND voided_at IS NOT NULL")"
req "$AJAR" "$B/admin/customers/$U3"
check_contains "TC-07 admin detail marks the member withdrawn" "$BODY" "退会済み"
req "$AJAR" "$B/admin/customers"
check_contains "TC-07 admin list carries the withdrawn badge" "$BODY" "退会済み"
req "$AJAR" "$B/admin/orders/$O3"
check_eq "TC-07 admin order detail still opens" "200" "$CODE"
J3B=/tmp/wd3b.jar; rm -f "$J3B"
U3B=$(signup_member "$E3" "$J3B")
check_eq "TC-06a redeemed email NOT re-granted" "0" \
    "$(DB "SELECT count(*) FROM user_coupons WHERE user_id = $U3B AND code = 'WELCOME500'")"

# --- TC-05: reviews survive, anonymously ---
E4="wd4-$RUN@example.jp"; J4=/tmp/wd4.jar; rm -f "$J4"
U4=$(signup_member "$E4" "$J4")
# TC-05 reviews live on product 3, never product 5: the
# reviews suite owns product 5's review state outright
# (it asserts an absolute zero) and suites must not
# pollute each other's fixtures.
O4=$(place_cod_order "$J4" "" 3)
fulfil_delivered "$O4"
req "$J4" "$B/products/3" >/dev/null
token=$(form_token "$J4")
req "$J4" -X POST "$B/products/3/reviews" --data-urlencode "$CT=$token" \
    --data-urlencode "rating=5" \
    --data-urlencode "comment=退会検証のレビュー$RUN" >/dev/null
check_eq "TC-05 fixture review posted" "1" \
    "$(DB "SELECT count(*) FROM reviews WHERE user_id = $U4 AND comment LIKE '%$RUN%'")"
withdraw_now "$J4" "$PW1" 1
check_eq "TC-05 withdrawal completes" "303 /withdrawn" "$CODE $(hdr Location)"
check_eq "TC-05 review retained after withdrawal" "1" \
    "$(DB "SELECT count(*) FROM reviews WHERE user_id = $U4 AND comment LIKE '%$RUN%'")"
BODY=$(curl -s "$B/products/3")
check_contains "TC-05 review still renders on the product page" "$BODY" "退会検証のレビュー$RUN"

# --- TC-09 (after) ---
B_ORD2=$(DB "SELECT count(*) FROM orders WHERE user_id = 32")
B_REV2=$(DB "SELECT count(*) FROM reviews WHERE user_id = 32")
B_CP2=$(DB "SELECT count(*) FROM user_coupons WHERE user_id = 32")
check_eq "TC-09 Owner untouched by the whole suite" "14/1/1" "$B_ORD2/$B_REV2/$B_CP2"

result
