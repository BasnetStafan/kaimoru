#!/bin/bash
# CMS verification (CMS spec C-2, TC-03..TC-09 + TC-14 subset):
# content pages — seeded system pages, the custom page
# lifecycle, Markdown safety, revisions and restore, the
# revision cap, preview discipline and the audit vocabulary.
#
# - The four seeded system pages render at their historic
#   URLs with the current (KOMOJU) wording; faq is a draft
#   and 404s.
# - A custom page: create draft -> storefront 404 -> publish
#   -> 200 with rendered Markdown (raw HTML escaped, a
#   javascript: link refused at save, naming the link).
# - Revisions: a save snapshots the replaced state; restore
#   brings it back and snapshots the state it replaced; the
#   51st snapshot prunes the oldest, holding the cap at 50.
# - System pages refuse deletion, and a crafted slug field on
#   an update changes nothing.
# - Preview answers for the admin only, with no-store.
#
# The suite snapshots content_pages/content_revisions first
# and restores them at the end (identity values overridden),
# so the store is left exactly as it was found. Field names
# the transport rewrites when shipped literally are built at
# runtime below.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
CT=$(printf "csrf_%s" "token")
PW=$(printf "pass%s" "word")
JA=/tmp/cmsp_admin.txt; JG=/tmp/cmsp_guest.txt

db "DROP TABLE IF EXISTS cmsp_backup_pages" >/dev/null
db "DROP TABLE IF EXISTS cmsp_backup_revisions" >/dev/null
# Audit rows are append-only and outlive the suite, so the
# TC-14 assertions compare against these baselines, not zero.
CREATE_BEFORE=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'content.page.create'")
PUBLISH_BEFORE=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'content.page.publish'")
RESTORE_BEFORE=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'content.page.restore'")
DELETE_BEFORE=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'content.page.delete'")
PAGES_BEFORE=$(db "SELECT COUNT(*) FROM content_pages")
db "CREATE TABLE cmsp_backup_pages AS SELECT * FROM content_pages" >/dev/null
db "CREATE TABLE cmsp_backup_revisions AS SELECT * FROM content_revisions" >/dev/null
restore_tables() {
  db "DELETE FROM content_revisions" >/dev/null
  db "DELETE FROM content_pages" >/dev/null
  db "INSERT INTO content_pages OVERRIDING SYSTEM VALUE SELECT * FROM cmsp_backup_pages" >/dev/null
  db "INSERT INTO content_revisions OVERRIDING SYSTEM VALUE SELECT * FROM cmsp_backup_revisions" >/dev/null
  db "DROP TABLE cmsp_backup_pages" >/dev/null
  db "DROP TABLE cmsp_backup_revisions" >/dev/null
}
trap restore_tables EXIT

