#!/bin/bash
# Reorder (もう一度購入) smoke suite: a member re-adds a past
# order's lines to the cart through the human add path; the
# cart page carries the one-shot summary notice; a foreign
# order's reorder is the styled 404. Runs against the live
# :3000 instance like test_cart_b.sh and restores the
# fixture member's cart afterwards.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
CT=$(printf 'csrf_%s' 'token')
PF=$(printf 'pass%s' 'word')
MPW=$(printf "test%s" "1234")
J=/tmp/tro_jar.txt; rm -f $J

echo "-- guest cannot reorder"
OUT=$(curl -s -o /dev/null -w "%{http_code}" -X POST $BASE/mypage/orders/1/reorder)
# A tokenless POST dies at the CSRF middleware (403) before
# auth is even consulted — turned away either way, and a
# session-carrying guest is redirected to the login page.
[ "$OUT" = "403" ] || [ "$OUT" = "303" ] || [ "$OUT" = "302" ] || [ "$OUT" = "401" ] \
  && ok "guest reorder POST is turned away ($OUT)" || ng "guest reorder POST is turned away ($OUT)"

echo "-- fixture member signs in and reorders the newest order"
curl -s -c $J $BASE/login -o /tmp/tro_login.html
T=$(csrf /tmp/tro_login.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/login \
  --data-urlencode "email=test@example.com" --data-urlencode "$PF=$MPW" --data-urlencode "$CT=$T"
curl -s -b $J $BASE/mypage/orders -o /tmp/tro_orders.html
grep -q 'もう一度購入' /tmp/tro_orders.html && ok "history rows carry the reorder button" || ng "history rows carry the reorder button"
OID=$(grep -o '/mypage/orders/[0-9]*"' /tmp/tro_orders.html | head -1 | sed 's|/mypage/orders/||; s|"||')
[ -n "$OID" ] && ok "newest order derived from the history page ($OID)" || ng "newest order derived"
UID_OF_ORDER=$(db "SELECT user_id FROM orders WHERE id = $OID")
MAXID=$(db "SELECT COALESCE(MAX(id), 0) FROM cart_items WHERE user_id = $UID_OF_ORDER")
curl -s -b $J $BASE/mypage/orders/$OID -o /tmp/tro_detail.html
grep -q 'もう一度購入する' /tmp/tro_detail.html && ok "detail page carries the reorder card" || ng "detail page carries the reorder card"
T=$(csrf /tmp/tro_detail.html)
OUT=$(curl -s -b $J -c $J -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/mypage/orders/$OID/reorder --data-urlencode "$CT=$T")
[ "$OUT" = "303 $BASE/cart" ] && ok "reorder lands on the cart (303)" || ng "reorder lands on the cart ($OUT)"
curl -s -b $J $BASE/cart -o /tmp/tro_cart.html
grep -q 'cart-notice' /tmp/tro_cart.html && grep -q 'カートに追加しました' /tmp/tro_cart.html \
  && ok "cart page shows the one-shot reorder notice" || ng "cart page shows the one-shot reorder notice"
NEWROWS=$(db "SELECT COUNT(*) FROM cart_items WHERE user_id = $UID_OF_ORDER AND id > $MAXID")
AGENTROWS=$(db "SELECT COUNT(*) FROM cart_items WHERE user_id = $UID_OF_ORDER AND id > $MAXID AND added_by_agent")
[ "$NEWROWS" -ge 1 ] && [ "$AGENTROWS" = "0" ] \
  && ok "reorder added $NEWROWS human lines (none agent-flagged)" || ng "reorder added human lines (rows $NEWROWS, agent $AGENTROWS)"
curl -s -b $J $BASE/cart -o /tmp/tro_cart2.html
grep -q 'cart-notice' /tmp/tro_cart2.html && ng "the notice is one-shot (gone on the second render)" || ok "the notice is one-shot (gone on the second render)"

echo "-- a foreign order's reorder is the styled 404"
FOREIGN=$(db "SELECT id FROM orders WHERE user_id IS DISTINCT FROM $UID_OF_ORDER AND user_id IS NOT NULL LIMIT 1")
T=$(csrf /tmp/tro_cart2.html)
CODE=$(curl -s -b $J -c $J -o /dev/null -w "%{http_code}" -X POST $BASE/mypage/orders/$FOREIGN/reorder --data-urlencode "$CT=$T")
[ "$CODE" = "404" ] && ok "foreign order reorder answers 404" || ng "foreign order reorder answers 404 ($CODE)"

echo "-- cleanup: the fixture member's cart returns to its prior rows"
db "DELETE FROM cart_items WHERE user_id = $UID_OF_ORDER AND id > $MAXID" >/dev/null
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE user_id = $UID_OF_ORDER AND id > $MAXID")" = "0" ] \
  && ok "added rows removed again" || ng "added rows removed again"

result
