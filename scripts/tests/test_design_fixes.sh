#!/bin/bash
# Smoke tests for the design-review fixes (run against localhost:3000).
BASE=http://127.0.0.1:3000
J=/tmp/smoke_cookies.txt
rm -f $J
# Shared harness: csrf() comes from lib.sh; ok() below is
# this suite's own (it takes an exit code) and overrides.
. "$(dirname "$0")/lib.sh"
pass=0; fail=0
ok() { if [ "$1" = "0" ]; then echo "  OK  $2"; pass=$((pass+1)); else echo "  NG  $2"; fail=$((fail+1)); fi }

echo "-- D4 fallback 404"
code=$(curl -s -o /tmp/sm_404.html -w "%{http_code}" $BASE/nonexistent/deep)
grep -q "ページが見つかりませんでした" /tmp/sm_404.html && grep -q "site-header" /tmp/sm_404.html; g=$?
[ "$code" = "404" ] && [ $g = 0 ]; ok $? "styled 404 for unknown URL (status $code)"

echo "-- D6 info pages"
for p in guide terms privacy legal; do
  code=$(curl -s -o /tmp/sm_$p.html -w "%{http_code}" $BASE/$p)
  [ "$code" = "200" ] && grep -q "<title>Agentic Store |" /tmp/sm_$p.html; ok $? "/$p 200 with its own title"
done

echo "-- D1a guest cart note + D5 suggest roles"
curl -s -c $J $BASE/ -o /tmp/sm_home.html
T=$(csrf /tmp/sm_home.html)
curl -s -b $J -c $J -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$T" -o /dev/null
curl -s -b $J $BASE/cart -o /tmp/sm_cart.html
grep -q "ゲストのカートは" /tmp/sm_cart.html; ok $? "guest note on cart page"
# test_search exhausts the 30/60s suggest budget on purpose
# (same IP), so wait out the limiter window if it is still hot.
for i in 1 2 3 4; do
  curl -s -b $J "$BASE/search/suggest?q=%E3%83%9E%E3%82%B0" -o /tmp/sm_sug.html
  grep -q 'role="listbox"' /tmp/sm_sug.html && break
  sleep 20
done
grep -q 'role="listbox"' /tmp/sm_sug.html && grep -q 'role="option"' /tmp/sm_sug.html; ok $? "suggest fragment has listbox/option roles"

echo "-- login as test member"
curl -s -b $J -c $J $BASE/login -o /tmp/sm_login.html
T=$(csrf /tmp/sm_login.html)
curl -s -b $J -c $J -X POST $BASE/login --data-urlencode "email=test@example.com" --data-urlencode "password=test1234" --data-urlencode "csrf_token=$T" --data-urlencode "next=" -o /dev/null -w "%{http_code}" > /tmp/sm_code.txt
curl -s -b $J $BASE/mypage -o /tmp/sm_my.html
grep -q "会員情報" /tmp/sm_my.html && grep -q "パスワード変更" /tmp/sm_my.html; ok $? "mypage hub shows profile + password items"

echo "-- order detail (order inserted directly for the test member)"
UID_=$(docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "SELECT id FROM users WHERE email_bidx='$(bidx test@example.com)';")
OID=$(docker exec agentic-db psql -U agentic -d agentic_commerce -q -t -A -c "INSERT INTO orders (user_id, email, family_name, given_name, postal_code, prefecture, city, address_line, building, phone, subtotal_yen, discount_yen, shipping_yen, cod_fee_yen, total_yen, coupon_code, status) VALUES ($UID_, 'test@example.com', '山田', '太郎', '1500001', '東京都', '渋谷区', '神南1-1-1', NULL, '09012345678', 2150, 500, 550, 330, 2530, 'WELCOME500', 'received') RETURNING id;")
docker exec agentic-db psql -U agentic -d agentic_commerce -q -c "INSERT INTO order_items (order_id, product_id, product_name, unit_price_yen, quantity) VALUES ($OID, 2, 'Axum マグカップ', 1650, 1), ($OID, 5, 'SQLx キーホルダー', 500, 1);"
curl -s -b $J $BASE/mypage/orders/$OID -o /tmp/sm_od.html
grep -q "ご注文内容" /tmp/sm_od.html && grep -q "クーポン(WELCOME500)" /tmp/sm_od.html && grep -q "注文確認メール送信先" /tmp/sm_od.html && grep -q "2026年" /tmp/sm_od.html; ok $? "order detail renders (lines, coupon row, long date)"
code=$(curl -s -o /tmp/sm_od404.html -w "%{http_code}" -b $J $BASE/mypage/orders/999999)
[ "$code" = "404" ] && grep -q "注文が見つかりませんでした" /tmp/sm_od404.html; ok $? "missing order -> styled 404"
code=$(curl -s -o /tmp/sm_odabc.html -w "%{http_code}" -b $J $BASE/mypage/orders/abc)
[ "$code" = "404" ] && grep -q "注文が見つかりませんでした" /tmp/sm_odabc.html; ok $? "non-numeric order id -> same 404"
GID=$(docker exec agentic-db psql -U agentic -d agentic_commerce -q -t -A -c "INSERT INTO orders (user_id, email, family_name, given_name, postal_code, prefecture, city, address_line, building, phone, subtotal_yen, discount_yen, shipping_yen, cod_fee_yen, total_yen, status) VALUES (NULL, 'guest@example.jp', '佐藤', '花', '1000001', '東京都', '千代田区', '丸の内1-1', NULL, '08011112222', 500, 0, 0, 330, 830, 'received') RETURNING id;")
code=$(curl -s -o /dev/null -w "%{http_code}" -b $J $BASE/mypage/orders/$GID)
[ "$code" = "404" ]; ok $? "guest order is not visible to a member (404)"