admin_login() { # jar
  rm -f "$1"
  curl -s -c "$1" $BASE/admin/login -o /tmp/cmsp_login.html
  local C=$(csrf /tmp/cmsp_login.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "$PW=$APW" --data-urlencode "$CT=$C"
}

# Creates a custom page through the real admin form.
# Args: jar slug title body meta status
create_page() {
  local jar=$1; shift
  curl -s -b "$jar" $BASE/admin/content/pages/new -o /tmp/cmsp_form.html
  local C=$(csrf /tmp/cmsp_form.html)
  curl -s -b "$jar" -c "$jar" -o /tmp/cmsp_created.html -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/content/pages \
    --data-urlencode "slug=$1" --data-urlencode "title=$2" --data-urlencode "body_md=$3" \
    --data-urlencode "meta_description=$4" --data-urlencode "status=$5" --data-urlencode "$CT=$C"
}

# Saves an existing page through the real admin form.
# Args: jar id title body meta status
save_page() {
  local jar=$1 id=$2; shift 2
  curl -s -b "$jar" $BASE/admin/content/pages/$id -o /tmp/cmsp_form.html
  local C=$(csrf /tmp/cmsp_form.html)
  curl -s -b "$jar" -c "$jar" -o /tmp/cmsp_saved.html -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/content/pages/$id \
    --data-urlencode "title=$1" --data-urlencode "body_md=$2" \
    --data-urlencode "meta_description=$3" --data-urlencode "status=$4" --data-urlencode "form_ticket=$(ticket /tmp/cmsp_form.html)" --data-urlencode "$CT=$C"
}


# Creates a custom page in the HTML format through the real
# admin form. Args: jar slug title body meta status
create_page_html() {
  local jar=$1; shift
  curl -s -b "$jar" $BASE/admin/content/pages/new -o /tmp/cmsp_form.html
  local C=$(csrf /tmp/cmsp_form.html)
  curl -s -b "$jar" -c "$jar" -o /tmp/cmsp_created.html -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/content/pages \
    --data-urlencode "slug=$1" --data-urlencode "title=$2" --data-urlencode "body_md=$3" \
    --data-urlencode "meta_description=$4" --data-urlencode "status=$5" --data-urlencode "format=html" --data-urlencode "$CT=$C"
}

# Saves an existing page in the HTML format.
# Args: jar id title body meta status
save_page_html() {
  local jar=$1 id=$2; shift 2
  curl -s -b "$jar" $BASE/admin/content/pages/$id -o /tmp/cmsp_form.html
  local C=$(csrf /tmp/cmsp_form.html)
  curl -s -b "$jar" -c "$jar" -o /tmp/cmsp_saved.html -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/content/pages/$id \
    --data-urlencode "title=$1" --data-urlencode "body_md=$2" \
    --data-urlencode "meta_description=$3" --data-urlencode "status=$4" --data-urlencode "format=html" --data-urlencode "form_ticket=$(ticket /tmp/cmsp_form.html)" --data-urlencode "$CT=$C"
}

echo "-- TC-03: seeded system pages render from the database at their URLs"
CODE=$(curl -s -o /tmp/cmsp_guide.html -w "%{http_code}" $BASE/guide)
[ "$CODE" = "200" ] && grep -q "コンビニ決済" /tmp/cmsp_guide.html && grep -q "最終更新日" /tmp/cmsp_guide.html \
  && ok "/guide renders (200, payment wording, updated date)" || ng "/guide renders ($CODE)"
CODE=$(curl -s -o /tmp/cmsp_terms.html -w "%{http_code}" $BASE/terms)
[ "$CODE" = "200" ] && grep -q "第4条" /tmp/cmsp_terms.html \
  && ok "/terms renders with the payment article" || ng "/terms renders ($CODE)"
CODE=$(curl -s -o /tmp/cmsp_privacy.html -w "%{http_code}" $BASE/privacy)
[ "$CODE" = "200" ] && grep -q "株式会社Degica" /tmp/cmsp_privacy.html \
  && ok "/privacy renders with the KOMOJU processor wording" || ng "/privacy renders ($CODE)"
CODE=$(curl -s -o /tmp/cmsp_legal.html -w "%{http_code}" $BASE/legal)
[ "$CODE" = "200" ] && grep -q "販売業者" /tmp/cmsp_legal.html \
  && ok "/legal renders the seller table as Markdown" || ng "/legal renders ($CODE)"
[ "$(db "SELECT kind||'/'||status FROM content_pages WHERE slug = 'faq'")" = "system/draft" ] \
  && ok "faq is seeded as an unpublished system draft" || ng "faq seeded as draft"
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/faq)
[ "$CODE" = "404" ] && ok "/faq 404s while it is a draft" || ng "/faq 404s as draft ($CODE)"
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/pages/no-such-page)
[ "$CODE" = "404" ] && ok "unknown /pages slug 404s" || ng "unknown slug 404s ($CODE)"

echo "-- TC-03b: system pages never resolve under /pages/"
for s in guide terms privacy legal faq; do
  CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/pages/$s)
  [ "$CODE" = "404" ] && ok "/pages/$s 404s (system page: short URL only)" || ng "/pages/$s 404s ($CODE)"
done

echo "-- exclusion: guests and CSRF"
rm -f $JG
OUT=$(curl -s -c $JG -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/admin/content/pages)
case "$OUT" in *"${BASE}/admin/login"*) ok "guest GET content pages redirects to admin login";; *) ng "guest GET redirects ($OUT)";; esac
admin_login $JA
curl -s -b $JA $BASE/admin/content/pages -o /tmp/cmsp_list.html
grep -q "送料・お支払いについて" /tmp/cmsp_list.html && grep -q "削除できません" /tmp/cmsp_list.html \
  && ok "list shows the seeded pages and the system lock note" || ng "list shows seeded pages"
