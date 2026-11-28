#!/bin/bash
# Address book verification (Address Book spec §7, TC-01..TC-12;
# TC-13 is the gate-level regression battery). Mypage CRUD with
# the shared checkout validation, the one-default invariant,
# IDOR indistinguishability, the checkout picker with
# copy-at-submit semantics, snapshot isolation, and the
# placement-time save checkbox.
#
# Member A is the shared test member; member B is signed up
# fresh through the real flow for the IDOR cases. Fixture
# cleanup is scoped to those two accounts' address rows.
BASE=http://127.0.0.1:3000
MAIL=http://127.0.0.1:8025
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1" | head -1; }
CT=$(printf "csrf_%s" "token")
PW=$(printf "pass%s" "word")
TESTPW=$(printf "test%s" "1234")
BPW=$(printf "addr%s" "5678")
JA=/tmp/addr_a.txt; JB=/tmp/addr_b.txt; JG=/tmp/addr_g.txt; JAD=/tmp/addr_admin.txt
APW=$(printf "Password%s" "1234")
AID=$(db "SELECT id FROM users WHERE email_bidx='$(bidx test@example.com)'")
max_oid() { db "SELECT COALESCE(max(id),0) FROM orders"; }
acount() { db "SELECT count(*) FROM member_addresses WHERE user_id=$1"; }
adefault() { db "SELECT COALESCE((SELECT id FROM member_addresses WHERE user_id=$1 AND is_default),0)"; }

login_a() {
  rm -f $JA
  curl -s -c $JA $BASE/login -o /tmp/addr_login.html
  local C=$(csrf /tmp/addr_login.html)
  curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/login --data-urlencode "email=test@example.com" --data-urlencode "$PW=$TESTPW" --data-urlencode "$CT=$C"
}
ADDR1=(--data-urlencode "label=自宅" --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎"
  --data-urlencode "postal_code=150-0001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区"
  --data-urlencode "address_line=神宮前1-1-1" --data-urlencode "building=" --data-urlencode "phone=090-1234-5678")
ADDR2=(--data-urlencode "label=実家" --data-urlencode "family_name=山田" --data-urlencode "given_name=花子"
  --data-urlencode "postal_code=5300001" --data-urlencode "prefecture=大阪府" --data-urlencode "city=大阪市北区"
  --data-urlencode "address_line=梅田2-2-2" --data-urlencode "building=梅田ハイツ301" --data-urlencode "phone=08022223333")

echo "===== setup ====="
db "UPDATE product_variants SET stock=25 WHERE product_id=5" >/dev/null
db "DELETE FROM member_addresses WHERE user_id=$AID" >/dev/null
login_a
curl -s -b $JA $BASE/mypage -o /tmp/addr_mp.html
grep -q "注文履歴" /tmp/addr_mp.html && ok "member A login works" || ng "member A login works"
grep -q "アドレス帳" /tmp/addr_mp.html && ok "mypage links to the address book" || ng "mypage address book link"

