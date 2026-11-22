#!/bin/bash
# Coverage-gap smoke suite: five live
# features no other suite exercises — guest order lookup and
# its receipt, the admin returns flow, admin segments with a
# segment-targeted campaign audience, admin holidays, and
# gift options at checkout. Smoke depth on purpose: each
# feature is proven end to end once, not exhaustively.
#
# Fixture hygiene: this run's guest order is deleted with
# its children, the campaign / segment /
# return / inquiry rows are removed, and the shop's invoice
# registration number is restored — all from a trap, so a
# failed run leaves nothing behind either.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
MPW=$(printf "test%s" "1234")
ADMIN_EMAIL=admin@example.com
MEMBER=test@example.com
GUEST=cov-guest@example.com
M_UID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx $MEMBER)'")
JG=/tmp/cg_guest.txt; JM=/tmp/cg_member.txt; JA=/tmp/cg_admin.txt; JL=/tmp/cg_lookup.txt
OLD_REG=$(db "SELECT invoice_registration_number FROM shop_settings")
GOID=""
SEG_NAME="カバー範囲セグメント"
CAMP_NAME="カバー範囲キャンペーン"

cleanup() {
  local G_UID=""
  db "DELETE FROM email_campaigns WHERE name = '$CAMP_NAME'" >/dev/null
  db "DELETE FROM customer_segments WHERE name = '$SEG_NAME'" >/dev/null
  db "DELETE FROM return_requests WHERE inquiry_id IN (SELECT id FROM inquiries WHERE body = 'カバー範囲の返品テスト')" >/dev/null
  db "DELETE FROM inquiries WHERE body = 'カバー範囲の返品テスト'" >/dev/null
  db "DELETE FROM shop_holidays WHERE label = 'カバー範囲テスト'" >/dev/null
  # The guest order (guest orders carry user_id NULL —
  # the email lives on the order): remove this run's
  # order and its children, by the id captured at
  # placement. Other suites' guest orders are not ours
  # to delete.
  if [ -n "$GOID" ]; then
    db "DELETE FROM order_items WHERE order_id = $GOID" >/dev/null
    db "DELETE FROM payments WHERE order_id = $GOID" >/dev/null
    db "DELETE FROM orders WHERE id = $GOID" >/dev/null
  fi
  db "UPDATE shop_settings SET invoice_registration_number = '$OLD_REG'" >/dev/null
}
trap cleanup EXIT
cleanup