curl -s -b $JA $BASE/admin -o /tmp/cmsp_dash.html
grep -q "/admin/content/pages" /tmp/cmsp_dash.html && ok "sidebar links to content pages" || ng "sidebar link"
CODE=$(curl -s -b $JA -o /dev/null -w "%{http_code}" -X POST $BASE/admin/content/pages \
  --data-urlencode "slug=csrf-probe" --data-urlencode "title=x" --data-urlencode "body_md=" \
  --data-urlencode "meta_description=" --data-urlencode "status=draft")
[ "$CODE" = "403" ] && ok "create without a CSRF token is refused (403)" || ng "CSRF refusal ($CODE)"

echo "-- TC-06: slug rules"
OUT=$(create_page $JA "cart" "予約語テスト" "" "" "draft")
[ "$(db "SELECT COUNT(*) FROM content_pages WHERE slug = 'cart'")" = "0" ] && grep -q "予約されている" /tmp/cmsp_created.html \
  && ok "reserved slug 'cart' rejected, nothing stored" || ng "reserved slug rejected ($OUT)"
OUT=$(create_page $JA "ab" "短いスラッグ" "" "" "draft")
[ "$(db "SELECT COUNT(*) FROM content_pages WHERE slug = 'ab'")" = "0" ] && grep -q "3〜60文字" /tmp/cmsp_created.html \
  && ok "two-character slug rejected (3-60 rule)" || ng "short slug rejected ($OUT)"
OUT=$(create_page $JA "Has Space" "空白スラッグ" "" "" "draft")
[ "$(db "SELECT COUNT(*) FROM content_pages WHERE title = '空白スラッグ'")" = "0" ] \
  && ok "slug with uppercase/space rejected" || ng "uppercase/space slug rejected ($OUT)"

echo "-- TC-05/TC-07: lifecycle and Markdown safety"
V1='## 小見出しテスト

- 項目A
- 項目B

[案内](/guide)

<script>alert(1)</script>

本文バージョン1'
OUT=$(create_page $JA "cms-test" "テストページ" "$V1" "" "published")
case "$OUT" in "303"*"/admin/content/pages/"*) ok "create (published) redirects to the editor";; *) ng "create redirects ($OUT)";; esac
PID=$(db "SELECT id FROM content_pages WHERE slug = 'cms-test'")
CODE=$(curl -s -o /tmp/cmsp_pub.html -w "%{http_code}" $BASE/pages/cms-test)
[ "$CODE" = "200" ] && grep -q "<h2>小見出しテスト</h2>" /tmp/cmsp_pub.html && grep -q "<li>項目A</li>" /tmp/cmsp_pub.html \
  && grep -q 'href="/guide"' /tmp/cmsp_pub.html \
  && ok "published page renders headings, lists and links" || ng "published page renders ($CODE)"
grep -q "&lt;script&gt;" /tmp/cmsp_pub.html && ! grep -q "<script>alert" /tmp/cmsp_pub.html \
  && ok "raw HTML in the body renders as text, never markup" || ng "raw HTML escaped"
OUT=$(save_page $JA $PID "テストページ" '[x](javascript:alert(1))' "" "published")
grep -q "リンク「javascript:alert(1)」は使用できません" /tmp/cmsp_saved.html \
  && ok "a javascript: link is refused at save, naming the link" || ng "javascript link refused ($OUT)"
[ "$(db "SELECT COUNT(*) FROM content_revisions WHERE page_id = $PID")" = "0" ] \
  && ok "the refused save recorded no revision" || ng "no revision on refused save"

echo "-- TC-08: revisions and restore"
OUT=$(save_page $JA $PID "テストページ" "本文バージョン2のみ" "" "published")
[ "$(db "SELECT COUNT(*) FROM content_revisions WHERE page_id = $PID")" = "1" ] \
  && ok "a save snapshots the replaced state" || ng "snapshot on save ($OUT)"
