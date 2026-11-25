#!/bin/bash
# Stock consistency verification (release audit issue 1):
# stock lives on product_variants only, and every surface
# must read the same rows — the storefront card and detail,
# the admin product list, the admin editor's combination
# grid, the dashboard low-stock list — while the cart gate
# and order placement enforce the same numbers.
#
# Scenario A (axis-less product): zero stock is unpurchasable
# everywhere and a cart POST is refused; an editor change to
# exactly one unit flips every view to "1 remaining" at once;
# a cart holding that unit cannot place once the editor
# takes the stock back to zero (placement re-validates the
# locked variant row); restoring one unit lets the same cart
# complete, and every view returns to zero together.
# Scenario B (variant product): per-combination stocks and
# their sum agree on the detail picker's data, the admin
# list and the editor grid; a zero-stock combination cannot
# be added; buying out a combination zeroes it everywhere.
#
# Fixtures are disposable products in the テスト category;
# they are archived at the end. Field names the transport
# rewrites when shipped literally are built at runtime.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
CT=$(printf "csrf_%s" "token")
PW=$(printf "pass%s" "word")
XH=$(printf "x-csrf-%s" "token")
JA=/tmp/stk_admin.txt
GA=/tmp/stk_guest_a.txt
GB=/tmp/stk_guest_b.txt
ANAME="管理テスト商品 在庫整合A"
BNAME="管理テスト商品 在庫整合B"
TEST_CAT_ID=$(db "SELECT id FROM categories WHERE name = 'テスト'")
PUBLIC_BEFORE=$(db "SELECT COUNT(*) FROM products WHERE status = 'public'")

admin_login() {
  rm -f $JA
  curl -s -c $JA $BASE/admin/login -o /tmp/stk_login.html
  local C=$(csrf /tmp/stk_login.html)
  curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "$PW=$APW" --data-urlencode "$CT=$C"
}

# Self-clean: same-name fixtures from an earlier run would
# break the name lookups below. Ordered ones are archived
# and renamed out of the way (house ritual); the rest go.
for N in "$ANAME" "$BNAME"; do
  db "DELETE FROM cart_items WHERE variant_id IN (SELECT v.id FROM product_variants v JOIN products p ON p.id = v.product_id WHERE p.name = '$N')" >/dev/null
  db "DELETE FROM products WHERE name = '$N' AND id NOT IN (SELECT product_id FROM order_items)" >/dev/null
  db "UPDATE products SET status = 'archived', name = name || '・旧' WHERE name = '$N'" >/dev/null
done

admin_login
# The product form requires an attached image: upload the
# 1x1 PNG fixture once and reuse its stored path.
printf 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==' | base64 -d > /tmp/stk_fixture.png
curl -s -b $JA $BASE/admin/products/new -o /tmp/stk_upform.html; C=$(csrf /tmp/stk_upform.html)
IMGURL=$(curl -s -b $JA -X POST $BASE/admin/products/image-upload -H "$XH: $C" -F "image=@/tmp/stk_fixture.png" | sed -n 's/.*"url"[ ]*:[ ]*"\([^"]*\)".*/\1/p')
[ -n "$IMGURL" ] && ok "fixture image uploaded" || ng "fixture image uploaded"

create_a() { # stock
  curl -s -b $JA $BASE/admin/products/new -o /tmp/stk_new.html; C=$(csrf /tmp/stk_new.html)
  curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/new \
    --data-urlencode "name=$ANAME" --data-urlencode "description=在庫整合のテスト商品" \
    --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=100" \
    --data-urlencode "combo_stock[0]=$1" --data-urlencode "status=public" \
    --data-urlencode "images=$IMGURL" --data-urlencode "$CT=$C"
}
edit_a() { # stock
  curl -s -b $JA $BASE/admin/products/$AID/edit -o /tmp/stk_edit.html; C=$(csrf /tmp/stk_edit.html)
  curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/$AID/edit \
    --data-urlencode "name=$ANAME" --data-urlencode "description=在庫整合のテスト商品" \
    --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=100" \
    --data-urlencode "combo_stock[0]=$1" --data-urlencode "status=public" \
    --data-urlencode "images=$IMGURL" --data-urlencode "$CT=$C"
}

db_sum() { db "SELECT COALESCE(SUM(stock),0) FROM product_variants WHERE product_id = $1 AND is_enabled"; }
edit_stocks() { # product id -> the editor grid's stock list
  curl -s -b $JA $BASE/admin/products/$1/edit -o /tmp/stk_editv.html
  python3 -c "
import re, json, sys
h = open('/tmp/stk_editv.html').read()
m = re.search(r'id=\"variant-existing-combos\">(.*?)</script>', h, re.S)
print(','.join(str(c['stock']) for c in json.loads(m.group(1))) if m else 'NONE')"
}
list_stock() { # product id -> stock cell of the admin list row
  curl -s -b $JA "$BASE/admin/products?q=$(python3 -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1]))" "$2")" -o /tmp/stk_list.html
  python3 -c "
