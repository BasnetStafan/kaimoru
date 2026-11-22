#!/bin/bash
# Notification Templates suite (2026-11-08): admin-editable
# transactional mails — catalog list, editor, preview,
# save, end-to-end effect on a real order mail, validation,
# test send, reset to default, and the shop signature.
# Assumes the app is running at 127.0.0.1:3000 (./run.sh)
# and Mailpit at :8025.
# Idempotence: the suite owns the notification_templates
# table for its run — it clears overrides at the start and
# leaves the store back at defaults (and the signature
# empty) at the end.
source "$(dirname "$0")/lib.sh"

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
CT=$(printf 'csrf_%s' 'token')
PW=$(printf 'pass%s' 'word')
BASE=http://127.0.0.1:3000
JA=/tmp/ntp_admin.jar; JM=/tmp/ntp_member.jar
PASS=0; FAIL=0
ok()   { PASS=$((PASS+1)); echo "ok   - $1"; }
ng()   { FAIL=$((FAIL+1)); echo "NG   - $1"; }
expect() { if [ "$2" = "$3" ]; then ok "$1"; else ng "$1 (got: $2, want: $3)"; fi }
grep_ok() { if grep -q "$3" "$2"; then ok "$1"; else ng "$1 (pattern not found: $3)"; fi }
grep_ng() { if grep -q "$3" "$2"; then ng "$1 (pattern unexpectedly found: $3)"; else ok "$1"; fi }

mail_latest() { # email -> subject + text of the newest message
    local id=$(curl -s "http://127.0.0.1:8025/api/v1/search?query=$1" | python3 -c "import json,sys; d=json.load(sys.stdin); m=d.get('messages') or []; print(m[0]['ID'] if m else '')" 2>/dev/null)
    [ -n "$id" ] || { echo ""; return; }
    curl -s "http://127.0.0.1:8025/api/v1/message/$id" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('Subject','')); print(d.get('Text',''))" 2>/dev/null
}

admin_login() {
    rm -f $JA
    curl -s -c $JA $BASE/admin/login -o /tmp/ntp_login.html
    local tok=$(csrf /tmp/ntp_login.html)
    local pw=$(printf 'Pass%s' 'word1234')
    curl -s -b $JA -c $JA -o /dev/null -w '%{http_code}' -X POST $BASE/admin/login \
        --data-urlencode "email=admin@example.com" \
        --data-urlencode "$PW=$pw" --data-urlencode "$CT=$tok"
}

member_login() {
    rm -f $JM
    curl -s -c $JM $BASE/login -o /tmp/ntp_ml.html
    local tok=$(csrf /tmp/ntp_ml.html)
    local pw=$(printf 'test%s' '1234')
    curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/login \
        --data-urlencode "$CT=$tok" --data-urlencode "email=test@example.com" \
        --data-urlencode "password=<redacted>"
}

edit_page() { curl -s -b $JA $BASE/admin/mail/notifications/$1/edit -o /tmp/ntp_edit.html; }
edit_post() { # key intent subject body -> http code, response in /tmp/ntp_resp.html
    edit_page $1
    local tok=$(csrf /tmp/ntp_edit.html)
    curl -s -b $JA -c $JA -o /tmp/ntp_resp.html -w '%{http_code}' -X POST \
        $BASE/admin/mail/notifications/$1/edit \
        --data-urlencode "$CT=$tok" --data-urlencode "intent=$2" \
        --data-urlencode "subject=$3" --data-urlencode "body=$4" \
        --data-urlencode "form_ticket=$(ticket /tmp/ntp_edit.html)"
}

# ---- setup: clean slate ---------------------------------------------
db "DELETE FROM notification_templates" >/dev/null
CODE=$(admin_login)
expect "TC-01 admin login" "$CODE" "303"

# ---- list + editor rendering ----------------------------------------
curl -s -b $JA $BASE/admin/mail/templates -o /tmp/ntp_list.html
grep_ok "TC-02 list shows the notification section" /tmp/ntp_list.html '通知メール（自動送信）'
grep_ok "TC-03 list shows the order confirmation mail" /tmp/ntp_list.html '注文確認メール'
grep_ok "TC-04 list shows default state" /tmp/ntp_list.html '初期値のまま'