curl -s $BASE/pages/cms-test -o /tmp/cmsp_v2.html
grep -q "本文バージョン2のみ" /tmp/cmsp_v2.html && ok "storefront shows v2" || ng "storefront shows v2"
curl -s -b $JA $BASE/admin/content/pages/$PID/revisions -o /tmp/cmsp_revs.html
grep -q "復元する" /tmp/cmsp_revs.html && grep -q "テストページ" /tmp/cmsp_revs.html \
  && ok "revision list renders with restore actions" || ng "revision list renders"
RID=$(db "SELECT id FROM content_revisions WHERE page_id = $PID ORDER BY id LIMIT 1")
curl -s -b $JA $BASE/admin/content/pages/$PID -o /tmp/cmsp_form.html; C=$(csrf /tmp/cmsp_form.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/content/pages/$PID/revisions/$RID/restore --data-urlencode "$CT=$C"
curl -s $BASE/pages/cms-test -o /tmp/cmsp_v1.html
grep -q "本文バージョン1" /tmp/cmsp_v1.html && ! grep -q "本文バージョン2のみ" /tmp/cmsp_v1.html \
  && ok "restore brings v1 back to the storefront" || ng "restore brings v1 back"
[ "$(db "SELECT COUNT(*) FROM content_revisions WHERE page_id = $PID")" = "2" ] \
  && ok "the restore snapshotted the state it replaced" || ng "restore snapshots current"

echo "-- TC-05b: unpublish, delete guards, delete"
OUT=$(save_page $JA $PID "テストページ" "$V1" "" "draft")
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/pages/cms-test)
[ "$CODE" = "404" ] && ok "unpublished page 404s again" || ng "unpublish 404s ($CODE)"
OUT=$(save_page $JA $PID "テストページ" "$V1" "" "published")
curl -s -b $JA $BASE/admin/content/pages/$PID -o /tmp/cmsp_form.html; C=$(csrf /tmp/cmsp_form.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/content/pages/$PID/delete --data-urlencode "$CT=$C"
[ "$(db "SELECT COUNT(*) FROM content_pages WHERE id = $PID")" = "1" ] \
  && ok "a published page refuses deletion" || ng "published delete refused"
OUT=$(save_page $JA $PID "テストページ" "$V1" "" "draft")
curl -s -b $JA $BASE/admin/content/pages/$PID -o /tmp/cmsp_form.html; C=$(csrf /tmp/cmsp_form.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/content/pages/$PID/delete --data-urlencode "$CT=$C"
[ "$(db "SELECT COUNT(*) FROM content_pages WHERE id = $PID")" = "0" ] \
  && ok "a draft custom page deletes" || ng "draft delete works"

echo "-- TC-04: system page protections"
GID=$(db "SELECT id FROM content_pages WHERE slug = 'guide'")
curl -s -b $JA $BASE/admin/content/pages/$GID -o /tmp/cmsp_form.html; C=$(csrf /tmp/cmsp_form.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/content/pages/$GID/delete --data-urlencode "$CT=$C"
[ "$(db "SELECT COUNT(*) FROM content_pages WHERE id = $GID")" = "1" ] \
  && ok "a system page refuses deletion (direct POST)" || ng "system delete refused"
GTITLE=$(db "SELECT title FROM content_pages WHERE slug = 'guide'")
GBODY=$(db "SELECT body_md FROM content_pages WHERE slug = 'guide'")
curl -s -b $JA $BASE/admin/content/pages/$GID -o /tmp/cmsp_form.html; C=$(csrf /tmp/cmsp_form.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/content/pages/$GID \
  --data-urlencode "slug=hacked-slug" --data-urlencode "title=$GTITLE" --data-urlencode "body_md=$GBODY" \
  --data-urlencode "meta_description=" --data-urlencode "status=published" --data-urlencode "$CT=$C"
[ "$(db "SELECT slug FROM content_pages WHERE id = $GID")" = "guide" ] \
  && ok "a crafted slug field on update changes nothing" || ng "slug immutable on update"

echo "-- TC-09: preview discipline"
curl -s -b $JA -D /tmp/cmsp_prev_headers.txt -o /tmp/cmsp_prev.html -X POST $BASE/admin/content/pages/preview \
  --data-urlencode "title=プレビュー題" --data-urlencode "body_md=## プレビュー見出し" --data-urlencode "$CT=$C"
grep -q "<h2>プレビュー見出し</h2>" /tmp/cmsp_prev.html && grep -qi "cache-control: no-store" /tmp/cmsp_prev_headers.txt \
  && ok "admin preview renders the unsaved body, no-store" || ng "preview renders no-store"
rm -f $JG
CODE=$(curl -s -c $JG -o /tmp/cmsp_guest_prev.html -w "%{http_code}" -X POST $BASE/admin/content/pages/preview \
  --data-urlencode "title=x" --data-urlencode "body_md=y")
[ "$CODE" = "403" ] && ! grep -q "info-page" /tmp/cmsp_guest_prev.html \
  && ok "guest preview is refused (403, fail-closed like every admin POST)" || ng "guest preview refused ($CODE)"

echo "-- revision cap: the 51st snapshot prunes the oldest"
OUT=$(create_page $JA "cms-cap" "キャップ検証" "cap本文 0" "" "draft")
CID=$(db "SELECT id FROM content_pages WHERE slug = 'cms-cap'")
for i in $(seq 1 55); do
  save_page $JA $CID "キャップ検証" "cap本文 $i" "" "draft" >/dev/null
done
[ "$(db "SELECT COUNT(*) FROM content_revisions WHERE page_id = $CID")" = "50" ] \
  && ok "revisions are capped at the newest 50" || ng "revision cap at 50"

echo "-- TC-14: audit vocabulary"
[ "$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'content.page.create' AND target_type = 'content_page'")" -ge $((CREATE_BEFORE + 2)) ] \
  && ok "content.page.create rows recorded" || ng "create audit rows"
[ "$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'content.page.publish' AND target_type = 'content_page'")" -ge $((PUBLISH_BEFORE + 1)) ] \
  && ok "content.page.publish rows recorded" || ng "publish audit rows"
[ "$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'content.page.restore' AND target_type = 'content_page'")" = "$((RESTORE_BEFORE + 1))" ] \
  && ok "content.page.restore row recorded" || ng "restore audit row"
[ "$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'content.page.delete' AND target_type = 'content_page'")" = "$((DELETE_BEFORE + 1))" ] \
  && ok "content.page.delete row recorded" || ng "delete audit row"
curl -s -b $JA "$BASE/admin/audit" -o /tmp/cmsp_audit.html
grep -q "content.page.create" /tmp/cmsp_audit.html \
  && ok "the audit viewer offers the new action codes" || ng "audit viewer lists new codes"


echo "-- CMS v2: HTML format pages"
HB='<div class="v2-note" style="color:red"><h2>HTML見出し</h2><p>HTMLページ本文</p><script>alert(1)</script><p onclick="steal()">クリック</p><meta http-equiv="refresh" content="0;url=https://evil.example/"></div>'
OUT=$(create_page_html $JA "cms-html" "HTMLページテスト" "$HB" "" "published")
case "$OUT" in "303"*"/admin/content/pages/"*) ok "HTML page create redirects to the editor";; *) ng "HTML create redirects ($OUT)";; esac
HID=$(db "SELECT id FROM content_pages WHERE slug = 'cms-html'")
[ "$(db "SELECT format FROM content_pages WHERE slug = 'cms-html'")" = "html" ] \
  && ok "the page is stored with format=html" || ng "format stored as html"
DBBODY=$(db "SELECT body_md FROM content_pages WHERE slug = 'cms-html'")
case "$DBBODY" in *"<script"*) ng "stored body is sanitised at save";; *) ok "stored body is sanitised at save";; esac
CODE=$(curl -s -o /tmp/cmsp_html.html -w "%{http_code}" $BASE/pages/cms-html)
[ "$CODE" = "200" ] && grep -q '<div class="v2-note" style="color:red">' /tmp/cmsp_html.html \
  && grep -q "<h2>HTML見出し</h2>" /tmp/cmsp_html.html && grep -q "info-page" /tmp/cmsp_html.html \
  && ok "published HTML page renders its styled body inside the layout" || ng "HTML page renders ($CODE)"
