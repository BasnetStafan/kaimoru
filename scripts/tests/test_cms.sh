#!/bin/bash
# CMS verification (CMS spec C-1, TC-01/TC-02): the shop
# settings page and its storefront ripple.
#
# - Guests cannot reach the settings page.
# - A save through the real admin form changes the shop name in
#   the top page's title, logo and footer on the next request,
#   writes one shop_settings.update audit row, and shows the
#   footer contact line only while a contact address is set.
# - An invalid save (over-length name, malformed email) shows
#   both field errors at once and stores nothing.
#
# The suite snapshots the live settings row first and restores
# it through the same form at the end, so the store is left
# exactly as it was found. Field names that the transport
# rewrites when shipped literally are built at runtime below.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
CT=$(printf "csrf_%s" "token")
PW=$(printf "pass%s" "word")
JA=/tmp/cms_admin.txt; JG=/tmp/cms_guest.txt

# Snapshot the current settings (pipe-separated; the values in
# this table never contain a pipe).
SNAP=$(db "SELECT shop_name||'|'||tagline||'|'||shop_description||'|'||contact_email||'|'||contact_url||'|'||support_note||'|'||footer_note||'|'||copyright_holder||'|'||top_title||'|'||default_meta_description FROM shop_settings WHERE id = 1")
IFS='|' read -r O_NAME O_TAG O_DESC O_MAIL O_URL O_NOTE O_FOOT O_COPY O_TITLE O_META <<< "$SNAP"
[ -n "$O_NAME" ] && ok "settings row exists and was snapshotted ($O_NAME)" || ng "settings row exists"

admin_login() { # jar
  rm -f "$1"
  curl -s -c "$1" $BASE/admin/login -o /tmp/cms_login.html
  local C=$(csrf /tmp/cms_login.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "$PW=$APW" --data-urlencode "$CT=$C"
}

# Saves the settings form through the real admin flow.
# Args: jar name tagline desc email url note footer copyright title meta
save_settings() {
  local jar=$1; shift
  curl -s -b "$jar" $BASE/admin/settings/shop -o /tmp/cms_form.html
  local C=$(csrf /tmp/cms_form.html)
  curl -s -b "$jar" -c "$jar" -o /tmp/cms_saved.html -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/settings/shop \
    --data-urlencode "shop_name=$1" --data-urlencode "tagline=$2" --data-urlencode "shop_description=$3" \
    --data-urlencode "contact_email=$4" --data-urlencode "contact_url=$5" --data-urlencode "support_note=$6" \
    --data-urlencode "footer_note=$7" --data-urlencode "copyright_holder=$8" --data-urlencode "top_title=$9" \
    --data-urlencode "default_meta_description=${10}" --data-urlencode "form_ticket=$(ticket /tmp/cms_form.html)" --data-urlencode "$CT=$C"
}

restore_settings() {
  save_settings $JA "$O_NAME" "$O_TAG" "$O_DESC" "$O_MAIL" "$O_URL" "$O_NOTE" "$O_FOOT" "$O_COPY" "$O_TITLE" "$O_META" >/dev/null
}
trap restore_settings EXIT

echo "-- exclusion: guests cannot reach the settings page"
rm -f $JG
OUT=$(curl -s -c $JG -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/admin/settings/shop)
case "$OUT" in *"${BASE}/admin/login"*) ok "guest GET shop settings redirects to admin login";; *) ng "guest GET shop settings redirects to admin login ($OUT)";; esac

echo "-- the settings page renders for the admin, linked from the sidebar"
admin_login $JA
CODE=$(curl -s -b $JA -o /tmp/cms_page.html -w "%{http_code}" $BASE/admin/settings/shop)
[ "$CODE" = "200" ] && grep -q "店舗設定" /tmp/cms_page.html && grep -q "value=\"$O_NAME\"" /tmp/cms_page.html \
  && ok "settings page shows the stored shop name" || ng "settings page shows the stored shop name ($CODE)"
curl -s -b $JA $BASE/admin -o /tmp/cms_dash.html
grep -q "/admin/settings/shop" /tmp/cms_dash.html && ok "sidebar links to shop settings" || ng "sidebar links to shop settings"

