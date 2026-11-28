#!/bin/bash
# CMS verification (CMS spec C-4, TC-12 + TC-13 + TC-14
# subset): header/footer menus — the seeded footer renders
# exactly as the hard-coded list did, the header menu stays
# absent while empty and appears with its first visible
# item, the URL allowlist, the label limits, the
# last-visible-item guard, and the audit vocabulary.
#
# - Every scenario drives the real admin form; the field
#   set is rebuilt from the database before each post, in
#   the form's own order, so an override names its row by
#   index exactly as the rendered form does.
# - A save is live on the very next storefront request
#   (the admin save invalidates the menu cache).
# - Audit: one menu.update row per changed menu in a save;
#   a no-change save writes none. The last-visible-item
#   guard covers the footer only (the store's guaranteed
#   navigation); the header menu may drop to zero visible
#   items — it renders only while it has one.
#
# The suite snapshots menu_items first and restores it at
# the end, so the store is left exactly as it was found.
# Field names the transport rewrites when shipped
# literally are built at runtime below.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
CT=$(printf "csrf_%s" "token")
PW=$(printf "pass%s" "word")
JA=/tmp/cmsm_admin.txt

db "DROP TABLE IF EXISTS cmsm_backup_items" >/dev/null
AUDIT_BEFORE=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'menu.update'")
db "CREATE TABLE cmsm_backup_items AS SELECT * FROM menu_items" >/dev/null
restore_tables() {
  db "DELETE FROM menu_items" >/dev/null
  db "INSERT INTO menu_items OVERRIDING SYSTEM VALUE SELECT * FROM cmsm_backup_items" >/dev/null
  db "DROP TABLE cmsm_backup_items" >/dev/null
}
trap restore_tables EXIT