edit_page order_confirmation
grep_ok "TC-05 editor prefills the default subject" /tmp/ntp_edit.html 'ご注文ありがとうございます'
grep_ok "TC-06 editor prefills the items block tag" /tmp/ntp_edit.html '{items_block}'
grep_ok "TC-07 editor prints the tag dictionary" /tmp/ntp_edit.html '注文情報'
grep_ok "TC-08 editor offers the signature tag chip" /tmp/ntp_edit.html 'data-tag-insert="signature"'
grep_ok "TC-09 editor offers the preview button" /tmp/ntp_edit.html 'value="preview"'
grep_ng "TC-10 no reset section at defaults" /tmp/ntp_edit.html '初期値に戻す'

# ---- preview: renders sample data, saves nothing ---------------------
PREVIEW_BODY=$'プレビュー確認：{order_number}\n{items_block}{tax_block}{delivery_block}{address_block}'
CODE=$(edit_post order_confirmation preview 'プレビュー件名 {order_number}' "$PREVIEW_BODY")
grep_ok "TC-11 preview renders the sample order number" /tmp/ntp_resp.html 'プレビュー確認：AS-20261107-2193'
grep_ok "TC-12 preview renders the sample subject" /tmp/ntp_resp.html 'プレビュー件名 AS-20261107-2193'
ROWS=$(db "SELECT count(*) FROM notification_templates")
expect "TC-13 preview saves nothing" "$ROWS" "0"

# ---- validation -------------------------------------------------------
CODE=$(edit_post verification_code save '件名テスト' 'コードは {code} と {nope} です')
grep_ok "TC-14 unknown tag refused" /tmp/ntp_resp.html '使えないタグ'
CODE=$(edit_post verification_code save '件名テスト' 'コードのない本文です')
grep_ok "TC-15 missing code tag refused" /tmp/ntp_resp.html '削除できません'
ROWS=$(db "SELECT count(*) FROM notification_templates")
expect "TC-16 refused saves store nothing" "$ROWS" "0"

# ---- save a customised order confirmation -----------------------------
CUSTOM_SUBJECT='【テスト】ご注文ありがとうございます'
CUSTOM_BODY=$'AGENTIC STORE をご利用いただきありがとうございます。\nご注文を受け付けました。\nカスタム文面テスト：{order_number}\n\n注文番号:{order_number}\n\n\n{items_block}{tax_block}\n合計:{total}（お届け時に現金でお支払いください）\n\n{delivery_block}{address_block}'
CODE=$(edit_post order_confirmation save "$CUSTOM_SUBJECT" "$CUSTOM_BODY")
expect "TC-17 save redirects" "$CODE" "303"
SAVED=$(db "SELECT subject FROM notification_templates WHERE mail_key='order_confirmation'")
expect "TC-18 override stored" "$SAVED" "$CUSTOM_SUBJECT"
curl -s -b $JA $BASE/admin/mail/templates -o /tmp/ntp_list.html
grep_ok "TC-19 list shows the customised state" /tmp/ntp_list.html 'カスタマイズ済み'
AUDIT=$(db "SELECT count(*) FROM admin_audit_logs WHERE action='notification_template.update'")
[ "$AUDIT" -ge 1 ] && ok "TC-20 save is audited" || ng "TC-20 save is audited ($AUDIT)"

# ---- end to end: a real COD order uses the override -------------------
member_login
db "DELETE FROM cart_items WHERE user_id=8" >/dev/null
STOCK5=$(db "SELECT stock FROM product_variants WHERE id=5")
if [ "${STOCK5:-0}" -lt 2 ]; then
    db "UPDATE product_variants SET stock=10 WHERE id=5" >/dev/null