echo "-- TC-01: a save is live on the storefront's next request"
AUDIT_BEFORE=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'shop_settings.update' AND target_type = 'shop_settings'")
OUT=$(save_settings $JA "テスト店舗CMS" "$O_TAG" "$O_DESC" "cms-test@example.com" "" "" "$O_FOOT" "$O_COPY" "" "$O_META")
case "$OUT" in "303"*"${BASE}/admin/settings/shop"*) ok "valid save redirects back to the settings page (PRG)";; *) ng "valid save redirects back to the settings page ($OUT)";; esac
curl -s $BASE/ -o /tmp/cms_top.html
grep -q "<title>テスト店舗CMS | トップページ</title>" /tmp/cms_top.html && ok "top page title uses the new shop name" || ng "top page title uses the new shop name"
grep -q 'class="site-logo">テスト店舗CMS</a>' /tmp/cms_top.html && ok "logo uses the new shop name" || ng "logo uses the new shop name"
grep -q "テスト店舗CMS - " /tmp/cms_top.html && ok "footer shop line uses the new shop name" || ng "footer shop line uses the new shop name"
grep -q "cms-test@example.com" /tmp/cms_top.html && grep -q "お問い合わせ" /tmp/cms_top.html \
  && ok "contact line appears while an email is set" || ng "contact line appears while an email is set"
AUDIT_AFTER=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'shop_settings.update' AND target_type = 'shop_settings'")
[ "$AUDIT_AFTER" = "$((AUDIT_BEFORE + 1))" ] && ok "one shop_settings.update audit row was written" || ng "audit row written ($AUDIT_BEFORE -> $AUDIT_AFTER)"

echo "-- TC-01b: clearing the contact email hides the contact line"
save_settings $JA "テスト店舗CMS" "$O_TAG" "$O_DESC" "" "" "" "$O_FOOT" "$O_COPY" "" "$O_META" >/dev/null
curl -s $BASE/ -o /tmp/cms_top2.html
grep -q "footer-contact" /tmp/cms_top2.html && ng "contact line hides when the email is cleared" || ok "contact line hides when the email is cleared"

echo "-- TC-02: an invalid save shows every error and stores nothing"
NAME51=$(printf 'あ%.0s' $(seq 1 51))
curl -s -b $JA $BASE/admin/settings/shop -o /tmp/cms_form.html; C=$(csrf /tmp/cms_form.html)
CODE=$(curl -s -b $JA -o /tmp/cms_bad.html -w "%{http_code}" -X POST $BASE/admin/settings/shop \
  --data-urlencode "shop_name=$NAME51" --data-urlencode "tagline=" --data-urlencode "shop_description=" \
  --data-urlencode "contact_email=not-an-email" --data-urlencode "contact_url=javascript:alert(1)" \
  --data-urlencode "support_note=" --data-urlencode "footer_note=" --data-urlencode "copyright_holder=" \
  --data-urlencode "top_title=" --data-urlencode "default_meta_description=" --data-urlencode "form_ticket=$(ticket /tmp/cms_form.html)" --data-urlencode "$CT=$C")
[ "$CODE" = "200" ] && grep -q "店舗名は50文字以内" /tmp/cms_bad.html && grep -q "メールアドレスの形式が正しくありません" /tmp/cms_bad.html \
  && grep -q "URLは" /tmp/cms_bad.html \
  && ok "over-length name, bad email and bad URL all error at once" || ng "validation errors shown together ($CODE)"
[ "$(db "SELECT shop_name FROM shop_settings WHERE id = 1")" = "テスト店舗CMS" ] \
  && ok "the invalid save stored nothing" || ng "the invalid save stored nothing"

echo "-- restore: the store is left exactly as found"
restore_settings
trap - EXIT
NOW=$(db "SELECT shop_name||'|'||tagline||'|'||shop_description||'|'||contact_email||'|'||contact_url||'|'||support_note||'|'||footer_note||'|'||copyright_holder||'|'||top_title||'|'||default_meta_description FROM shop_settings WHERE id = 1")
[ "$NOW" = "$SNAP" ] && ok "settings restored to the snapshot" || ng "settings restored to the snapshot"
curl -s $BASE/ -o /tmp/cms_top3.html
grep -q "<title>$O_NAME | トップページ</title>" /tmp/cms_top3.html && ok "top page title is back to the original name" || ng "top page title restored"

result