echo "-- profile edit"
curl -s -b $J $BASE/mypage/edit -o /tmp/sm_edit.html
T=$(csrf /tmp/sm_edit.html)
post_edit() { curl -s -b $J $BASE/mypage/edit -o /tmp/sm_edit.html; T=$(csrf /tmp/sm_edit.html); curl -s -b $J -c $J -X POST $BASE/mypage/edit --data-urlencode "form_ticket=$(ticket /tmp/sm_edit.html)" --data-urlencode "csrf_token=$T" --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" --data-urlencode "family_name_kana=ヤマダ" --data-urlencode "given_name_kana=タロウ" --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=$1" --data-urlencode "address_line=神南1-1-1" --data-urlencode "phone=$2" -o /tmp/sm_edit_r.html; }
post_edit "渋谷区" "123"
grep -q "電話番号は10桁または11桁" /tmp/sm_edit_r.html; ok $? "invalid phone shows its field error"
post_edit "渋谷区桜丘町2-2" "09012345678"
grep -q "会員情報を更新しました" /tmp/sm_edit_r.html; ok $? "valid edit saves with notice"
CITY=$(pii_dec "$(docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "SELECT city FROM users WHERE id=$UID_;")")
[ "$CITY" = "渋谷区桜丘町2-2" ]; ok $? "new city stored in DB"

echo "-- password change"
curl -s -b $J $BASE/mypage/password -o /tmp/sm_pw.html
T=$(csrf /tmp/sm_pw.html)
curl -s -b $J -X POST $BASE/mypage/password --data-urlencode "csrf_token=$T" --data-urlencode "current_password=wrongpass1" --data-urlencode "new_password=newpass123" --data-urlencode "new_password_confirm=newpass123" -o /tmp/sm_pw_r.html
grep -q "現在のパスワードが正しくありません" /tmp/sm_pw_r.html; ok $? "wrong current password refused"
curl -s -b $J -c $J -X POST $BASE/mypage/password --data-urlencode "csrf_token=$T" --data-urlencode "current_password=test1234" --data-urlencode "new_password=newpass123" --data-urlencode "new_password_confirm=newpass123" -o /tmp/sm_pw_r2.html
grep -q "パスワードを変更しました" /tmp/sm_pw_r2.html; ok $? "password changed"
# change it back so the dev account stays test1234
curl -s -b $J $BASE/mypage/password -o /tmp/sm_pw.html
T=$(csrf /tmp/sm_pw.html)
curl -s -b $J -X POST $BASE/mypage/password --data-urlencode "csrf_token=$T" --data-urlencode "current_password=newpass123" --data-urlencode "new_password=test1234" --data-urlencode "new_password_confirm=test1234" -o /tmp/sm_pw_r3.html
grep -q "パスワードを変更しました" /tmp/sm_pw_r3.html; ok $? "password restored to test1234"

echo "-- log identity fields"
LOG=$(ls -t logs/front_app.log.* 2>/dev/null | head -1)
grep -q "session_fp=" "$LOG" && grep -q "user=" "$LOG"; ok $? "request lines carry user + session_fp ($LOG)"
RAW=$(grep -o "id=[^;]*" $J | head -1 | cut -d= -f2-)
if [ -n "$RAW" ]; then
  grep -q "$RAW" "$LOG" && ok 1 "raw session id must NOT appear in log" || ok 0 "raw session id absent from log"
fi
grep "user=$UID_" "$LOG" | tail -1 | grep -q "session_fp=[0-9a-f]\{12\}"; ok $? "member line shows user id + 12-hex fingerprint"

echo "RESULT: $pass passed, $fail failed"