admin_login() { # jar
  rm -f "$1"
  curl -s -c "$1" $BASE/admin/login -o /tmp/cmsm_login.html
  local C=$(csrf /tmp/cmsm_login.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "$PW=$APW" --data-urlencode "$CT=$C"
}

audit_count() { db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'menu.update'"; }

# Rebuilds the form's field set from the database, in the
# form's own order: stored rows (sort position, then id),
# then three blank add rows per menu, visible with the
# next sort positions filled in — what the editor renders.
declare -A F=()
load_form_defaults() {
  F=()
  local -A IDX=()
  local menu id label url visible sort i b
  while IFS='|' read -r menu id label url visible sort; do
    i=${IDX[$menu]:-0}
    F["items[$menu][$i][id]"]="$id"
    F["items[$menu][$i][label]"]="$label"
    F["items[$menu][$i][url]"]="$url"
    F["items[$menu][$i][sort]"]="$sort"
    [ "$visible" = "t" ] && F["items[$menu][$i][visible]"]="on"
    IDX[$menu]=$((i+1))
  done < <(db "SELECT menu_key, id, label, url, is_visible, sort_order FROM menu_items ORDER BY menu_key, sort_order, id")
  for menu in header footer; do
    local n=${IDX[$menu]:-0}
    for b in 0 1 2; do
      i=$((n+b))
      F["items[$menu][$i][id]"]=""
      F["items[$menu][$i][label]"]=""
      F["items[$menu][$i][url]"]=""
      F["items[$menu][$i][sort]"]="$(((n+b+1)*10))"
      F["items[$menu][$i][visible]"]="on"
    done
  done
}

# Posts the whole menus form: the current database state,
# with name=value overrides applied on top (the value
# __ABSENT__ removes the field, which is how a checkbox is
# unticked). Echoes "code redirect_url".
post_menus() { # jar [name=value ...]
  local jar=$1; shift
  curl -s -b "$jar" $BASE/admin/content/menus -o /tmp/cmsm_form.html
  local C=$(csrf /tmp/cmsm_form.html)
  load_form_defaults
  local kv k v
  for kv in "$@"; do
    k=${kv%%=*}; v=${kv#*=}
    if [ "$v" = "__ABSENT__" ]; then unset "F[$k]"; else F[$k]="$v"; fi
  done
  local args=()
  for k in "${!F[@]}"; do args+=(--data-urlencode "$k=${F[$k]}"); done
  curl -s -b "$jar" -c "$jar" -o /tmp/cmsm_saved.html -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/content/menus \
    "${args[@]}" --data-urlencode "form_ticket=$(ticket /tmp/cmsm_form.html)" --data-urlencode "$CT=$C"
}

fetch_top() {
  curl -s $BASE/ -o /tmp/cmsm_top.html
  awk '/<header class="site-header">/,/<\/header>/' /tmp/cmsm_top.html > /tmp/cmsm_header.html
  # The menu row renders below the coupon stripe since the
  # 2026-11-06 layout change, outside the <header> element;
  # the extract covers the header furniture either way.
  awk '/<nav class="header-menu"/,/<\/nav>/' /tmp/cmsm_top.html >> /tmp/cmsm_header.html
  awk '/<footer class="site-footer">/,/<\/footer>/' /tmp/cmsm_top.html > /tmp/cmsm_footer.html
}
posf() { grep -n -m1 -F "$1" /tmp/cmsm_footer.html | cut -d: -f1; }

echo "-- setup: the editor is admin-only and shows the stored menus"
OUT=$(curl -s -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/admin/content/menus)
case "$OUT" in *"/admin/login"*) ok "guest is redirected to the admin login" ;; *) ng "guest redirected ($OUT)" ;; esac
admin_login $JA
CODE=$(curl -s -b $JA -o /tmp/cmsm_form.html -w "%{http_code}" $BASE/admin/content/menus)
[ "$CODE" = "200" ] && grep -q "メニュー設定" /tmp/cmsm_form.html \
  && grep -q "送料・お支払いについて" /tmp/cmsm_form.html \
  && grep -q "特定商取引法に基づく表記" /tmp/cmsm_form.html \
  && ok "editor renders with the seeded footer links" || ng "editor renders ($CODE)"

echo "-- seeds: the footer renders the five links in order; the header row follows the stored items"
fetch_top
[ "$(posf 'href="/guide"')" -lt "$(posf 'href="/contact"')" ] \
  && [ "$(posf 'href="/contact"')" -lt "$(posf 'href="/terms"')" ] \
  && [ "$(posf 'href="/terms"')" -lt "$(posf 'href="/privacy"')" ] \
  && [ "$(posf 'href="/privacy"')" -lt "$(posf 'href="/legal"')" ] \
  && ok "footer renders the seeded five links in order" || ng "footer seed order"
if [ "$(db "SELECT COUNT(*) FROM menu_items WHERE menu_key='header' AND is_visible")" = "0" ]; then
  ! grep -q "header-menu" /tmp/cmsm_header.html \
    && ok "no header menu row while the header menu is empty" || ng "header menu absent"
else
  grep -q 'class="header-menu"' /tmp/cmsm_header.html \
    && ok "the stored visible header items render a menu row" || ng "header menu renders"
fi
curl -s $BASE/products/1 -o /tmp/cmsm_product.html
grep -q 'href="/legal">特定商取引法に基づく表記' /tmp/cmsm_product.html \
  && ok "the seeded footer renders on a second page too" || ng "footer on second page"

echo "-- TC-14: a no-change save writes nothing — and the stored state stays savable"
B=$(audit_count)
OUT=$(post_menus $JA)
A=$(audit_count)
case "$OUT" in "303 "*) ok "the stored state saves unchanged" ;; *) ng "stored state saves ($OUT)" ;; esac
[ "$A" = "$B" ] && ok "a no-change save writes no audit row" || ng "no-change audit ($B -> $A)"

echo "-- edit: a footer label change is live on the next request, one audit row"
B=$(audit_count)
OUT=$(post_menus $JA "items[footer][0][label]=送料・お支払い（テスト）")
A=$(audit_count)
case "$OUT" in "303 "*) ok "label edit redirects" ;; *) ng "label edit ($OUT)" ;; esac
[ "$A" = "$((B+1))" ] && ok "a footer-only change writes exactly one menu.update row" || ng "one audit row ($B -> $A)"
fetch_top
grep -q 'href="/guide">送料・お支払い（テスト）' /tmp/cmsm_footer.html \
  && ok "the new label renders on the next request (cache invalidated)" || ng "label renders"
