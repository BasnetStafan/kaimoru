#!/bin/bash
# Verifies: hover CSS served, required badges, per-field validation errors.
BASE=http://127.0.0.1:3000
# Shared harness: ok/ng, csrf() and result() live in lib.sh (ST-6).
. "$(dirname "$0")/lib.sh"
has() { grep -q "$2" "$3" && ok "$1" || ng "$1"; }
count_is() { local n=$(grep -o "$2" "$3" | wc -l); [ "$n" = "$4" ] && ok "$1 ($n)" || ng "$1 (want $4 got $n)"; }

J=/tmp/val_jar.txt; rm -f $J
EMAIL="val-test@example.com"

echo "-- hover css"
curl -s $BASE/static/css/style.css -o /tmp/v_css.css
grep -A2 ".nav-link:hover" /tmp/v_css.css | grep -q "background-color: var(--accent)" && ok "nav hover is filled teal" || ng "nav hover is filled teal"

echo "-- login badges"
curl -s -c $J $BASE/login -o /tmp/v_login.html
count_is "login required badges" "badge-required" /tmp/v_login.html 2

echo "-- signup email step badges"
curl -s -b $J -c $J $BASE/signup -o /tmp/v_su.html
count_is "signup email badges" "badge-required" /tmp/v_su.html 2
CSRF=$(csrf /tmp/v_su.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/signup --data-urlencode "email=$EMAIL" --data-urlencode "email_confirm=$EMAIL" --data-urlencode "csrf_token=$CSRF"

echo "-- read code from Mailpit"
sleep 1
CODE=$(curl -s "http://127.0.0.1:8025/api/v1/search?query=$EMAIL" | grep -o '"Snippet":"[^"]*"' | grep -o '[0-9]\{6\}' | head -1)
[ -n "$CODE" ] && ok "code read from mail ($CODE)" || { ng "code read from mail"; echo "RESULT: $PASS passed, $FAIL failed (blocked)"; exit 1; }

curl -s -b $J -c $J $BASE/signup/verify -o /tmp/v_vf.html
count_is "verify required badge" "badge-required" /tmp/v_vf.html 1
CSRF=$(csrf /tmp/v_vf.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/signup/verify --data-urlencode "code=$CODE" --data-urlencode "csrf_token=$CSRF"

echo "-- details form: badges"
curl -s -b $J -c $J $BASE/signup/details -o /tmp/v_det.html
count_is "details required badges" "badge-required" /tmp/v_det.html 10
count_is "details optional badges" "badge-optional" /tmp/v_det.html 3

echo "-- details form: submit everything wrong, expect all field errors at once"
CSRF=$(csrf /tmp/v_det.html)
curl -s -b $J -c $J -X POST $BASE/signup/details -o /tmp/v_err.html \
  --data-urlencode "password=abc" \
  --data-urlencode "family_name=" --data-urlencode "given_name=" \
  --data-urlencode "family_name_kana=やまだ" --data-urlencode "given_name_kana=たろう" \
  --data-urlencode "birth_date=" --data-urlencode "gender=" \
  --data-urlencode "postal_code=123" --data-urlencode "prefecture=" \
  --data-urlencode "city=" --data-urlencode "address_line=" --data-urlencode "building=" \
  --data-urlencode "phone=123" --data-urlencode "agree_terms=true" \
  --data-urlencode "csrf_token=$CSRF"
count_is "field errors shown at once" "field-error" /tmp/v_err.html 10
has "password error text" "パスワードは8文字以上32文字以内" /tmp/v_err.html
has "kana error text" "セイは全角カタカナで入力してください" /tmp/v_err.html
has "postal error text" "郵便番号は7桁の数字で入力してください" /tmp/v_err.html
has "prefecture error text" "都道府県を選択してください" /tmp/v_err.html
has "phone error text" "電話番号は10桁または11桁の数字で入力してください" /tmp/v_err.html

result
