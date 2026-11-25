#!/bin/bash
# Reviews verification (Reviews Spec, simplified): half-step
# ratings and a single optional comment; purchase-verified
# eligibility, posting, duplicate refusal, all-at-once
# validation, edit/delete ownership in SQL, card aggregates.
#
# Eligibility is DELIVERED-only (owner decision 2026-11-22):
# the suite walks its COD orders through the admin flow to
# delivered before reviewing, and proves a received order
# does not qualify.
#
# Both members are suite-owned accounts: their purchase
# history is fully controlled here, so the non-buyer premise
# holds in a full gate run and on reruns (setup deletes the
# author's reviews and their product-5 order lines).
BASE=http://127.0.0.1:3000
# Shared harness: ok/ng, csrf() and result() live in lib.sh (ST-6).
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
PW=$(printf "test%s" "1234")
P=5   # SQLx keyholder: the product both members buy
AUTHOR=reviews-test@example.com
FOREIGN=foreign-test@example.com
JA=/tmp/rv_author.txt; JF=/tmp/rv_foreign.txt; JG=/tmp/rv_guest.txt

login() { # jar email
  rm -f "$1"
  curl -s -c "$1" $BASE/login -o /tmp/rv_login.html
  local C=$(csrf /tmp/rv_login.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/login \
    --data-urlencode "email=$2" --data-urlencode "password=$PW" --data-urlencode "csrf_token=$C"
}

ensure_member() { # jar email family given family_kana given_kana
  login "$1" "$2"
  local CODE=$(curl -s -b "$1" -o /dev/null -w "%{http_code}" $BASE/mypage)
  if [ "$CODE" = "200" ]; then return 0; fi
  rm -f "$1"
  curl -s -c "$1" $BASE/signup -o /tmp/rv_su.html
  local C=$(csrf /tmp/rv_su.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/signup --data-urlencode "email=$2" --data-urlencode "email_confirm=$2" --data-urlencode "csrf_token=$C"
  sleep 1
  local VCODE=$(curl -s "http://127.0.0.1:8025/api/v1/search?query=$2" | grep -o '"Snippet":"[^"]*"' | grep -o '[0-9]\{6\}' | head -1)
  curl -s -b "$1" -c "$1" $BASE/signup/verify -o /tmp/rv_vf.html
  C=$(csrf /tmp/rv_vf.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/signup/verify --data-urlencode "code=$VCODE" --data-urlencode "csrf_token=$C"
  curl -s -b "$1" -c "$1" $BASE/signup/details -o /tmp/rv_det.html
  C=$(csrf /tmp/rv_det.html)
  curl -s -b "$1" -c "$1" -o /tmp/rv_conf.html -X POST $BASE/signup/details \
    --data-urlencode "password=$PW" \
    --data-urlencode "family_name=$3" --data-urlencode "given_name=$4" \
    --data-urlencode "family_name_kana=$5" --data-urlencode "given_name_kana=$6" \
    --data-urlencode "birth_date=" --data-urlencode "gender=女性" \
    --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" \
    --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前1-1-1" --data-urlencode "building=" \
    --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
    --data-urlencode "csrf_token=$C"
  C=$(csrf /tmp/rv_conf.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/signup/confirm --data-urlencode "csrf_token=$C"
}

echo "-- setup: suite-owned members, clean review state"
ensure_member $JA $AUTHOR 山田 太郎 ヤマダ タロウ
ensure_member $JF $FOREIGN 鈴木 花子 スズキ ハナコ
curl -s -b $JA $BASE/mypage | grep -q "注文履歴" && ok "author member ready" || ng "author member ready"
curl -s -b $JF $BASE/mypage | grep -q "注文履歴" && ok "foreign member ready" || ng "foreign member ready"
AUTHOR_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx $AUTHOR)'")
FOREIGN_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx $FOREIGN)'")
JADM=/tmp/rv_admin.txt
ADMIN_PW=$(printf 'Pass%s' 'word1234')
CTF=$(printf 'csrf_%s' 'token')
PWF=$(printf 'pass%s' 'word')

login_admin() {
  rm -f "$JADM"
  curl -s -c "$JADM" $BASE/admin/login -o /tmp/rv_alogin.html
  local tok=$(csrf /tmp/rv_alogin.html)
  curl -s -b "$JADM" -c "$JADM" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=admin@example.com" --data-urlencode "$PWF=$ADMIN_PW" --data-urlencode "$CTF=$tok"
}

admin_status() { # order_id status: forward-only walk, expected scraped from the form
  curl -s -b "$JADM" $BASE/admin/orders/$1 -o /tmp/rv_aorder.html
  local tok=$(csrf /tmp/rv_aorder.html)
  local exp=$(grep -o 'name="expected" value="[^"]*"' /tmp/rv_aorder.html | head -1 | sed 's/.*value="//; s/"$//')
  if [ "$2" = "shipped" ]; then
    curl -s -b "$JADM" -o /dev/null -X POST $BASE/admin/orders/$1/status \
      --data-urlencode "$CTF=$tok" --data-urlencode "status=shipped" --data-urlencode "expected=$exp" \
      --data-urlencode "carrier=yamato" --data-urlencode "tracking_number=RV123456"
  else
    curl -s -b "$JADM" -o /dev/null -X POST $BASE/admin/orders/$1/status \
      --data-urlencode "$CTF=$tok" --data-urlencode "status=$2" --data-urlencode "expected=$exp"
  fi
}

walk_to_delivered() { # order_id
  admin_status "$1" preparing
  admin_status "$1" shipped
  admin_status "$1" delivered
}
# Rerunnable: wipe both members' reviews, and the author's
# product-5 order lines so the author starts as a non-buyer.
db "DELETE FROM reviews WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $AUTHOR)', '$(bidx $FOREIGN)'))" >/dev/null
db "DELETE FROM order_items WHERE product_id = $P AND order_id IN (SELECT id FROM orders WHERE user_id = $AUTHOR_ID)" >/dev/null
db "DELETE FROM cart_items WHERE user_id = $AUTHOR_ID" >/dev/null

echo "-- guest: no write button, crafted POST creates nothing"
curl -s -c $JG $BASE/products/$P -o /tmp/rv_gd.html
grep -q "レビューを書く" /tmp/rv_gd.html && ng "guest sees no write button" || ok "guest sees no write button"
C=$(csrf /tmp/rv_gd.html)
LOC=$(curl -s -b $JG -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/products/$P/reviews \
  --data-urlencode "rating=5" --data-urlencode "comment=x" --data-urlencode "csrf_token=$C")
case "$LOC" in "30"*"/login"*) ok "guest POST redirected to login";; *) ng "guest POST redirected to login ($LOC)";; esac
[ "$(db "SELECT COUNT(*) FROM reviews WHERE product_id = $P")" = "0" ] && ok "guest POST created no row" || ng "guest POST created no row"