post_menus $JA "items[footer][0][label]=送料・お支払いについて" >/dev/null
fetch_top
grep -q 'href="/guide">送料・お支払いについて' /tmp/cmsm_footer.html \
  && ok "the label restores" || ng "label restores"

echo "-- edit: reordering two footer items moves them on the storefront"
post_menus $JA "items[footer][0][sort]=60" >/dev/null
fetch_top
[ "$(posf 'href="/legal"')" -lt "$(posf 'href="/guide"')" ] \
  && ok "the reordered item moves on the storefront" || ng "reorder moves the item"
# The moved item is now the LAST row: indices follow the
# current order, so the restore names it by its new index.
post_menus $JA "items[footer][4][sort]=10" >/dev/null

echo "-- header: adding items makes the row appear; hiding and deleting behave"
B=$(audit_count)
OUT=$(post_menus $JA "items[header][0][label]=商品検索" "items[header][0][url]=/search" \
  "items[header][1][label]=セール" "items[header][1][url]=#sale")
A=$(audit_count)
case "$OUT" in "303 "*) ok "header add redirects" ;; *) ng "header add ($OUT)" ;; esac
[ "$A" = "$((B+1))" ] && ok "a header-only change writes exactly one menu.update row" || ng "one audit row ($B -> $A)"
fetch_top
grep -q 'class="header-menu"' /tmp/cmsm_header.html \
  && grep -q 'href="/search">商品検索' /tmp/cmsm_header.html \
  && grep -q 'href="#sale">セール' /tmp/cmsm_header.html \
  && ok "the header menu row appears with both items" || ng "header row appears"
post_menus $JA "items[header][0][visible]=__ABSENT__" >/dev/null
fetch_top
grep -q 'href="#sale">セール' /tmp/cmsm_header.html \
  && ! grep -q 'href="/search">商品検索' /tmp/cmsm_header.html \
  && ok "hiding one header item leaves the other" || ng "hide one header item"
post_menus $JA "items[header][0][delete]=on" >/dev/null
fetch_top
grep -q 'href="#sale">セール' /tmp/cmsm_header.html \
  && ok "deleting the hidden item keeps the visible one" || ng "delete hidden item"
OUT=$(post_menus $JA "items[header][0][delete]=on")
case "$OUT" in "303 "*) ok "deleting the last visible header item is allowed (the guard is footer-only)" ;; *) ng "header delete-to-empty allowed ($OUT)" ;; esac
fetch_top
! grep -q 'class="header-menu"' /tmp/cmsm_header.html \
  && ok "an emptied header menu renders no menu row on the storefront" || ng "empty header renders no row"
post_menus $JA "items[header][0][label]=セール" "items[header][0][url]=#sale" >/dev/null
OUT=$(post_menus $JA "items[header][0][visible]=__ABSENT__")
case "$OUT" in "303 "*) ok "hiding the last visible header item is allowed" ;; *) ng "header hide-to-empty allowed ($OUT)" ;; esac
fetch_top
! grep -q 'class="header-menu"' /tmp/cmsm_header.html \
  && ok "a header with only hidden items renders no menu row" || ng "hidden-only header renders no row"
post_menus $JA "items[header][0][delete]=on" >/dev/null

echo "-- TC-12: URL validation on a footer item"
for BAD in "javascript:alert(1)" "//evil.example" "http://example.com" "example.com"; do
  OUT=$(post_menus $JA "items[footer][1][url]=$BAD")
  if [ "${OUT%% *}" = "200" ] && grep -q "リンク先は「/」で始まるパス" /tmp/cmsm_saved.html \
    && [ "$(db "SELECT url FROM menu_items WHERE menu_key='footer' AND label='お問い合わせ'")" = "/contact" ]; then
    ok "refused: $BAD"
  else
    ng "refused: $BAD ($OUT)"
  fi
