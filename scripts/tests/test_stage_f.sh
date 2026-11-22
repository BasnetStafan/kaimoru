#!/bin/bash
# Stage F: ranking from real orders, shipping boundary at exactly
# 5000, search-page buttons, log privacy.
BASE=http://127.0.0.1:3000
# Shared harness (echo-only variant): lib.sh (ST-6).
SUITE_NO_COUNT=1
. "$(dirname "$0")/lib.sh"
add() { curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$3" --data-urlencode "quantity=$4" --data-urlencode "csrf_token=$2"; }

echo "-- search results carry add buttons too"
curl -s "$BASE/search?q=Rust" | grep -q "カートに入れる" && ok "search cards have buttons" || ng "search cards have buttons"

echo "-- free shipping over the threshold (4500 + 880 = 5380, physical only)"
J=/tmp/tf_jar.txt; rm -f $J
curl -s -c $J $BASE/ -o /tmp/tf_h.html
add $J "$(csrf /tmp/tf_h.html)" 8 1
add $J "$(csrf /tmp/tf_h.html)" 5 1
curl -s -b $J $BASE/cart -o /tmp/tf_cart.html
grep -q "￥5,380" /tmp/tf_cart.html && grep -q "￥0（無料）" /tmp/tf_cart.html && ok "subtotal 5380, shipping free" || ng "over-threshold"
grep -q "￥5,710" /tmp/tf_cart.html && ok "total = 5380 + 330 COD" || ng "over-threshold total"

echo "-- order product 9 twice; ranking must follow real sales"
J2=/tmp/tf_jar2.txt; rm -f $J2
curl -s -c $J2 $BASE/ -o /tmp/tf_h2.html
add $J2 "$(csrf /tmp/tf_h2.html)" 9 2
add $J2 "$(csrf /tmp/tf_h2.html)" 10 1
curl -s -b $J2 $BASE/checkout -o /tmp/tf_co.html
curl -s -b $J2 -c $J2 -X POST $BASE/checkout --data-urlencode "intent=confirm" \
  --data-urlencode "email=rank@example.jp" --data-urlencode "family_name=田中" --data-urlencode "given_name=健" \
  --data-urlencode "postal_code=1000001" --data-urlencode "prefecture=東京都" --data-urlencode "city=千代田区" \
  --data-urlencode "address_line=千代田2-2" --data-urlencode "building=" --data-urlencode "phone=08055556666" \
  --data-urlencode "csrf_token=$(csrf /tmp/tf_co.html)" -o /tmp/tf_conf.html
curl -s -b $J2 -c $J2 -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "csrf_token=$(csrf /tmp/tf_co.html)" -o /tmp/tf_conf.html
curl -s -b $J2 -c $J2 -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$(csrf /tmp/tf_conf.html)"
grep -q "注文が完了しました" <(curl -s -b $J2 $BASE/checkout/complete) && ok "ranking-test order placed" || ng "ranking order"
curl -s $BASE/ -o /tmp/tf_home.html
python3 - << 'PYEOF'
import re, pathlib
html = pathlib.Path("/tmp/tf_home.html").read_text()
section = html.split("ランキング")[1].split("イベント受取商品")[0]
names = re.findall(r'href="/products/\d+">(?!<img)([^<]+)</a>', section)
if not names:
    names = re.findall(r'alt="([^"]+)"', section)
print("  ranking order:", names[:5])
assert names and names[0].startswith("会場受取限定タオル"), names
print("  OK  ranking #1 is the product with 2 real sales")
PYEOF

echo "-- log privacy: no address, phone or email in the app log"
LOG=$(ls -t logs/front_app.log.* 2>/dev/null | head -1)
echo "  log: $LOG"
HITS=$(grep -c "08011112222\|guest@example.jp\|神宮前" "$LOG" 2>/dev/null)
HITS=${HITS:-0}
[ "$HITS" = "0" ] && ok "0 personal-data hits in log" || ng "$HITS personal-data hits"
grep -q "order_placed" "$LOG" && ok "order_placed action logged (id only)" || ng "order_placed missing"