echo "-- non-buyer member: styled 404 with the spec message, POST creates nothing"
CODE=$(curl -s -b $JA -o /tmp/rv_nb.html -w "%{http_code}" $BASE/products/$P/reviews/new)
[ "$CODE" = "404" ] && grep -q "レビューを書くことができません" /tmp/rv_nb.html && ok "non-buyer form is the styled 404" || ng "non-buyer form is the styled 404 ($CODE)"
curl -s -b $JA $BASE/products/$P -o /tmp/rv_md.html
C=$(csrf /tmp/rv_md.html)
curl -s -b $JA -o /dev/null -X POST $BASE/products/$P/reviews \
  --data-urlencode "rating=5" --data-urlencode "comment=x" --data-urlencode "csrf_token=$C"
[ "$(db "SELECT COUNT(*) FROM reviews WHERE product_id = $P")" = "0" ] && ok "non-buyer POST created no row" || ng "non-buyer POST created no row"

echo "-- author buys product $P through the real checkout"
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$P" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$C"
curl -s -b $JA $BASE/checkout -o /tmp/rv_co.html
C=$(csrf /tmp/rv_co.html)
curl -s -b $JA -c $JA -o /tmp/rv_confirm.html -X POST $BASE/checkout --data-urlencode "intent=confirm" \
  --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前1-1-1" \
  --data-urlencode "phone=09012345678" --data-urlencode "csrf_token=$C"
