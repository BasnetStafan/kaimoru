#!/bin/bash
# Stage E verification: login cart merge + sign-up cart re-key.
BASE=http://127.0.0.1:3000
MAIL=http://127.0.0.1:8025
# Shared harness (echo-only variant): lib.sh (ST-6).
SUITE_NO_COUNT=1
. "$(dirname "$0")/lib.sh"
add() { # jar csrf product qty
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$3" --data-urlencode "quantity=$4" --data-urlencode "csrf_token=$2"
}
login() { # jar
  curl -s -c "$1" $BASE/login -o /tmp/te_login.html
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/login --data-urlencode "email=test@example.com" --data-urlencode "password=test1234" --data-urlencode "csrf_token=$(csrf /tmp/te_login.html)"
}
qty_of() { # jar product -> prints qty-value next to product name in cart page (rough: whole page grep)
  curl -s -b "$1" $BASE/cart -o /tmp/te_cart.html
}

echo "===== LOGIN MERGE ====="
# Deterministic premise: the fixture member starts with an
# empty cart, whatever earlier suites left behind.
docker exec agentic-db psql -U agentic -d agentic_commerce -q -c "DELETE FROM cart_items WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx test@example.com)');" >/dev/null 2>&1
JM=/tmp/te_m.txt; rm -f $JM
login $JM
curl -s -b $JM $BASE/ -o /tmp/te_h.html; add $JM "$(csrf /tmp/te_h.html)" 5 1
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/logout --data-urlencode "csrf_token=$(csrf /tmp/te_h.html)"
echo "-- member had p5 x1; now guest adds p5 x2 + p2 x1"
JG=/tmp/te_g.txt; rm -f $JG
curl -s -c $JG $BASE/ -o /tmp/te_h.html
add $JG "$(csrf /tmp/te_h.html)" 5 2
add $JG "$(csrf /tmp/te_h.html)" 2 1
# log in through the guest jar (same session becomes member)
curl -s -b $JG -c $JG $BASE/login -o /tmp/te_login2.html
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/login --data-urlencode "email=test@example.com" --data-urlencode "password=test1234" --data-urlencode "csrf_token=$(csrf /tmp/te_login2.html)"
qty_of $JG
grep -q "SQLx キーホルダー" /tmp/te_cart.html && ok "merged cart has p5" || ng "merged cart has p5"
grep -q "Axum マグカップ" /tmp/te_cart.html && ok "merged cart has p2" || ng "merged cart has p2"
grep -o '<span class="cart-badge" id="cart-count"[^>]*>[0-9]*' /tmp/te_cart.html | grep -q ">4" && ok "badge = 4 (3+1 merged)" || ng "badge after merge (want 4)"
python3 - << 'PYEOF'
import re, pathlib
html = pathlib.Path("/tmp/te_cart.html").read_text()
rows = re.findall(r"(SQLx キーホルダー|Axum マグカップ).*?qty-value\">(\d+)", html, re.S)
d = dict(rows)
print("  quantities:", d)
assert d.get("SQLx キーホルダー") == "3", d
assert d.get("Axum マグカップ") == "1", d
print("  OK  merge sums quantities (p5: 1+2=3)")
PYEOF

echo "===== SIGN-UP RE-KEY ====="
JS=/tmp/te_s.txt; rm -f $JS
curl -s -c $JS $BASE/ -o /tmp/te_h.html
add $JS "$(csrf /tmp/te_h.html)" 1 1
curl -s -b $JS $BASE/signup -o /tmp/te_su.html
curl -s -b $JS -c $JS -o /dev/null -X POST $BASE/signup --data-urlencode "email=cart-test@example.jp" --data-urlencode "email_confirm=cart-test@example.jp" --data-urlencode "csrf_token=$(csrf /tmp/te_su.html)"
sleep 1
CODE=$(curl -s "$MAIL/api/v1/messages" | python3 -c "import json,sys,re; msgs=[m for m in json.load(sys.stdin)['messages'] if any(t['Address']=='cart-test@example.jp' for t in m['To'])]; body=msgs[0]['Snippet'] if msgs else ''; print(re.search(r'(\d{6})', body).group(1) if re.search(r'(\d{6})', body) else '')")
MID=$(curl -s "$MAIL/api/v1/messages" | python3 -c "import json,sys; msgs=[m for m in json.load(sys.stdin)['messages'] if any(t['Address']=='cart-test@example.jp' for t in m['To'])]; print(msgs[0]['ID'] if msgs else '')")
if [ -z "$CODE" ]; then CODE=$(curl -s "$MAIL/api/v1/message/$MID" | python3 -c "import json,sys,re; print(re.search(r'認証コード:(\d{6})', json.load(sys.stdin).get('Text','')).group(1))"); fi
echo "  code read: ${CODE:0:1}*****"
curl -s -b $JS -c $JS $BASE/signup/verify -o /tmp/te_v.html
curl -s -b $JS -c $JS -o /dev/null -X POST $BASE/signup/verify --data-urlencode "code=$CODE" --data-urlencode "csrf_token=$(csrf /tmp/te_v.html)"
curl -s -b $JS $BASE/signup/details -o /tmp/te_d.html
curl -s -b $JS -c $JS -X POST $BASE/signup/details \
  --data-urlencode "password=abc12345" --data-urlencode "family_name=鈴木" --data-urlencode "given_name=一郎" \
  --data-urlencode "family_name_kana=スズキ" --data-urlencode "given_name_kana=イチロウ" \
  --data-urlencode "birth_date=" --data-urlencode "gender=" \
  --data-urlencode "postal_code=1000001" --data-urlencode "prefecture=東京都" --data-urlencode "city=千代田区" \
  --data-urlencode "address_line=千代田1-1" --data-urlencode "building=" --data-urlencode "phone=09033334444" \
  --data-urlencode "agree_terms=true" --data-urlencode "csrf_token=$(csrf /tmp/te_d.html)" -o /tmp/te_conf.html
grep -q "入力内容の確認" /tmp/te_conf.html && ok "signup reached confirmation" || ng "signup confirmation"
curl -s -b $JS -c $JS -o /dev/null -X POST $BASE/signup/confirm --data-urlencode "csrf_token=$(csrf /tmp/te_conf.html)"
curl -s -b $JS $BASE/cart -o /tmp/te_scart.html
grep -q "Rust Tシャツ" /tmp/te_scart.html && ok "new member cart kept guest item (re-key)" || ng "re-key failed"
grep -o '<span class="cart-badge" id="cart-count"[^>]*>[0-9]*' /tmp/te_scart.html | grep -q ">1" && ok "badge = 1 after signup" || ng "badge after signup"