import re, sys
h = open('/tmp/stk_list.html').read()
i = h.find('/admin/products/$1/edit')
seg = h[max(0,i-1500):i] if i >= 0 else ''
nums = re.findall(r'>(\d+)</td>', seg)
print(nums[-1] if nums else 'NONE')"
}
detail_state() { # product id -> SOLDOUT / ONE / OTHER
  curl -s $BASE/products/$1 -o /tmp/stk_detail.html
  if grep -q "残りわずか（あと1点）" /tmp/stk_detail.html; then echo ONE
  elif grep -q "売り切れ" /tmp/stk_detail.html; then echo SOLDOUT
  else echo OTHER; fi
}
guest_add() { # jar product [variant]
  rm -f "$1"
  curl -s -c "$1" $BASE/products/$2 -o /tmp/stk_p.html; C=$(csrf /tmp/stk_p.html)
  if [ -n "$3" ]; then EXTRA="--data-urlencode variant_id=$3"; else EXTRA=""; fi
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/cart/items \
    --data-urlencode "product_id=$2" --data-urlencode "quantity=1" $EXTRA --data-urlencode "$CT=$C"
  curl -s -b "$1" $BASE/cart -o /tmp/stk_cart.html
}
guest_checkout() { # jar -> confirm POST redirect target
  curl -s -b "$1" $BASE/checkout -o /tmp/stk_co.html; C=$(csrf /tmp/stk_co.html)
  curl -s -b "$1" -c "$1" -X POST $BASE/checkout -o /tmp/stk_conf.html \
    --data-urlencode "intent=confirm" \
    --data-urlencode "email=stock-guest@example.jp" --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" \
    --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
    --data-urlencode "address_line=神宮前1-1-1" --data-urlencode "building=" --data-urlencode "phone=09012345678" \
    --data-urlencode "$CT=$C"
  curl -s -b "$1" -c "$1" -X POST $BASE/checkout/payment -o /tmp/stk_conf.html \
    --data-urlencode "payment_method=cod" --data-urlencode "$CT=$C"
  C=$(csrf /tmp/stk_conf.html)
  curl -s -b "$1" -c "$1" -o /tmp/stk_confresp.html -w "%{redirect_url}" -X POST $BASE/checkout/confirm --data-urlencode "$CT=$C"
}
items_of() { db "SELECT COUNT(*) FROM order_items WHERE product_id = $1"; }

echo "-- scenario A: axis-less product, zero stock"
create_a 0
AID=$(db "SELECT id FROM products WHERE name = '$ANAME'")
[ -n "$AID" ] && ok "fixture A created" || ng "fixture A created"
[ "$(db_sum $AID)" = "0" ] && ok "A: DB sum is 0" || ng "A: DB sum is 0"
[ "$(detail_state $AID)" = "SOLDOUT" ] && ok "A: detail says sold out at 0" || ng "A: detail says sold out at 0"
[ "$(edit_stocks $AID)" = "0" ] && ok "A: editor grid shows 0" || ng "A: editor grid shows 0 ($(edit_stocks $AID))"
[ "$(list_stock $AID "$ANAME")" = "0" ] && ok "A: admin list shows 0" || ng "A: admin list shows 0 ($(list_stock $AID "$ANAME"))"
curl -s -b $JA $BASE/admin -o /tmp/stk_dash.html
grep -q "$ANAME" /tmp/stk_dash.html && ok "A: dashboard low-stock lists it at 0" || ng "A: dashboard low-stock lists it at 0"
guest_add $GA $AID
grep -q "$ANAME" /tmp/stk_cart.html && ng "A: cart POST refused at 0" || ok "A: cart POST refused at 0"

echo "-- scenario A: editor raises stock to exactly one"
edit_a 1
[ "$(db_sum $AID)" = "1" ] && ok "A: DB sum is 1 after edit" || ng "A: DB sum is 1 after edit"
[ "$(detail_state $AID)" = "ONE" ] && ok "A: detail says 1 remaining" || ng "A: detail says 1 remaining"
[ "$(edit_stocks $AID)" = "1" ] && ok "A: editor grid shows 1" || ng "A: editor grid shows 1"
[ "$(list_stock $AID "$ANAME")" = "1" ] && ok "A: admin list shows 1" || ng "A: admin list shows 1"

echo "-- scenario A: placement refuses a cart holding stock that vanished"
guest_add $GA $AID
grep -q "$ANAME" /tmp/stk_cart.html && ok "A: cart holds the last unit" || ng "A: cart holds the last unit"
edit_a 0
BEFORE=$(items_of $AID)
OUT=$(curl -s -b $GA -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/checkout)
case "$OUT" in "303 $BASE/cart") ok "A: checkout entry bounces to cart at stock 0";; *) ng "A: checkout entry bounces to cart at stock 0 ($OUT)";; esac
curl -s -b $GA $BASE/cart -o /tmp/stk_cart.html
grep -q "在庫切れのため購入できません" /tmp/stk_cart.html && ok "A: cart marks the vanished-stock line unpurchasable" || ng "A: cart marks the vanished-stock line unpurchasable"
AFTER=$(items_of $AID)
[ "$BEFORE" = "$AFTER" ] && ok "A: no order created at stock 0" || ng "A: no order created at stock 0"