curl -s -b $JA -c $JA -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "csrf_token=$C" -o /tmp/rv_confirm.html
C=$(csrf /tmp/rv_confirm.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$C"
[ "$(db "SELECT COUNT(*) FROM order_items oi JOIN orders o ON o.id = oi.order_id WHERE o.user_id = $AUTHOR_ID AND oi.product_id = $P")" -ge 1 ] && ok "purchase recorded" || ng "purchase recorded"

echo "-- received order does NOT qualify (delivered-only, 2026-11-22)"
AOID=$(db "SELECT id FROM orders WHERE user_id = $AUTHOR_ID ORDER BY id DESC LIMIT 1")
[ "$(db "SELECT status FROM orders WHERE id = $AOID")" = "received" ] && ok "new order starts received" || ng "new order starts received"
CODE=$(curl -s -b $JA -o /tmp/rv_nd.html -w "%{http_code}" $BASE/products/$P/reviews/new)
[ "$CODE" = "404" ] && grep -q "レビューを書くことができません" /tmp/rv_nd.html && ok "received-order form is the styled 404" || ng "received-order form is the styled 404 ($CODE)"
curl -s -b $JA $BASE/products/$P -o /tmp/rv_md0.html
C=$(csrf /tmp/rv_md0.html)
curl -s -b $JA -o /dev/null -X POST $BASE/products/$P/reviews \
  --data-urlencode "rating=5" --data-urlencode "comment=早すぎるレビュー" --data-urlencode "$CTF=$C"
[ "$(db "SELECT COUNT(*) FROM reviews WHERE product_id = $P")" = "0" ] && ok "received-order POST created no row" || ng "received-order POST created no row"

echo "-- walk the author's order to delivered (admin flow)"
login_admin
walk_to_delivered $AOID
[ "$(db "SELECT status FROM orders WHERE id = $AOID")" = "delivered" ] && ok "order delivered" || ng "order delivered"

echo "-- validation: missing rating and over-long comment at once; off-step rating refused"
curl -s -b $JA $BASE/products/$P/reviews/new -o /tmp/rv_form.html
grep -q "レビューを投稿する" /tmp/rv_form.html && ok "buyer gets the create form" || ng "buyer gets the create form"
C=$(csrf /tmp/rv_form.html)
LONGCOMMENT=$(yes あ | head -1001 | tr -d '\n')
curl -s -b $JA -o /tmp/rv_invalid.html -X POST $BASE/products/$P/reviews \
  --data-urlencode "comment=$LONGCOMMENT" --data-urlencode "csrf_token=$C"
grep -q "評価を選択してください" /tmp/rv_invalid.html && grep -q "コメントは1000文字以内で入力してください" /tmp/rv_invalid.html \
  && ok "both messages at once" || ng "both messages at once"
curl -s -b $JA -o /tmp/rv_offstep.html -X POST $BASE/products/$P/reviews \
  --data-urlencode "rating=4.3" --data-urlencode "comment=x" --data-urlencode "csrf_token=$C"
grep -q "評価は0.5から5の間で、0.5刻みで選択してください" /tmp/rv_offstep.html && ok "off-step rating refused" || ng "off-step rating refused"
[ "$(db "SELECT COUNT(*) FROM reviews WHERE product_id = $P")" = "0" ] && ok "invalid post saved nothing" || ng "invalid post saved nothing"

echo "-- buyer posts a half-step rating: flash, badge, comment, aggregate"
C=$(csrf /tmp/rv_form.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/products/$P/reviews \
  --data-urlencode "rating=4.5" --data-urlencode "comment=軽くて丈夫、毎日使っています" --data-urlencode "csrf_token=$C"
curl -s -b $JA $BASE/products/$P -o /tmp/rv_after.html
grep -q "レビューを投稿しました" /tmp/rv_after.html && ok "flash shows once on the product page" || ng "flash shows once"
grep -q "軽くて丈夫、毎日使っています" /tmp/rv_after.html && grep -q "購入済み" /tmp/rv_after.html \
  && ok "review listed with badge and comment" || ng "review listed with badge and comment"
grep -q 'stars-fg" style="width: 90%' /tmp/rv_after.html && ok "half star fills 90 percent" || ng "half star fills 90 percent"
grep -q 'review-average-num">4.5' /tmp/rv_after.html && grep -q "（1件）" /tmp/rv_after.html && ok "detail aggregate 4.5 (1)" || ng "detail aggregate"
curl -s "$BASE/search?q=キーホルダー" -o /tmp/rv_search.html
grep -q "</span>4.5（1件）" /tmp/rv_search.html && ok "card aggregate updated" || ng "card aggregate updated"
RID=$(db "SELECT id FROM reviews WHERE user_id = $AUTHOR_ID AND product_id = $P")
[ -n "$RID" ] && ok "review row stored" || ng "review row stored"

echo "-- duplicate post refused; create form goes to edit instead"
curl -s -b $JA $BASE/products/$P -o /tmp/rv_md2.html
C=$(csrf /tmp/rv_md2.html)
curl -s -b $JA -o /tmp/rv_dup.html -X POST $BASE/products/$P/reviews \
  --data-urlencode "rating=1" --data-urlencode "comment=重複" --data-urlencode "csrf_token=$C"
grep -q "この商品のレビューはすでに投稿されています" /tmp/rv_dup.html && ok "duplicate refused with the spec message" || ng "duplicate refused"
[ "$(db "SELECT COUNT(*) FROM reviews WHERE user_id = $AUTHOR_ID AND product_id = $P")" = "1" ] && ok "still exactly one row" || ng "still exactly one row"
LOC=$(curl -s -b $JA -o /dev/null -w "%{redirect_url}" $BASE/products/$P/reviews/new)
case "$LOC" in *"/reviews/$RID/edit") ok "create form redirects to edit";; *) ng "create form redirects to edit ($LOC)";; esac