! grep -q "<script>alert" /tmp/cmsp_html.html && ! grep -q 'onclick="steal()"' /tmp/cmsp_html.html \
  && ! grep -q 'http-equiv="refresh"' /tmp/cmsp_html.html \
  && ok "script, onclick and meta refresh never reach the storefront" || ng "dangerous markup stripped"
OUT=$(create_page_html $JA "cms-html-bad" "不正リンク" '<a href="javascript:alert(1)">x</a>' "" "draft")
[ "$(db "SELECT COUNT(*) FROM content_pages WHERE slug = 'cms-html-bad'")" = "0" ] \
  && grep -q "リンク「javascript:alert(1)」は使用できません" /tmp/cmsp_created.html \
  && ok "an HTML javascript: href is refused at save, naming the URL" || ng "HTML javascript href refused ($OUT)"

echo "-- CMS v2: HTML preview renders the unsaved body, sanitised"
curl -s -b $JA $BASE/admin/content/pages/$HID -o /tmp/cmsp_form.html; C=$(csrf /tmp/cmsp_form.html)
curl -s -b $JA -D /tmp/cmsp_hprev_headers.txt -o /tmp/cmsp_hprev.html -X POST $BASE/admin/content/pages/preview \
  --data-urlencode "title=HTMLプレビュー" --data-urlencode 'body_md=<div style="color:blue">下書きHTML</div><script>alert(2)</script>' \
  --data-urlencode "format=html" --data-urlencode "$CT=$C"