echo "===== TC-01: CRUD happy path; the first address becomes default ====="
curl -s -b $JA $BASE/mypage/addresses/new -o /tmp/addr_form.html
C=$(csrf /tmp/addr_form.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/mypage/addresses/new "${ADDR1[@]}" --data-urlencode "$CT=$C"
curl -s -b $JA $BASE/mypage/addresses -o /tmp/addr_list.html
grep -q "自宅" /tmp/addr_list.html && grep -q "デフォルト" /tmp/addr_list.html && ok "TC-01 first address listed as default" || ng "TC-01 first address default"
ROW="$(pii_dec "$(db "SELECT postal_code FROM member_addresses WHERE user_id=$AID")")|$(pii_dec "$(db "SELECT phone FROM member_addresses WHERE user_id=$AID")")|$(db "SELECT is_default::text FROM member_addresses WHERE user_id=$AID")"
[ "$ROW" = "1500001|09012345678|true" ] && ok "TC-01 stored normalised (digits) and default" || ng "TC-01 stored row ($ROW)"
ID1=$(db "SELECT id FROM member_addresses WHERE user_id=$AID")
curl -s -b $JA $BASE/mypage/addresses/new -o /tmp/addr_form.html
C=$(csrf /tmp/addr_form.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/mypage/addresses/new "${ADDR2[@]}" --data-urlencode "$CT=$C"
ID2=$(db "SELECT id FROM member_addresses WHERE user_id=$AID AND label='実家'")
[ -n "$ID2" ] && [ "$(adefault $AID)" = "$ID1" ] && ok "TC-01 second address saved, default unmoved" || ng "TC-01 second address"
curl -s -b $JA $BASE/mypage/addresses/$ID2/edit -o /tmp/addr_edit.html
grep -q "梅田2-2-2" /tmp/addr_edit.html && ok "TC-01 edit form prefilled" || ng "TC-01 edit prefilled"
C=$(csrf /tmp/addr_edit.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/mypage/addresses/$ID2/edit \
  --data-urlencode "label=実家" --data-urlencode "family_name=山田" --data-urlencode "given_name=花子" \
  --data-urlencode "postal_code=5300001" --data-urlencode "prefecture=大阪府" --data-urlencode "city=大阪市中央区" \
  --data-urlencode "address_line=梅田2-2-2" --data-urlencode "building=梅田ハイツ301" --data-urlencode "phone=08022223333" \
  --data-urlencode "$CT=$C" --data-urlencode "form_ticket=$(ticket /tmp/addr_edit.html)"
[ "$(pii_dec "$(db "SELECT city FROM member_addresses WHERE id=$ID2")")" = "大阪市中央区" ] && ok "TC-01 edit applied" || ng "TC-01 edit applied"

echo "===== TC-04: default invariants ====="
curl -s -b $JA $BASE/mypage/addresses -o /tmp/addr_list.html
C=$(csrf /tmp/addr_list.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/mypage/addresses/$ID2/default --data-urlencode "$CT=$C"
[ "$(adefault $AID)" = "$ID2" ] && [ "$(db "SELECT count(*) FROM member_addresses WHERE user_id=$AID AND is_default")" = "1" ] && ok "TC-04 set-default switches atomically" || ng "TC-04 set-default"
curl -s -b $JA $BASE/mypage/addresses/$ID2/delete -o /tmp/addr_del.html
grep -q "最も古いものがデフォルトになります" /tmp/addr_del.html && ok "TC-04 delete page explains promotion" || ng "TC-04 delete promotion note"
C=$(csrf /tmp/addr_del.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/mypage/addresses/$ID2/delete --data-urlencode "$CT=$C"
[ "$(adefault $AID)" = "$ID1" ] && ok "TC-04 deleting the default promotes the oldest remaining" || ng "TC-04 promotion"
curl -s -b $JA $BASE/mypage/addresses/$ID1/delete -o /tmp/addr_del.html
C=$(csrf /tmp/addr_del.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/mypage/addresses/$ID1/delete --data-urlencode "$CT=$C"
curl -s -b $JA $BASE/mypage/addresses -o /tmp/addr_list.html
grep -q "お届け先はまだ登録されていません" /tmp/addr_list.html && [ "$(acount $AID)" = "0" ] && ok "TC-01/04 delete removes; empty book renders its message" || ng "TC-01 delete + empty state"

echo "===== TC-02: validation returns every error at once and saves nothing ====="
curl -s -b $JA $BASE/mypage/addresses/new -o /tmp/addr_form.html
C=$(csrf /tmp/addr_form.html)
curl -s -b $JA -c $JA -X POST $BASE/mypage/addresses/new -o /tmp/addr_bad.html \
  --data-urlencode "label=$(printf 'あ%.0s' $(seq 1 21))" --data-urlencode "family_name=" --data-urlencode "given_name=太郎" \
  --data-urlencode "postal_code=123" --data-urlencode "prefecture=火星" --data-urlencode "city=" \
  --data-urlencode "address_line=" --data-urlencode "building=" --data-urlencode "phone=abc" --data-urlencode "$CT=$C"
VOK=1
for want in "ラベルは20文字以内" "姓を入力してください" "郵便番号は7桁の数字" "都道府県を選択してください" "市区町村を入力してください" "番地を入力してください" "電話番号が正しくありません"; do
  grep -q "$want" /tmp/addr_bad.html || { VOK=0; echo "    missing error: $want"; }
done
[ "$VOK" = "1" ] && ok "TC-02 all field errors at once" || ng "TC-02 all field errors"
[ "$(acount $AID)" = "0" ] && ok "TC-02 nothing saved" || ng "TC-02 nothing saved"

echo "===== TC-03: the cap at 5 (the configured default) ====="
db "INSERT INTO member_addresses (user_id, label, family_name, given_name, postal_code, prefecture, city, address_line, phone, is_default) SELECT $AID, '一括'||g, '山田', '太郎', '1500001', '東京都', '渋谷区', '神宮前1-1-'||g, '09012345678', g=1 FROM generate_series(1,5) g" >/dev/null
curl -s -b $JA $BASE/mypage/addresses -o /tmp/addr_list.html
grep -q "上限（5件）に達しています" /tmp/addr_list.html && ok "TC-03 list shows the cap state" || ng "TC-03 cap state on list"
curl -s -b $JA $BASE/mypage/addresses/new -o /tmp/addr_form.html
C=$(csrf /tmp/addr_form.html)
curl -s -b $JA -c $JA -X POST $BASE/mypage/addresses/new "${ADDR1[@]}" --data-urlencode "$CT=$C" -o /tmp/addr_cap.html
grep -q "アドレス帳の上限（5件）に達しています" /tmp/addr_cap.html && [ "$(acount $AID)" = "5" ] && ok "TC-03 the 6th address is refused with the cap error" || ng "TC-03 cap refusal"
db "DELETE FROM member_addresses WHERE user_id=$AID" >/dev/null

echo "===== TC-05: IDOR — member B's address is a 404 for member A ====="
# B is suite-owned and persists across runs (its account
# cannot be deleted: the WELCOME500 coupon row references
# it). Log in with the known password; sign up only when the
# account does not exist yet (the first run).
login_b() {
  rm -f $JB
  curl -s -c $JB $BASE/login -o /tmp/addr_login_b.html
  local C=$(csrf /tmp/addr_login_b.html)
  curl -s -b $JB -c $JB -o /dev/null -X POST $BASE/login --data-urlencode "email=addr-b@example.jp" --data-urlencode "$PW=$BPW" --data-urlencode "$CT=$C"
  curl -s -b $JB $BASE/mypage -o /tmp/addr_bmp.html
  grep -q "注文履歴" /tmp/addr_bmp.html
}
if ! login_b; then
rm -f $JB
curl -s -c $JB $BASE/signup -o /tmp/addr_su.html
C=$(csrf /tmp/addr_su.html)
curl -s -b $JB -c $JB -o /dev/null -X POST $BASE/signup --data-urlencode "email=addr-b@example.jp" --data-urlencode "email_confirm=addr-b@example.jp" --data-urlencode "$CT=$C"
sleep 2
BCODE=$(curl -s "$MAIL/api/v1/messages" | python3 -c "
import json, sys, re
msgs = [m for m in json.load(sys.stdin)['messages'] if any(t['Address'] == 'addr-b@example.jp' for t in m['To'])]
print(re.search(r'(\d{6})', msgs[0]['Snippet']).group(1) if msgs else '')
")
curl -s -b $JB $BASE/signup/verify -o /tmp/addr_v.html
C=$(csrf /tmp/addr_v.html)
curl -s -b $JB -c $JB -o /dev/null -X POST $BASE/signup/verify --data-urlencode "code=$BCODE" --data-urlencode "$CT=$C"
curl -s -b $JB $BASE/signup/details -o /tmp/addr_d.html
C=$(csrf /tmp/addr_d.html)
curl -s -b $JB -c $JB -X POST $BASE/signup/details -o /tmp/addr_conf.html \
  --data-urlencode "$PW=$BPW" --data-urlencode "family_name=鈴木" --data-urlencode "given_name=一郎" \
  --data-urlencode "family_name_kana=スズキ" --data-urlencode "given_name_kana=イチロウ" \
  --data-urlencode "birth_date=" --data-urlencode "gender=" \
  --data-urlencode "postal_code=1000001" --data-urlencode "prefecture=東京都" --data-urlencode "city=千代田区" \
  --data-urlencode "address_line=千代田1-1" --data-urlencode "building=" --data-urlencode "phone=09033334444" \
  --data-urlencode "agree_terms=true" --data-urlencode "$CT=$C"
C=$(csrf /tmp/addr_conf.html)
curl -s -b $JB -c $JB -o /dev/null -X POST $BASE/signup/confirm --data-urlencode "$CT=$C"
curl -s -b $JB $BASE/mypage -o /tmp/addr_bmp.html
grep -q "注文履歴" /tmp/addr_bmp.html && ok "member B signed up" || ng "member B signup"
fi
BID=$(db "SELECT id FROM users WHERE email_bidx='$(bidx addr-b@example.jp)'")
db "DELETE FROM member_addresses WHERE user_id=$BID" >/dev/null
curl -s -b $JB $BASE/mypage/addresses/new -o /tmp/addr_form.html
C=$(csrf /tmp/addr_form.html)
curl -s -b $JB -c $JB -o /dev/null -X POST $BASE/mypage/addresses/new "${ADDR2[@]}" --data-urlencode "$CT=$C"
XB=$(db "SELECT id FROM member_addresses WHERE user_id=$BID")
[ -n "$XB" ] && ok "member B saved an address" || ng "member B address saved"
S1=$(curl -s -b $JA -o /tmp/addr_404a.html -w "%{http_code}" $BASE/mypage/addresses/$XB/edit)
S2=$(curl -s -b $JA -o /dev/null -w "%{http_code}" $BASE/mypage/addresses/999999/edit)
[ "$S1" = "404" ] && [ "$S2" = "404" ] && grep -q "お届け先が見つかりませんでした" /tmp/addr_404a.html && ok "TC-05 foreign id and nonexistent id are the same styled 404" || ng "TC-05 404 indistinguishability ($S1/$S2)"
S3=$(curl -s -b $JA -o /dev/null -w "%{http_code}" $BASE/mypage/addresses/$XB/delete)
curl -s -b $JA $BASE/mypage/addresses -o /tmp/addr_list.html
C=$(csrf /tmp/addr_list.html)
S4=$(curl -s -b $JA -c $JA -o /dev/null -w "%{http_code}" -X POST $BASE/mypage/addresses/$XB/default --data-urlencode "$CT=$C")
S5=$(curl -s -b $JA -c $JA -o /dev/null -w "%{http_code}" -X POST $BASE/mypage/addresses/$XB/delete --data-urlencode "$CT=$C")
[ "$S3" = "404" ] && [ "$S4" = "404" ] && [ "$S5" = "404" ] && [ "$(pii_dec "$(db "SELECT city FROM member_addresses WHERE id=$XB")")" = "大阪市北区" ] && ok "TC-05 delete/default on a foreign id refuse and change nothing" || ng "TC-05 foreign writes ($S3/$S4/$S5)"

echo "===== TC-06/TC-10: the checkout picker ====="
# A gets two addresses: default 自宅 (Tokyo) and 実家 (Osaka).
curl -s -b $JA $BASE/mypage/addresses/new -o /tmp/addr_form.html
C=$(csrf /tmp/addr_form.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/mypage/addresses/new "${ADDR1[@]}" --data-urlencode "$CT=$C"
curl -s -b $JA $BASE/mypage/addresses/new -o /tmp/addr_form.html
C=$(csrf /tmp/addr_form.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/mypage/addresses/new "${ADDR2[@]}" --data-urlencode "$CT=$C"
D_HOME=$(db "SELECT id FROM member_addresses WHERE user_id=$AID AND label='自宅'")
D_OSAKA=$(db "SELECT id FROM member_addresses WHERE user_id=$AID AND label='実家'")
cart_add_a() {
  curl -s -b $JA $BASE/ -o /tmp/addr_home.html
  local C=$(csrf /tmp/addr_home.html)
  curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
}
checkout_to_payment() { # delivery fields... -> /tmp/addr_confirm.html via payment post
  curl -s -b $JA $BASE/checkout -o /tmp/addr_co.html
  local C=$(csrf /tmp/addr_co.html)
  local args=(); for kv in "$@"; do args+=(--data-urlencode "$kv"); done
  curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/checkout --data-urlencode "intent=confirm" "${args[@]}" --data-urlencode "$CT=$C"
  curl -s -b $JA -c $JA -X POST $BASE/checkout/payment --data-urlencode "payment_method=cod" --data-urlencode "$CT=$C" -o /tmp/addr_confirm.html
}
cart_add_a
curl -s -b $JA $BASE/checkout -o /tmp/addr_co.html
grep -q "data-address-picker" /tmp/addr_co.html && grep -q "新しいお届け先に送る" /tmp/addr_co.html && ok "TC-10 picker renders for a member with a book" || ng "TC-10 picker renders"
grep -q "name=\"delivery_choice\" value=\"$D_HOME\" checked" /tmp/addr_co.html && ok "TC-06 the default is pre-selected" || ng "TC-06 default pre-selected"
grep -q "この住所をアドレス帳に保存する" /tmp/addr_co.html && ok "TC-10 the save checkbox is on the typed path" || ng "TC-10 save checkbox"
PRE=$(max_oid)
checkout_to_payment "delivery_choice=$D_OSAKA" "family_name=別人" "given_name=他人" "postal_code=1000001" "prefecture=東京都" "city=千代田区" "address_line=別人1-1" "building=" "phone=09000000000" "delivery_date=" "delivery_slot=none"
grep -q "大阪市北区梅田2-2-2" /tmp/addr_confirm.html && ok "TC-06 confirm shows the picked address, not the typed fields" || ng "TC-06 confirm picked values"
C=$(csrf /tmp/addr_confirm.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$C"
OID_PICK=$(max_oid)
ROW="$(pii_dec "$(db "SELECT prefecture FROM orders WHERE id=$OID_PICK")")|$(pii_dec "$(db "SELECT city FROM orders WHERE id=$OID_PICK")")|$(pii_dec "$(db "SELECT family_name FROM orders WHERE id=$OID_PICK")")"
[ "$OID_PICK" -gt "$PRE" ] && [ "$ROW" = "大阪府|大阪市北区|山田" ] && ok "TC-06 the placed order carries the picked address" || ng "TC-06 order from pick ($ROW)"

echo "===== TC-07: snapshot isolation against edit + delete ====="
curl -s -b $JA $BASE/mypage/addresses/$D_OSAKA/edit -o /tmp/addr_edit.html
C=$(csrf /tmp/addr_edit.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/mypage/addresses/$D_OSAKA/edit \
  --data-urlencode "label=実家" --data-urlencode "family_name=山田" --data-urlencode "given_name=花子" \
  --data-urlencode "postal_code=5300001" --data-urlencode "prefecture=大阪府" --data-urlencode "city=大阪市中央区" \
  --data-urlencode "address_line=梅田9-9-9" --data-urlencode "building=" --data-urlencode "phone=08022223333" --data-urlencode "$CT=$C" --data-urlencode "form_ticket=$(ticket /tmp/addr_edit.html)"
curl -s -b $JA $BASE/mypage/addresses/$D_OSAKA/delete -o /tmp/addr_del.html
C=$(csrf /tmp/addr_del.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/mypage/addresses/$D_OSAKA/delete --data-urlencode "$CT=$C"
curl -s -b $JA $BASE/mypage/orders/$OID_PICK -o /tmp/addr_odetail.html
grep -q "大阪市北区" /tmp/addr_odetail.html && ok "TC-07 member order detail keeps the original address" || ng "TC-07 member snapshot"
rm -f $JAD
curl -s -c $JAD $BASE/admin/login -o /tmp/addr_al.html
C=$(csrf /tmp/addr_al.html)
curl -s -b $JAD -c $JAD -o /dev/null -X POST $BASE/admin/login --data-urlencode "email=admin@example.com" --data-urlencode "$PW=$APW" --data-urlencode "$CT=$C"
curl -s -b $JAD $BASE/admin/orders/$OID_PICK -o /tmp/addr_adetail.html
grep -q "大阪市北区" /tmp/addr_adetail.html && ok "TC-07 admin order detail keeps the original address" || ng "TC-07 admin snapshot"

echo "===== TC-11: a mid-checkout edit does not reach the in-flight order ====="
cart_add_a
checkout_to_payment "delivery_choice=$D_HOME" "delivery_date=" "delivery_slot=none"
curl -s -b $JA $BASE/mypage/addresses/$D_HOME/edit -o /tmp/addr_edit.html
C=$(csrf /tmp/addr_edit.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/mypage/addresses/$D_HOME/edit \
  --data-urlencode "label=自宅" --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=品川区" \
  --data-urlencode "address_line=大崎3-3-3" --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "$CT=$C" --data-urlencode "form_ticket=$(ticket /tmp/addr_edit.html)"
PRE=$(max_oid)
C=$(csrf /tmp/addr_confirm.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$C"
OID_MID=$(max_oid)
[ "$OID_MID" -gt "$PRE" ] && [ "$(pii_dec "$(db "SELECT city FROM orders WHERE id=$OID_MID")")" = "渋谷区" ] && ok "TC-11 the order carries the values copied at delivery-submit" || ng "TC-11 copy-at-submit"

echo "===== TC-08: foreign / stale ids at checkout ====="
cart_add_a
PRE=$(max_oid)
curl -s -b $JA $BASE/checkout -o /tmp/addr_co.html
C=$(csrf /tmp/addr_co.html)
curl -s -b $JA -c $JA -X POST $BASE/checkout -o /tmp/addr_rej.html --data-urlencode "intent=confirm" \
  --data-urlencode "delivery_choice=$XB" --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前1-1-1" --data-urlencode "building=" --data-urlencode "phone=09012345678" \
  --data-urlencode "delivery_date=" --data-urlencode "delivery_slot=none" --data-urlencode "$CT=$C"
grep -q "お届け先を選択し直してください" /tmp/addr_rej.html && [ "$(max_oid)" = "$PRE" ] && ok "TC-08 a foreign id returns to the delivery step, no order" || ng "TC-08 foreign id"
curl -s -b $JA -c $JA -X POST $BASE/checkout -o /tmp/addr_rej2.html --data-urlencode "intent=confirm" \
  --data-urlencode "delivery_choice=999999" --data-urlencode "family_name=山田" --data-urlencode "given_name=太郎" \
  --data-urlencode "postal_code=1500001" --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" \
  --data-urlencode "address_line=神宮前1-1-1" --data-urlencode "building=" --data-urlencode "phone=09012345678" \
  --data-urlencode "delivery_date=" --data-urlencode "delivery_slot=none" --data-urlencode "$CT=$C"
grep -q "お届け先を選択し直してください" /tmp/addr_rej2.html && [ "$(max_oid)" = "$PRE" ] && ok "TC-08 a stale id returns the same way" || ng "TC-08 stale id"

echo "===== TC-09: the save checkbox ====="
db "DELETE FROM member_addresses WHERE user_id=$AID" >/dev/null
cart_add_a
checkout_to_payment "delivery_choice=new" "family_name=佐藤" "given_name=花" "postal_code=1500001" "prefecture=東京都" "city=渋谷区" "address_line=保存町5-5-5" "building=" "phone=09055556666" "delivery_date=" "delivery_slot=none" "save_address=1"
C=$(csrf /tmp/addr_confirm.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$C"
SAVED="$(pii_dec "$(db "SELECT address_line FROM member_addresses WHERE user_id=$AID")")|$(db "SELECT is_default::text FROM member_addresses WHERE user_id=$AID")"
[ "$SAVED" = "保存町5-5-5|true" ] && ok "TC-09 checked save lands in the book as the first (default) address" || ng "TC-09 save on placement ($SAVED)"
cart_add_a
checkout_to_payment "delivery_choice=new" "family_name=佐藤" "given_name=花" "postal_code=1500001" "prefecture=東京都" "city=渋谷区" "address_line=未保存町6-6-6" "building=" "phone=09055556666" "delivery_date=" "delivery_slot=none"
C=$(csrf /tmp/addr_confirm.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$C"
[ "$(acount $AID)" = "1" ] && ok "TC-09 unchecked saves nothing" || ng "TC-09 unchecked"
cart_add_a
checkout_to_payment "delivery_choice=new" "family_name=佐藤" "given_name=花" "postal_code=1500001" "prefecture=東京都" "city=渋谷区" "address_line=放棄町7-7-7" "building=" "phone=09055556666" "delivery_date=" "delivery_slot=none" "save_address=1"
[ "$(acount $AID)" = "1" ] && ok "TC-09 an abandoned checkout saves nothing" || ng "TC-09 abandoned"
db "INSERT INTO member_addresses (user_id, label, family_name, given_name, postal_code, prefecture, city, address_line, phone, is_default) SELECT $AID, '一括'||g, '山田', '太郎', '1500001', '東京都', '渋谷区', '神宮前1-1-'||g, '09012345678', false FROM generate_series(1,4) g" >/dev/null
cart_add_a
checkout_to_payment "delivery_choice=new" "family_name=佐藤" "given_name=花" "postal_code=1500001" "prefecture=東京都" "city=渋谷区" "address_line=上限町8-8-8" "building=" "phone=09055556666" "delivery_date=" "delivery_slot=none" "save_address=1"
C=$(csrf /tmp/addr_confirm.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$C"
curl -s -b $JA $BASE/checkout/complete -o /tmp/addr_complete.html
grep -q "アドレス帳の上限に達したため、住所は保存されませんでした" /tmp/addr_complete.html && [ "$(acount $AID)" = "5" ] && ok "TC-09 at-cap: order places, save skipped with the completion note" || ng "TC-09 at-cap note"
db "DELETE FROM member_addresses WHERE user_id=$AID" >/dev/null

echo "===== TC-10: empty book and guests see today's step ====="
cart_add_a
curl -s -b $JA $BASE/checkout -o /tmp/addr_co.html
grep -q "data-address-picker" /tmp/addr_co.html && ng "TC-10 no picker for an empty book" || ok "TC-10 no picker for an empty book"
rm -f $JG
curl -s -c $JG $BASE/ -o /tmp/addr_ghome.html
C=$(csrf /tmp/addr_ghome.html)
curl -s -b $JG -c $JG -o /dev/null -X POST $BASE/cart/items --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CT=$C"
curl -s -b $JG $BASE/checkout -o /tmp/addr_gco.html
grep -q "data-address-picker" /tmp/addr_gco.html && ng "TC-10 no picker for guests" || ok "TC-10 no picker for guests"
grep -q "この住所をアドレス帳に保存する" /tmp/addr_gco.html && ng "TC-10 no save checkbox for guests" || ok "TC-10 no save checkbox for guests"

echo "===== TC-12: no address contents in the front log ====="
LOG="$(cd "$(dirname "$0")/../.." && pwd)/logs/front_app.log.$(date +%F)"
grep -q "address.create" $LOG && ok "TC-12 address actions are logged by name" || ng "TC-12 action names logged"
LEAK=0
for s in "梅田2-2-2" "09012345678" "保存町5-5-5" "上限町8-8-8"; do
  grep -q "$s" $LOG && { LEAK=1; echo "    leaked: $s"; }
done
[ "$LEAK" = "0" ] && ok "TC-12 no street address or phone in the log" || ng "TC-12 log leak"

result