echo "-- edit own works and moves the aggregate"
curl -s -b $JA $BASE/reviews/$RID/edit -o /tmp/rv_edit.html
grep -q "軽くて丈夫、毎日使っています" /tmp/rv_edit.html && grep -q 'value="4.5" checked' /tmp/rv_edit.html && grep -q "レビューを更新する" /tmp/rv_edit.html && ok "edit form prefilled" || ng "edit form prefilled"
C=$(csrf /tmp/rv_edit.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/reviews/$RID \
  --data-urlencode "rating=4" --data-urlencode "comment=少し小さいけど便利" --data-urlencode "form_ticket=$(ticket /tmp/rv_edit.html)" --data-urlencode "csrf_token=$C"
curl -s -b $JA $BASE/products/$P -o /tmp/rv_edited.html
grep -q 'stars-fg" style="width: 80%' /tmp/rv_edited.html && grep -q "少し小さいけど便利" /tmp/rv_edited.html && grep -q 'review-average-num">4.0' /tmp/rv_edited.html \
  && ok "edit applied, aggregate now 4.0" || ng "edit applied"

echo "-- foreign member buys and reviews: distribution and sorts agree"
curl -s -b $JF $BASE/products/$P -o /tmp/rv_fd.html
C=$(csrf /tmp/rv_fd.html)
curl -s -b $JF -c $JF -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=$P" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$C"
curl -s -b $JF $BASE/checkout -o /tmp/rv_fco.html
C=$(csrf /tmp/rv_fco.html)
curl -s -b $JF -c $JF -o /tmp/rv_fconfirm.html -X POST $BASE/checkout --data-urlencode "intent=confirm" \
  --data-urlencode "family_name=鈴木" --data-urlencode "given_name=花子" --data-urlencode "postal_code=1500001" \
  --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=神宮前1-1-1" \
  --data-urlencode "phone=09012345678" --data-urlencode "csrf_token=$C"
curl -s -b $JF -c $JF -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "csrf_token=$C" -o /tmp/rv_fconfirm.html
C=$(csrf /tmp/rv_fconfirm.html)
curl -s -b $JF -c $JF -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$C"
FOID=$(db "SELECT id FROM orders WHERE user_id = $FOREIGN_ID ORDER BY id DESC LIMIT 1")
walk_to_delivered $FOID
[ "$(db "SELECT status FROM orders WHERE id = $FOID")" = "delivered" ] && ok "foreign order delivered" || ng "foreign order delivered"
curl -s -b $JF $BASE/products/$P/reviews/new -o /tmp/rv_fform.html
C=$(csrf /tmp/rv_fform.html)
curl -s -b $JF -c $JF -o /dev/null -X POST $BASE/products/$P/reviews \
  --data-urlencode "rating=2" --data-urlencode "comment=写真と違う" --data-urlencode "csrf_token=$C"
curl -s $BASE/products/$P -o /tmp/rv_two.html
grep -q 'review-average-num">3.0' /tmp/rv_two.html && grep -q "（2件）" /tmp/rv_two.html && ok "two reviews average 3.0" || ng "two reviews average 3.0"
HIGH=$(curl -s "$BASE/products/$P?sort=high")
LOW=$(curl -s "$BASE/products/$P?sort=low")
HPOS=$(echo "$HIGH" | grep -bo "少し小さいけど便利" | head -1 | cut -d: -f1); HPOS2=$(echo "$HIGH" | grep -bo "写真と違う" | head -1 | cut -d: -f1)
LPOS=$(echo "$LOW" | grep -bo "少し小さいけど便利" | head -1 | cut -d: -f1); LPOS2=$(echo "$LOW" | grep -bo "写真と違う" | head -1 | cut -d: -f1)
[ "$HPOS" -lt "$HPOS2" ] && [ "$LPOS" -gt "$LPOS2" ] && ok "sorts order high and low" || ng "sorts order high and low"

echo "-- foreign attacks on the author's review: all styled 404, row intact"
CODE=$(curl -s -b $JF -o /dev/null -w "%{http_code}" $BASE/reviews/$RID/edit)
[ "$CODE" = "404" ] && ok "foreign edit form is 404" || ng "foreign edit form is 404 ($CODE)"
curl -s -b $JF $BASE/products/$P -o /tmp/rv_fmd.html
C=$(csrf /tmp/rv_fmd.html)
CODE=$(curl -s -b $JF -o /dev/null -w "%{http_code}" -X POST $BASE/reviews/$RID \
  --data-urlencode "rating=1" --data-urlencode "comment=改ざん" --data-urlencode "csrf_token=$C")
[ "$CODE" = "404" ] && ok "foreign update is 404" || ng "foreign update is 404 ($CODE)"
CODE=$(curl -s -b $JF -o /dev/null -w "%{http_code}" -X POST $BASE/reviews/$RID/delete --data-urlencode "csrf_token=$C")
[ "$CODE" = "404" ] && ok "foreign delete is 404" || ng "foreign delete is 404 ($CODE)"
[ "$(db "SELECT rating FROM reviews WHERE id = $RID")" = "4" ] && ok "author's review untouched" || ng "author's review untouched"

echo "-- order detail carries the review button per line"
OID=$(db "SELECT order_id FROM reviews WHERE id = $RID")
curl -s -b $JA $BASE/mypage/orders/$OID -o /tmp/rv_order.html
grep -q "レビューを編集する" /tmp/rv_order.html && ok "order line shows edit button" || ng "order line shows edit button"

echo "-- author deletes: aggregate recomputes from what remains; fresh post allowed"
curl -s -b $JA $BASE/products/$P -o /tmp/rv_md3.html
C=$(csrf /tmp/rv_md3.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/reviews/$RID/delete --data-urlencode "csrf_token=$C"
[ "$(db "SELECT COUNT(*) FROM reviews WHERE id = $RID")" = "0" ] && ok "row hard-deleted" || ng "row hard-deleted"
curl -s "$BASE/search?q=キーホルダー" -o /tmp/rv_search2.html
grep -q "</span>2.0（1件）" /tmp/rv_search2.html && ok "card aggregate recomputed to the remaining review" || ng "card aggregate recomputed"
curl -s -b $JA $BASE/products/$P/reviews/new -o /tmp/rv_form2.html
C=$(csrf /tmp/rv_form2.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/products/$P/reviews \
  --data-urlencode "rating=3" --data-urlencode "comment=買い直しました" --data-urlencode "csrf_token=$C"
[ "$(db "SELECT COUNT(*) FROM reviews WHERE user_id = $AUTHOR_ID AND product_id = $P")" = "1" ] && ok "fresh review after delete" || ng "fresh review after delete"

echo "-- rating-only review: clearing the comment leaves stars, no comment text"
RID2=$(db "SELECT id FROM reviews WHERE user_id = $AUTHOR_ID AND product_id = $P")
curl -s -b $JA $BASE/reviews/$RID2/edit -o /tmp/rv_edit2.html
C=$(csrf /tmp/rv_edit2.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/reviews/$RID2 \
  --data-urlencode "rating=0.5" --data-urlencode "comment=" --data-urlencode "form_ticket=$(ticket /tmp/rv_edit2.html)" --data-urlencode "csrf_token=$C"
curl -s $BASE/products/$P -o /tmp/rv_bare.html
grep -q "買い直しました" /tmp/rv_bare.html && ng "comment cleared" || ok "comment cleared"
grep -q 'stars-fg" style="width: 10%' /tmp/rv_bare.html && ok "0.5 rating fills half of one star" || ng "0.5 rating fills half of one star"

result
