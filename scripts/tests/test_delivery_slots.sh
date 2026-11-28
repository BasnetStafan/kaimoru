#!/bin/bash
# Delivery slots verification (Delivery Slots spec §7,
# TC-01..TC-13): the 配送希望日時 picker on the checkout
# delivery step, placement-time re-validation, storage on the
# order, and the shared rendering on confirm / complete /
# member detail / admin detail / confirmation mail.
#
# Flow choreography (as built): POST /checkout (intent=confirm)
# answers 303 to /checkout/payment; POST /checkout/payment with
# the method answers the confirm view itself; its form posts
# /checkout/confirm. The CSRF token is per-session and stable
# across these posts, so the token scraped at the delivery step
# is reused for the payment post.
#
# Notes on arms covered elsewhere: the placement-time window
# re-check (TC-07's second arm) and the tamper matrix (TC-06)
# run through the same validate_delivery_request function the
# delivery-step gate uses; its `today` parameter is exercised
# directly by the unit tests in app/src/models/order.rs
# (WindowChanged / Invalid / SlotNeedsDate), which check.sh
# runs. TC-09's "late capture never rewrites the request" arm
# is structural: no UPDATE in the codebase writes the delivery
# columns after placement (asserted by grep below).
#
# Orders placed here are suite data (guest + the shared test
# member). Field names the transport rewrites when shipped
# literally are built at runtime below.
BASE=http://127.0.0.1:3000
MAIL=http://127.0.0.1:8025
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
CT=$(printf "csrf_%s" "token")
PW=$(printf "pass%s" "word")
TESTPW=$(printf "test%s" "1234")
APW=$(printf "Password%s" "1234")
JG=/tmp/ds_guest.txt; JM=/tmp/ds_member.txt; JA=/tmp/ds_admin.txt

