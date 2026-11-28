#!/bin/bash
# Happy-path regression after the validation refactor.
BASE=http://127.0.0.1:3000
# Shared harness: ok/ng, csrf() and result() live in lib.sh (ST-6).
. "$(dirname "$0")/lib.sh"

J=/tmp/happy_jar.txt; rm -f $J
EMAIL="happy-test@example.com"
# Rerunnable: a leftover fixture account from an earlier run
# makes sign-up completion refuse the duplicate email.
docker exec agentic-db psql -U agentic -d agentic_commerce -q -c "DELETE FROM user_coupons WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $EMAIL)'); DELETE FROM users WHERE email_bidx = '$(bidx $EMAIL)';" >/dev/null 2>&1
curl -s -c $J $BASE/signup -o /tmp/hp_su.html
CSRF=$(csrf /tmp/hp_su.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/signup --data-urlencode "email=$EMAIL" --data-urlencode "email_confirm=$EMAIL" --data-urlencode "csrf_token=$CSRF"
sleep 1
CODE=$(curl -s "http://127.0.0.1:8025/api/v1/search?query=$EMAIL" | grep -o '"Snippet":"[^"]*"' | grep -o '[0-9]\{6\}' | head -1)
[ -n "$CODE" ] && ok "code received" || { ng "code received"; echo "RESULT: blocked"; exit 1; }
curl -s -b $J -c $J $BASE/signup/verify -o /tmp/hp_vf.html
CSRF=$(csrf /tmp/hp_vf.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/signup/verify --data-urlencode "code=$CODE" --data-urlencode "csrf_token=$CSRF"
curl -s -b $J -c $J $BASE/signup/details -o /tmp/hp_det.html
CSRF=$(csrf /tmp/hp_det.html)
curl -s -b $J -c $J -X POST $BASE/signup/details -o /tmp/hp_conf.html \
  --data-urlencode "password=test1234" \
  --data-urlencode "family_name=鈴木" --data-urlencode "given_name=花子" \
  --data-urlencode "family_name_kana=スズキ" --data-urlencode "given_name_kana=ハナコ" \
  --data-urlencode "birth_date=" --data-urlencode "gender=女性" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" \
  --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前1-1-1" --data-urlencode "building=" \
  --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
  --data-urlencode "csrf_token=$CSRF"
grep -q "入力内容の確認" /tmp/hp_conf.html && ok "valid details reach confirmation" || ng "valid details reach confirmation"
grep -q "field-error" /tmp/hp_conf.html && ng "no field errors on valid input" || ok "no field errors on valid input"
CSRF=$(csrf /tmp/hp_conf.html)
BODY=$(curl -s -b $J -c $J -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/confirm --data-urlencode "csrf_token=$CSRF")
[ "$BODY" = "303 $BASE/" ] && ok "completion redirects to top" || ng "completion redirects to top ($BODY)"
curl -s -b $J $BASE/ | grep -q "ようこそ、鈴木さん" && ok "member header after signup" || ng "member header after signup"
result