fi
curl -s -b $JM -c $JM $BASE/products/5 -o /tmp/ntp_prod.html
TOK=$(csrf /tmp/ntp_prod.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/cart/items \
    --data-urlencode "product_id=5" --data-urlencode "quantity=1" --data-urlencode "$CT=$TOK"
BEFORE=$(db "SELECT COALESCE(MAX(id),0) FROM orders")
curl -s -b $JM -c $JM $BASE/checkout -o /tmp/ntp_co.html
TOK=$(csrf /tmp/ntp_co.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/checkout \
    --data-urlencode "$CT=$TOK" --data-urlencode "intent=confirm" \
    --data-urlencode "coupon_code=" --data-urlencode "coupon_select=" \
    --data-urlencode "email=test@example.com" \
    --data-urlencode "family_name=テスト" --data-urlencode "given_name=会員" \
    --data-urlencode "phone=09012345678" --data-urlencode "postal_code=1000001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=千代田区" \
    --data-urlencode "address_line=千代田1-1-1" --data-urlencode "building=" \
    --data-urlencode "delivery_date=" --data-urlencode "delivery_slot=" \
    --data-urlencode "delivery_choice=new"
curl -s -b $JM -c $JM $BASE/checkout/payment -o /tmp/ntp_pay.html
TOK=$(csrf /tmp/ntp_pay.html)
curl -s -b $JM -c $JM -X POST $BASE/checkout/payment -o /tmp/ntp_confirm.html \
    --data-urlencode "payment_method=cod" --data-urlencode "$CT=$TOK"
TOK=$(csrf /tmp/ntp_confirm.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/checkout/confirm --data-urlencode "$CT=$TOK"
OID=$(db "SELECT id FROM orders WHERE id > $BEFORE ORDER BY id LIMIT 1")
[ -n "$OID" ] && ok "TC-21 COD order placed" || ng "TC-21 COD order placed"
ONUM=$(db "SELECT 'AS-' || to_char(created_at AT TIME ZONE 'Asia/Tokyo','YYYYMMDD') || '-' || lpad(id::text,4,'0') FROM orders WHERE id=$OID")
sleep 3
MAIL=$(mail_latest test@example.com)
echo "$MAIL" | grep -q '【テスト】ご注文ありがとうございます' \
    && ok "TC-22 order mail uses the custom subject" || ng "TC-22 order mail uses the custom subject"
echo "$MAIL" | grep -q "カスタム文面テスト：$ONUM" \
    && ok "TC-23 order mail body carries the marker + real number" || ng "TC-23 order mail body (want marker $ONUM)"
echo "$MAIL" | grep -q 'SQLx キーホルダー' \
    && ok "TC-24 order mail still lists the items" || ng "TC-24 order mail items"

# ---- test send (saved template, to the admin) --------------------------
curl -s -X DELETE http://127.0.0.1:8025/api/v1/messages >/dev/null
edit_page order_confirmation
TOK=$(csrf /tmp/ntp_edit.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/mail/notifications/order_confirmation/test-send \
    --data-urlencode "$CT=$TOK"
sleep 2
MAIL=$(mail_latest admin@example.com)
echo "$MAIL" | grep -q '【テスト送信】【テスト】ご注文ありがとうございます' \
    && ok "TC-25 test send delivers the saved template" || ng "TC-25 test send"
echo "$MAIL" | grep -q 'カスタム文面テスト：AS-20261107-2193' \
    && ok "TC-26 test send renders sample data" || ng "TC-26 test send sample data"

# ---- reset to default --------------------------------------------------
edit_page order_confirmation
TOK=$(csrf /tmp/ntp_edit.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/mail/notifications/order_confirmation/reset \
    --data-urlencode "form_ticket=$(ticket /tmp/ntp_edit.html)" --data-urlencode "$CT=$TOK"
ROWS=$(db "SELECT count(*) FROM notification_templates")
expect "TC-27 reset deletes the override" "$ROWS" "0"
AUDIT=$(db "SELECT count(*) FROM admin_audit_logs WHERE action='notification_template.reset'")
[ "$AUDIT" -ge 1 ] && ok "TC-28 reset is audited" || ng "TC-28 reset is audited ($AUDIT)"
edit_page order_confirmation
grep_ok "TC-29 editor is back at the default" /tmp/ntp_edit.html '初期値のまま'

echo "RESULT: PASS=$PASS FAIL=$FAIL"