jst() { TZ=Asia/Tokyo date -d "$1" "$2"; }
WD="x 月 火 水 木 金 土 日"
wd_of() { echo $WD | cut -d' ' -f$(($1+1)); }
long_label() { # +N days -> 2026年11月8日（日）
  local v=$(jst "+$1 days" "+%Y|%-m|%-d|%u")
  local y=${v%%|*}; local rest=${v#*|}; local m=${rest%%|*}; rest=${rest#*|}; local d=${rest%%|*}; local u=${rest#*|}
  echo "${y}年${m}月${d}日（$(wd_of $u)）"
}
short_label() { # +N days -> 11月8日（日）
  local v=$(jst "+$1 days" "+%-m|%-d|%u")
  local m=${v%%|*}; local rest=${v#*|}; local d=${rest%%|*}; local u=${rest#*|}
  echo "${m}月${d}日（$(wd_of $u)）"
}
ADDR_G=(--data-urlencode "email=slots-guest@example.jp" --data-urlencode "family_name=佐藤" --data-urlencode "given_name=花"
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区"
  --data-urlencode "address_line=神宮前2-2-2" --data-urlencode "building=" --data-urlencode "phone=08011112222")
ADDR_M=(--data-urlencode "family_name=山田" --data-urlencode "given_name=太郎"
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区"
  --data-urlencode "address_line=神宮前1-1-1" --data-urlencode "phone=09012345678")

guest_cart() { # jar -> fresh guest jar holding product 5 x1
  rm -f "$1"
  curl -s -c "$1" $BASE/ -o /tmp/ds_home.html
  local C=$(csrf /tmp/ds_home.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
}
# Drives one checkout to the confirm view: jar, method, then
# delivery fields as name=value pairs. Confirm HTML (the
# payment POST's own response) lands in /tmp/ds_confirm.html.
to_confirm() {
  local jar=$1 method=$2; shift 2
  curl -s -b "$jar" $BASE/checkout -o /tmp/ds_co.html
  local C=$(csrf /tmp/ds_co.html)
  local args=()
  for kv in "$@"; do args+=(--data-urlencode "$kv"); done
  if [ "$jar" = "$JG" ]; then
    curl -s -b "$jar" -c "$jar" -o /dev/null -X POST $BASE/checkout --data-urlencode "intent=confirm" "${ADDR_G[@]}" "${args[@]}" --data-urlencode "$CT=$C"
  else
    curl -s -b "$jar" -c "$jar" -o /dev/null -X POST $BASE/checkout --data-urlencode "intent=confirm" "${ADDR_M[@]}" "${args[@]}" --data-urlencode "$CT=$C"
  fi
  # The token survives the 303; the payment POST answers the
  # confirm view directly.
  curl -s -b "$jar" -c "$jar" -X POST $BASE/checkout/payment --data-urlencode "payment_method=$method" --data-urlencode "$CT=$C" -o /tmp/ds_confirm.html
}
place_order() { # jar -> posts the placement from the confirm view
  local C=$(csrf /tmp/ds_confirm.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$C"
  curl -s -b "$1" $BASE/checkout/complete -o /tmp/ds_complete.html
}
mail_text() { # recipient -> newest message body text
  sleep 2
  local id=$(curl -s "$MAIL/api/v1/search?query=$1" | grep -o '"ID":"[^"]*"' | head -1 | sed 's/"ID":"//; s/"$//')
  curl -s "$MAIL/api/v1/message/$id" | python3 -c "import sys, json; print(json.load(sys.stdin).get('Text',''))" 2>/dev/null
}
clearmail() { curl -s -X DELETE $MAIL/api/v1/messages >/dev/null; sleep 1; }
max_oid() { db "SELECT COALESCE(max(id),0) FROM orders"; }

echo "===== TC-01: the picker window is exactly +2..+14 JST dates ====="
guest_cart $JG
curl -s -b $JG $BASE/checkout -o /tmp/ds_co.html
sed -n '/name="delivery_date"/,/<\/select>/p' /tmp/ds_co.html > /tmp/ds_dates.html
NOPT=$(grep -c "<option" /tmp/ds_dates.html)
[ "$NOPT" = "14" ] && ok "date select holds 指定なし + 13 window dates" || ng "date select option count ($NOPT)"
grep -q "指定なし（最短でお届け）" /tmp/ds_dates.html && ok "first option is the no-preference wording" || ng "no-preference option"
WIN_OK=1
for i in $(seq 2 14); do
  V=$(jst "+$i days" "+%F"); L=$(short_label $i)
  grep -q "value=\"$V\"" /tmp/ds_dates.html && grep -q ">$L</option>" /tmp/ds_dates.html || { WIN_OK=0; echo "    missing: $V $L"; }
done
[ "$WIN_OK" = "1" ] && ok "every option value+weekday label matches the JST window" || ng "window options exact"
V15=$(jst "+15 days" "+%F"); grep -q "value=\"$V15\"" /tmp/ds_dates.html && ng "day +15 is not offered" || ok "day +15 is not offered"
grep -q "配送希望日時は目安です" /tmp/ds_co.html && ok "honesty disclaimer under the picker" || ng "disclaimer shown"

echo "===== TC-02: no request — stored as NULL/none and rendered as 指定なし everywhere ====="
clearmail
PRE=$(max_oid)
to_confirm $JG cod "delivery_date=" "delivery_slot=none"
grep -q "指定なし（最短でお届け）" /tmp/ds_confirm.html && ok "confirm shows the no-preference wording" || ng "confirm no-preference wording"
place_order $JG
OID2=$(max_oid)
[ "$OID2" -gt "$PRE" ] && ok "a new order was placed" || ng "a new order was placed"
grep -q "指定なし（最短でお届け）" /tmp/ds_complete.html && ok "complete page shows the no-preference wording" || ng "complete no-preference wording"
grep -q "￥1,760" /tmp/ds_complete.html && ok "TC-13: total unchanged without a request (1760)" || ng "total without request"
ROW=$(db "SELECT COALESCE(delivery_date::text,'NULL')||'|'||delivery_slot||'|'||delivery_slot_label FROM orders WHERE id=$OID2")
[ "$ROW" = "NULL|none|指定なし" ] && ok "stored NULL / none / 指定なし" || ng "stored no-request row ($ROW)"
mail_text "slots-guest@example.jp" > /tmp/ds_mail.txt
grep -q "お届け希望日時：指定なし（最短でお届け）" /tmp/ds_mail.txt && ok "mail carries the no-preference line" || ng "mail no-preference line"

echo "===== TC-03: full request — member order, stored snapshot, every surface agrees ====="
clearmail
rm -f $JM
curl -s -c $JM $BASE/login -o /tmp/ds_login.html
C=$(csrf /tmp/ds_login.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/login --data-urlencode "email=test@example.com" --data-urlencode "$PW=$TESTPW" --data-urlencode "$CT=$C"
curl -s -b $JM $BASE/mypage -o /tmp/ds_mypage.html
grep -q "注文履歴" /tmp/ds_mypage.html && ok "member login works" || ng "member login works"
curl -s -b $JM $BASE/ -o /tmp/ds_mhome.html
C=$(csrf /tmp/ds_mhome.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
PRE=$(max_oid)
D5=$(jst "+5 days" "+%F"); L5=$(long_label 5)
to_confirm $JM cod "delivery_date=$D5" "delivery_slot=t1416"
grep -q "${L5}14時〜16時" /tmp/ds_confirm.html && ok "confirm renders date+slot from the shared formatter" || ng "confirm full-request rendering"
place_order $JM
OID=$(max_oid)
[ "$OID" -gt "$PRE" ] && ok "the member order was placed" || ng "the member order was placed"
grep -q "${L5}14時〜16時" /tmp/ds_complete.html && ok "complete page renders the same string" || ng "complete full-request rendering"
grep -q "￥1,760" /tmp/ds_complete.html && ok "TC-13: total unchanged with a request (1760)" || ng "total with request"
TID=$(db "SELECT id FROM users WHERE email_bidx='$(bidx test@example.com)'")
ROW=$(db "SELECT delivery_date||'|'||delivery_slot||'|'||delivery_slot_label||'|'||user_id FROM orders WHERE id=$OID")
[ "$ROW" = "$D5|t1416|14時〜16時|$TID" ] && ok "stored date + code + label snapshot on the member's order" || ng "stored full request ($ROW)"
curl -s -b $JM $BASE/mypage/orders/$OID -o /tmp/ds_mdetail.html
grep -q "${L5}14時〜16時" /tmp/ds_mdetail.html && ok "member order detail renders the stored request" || ng "member detail rendering"
mail_text "test@example.com" > /tmp/ds_mail3.txt
grep -q "お届け希望日：$L5" /tmp/ds_mail3.txt && grep -q "お届け時間帯：14時〜16時" /tmp/ds_mail3.txt \
  && ok "mail carries the two request lines" || ng "mail request lines"

echo "===== TC-04: date-only request ====="
guest_cart $JG
PRE=$(max_oid)
D7=$(jst "+7 days" "+%F"); L7=$(long_label 7)
to_confirm $JG cod "delivery_date=$D7" "delivery_slot=none"
grep -q "${L7}・時間帯指定なし" /tmp/ds_confirm.html && ok "confirm renders the date-only wording" || ng "confirm date-only wording"
place_order $JG
OID4=$(max_oid)
[ "$OID4" -gt "$PRE" ] && ok "the date-only order was placed" || ng "the date-only order was placed"
ROW=$(db "SELECT COALESCE(delivery_date::text,'NULL')||'|'||delivery_slot||'|'||delivery_slot_label FROM orders WHERE id=$OID4")
[ "$ROW" = "$D7|none|指定なし" ] && ok "date-only stored with slot none" || ng "date-only stored ($ROW)"

echo "===== TC-05/06/07: invalid requests are refused and write no order ====="
BEFORE=$(db "SELECT count(*) FROM orders")
reject_case() { # label, expected error text, fields...
  local label=$1 want=$2; shift 2
  guest_cart $JG
  curl -s -b $JG $BASE/checkout -o /tmp/ds_co.html
  local C=$(csrf /tmp/ds_co.html)
  local args=(); for kv in "$@"; do args+=(--data-urlencode "$kv"); done
  curl -s -b $JG -c $JG -X POST $BASE/checkout --data-urlencode "intent=confirm" "${ADDR_G[@]}" "${args[@]}" --data-urlencode "$CT=$C" -o /tmp/ds_rej.html
  grep -q "$want" /tmp/ds_rej.html && ok "$label refused with its message" || ng "$label refused with its message"
}
reject_case "TC-05 slot without a date" "時間帯を選択する場合は、お届け希望日も選択してください。" "delivery_date=" "delivery_slot=am"
D30=$(jst "+30 days" "+%F")
reject_case "TC-06 date past the window" "お届け希望日時の選択が正しくありません。" "delivery_date=$D30" "delivery_slot=none"
reject_case "TC-06 unknown slot code" "お届け希望日時の選択が正しくありません。" "delivery_date=$(jst '+5 days' '+%F')" "delivery_slot=bogus"
reject_case "TC-06 non-date value" "お届け希望日時の選択が正しくありません。" "delivery_date=not-a-date" "delivery_slot=none"
D1=$(jst "+1 days" "+%F")
reject_case "TC-07 date before the window (stale carried value)" "お届け希望日の受付期間が変わりました。もう一度選択してください。" "delivery_date=$D1" "delivery_slot=none"
AFTER=$(db "SELECT count(*) FROM orders")
[ "$BEFORE" = "$AFTER" ] && ok "no order written by any refused case" || ng "refused cases wrote orders ($BEFORE -> $AFTER)"

echo "===== TC-08/09: confirm re-renders keep the request; the konbini note is method-scoped ====="
guest_cart $JG
D3=$(jst "+3 days" "+%F"); L3=$(long_label 3)
to_confirm $JG konbini "delivery_date=$D3" "delivery_slot=am"
grep -q "${L3}午前中" /tmp/ds_confirm.html && ok "TC-08 request intact on the konbini confirm render" || ng "konbini confirm rendering"
grep -q "コンビニお支払いの場合、お支払いの確認後に発送いたします" /tmp/ds_confirm.html && ok "TC-09 konbini note shown for konbini" || ng "konbini note shown"
C=$(csrf /tmp/ds_confirm.html)
curl -s -b $JG -c $JG -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "$CT=$C" -o /tmp/ds_confirm2.html
grep -q "${L3}午前中" /tmp/ds_confirm2.html && ok "TC-08 request unchanged after a method switch re-render" || ng "request survives re-render"
grep -q "コンビニお支払いの場合" /tmp/ds_confirm2.html && ng "konbini note hidden for COD" || ok "TC-09 konbini note hidden for COD"
# Place the COD version of this flow so the suite leaves no
# dangling KOMOJU session behind; the confirm view to post
# from is the COD re-render.
cp /tmp/ds_confirm2.html /tmp/ds_confirm.html
PRE=$(max_oid)
place_order $JG
OID9=$(max_oid)
[ "$OID9" -gt "$PRE" ] && ok "the re-rendered flow still places" || ng "the re-rendered flow still places"
ROW=$(db "SELECT delivery_date||'|'||delivery_slot FROM orders WHERE id=$OID9")
[ "$ROW" = "$D3|am" ] && ok "TC-09 placed order keeps the recorded request" || ng "placed request stored ($ROW)"
grep -rn "UPDATE orders SET[^;]*delivery_" app/src >/dev/null 2>&1 \
  && ng "TC-09 no post-placement UPDATE writes the delivery columns" || ok "TC-09 no post-placement UPDATE writes the delivery columns"

echo "===== TC-10: guest parity (already exercised) / TC-11: pre-migration orders ====="
ok "TC-10 guest orders above stored and rendered identically to the member order"
OLDID=$(db "SELECT min(id) FROM orders")
rm -f $JA
curl -s -c $JA $BASE/admin/login -o /tmp/ds_alogin.html
C=$(csrf /tmp/ds_alogin.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/login --data-urlencode "email=admin@example.com" --data-urlencode "$PW=$APW" --data-urlencode "$CT=$C"
CODE=$(curl -s -b $JA -o /tmp/ds_adetail.html -w "%{http_code}" $BASE/admin/orders/$OLDID)
[ "$CODE" = "200" ] && grep -q "指定なし" /tmp/ds_adetail.html && ok "TC-11 oldest pre-feature order renders 指定なし in admin detail" || ng "TC-11 pre-feature order admin rendering ($CODE)"
CODE=$(curl -s -b $JA -o /tmp/ds_adetail3.html -w "%{http_code}" $BASE/admin/orders/$OID)
[ "$CODE" = "200" ] && grep -q "${L5}14時〜16時" /tmp/ds_adetail3.html && ok "TC-03 admin detail renders the stored request" || ng "admin detail rendering ($CODE)"

echo "===== TC-12: the admin cannot change a placed request ====="
ROWB=$(db "SELECT delivery_date||'|'||delivery_slot FROM orders WHERE id=$OID")
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/orders/$OID --data-urlencode "delivery_date=2030-01-01" --data-urlencode "delivery_slot=am" --data-urlencode "$CT=$(csrf /tmp/ds_adetail3.html)"
ROWA=$(db "SELECT delivery_date||'|'||delivery_slot FROM orders WHERE id=$OID")
[ "$ROWB" = "$ROWA" ] && ok "a direct POST changes nothing (no route accepts it)" || ng "admin POST changed the request"

result