done
OUT=$(post_menus $JA "items[footer][1][url]=https://example.com")
case "$OUT" in "303 "*) ok "an https URL is accepted" ;; *) ng "https accepted ($OUT)" ;; esac
fetch_top
grep -q 'href="https://example.com" rel="noopener"' /tmp/cmsm_footer.html \
  && ok "the external link renders with rel=noopener" || ng "external link renders"
post_menus $JA "items[footer][1][url]=#top" >/dev/null
fetch_top
grep -q 'href="#top">お問い合わせ' /tmp/cmsm_footer.html \
  && ok "a fragment URL is accepted" || ng "fragment accepted"
post_menus $JA "items[footer][1][url]=/contact" >/dev/null

echo "-- label limits: over 30 chars and empty are refused"
printf -v LONG_LABEL 'あ%.0s' $(seq 31)
OUT=$(post_menus $JA "items[footer][2][label]=$LONG_LABEL")
[ "${OUT%% *}" = "200" ] && grep -q "ラベルは30文字以内" /tmp/cmsm_saved.html \
  && ok "a 31-char label is refused" || ng "long label refused ($OUT)"
OUT=$(post_menus $JA "items[footer][2][label]=")
[ "${OUT%% *}" = "200" ] && grep -q "ラベルを入力してください" /tmp/cmsm_saved.html \
  && ok "an empty label is refused" || ng "empty label refused ($OUT)"

echo "-- TC-13: the footer cannot lose its last visible item"
OUT=$(post_menus $JA "items[footer][0][visible]=__ABSENT__" "items[footer][1][visible]=__ABSENT__" \
  "items[footer][2][visible]=__ABSENT__" "items[footer][3][visible]=__ABSENT__" "items[footer][4][visible]=__ABSENT__")
[ "${OUT%% *}" = "200" ] && grep -q "表示するリンクが1件もなくなります" /tmp/cmsm_saved.html \
  && ok "hiding every footer item in one save is refused" || ng "guard refuses hide-all ($OUT)"
OUT=$(post_menus $JA "items[footer][0][delete]=on" "items[footer][1][delete]=on" \
  "items[footer][2][delete]=on" "items[footer][3][delete]=on" "items[footer][4][delete]=on")
[ "${OUT%% *}" = "200" ] && grep -q "表示するリンクが1件もなくなります" /tmp/cmsm_saved.html \
  && ok "deleting every footer item in one save is refused" || ng "guard refuses delete-all ($OUT)"
fetch_top
grep -q 'href="/legal">特定商取引法に基づく表記' /tmp/cmsm_footer.html \
  && ok "the footer still renders the seeds after the refused saves" || ng "footer intact"

echo "-- visibility: a hidden footer item does not render"
post_menus $JA "items[footer][4][visible]=__ABSENT__" >/dev/null
fetch_top
! grep -q "特定商取引法に基づく表記" /tmp/cmsm_footer.html \
  && grep -q 'href="/privacy">プライバシーポリシー' /tmp/cmsm_footer.html \
  && ok "the hidden item is gone, the others unchanged" || ng "hide one footer item"
post_menus $JA "items[footer][4][visible]=on" >/dev/null

echo "-- TC-14: one save changing both menus writes two audit rows; the viewer lists the code"
B=$(audit_count)
post_menus $JA "items[header][1][label]=新着情報" "items[header][1][url]=/search" \
  "items[footer][0][label]=送料・お支払い（再）" >/dev/null
A=$(audit_count)
[ "$A" = "$((B+2))" ] && ok "a both-menus change writes two menu.update rows" || ng "two audit rows ($B -> $A)"
curl -s -b $JA "$BASE/admin/audit?action=menu.update" -o /tmp/cmsm_audit.html
grep -q "メニュー設定を更新" /tmp/cmsm_audit.html \
  && ok "the audit viewer lists menu.update rows under the new label" || ng "audit viewer lists the code"
B=$(audit_count)
post_menus $JA >/dev/null
A=$(audit_count)
[ "$A" = "$B" ] && ok "a final no-change save writes no audit row" || ng "final no-change audit ($B -> $A)"

result