admin_login() {
  rm -f "$JA"
  curl -s -c "$JA" $BASE/admin/login -o /tmp/cg_alogin.html
  local C=$(csrf /tmp/cg_alogin.html)
  curl -s -b "$JA" -c "$JA" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "password=$APW" --data-urlencode "csrf_token=$C"
}
member_login() {
  rm -f "$JM"
  curl -s -c "$JM" $BASE/login -o /tmp/cg_mlogin.html
  local C=$(csrf /tmp/cg_mlogin.html)
  curl -s -b "$JM" -c "$JM" -o /dev/null -X POST $BASE/login \
    --data-urlencode "email=$MEMBER" --data-urlencode "password=$MPW" --data-urlencode "csrf_token=$C"
}
order_number_of() { # order id -> AS-YYYYMMDD-NNNN (JST placement date)
  echo "AS-$(TZ=Asia/Tokyo date +%Y%m%d)-$(printf %04d $(( $1 % 10000 )))"
}
# Places one product-5 order on the given jar for the given
# email; extra args are appended to the delivery submit
# (the gift fields ride there). The step-mail suite's
# proven shape: every step's token comes from that step's
# own GET page — the delivery submit answers 303, so its
# body carries no token.
place_order() { # jar email [extra delivery fields...]
  local jar=$1 email=$2; shift 2
  # The token comes from the product page: a guest's
  # empty cart page carries no POST form, so no token is
  # injected into it (the member cart page borrows the
  # layout's logout form — a guest has none).
  curl -s -b $jar $BASE/products/5 -o /tmp/cg_cart.html
  local tok=$(csrf /tmp/cg_cart.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/cart/items \
    --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "csrf_token=$tok"
  curl -s -b $jar $BASE/checkout -o /tmp/cg_co.html
  tok=$(csrf /tmp/cg_co.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/checkout \
    --data-urlencode "csrf_token=$tok" --data-urlencode "intent=confirm" \
    --data-urlencode "email=$email" --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" \
    --data-urlencode "family_name_kana=ヤマダ" --data-urlencode "given_name_kana=タロウ" \
    --data-urlencode "phone=09012345678" --data-urlencode "postal_code=1500001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
    --data-urlencode "address_line=神宮前1-1-1" --data-urlencode "building=" \
    --data-urlencode "delivery_date=" --data-urlencode "delivery_slot=" --data-urlencode "delivery_choice=new" \
    --data-urlencode "coupon_code=" --data-urlencode "coupon_select=" "$@"
  curl -s -b $jar $BASE/checkout/payment -o /tmp/cg_pay.html
  tok=$(csrf /tmp/cg_pay.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/checkout/payment \
    --data-urlencode "csrf_token=$tok" --data-urlencode "payment_method=cod" --data-urlencode "points_to_use=0"
  curl -s -b $jar $BASE/checkout -o /tmp/cg_co2.html
  tok=$(csrf /tmp/cg_co2.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "csrf_token=$tok"
}

echo "-- guest order: place, look up, wrong email refused, receipt"
rm -f $JG; curl -s -c $JG $BASE/ -o /dev/null
G_BEFORE=$(db "SELECT COALESCE(MAX(id), 0) FROM orders WHERE user_id IS NULL")
place_order $JG $GUEST
GOID=$(db "SELECT MAX(id) FROM orders WHERE user_id IS NULL AND id > $G_BEFORE")
[ -n "$GOID" ] && ok "CG-01 guest order placed ($GOID)" || ng "CG-01 guest order"
GNUM=$(order_number_of $GOID)
rm -f $JL
curl -s -c $JL $BASE/order-lookup -o /tmp/cg_lk.html; C=$(csrf /tmp/cg_lk.html)
curl -s -b $JL -c $JL -X POST $BASE/order-lookup -o /tmp/cg_lk_ok.html \
  --data-urlencode "order_number=$GNUM" --data-urlencode "email=$GUEST" --data-urlencode "csrf_token=$C"
grep -q "$GNUM" /tmp/cg_lk_ok.html && grep -q "神宮前1-1-1" /tmp/cg_lk_ok.html \
  && ok "CG-02 lookup by number+email shows the order" || ng "CG-02 lookup success"
curl -s -b $JL $BASE/order-lookup -o /tmp/cg_lk.html; C=$(csrf /tmp/cg_lk.html)
curl -s -b $JL -c $JL -X POST $BASE/order-lookup -o /tmp/cg_lk_ng.html \
  --data-urlencode "order_number=$GNUM" --data-urlencode "email=cov-other@example.com" --data-urlencode "csrf_token=$C"
grep -q "注文番号またはメールアドレスが一致しませんでした" /tmp/cg_lk_ng.html && ! grep -q "神宮前1-1-1" /tmp/cg_lk_ng.html \
  && ok "CG-03 wrong email refused with the indistinguishable message" || ng "CG-03 lookup refusal"
db "UPDATE shop_settings SET invoice_registration_number = 'T1234567890123'" >/dev/null
FOUND=""
for _ in $(seq 1 15); do
  curl -s -b $JL $BASE/order-lookup -o /tmp/cg_lk.html; C=$(csrf /tmp/cg_lk.html)
  curl -s -b $JL -c $JL -X POST $BASE/order-lookup/receipt -o /tmp/cg_receipt.html \
    --data-urlencode "order_number=$GNUM" --data-urlencode "email=$GUEST" --data-urlencode "csrf_token=$C"
  if grep -q "登録番号：T1234567890123" /tmp/cg_receipt.html; then FOUND=1; break; fi
  sleep 5
done
[ -n "$FOUND" ] && ok "CG-04 guest receipt renders the shop invoice number" || ng "CG-04 receipt invoice number"

echo "-- gift options: chosen at checkout, stored, shown on the order detail"
member_login
db "DELETE FROM cart_items WHERE user_id = $M_UID" >/dev/null
place_order $JM $MEMBER --data-urlencode "noshi=celebration" --data-urlencode "noshi_name=" \
  --data-urlencode "gift_message=お誕生日おめでとうございます"
MOID=$(db "SELECT MAX(id) FROM orders WHERE user_id = $M_UID")
# The message is PII ciphertext at rest (v1:...) — the
# plaintext proof is the detail page in CG-06.
[ "$(db "SELECT noshi_code || '/' || (gift_message LIKE 'v1:%') FROM orders WHERE id = $MOID")" = "celebration/true" ] \
  && ok "CG-05 gift options stored on the order" || ng "CG-05 gift stored"
curl -s -b $JM $BASE/mypage/orders/$MOID -o /tmp/cg_detail.html
grep -q "御祝" /tmp/cg_detail.html && grep -q "お誕生日おめでとうございます" /tmp/cg_detail.html \
  && ok "CG-06 gift options visible on the member order detail" || ng "CG-06 gift visible"

echo "-- admin returns: inquiry becomes a return, status advances"
MNUM=$(order_number_of $MOID)
db "INSERT INTO inquiries (user_id, name, email, category, order_number, body) VALUES ($M_UID, '山田太郎', '$MEMBER', 'returns', '$MNUM', 'カバー範囲の返品テスト')" >/dev/null
INQ=$(db "SELECT MAX(id) FROM inquiries WHERE body = 'カバー範囲の返品テスト'")
admin_login
curl -s -b $JA $BASE/admin/returns -o /tmp/cg_ret.html; C=$(csrf /tmp/cg_ret.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/returns/from-inquiry/$INQ --data-urlencode "csrf_token=$C"
RID=$(db "SELECT id FROM return_requests WHERE inquiry_id = $INQ")
[ -n "$RID" ] && [ "$(db "SELECT status FROM return_requests WHERE id = $RID")" = "requested" ] \
  && ok "CG-07 return created from the inquiry" || ng "CG-07 return create"
curl -s -b $JA $BASE/admin/returns -o /tmp/cg_ret.html; C=$(csrf /tmp/cg_ret.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/returns/$RID/status \
  --data-urlencode "status=approved" --data-urlencode "csrf_token=$C"
[ "$(db "SELECT status FROM return_requests WHERE id = $RID")" = "approved" ] \
  && ok "CG-08 return status change persists" || ng "CG-08 return status"

echo "-- admin segments + campaign audience"
curl -s -b $JA $BASE/admin/segments -o /tmp/cg_seg.html; C=$(csrf /tmp/cg_seg.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/segments \
  --data-urlencode "name=$SEG_NAME" --data-urlencode "tag=" \
  --data-urlencode "min_total_spend_yen=" --data-urlencode "min_order_count=1" \
  --data-urlencode "last_order_before=" --data-urlencode "last_order_after=" \
  --data-urlencode "csrf_token=$C"
SEGID=$(db "SELECT id FROM customer_segments WHERE name = '$SEG_NAME'")
[ -n "$SEGID" ] && ok "CG-09 segment created" || ng "CG-09 segment create"
db "UPDATE email_subscriptions SET status='subscribed' WHERE user_id = $M_UID" >/dev/null
curl -s -b $JA $BASE/admin/mail/campaigns/new -o /tmp/cg_camp.html; C=$(csrf /tmp/cg_camp.html)
TICKET=$(grep -o 'name="form_ticket" value="[^"]*"' /tmp/cg_camp.html | head -1 | sed 's/.*value="//; s/"$//')
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/mail/campaigns/new \
  --data-urlencode "name=$CAMP_NAME" --data-urlencode "subject=カバー範囲の件名" \
  --data-urlencode "body_html=<p>カバー範囲</p>" --data-urlencode "body_text=カバー範囲" \
  --data-urlencode "template_id=" --data-urlencode "segment_id=$SEGID" \
  --data-urlencode "scheduled_at=" --data-urlencode "form_ticket=$TICKET" --data-urlencode "csrf_token=$C"
CID=$(db "SELECT id FROM email_campaigns WHERE name = '$CAMP_NAME'")
[ -n "$CID" ] && ok "CG-10 segment-targeted campaign drafted" || ng "CG-10 campaign create"
curl -s -b $JA $BASE/admin/mail/campaigns/$CID/confirm -o /tmp/cg_confirm2.html
COUNT=$(grep -o '購読者 <strong>[0-9]*' /tmp/cg_confirm2.html | grep -o '[0-9]*$')
[ -n "$COUNT" ] && [ "$COUNT" -ge 1 ] && ok "CG-11 confirm page shows the segment audience ($COUNT)" || ng "CG-11 audience count"

echo "-- admin holidays: add and remove"
curl -s -b $JA $BASE/admin/settings/holidays -o /tmp/cg_hol.html; C=$(csrf /tmp/cg_hol.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/settings/holidays \
  --data-urlencode "holiday_date=2027-01-01" --data-urlencode "label=カバー範囲テスト" --data-urlencode "csrf_token=$C"
[ "$(db "SELECT COUNT(*) FROM shop_holidays WHERE holiday_date = '2027-01-01' AND label = 'カバー範囲テスト'")" = "1" ] \
  && ok "CG-12 holiday added" || ng "CG-12 holiday add"
curl -s -b $JA $BASE/admin/settings/holidays -o /tmp/cg_hol.html; C=$(csrf /tmp/cg_hol.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/settings/holidays/delete \
  --data-urlencode "holiday_date=2027-01-01" --data-urlencode "csrf_token=$C"
[ "$(db "SELECT COUNT(*) FROM shop_holidays WHERE holiday_date = '2027-01-01'")" = "0" ] \
  && ok "CG-13 holiday removed" || ng "CG-13 holiday remove"

result