echo "-- scenario A: the same cart completes once stock is back"
edit_a 1
REDIR=$(guest_checkout $GA)
AFTER=$(items_of $AID)
[ "$AFTER" = "$((BEFORE+1))" ] && ok "A: placement succeeds at stock 1" || ng "A: placement succeeds at stock 1"
case "$REDIR" in *"complete"*) ok "A: placement reaches the complete page";; *) ng "A: placement reaches the complete page ($REDIR)";; esac
[ "$(db_sum $AID)" = "0" ] && ok "A: DB sum back to 0 after the order" || ng "A: DB sum back to 0 after the order"
[ "$(detail_state $AID)" = "SOLDOUT" ] && ok "A: detail sold out again" || ng "A: detail sold out again"
[ "$(list_stock $AID "$ANAME")" = "0" ] && ok "A: admin list back to 0" || ng "A: admin list back to 0"
guest_add $GB $AID
grep -q "$ANAME" /tmp/stk_cart.html && ng "A: second buyer refused at 0" || ok "A: second buyer refused at 0"

echo "-- scenario B: variant product sums"
curl -s -b $JA $BASE/admin/products/new -o /tmp/stk_new.html; C=$(csrf /tmp/stk_new.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/new \
  --data-urlencode "name=$BNAME" --data-urlencode "description=バリエーション在庫整合のテスト商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=2000" --data-urlencode "status=public" \
  --data-urlencode "images=$IMGURL" \
  --data-urlencode "axes[0][name]=サイズ" \
  --data-urlencode "axes[0][options][0][value]=S" --data-urlencode "axes[0][options][1][value]=M" \
  --data-urlencode "combo_stock[0]=2" --data-urlencode "combo_stock[1]=0" \
  --data-urlencode "$CT=$C"
BID=$(db "SELECT id FROM products WHERE name = '$BNAME'")
[ -n "$BID" ] && ok "fixture B created" || ng "fixture B created"
[ "$(db_sum $BID)" = "2" ] && ok "B: DB sum is 2 (S2 + M0)" || ng "B: DB sum is 2"
[ "$(edit_stocks $BID)" = "2,0" ] && ok "B: editor grid shows 2,0" || ng "B: editor grid shows 2,0 ($(edit_stocks $BID))"
[ "$(list_stock $BID "$BNAME")" = "2" ] && ok "B: admin list shows the sum 2" || ng "B: admin list shows the sum 2 ($(list_stock $BID "$BNAME"))"
curl -s $BASE/products/$BID -o /tmp/stk_bdetail.html
BSTOCKS=$(python3 -c "
import re, json
h = open('/tmp/stk_bdetail.html').read()
m = re.search(r'data-variant-data>(.*?)</script>', h, re.S)
print(','.join(str(v['stock']) for v in json.loads(m.group(1))['variants']) if m else 'NONE')")
[ "$BSTOCKS" = "2,0" ] && ok "B: picker data shows 2,0" || ng "B: picker data shows 2,0 ($BSTOCKS)"
SVM=$(db "SELECT id FROM product_variants WHERE product_id = $BID ORDER BY id LIMIT 1")
MVM=$(db "SELECT id FROM product_variants WHERE product_id = $BID ORDER BY id OFFSET 1 LIMIT 1")
guest_add $GB $BID $MVM
grep -q "$BNAME" /tmp/stk_cart.html && ng "B: zero-stock combination refused" || ok "B: zero-stock combination refused"
guest_add $GB $BID $SVM
# second unit of S: raise the cart line to 2 through a second add
curl -s -b $GB $BASE/products/$BID -o /tmp/stk_p.html; C=$(csrf /tmp/stk_p.html)
curl -s -b $GB -c $GB -o /dev/null -X POST $BASE/cart/items \
  --data-urlencode "product_id=$BID" --data-urlencode "quantity=1" --data-urlencode "variant_id=$SVM" --data-urlencode "$CT=$C"
grep -q "$BNAME" /tmp/stk_cart.html && ok "B: in-stock combination added" || ng "B: in-stock combination added"
REDIR=$(guest_checkout $GB)
[ "$(db "SELECT stock FROM product_variants WHERE id = $SVM")" = "0" ] && ok "B: S bought out to 0" || ng "B: S bought out to 0"
[ "$(list_stock $BID "$BNAME")" = "0" ] && ok "B: admin list sum back to 0" || ng "B: admin list sum back to 0"

echo "-- cleanup: fixtures archived, public count restored"
for P in $AID $BID; do
  curl -s -b $JA $BASE/admin/products/$P/edit -o /tmp/stk_edit.html; C=$(csrf /tmp/stk_edit.html)
  curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/$P/archive --data-urlencode "$CT=$C"
done
PUBLIC_AFTER=$(db "SELECT COUNT(*) FROM products WHERE status = 'public'")
[ "$PUBLIC_AFTER" = "$PUBLIC_BEFORE" ] && ok "public product count restored ($PUBLIC_AFTER)" || ng "public product count restored ($PUBLIC_AFTER vs $PUBLIC_BEFORE)"

result
