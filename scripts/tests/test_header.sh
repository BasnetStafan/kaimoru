#!/bin/bash
# Verifies the new header as guest and as member.
BASE=http://127.0.0.1:3000
# Shared harness: ok/ng, csrf() and result() live in lib.sh (ST-6).
. "$(dirname "$0")/lib.sh"

J=/tmp/header_jar.txt; rm -f $J
echo "-- guest header"
curl -s -c $J $BASE/ -o /tmp/h_guest.html
grep -q '<a href="/" class="site-logo">AGENTIC STORE</a>' /tmp/h_guest.html && ok "logo links to top page" || ng "logo links to top page"
grep -q '>トップ</a>' /tmp/h_guest.html && ng "no Top link in header" || ok "no Top link in header"
grep -q 'class="nav-button">新規会員登録' /tmp/h_guest.html && ok "signup is a pill button" || ng "signup is a pill button"

echo "-- member header"
curl -s -b $J -c $J $BASE/login -o /tmp/h_login.html
CSRF=$(csrf /tmp/h_login.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/login --data-urlencode "email=test@example.com" --data-urlencode "password=test1234" --data-urlencode "csrf_token=$CSRF"
curl -s -b $J $BASE/ -o /tmp/h_member.html
grep -A1 'aria-label="マイページ"' /tmp/h_member.html | grep -q '<svg' && ok "mypage has SVG icon" || ng "mypage has SVG icon"
grep -A1 'aria-label="ログアウト"' /tmp/h_member.html | grep -q '<svg' && ok "logout has SVG icon" || ng "logout has SVG icon"
grep -q '>トップ</a>' /tmp/h_member.html && ng "no Top link for member" || ok "no Top link for member"
grep -c '<svg class="nav-icon"' /tmp/h_member.html

echo "-- css served"
curl -s $BASE/static/css/style.css -o /tmp/h_css.css
grep -q "position: sticky" /tmp/h_css.css && ok "sticky header css served" || ng "sticky header css served"
grep -q "backdrop-filter" /tmp/h_css.css && ok "blur css served" || ng "blur css served"

result