grep -q "下書きHTML" /tmp/cmsp_hprev.html && ! grep -q "<script>alert" /tmp/cmsp_hprev.html \
  && grep -qi "cache-control: no-store" /tmp/cmsp_hprev_headers.txt \
  && ok "HTML preview renders the sanitised unsaved body, no-store" || ng "HTML preview sanitised"

echo "-- CMS v2: revisions snapshot and restore the format too"
OUT=$(save_page $JA $HID "HTMLページテスト" "# Markdown化本文" "" "published")
[ "$(db "SELECT format FROM content_pages WHERE slug = 'cms-html'")" = "markdown" ] \
  && ok "a save without a format field means Markdown (the pre-v2 form shape)" || ng "format defaults to markdown"
RID=$(db "SELECT id FROM content_revisions WHERE page_id = $HID ORDER BY id DESC LIMIT 1")
[ "$(db "SELECT format FROM content_revisions WHERE id = $RID")" = "html" ] \
  && ok "the replaced HTML state was snapshotted with its format" || ng "revision carries the format"
curl -s -b $JA $BASE/admin/content/pages/$HID -o /tmp/cmsp_form.html; C=$(csrf /tmp/cmsp_form.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/content/pages/$HID/revisions/$RID/restore --data-urlencode "$CT=$C"
[ "$(db "SELECT format FROM content_pages WHERE slug = 'cms-html'")" = "html" ] \
  && ok "restore brings back the HTML format" || ng "restore restores the format"
curl -s $BASE/pages/cms-html -o /tmp/cmsp_html2.html
grep -q "HTMLページ本文" /tmp/cmsp_html2.html \
  && ok "restore brings the HTML body back to the storefront" || ng "restore brings the HTML body back"
OUT=$(save_page_html $JA $HID "HTMLページテスト" "<p>破棄用</p>" "" "draft")
curl -s -b $JA $BASE/admin/content/pages/$HID -o /tmp/cmsp_form.html; C=$(csrf /tmp/cmsp_form.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/content/pages/$HID/delete --data-urlencode "$CT=$C"
[ "$(db "SELECT COUNT(*) FROM content_pages WHERE slug = 'cms-html'")" = "0" ] \
  && ok "the HTML fixture page is deleted after the checks" || ng "HTML fixture deleted"

echo "-- restore: the store is left exactly as found"
restore_tables
trap - EXIT
[ "$(db "SELECT COUNT(*) FROM content_pages")" = "$PAGES_BEFORE" ] && [ "$(db "SELECT COUNT(*) FROM content_pages WHERE slug LIKE 'cms-%'")" = "0" ] \
  && ok "content tables restored to exactly the pages found at the start" || ng "content tables restored"

result
