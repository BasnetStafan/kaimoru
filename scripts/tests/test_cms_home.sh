#!/bin/bash
# CMS verification (CMS spec C-3, TC-10 + TC-11 + TC-14
# subset): home content — the announcement bar and hero
# slots with their display windows, section visibility,
# order and heading overrides, the link allowlist, and the
# audit vocabulary.
#
# - With seed defaults the top page shows the hero and the
#   sections in today's order, and no announcement bar.
# - A live announcement renders on the top page only,
#   below the coupon stripe; other pages never show it.
#   an expired or future window renders nothing — windows
#   are set through the real admin form, never by hand.
# - The link rule refuses script schemes and plain http;
#   a window that ends before it starts is refused.
# - Hiding a section removes only that section; reordering
#   moves it; a heading override replaces the heading.
# - Every changing save writes one home.update audit row;
#   a no-change save writes none.
#
# The suite snapshots home_slots/home_sections first and
# restores them at the end, so the store is left exactly as
# it was found. Field names the transport rewrites when
# shipped literally are built at runtime below.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
CT=$(printf "csrf_%s" "token")
PW=$(printf "pass%s" "word")
JA=/tmp/cmsh_admin.txt

db "DROP TABLE IF EXISTS cmsh_backup_slots" >/dev/null
db "DROP TABLE IF EXISTS cmsh_backup_sections" >/dev/null
AUDIT_BEFORE=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'home.update'")
db "CREATE TABLE cmsh_backup_slots AS SELECT * FROM home_slots" >/dev/null
db "CREATE TABLE cmsh_backup_sections AS SELECT * FROM home_sections" >/dev/null
restore_tables() {
  db "DELETE FROM home_slots" >/dev/null
  db "DELETE FROM home_sections" >/dev/null
  db "INSERT INTO home_slots SELECT * FROM cmsh_backup_slots" >/dev/null
  db "INSERT INTO home_sections SELECT * FROM cmsh_backup_sections" >/dev/null
  db "DROP TABLE cmsh_backup_slots" >/dev/null
  db "DROP TABLE cmsh_backup_sections" >/dev/null
}
trap restore_tables EXIT

admin_login() { # jar
  rm -f "$1"
  curl -s -c "$1" $BASE/admin/login -o /tmp/cmsh_login.html
  local C=$(csrf /tmp/cmsh_login.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "$PW=$APW" --data-urlencode "$CT=$C"
}

# Posts the whole home content form: the seed defaults,
# with name=value overrides applied on top (the value
# __ABSENT__ removes the field, which is how a checkbox is
# unticked). Echoes "code redirect_url".
post_home() { # jar [name=value ...]
  local jar=$1; shift
  curl -s -b "$jar" $BASE/admin/content/home -o /tmp/cmsh_form.html
  local C=$(csrf /tmp/cmsh_form.html)
  local -A F=(
    [announcement_text]="" [announcement_link_label]="" [announcement_link_url]=""
    [announcement_starts_at]="" [announcement_ends_at]=""
    [hero_enabled]="on" [hero_headline]="好きなものを、もっと手軽に。"
    [hero_text]="ふだんの買い物はそのままに。AIエージェントにも頼める、新しいECストア。"
    [hero_cta_label]="新着商品を見る" [hero_cta_url]="#new"
    [hero_starts_at]="" [hero_ends_at]="" [hero_images]=""
    [heading_categories]="" [heading_new_arrivals]="" [heading_free_space]="" [heading_ranking]=""
    [heading_featured]="" [heading_notices]="" [heading_staff_picks]="" [description_categories]="" [description_new_arrivals]="" [description_free_space]="" [description_ranking]="" [description_featured]="" [description_staff_picks]="" [description_notices]=""
    [visible_categories]="on" [visible_new_arrivals]="on" [visible_free_space]="on" [visible_ranking]="on"
    [visible_featured]="on" [visible_notices]="on" [visible_staff_picks]="on"
    [sort_categories]="10" [sort_new_arrivals]="20" [sort_free_space]="25" [sort_ranking]="30"
    [sort_featured]="40" [sort_notices]="50" [sort_staff_picks]="50"
  )
  local kv k v
  for kv in "$@"; do
    k=${kv%%=*}; v=${kv#*=}
    if [ "$v" = "__ABSENT__" ]; then unset "F[$k]"; else F[$k]="$v"; fi
  done
  local args=()
  for k in "${!F[@]}"; do args+=(--data-urlencode "$k=${F[$k]}"); done
  curl -s -b "$jar" -c "$jar" -o /tmp/cmsh_saved.html -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/content/home \
    "${args[@]}" --data-urlencode "form_ticket=$(ticket /tmp/cmsh_form.html)" --data-urlencode "$CT=$C"
}

fetch_top() { curl -s $BASE/ -o /tmp/cmsh_top.html; }
pos() { grep -n -m1 -F "$1" /tmp/cmsh_top.html | cut -d: -f1; }

YESTERDAY=$(date -d "yesterday" +%Y-%m-%dT23:59)
TOMORROW=$(date -d "tomorrow" +%Y-%m-%dT00:00)
TODAY_END=$(date +%Y-%m-%dT23:59)

echo "-- setup: the editor is admin-only and shows the stored values"
OUT=$(curl -s -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/admin/content/home)
case "$OUT" in *"/admin/login"*) ok "guest is redirected to the admin login" ;; *) ng "guest redirected ($OUT)" ;; esac
admin_login $JA
CODE=$(curl -s -b $JA -o /tmp/cmsh_form.html -w "%{http_code}" $BASE/admin/content/home)
[ "$CODE" = "200" ] && grep -q "トップページ設定" /tmp/cmsh_form.html \
  && grep -q "好きなものを、もっと手軽に。" /tmp/cmsh_form.html \
  && ok "editor renders with the seeded hero copy" || ng "editor renders ($CODE)"

echo "-- TC-11: seed defaults reproduce today's top page"
fetch_top
grep -q '<section class="hero">' /tmp/cmsh_top.html \
  && grep -q "好きなものを、もっと手軽に。" /tmp/cmsh_top.html \
  && ok "hero renders with its built-in copy" || ng "hero renders by default"
! grep -q 'hero-button' /tmp/cmsh_top.html \
  && ok "the hero has no button (slides carry the links)" || ng "hero button absent"
! grep -q "announcement-bar" /tmp/cmsh_top.html \
  && ok "no announcement bar by default" || ng "no announcement bar by default"
[ "$(pos '<h2>カテゴリーから探す</h2>')" -lt "$(pos '<h2>新着商品</h2>')" ] \
  && [ "$(pos '<h2>新着商品</h2>')" -lt "$(pos '<h2>ランキング</h2>')" ] \
  && [ "$(pos '<h2>ランキング</h2>')" -lt "$(pos '<h2>特集:今月のピックアップ</h2>')" ] \
  && [ "$(pos '<h2>特集:今月のピックアップ</h2>')" -lt "$(pos '<h2>お知らせ</h2>')" ] \
  && ok "sections render in the seeded order" || ng "sections in seeded order"
[ "$(pos '<h2>新着商品</h2>')" -lt "$(pos '<h2>AIエージェントにおまかせ</h2>')" ] \
  && [ "$(pos '<h2>AIエージェントにおまかせ</h2>')" -lt "$(pos '<h2>ランキング</h2>')" ] \
  && ok "the free-space strip renders between new arrivals and ranking" || ng "free-space strip position"

echo "-- TC-10: a live announcement renders on the top page only, below the stripe"
OUT=$(post_home $JA "announcement_enabled=on" "announcement_text=年末年始の配送について" \
  "announcement_link_label=詳しく見る" "announcement_link_url=/guide")
case "$OUT" in "303 "*) ok "announcement save redirects" ;; *) ng "announcement save ($OUT)" ;; esac
fetch_top
grep -q "announcement-bar" /tmp/cmsh_top.html && grep -q "年末年始の配送について" /tmp/cmsh_top.html \
  && grep -q 'href="/guide">詳しく見る' /tmp/cmsh_top.html \
  && ok "announcement bar renders with its link" || ng "announcement bar renders"
[ "$(pos 'coupon-band')" -lt "$(pos 'announcement-bar')" ] \
  && ok "announcement renders below the coupon band" || ng "announcement below stripe"
grep -q "coupon-band" /tmp/cmsh_top.html \
  && ok "coupon band behaviour unchanged" || ng "coupon band still renders"
curl -s $BASE/products/1 -o /tmp/cmsh_product.html
! grep -q "announcement-bar" /tmp/cmsh_product.html \
  && ok "the bar does not render on a product page" || ng "bar absent on product page"
curl -s $BASE/search -o /tmp/cmsh_search2.html
! grep -q "announcement-bar" /tmp/cmsh_search2.html \
  && ok "the bar does not render on the search page" || ng "bar absent on search page"
post_home $JA >/dev/null
fetch_top
! grep -q "announcement-bar" /tmp/cmsh_top.html \
  && ok "switched off, the bar renders nothing" || ng "announcement off renders nothing"

echo "-- TC-10: windows — expired and future announcements never render"
post_home $JA "announcement_enabled=on" "announcement_text=終了したお知らせ" "announcement_ends_at=$YESTERDAY" >/dev/null
fetch_top
! grep -q "announcement-bar" /tmp/cmsh_top.html \
  && ok "expired-window announcement does not render" || ng "expired announcement hidden"
post_home $JA "announcement_enabled=on" "announcement_text=未来のお知らせ" "announcement_starts_at=$TOMORROW" >/dev/null
fetch_top
! grep -q "announcement-bar" /tmp/cmsh_top.html \
  && ok "future-window announcement does not render" || ng "future announcement hidden"
post_home $JA "hero_ends_at=$YESTERDAY" >/dev/null
fetch_top
! grep -q '<section class="hero">' /tmp/cmsh_top.html \
  && ok "expired-window hero does not render" || ng "expired hero hidden"
post_home $JA "hero_enabled=__ABSENT__" >/dev/null
fetch_top
! grep -q '<section class="hero">' /tmp/cmsh_top.html \
  && ok "switched-off hero does not render" || ng "hero off renders nothing"
post_home $JA >/dev/null

echo "-- TC-10: the link allowlist and the window order are enforced"
OUT=$(post_home $JA "announcement_enabled=on" "announcement_text=リンク検査" \
  "announcement_link_label=詳細" "announcement_link_url=javascript:alert(1)")
[ "${OUT%% *}" = "200" ] && grep -q "リンク先は" /tmp/cmsh_saved.html \
  && ok "a script-scheme link is refused with a field error" || ng "script link refused ($OUT)"
fetch_top
! grep -q "announcement-bar" /tmp/cmsh_top.html \
  && ok "a refused save changes nothing" || ng "refused save changed nothing"
OUT=$(post_home $JA "announcement_enabled=on" "announcement_text=リンク検査" \
  "announcement_link_label=詳細" "announcement_link_url=http://example.com")
[ "${OUT%% *}" = "200" ] && grep -q "リンク先は" /tmp/cmsh_saved.html \
  && ok "a plain http link is refused (https only)" || ng "http link refused ($OUT)"
OUT=$(post_home $JA "announcement_enabled=on" "announcement_text=リンク検査" \
  "announcement_starts_at=$TOMORROW" "announcement_ends_at=$TODAY_END")
[ "${OUT%% *}" = "200" ] && grep -q "終了日時は開始日時より後に" /tmp/cmsh_saved.html \
  && ok "a window ending before it starts is refused" || ng "window order refused ($OUT)"
post_home $JA >/dev/null

echo "-- TC-11: section visibility, order and heading overrides"
post_home $JA "visible_ranking=__ABSENT__" >/dev/null
fetch_top
! grep -q '<h2>ランキング</h2>' /tmp/cmsh_top.html && grep -q "イベント受取商品" /tmp/cmsh_top.html \
  && ok "hiding ランキング removes it; the event section stays" || ng "hide ranking only"
post_home $JA "sort_featured=5" >/dev/null
fetch_top
[ "$(pos '<h2>特集:今月のピックアップ</h2>')" -lt "$(pos '<h2>カテゴリーから探す</h2>')" ] \
  && ok "reordering moves 特集 above カテゴリー" || ng "reorder moves the section"
post_home $JA "heading_notices=最新のお知らせ" >/dev/null
fetch_top
grep -q '<h2>最新のお知らせ</h2>' /tmp/cmsh_top.html && grep -q 'class="news"' /tmp/cmsh_top.html \
  && ok "a heading override replaces the section heading" || ng "heading override renders"
post_home $JA "visible_categories=__ABSENT__" >/dev/null
fetch_top
! grep -q "category-nav" /tmp/cmsh_top.html \
  && ok "hiding カテゴリー removes the top-page category nav" || ng "hide categories section"
CODE=$(curl -s -o /tmp/cmsh_search.html -w "%{http_code}" "$BASE/search")
[ "$CODE" = "200" ] && grep -q "アパレル" /tmp/cmsh_search.html \
  && ok "search and its category filter are unaffected" || ng "search unaffected ($CODE)"
post_home $JA >/dev/null

echo "-- TC-14: audit — changing saves write home.update, a no-change save does not"
AUDIT_MID=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'home.update'")
[ "$AUDIT_MID" -gt "$AUDIT_BEFORE" ] \
  && ok "changing saves wrote home.update rows ($AUDIT_BEFORE -> $AUDIT_MID)" || ng "home.update rows written"
post_home $JA >/dev/null
AUDIT_END=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'home.update'")
[ "$AUDIT_END" = "$AUDIT_MID" ] \
  && ok "a no-change save writes no audit row" || ng "no-change save wrote a row ($AUDIT_MID -> $AUDIT_END)"

echo "-- S2: hero image — a path naming no uploads file is refused"
OUT=$(post_home $JA "hero_images=/static/images/uploads/no-such-file.png")
[ "${OUT%% *}" = "200" ] && grep -q "画像ファイルが見つかりません" /tmp/cmsh_saved.html \
  && ok "a hero image naming no uploads file is refused" || ng "hero image refused ($OUT)"
fetch_top
! grep -q "hero-slide" /tmp/cmsh_top.html \
  && ok "a refused hero image changes nothing" || ng "refused hero image changed the hero"

echo "-- S2: hero image — upload, set, render above categories, clear"
printf "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==" | base64 -d > /tmp/cmsh_hero.png
curl -s -b $JA $BASE/admin/content/home -o /tmp/cmsh_form2.html
HC=$(csrf /tmp/cmsh_form2.html)
UP=$(curl -s -b $JA -H "x-csrf-token: $HC" -F "image=@/tmp/cmsh_hero.png;type=image/png" $BASE/admin/products/image-upload)
HERO_PATH=$(printf "%s" "$UP" | sed -n 's/.*"url":"\([^"]*\)".*/\1/p')
[ -n "$HERO_PATH" ] && ok "hero image uploads through the shared endpoint" || ng "hero image upload ($UP)"
OUT=$(post_home $JA "hero_images=$HERO_PATH")
case "$OUT" in "303 "*) ok "hero image save redirects" ;; *) ng "hero image save ($OUT)" ;; esac
fetch_top
grep -q "hero-slide" /tmp/cmsh_top.html && grep -q "$HERO_PATH" /tmp/cmsh_top.html \
  && ok "the hero renders its image" || ng "hero image renders"
[ "$(pos '<section class="hero')" -lt "$(pos '<h2>カテゴリーから探す</h2>')" ] \
  && ok "the hero renders above the categories" || ng "hero above categories"
curl -s -b $JA $BASE/admin/content/home -o /tmp/cmsh_form3.html
grep -q "hero_image" /tmp/cmsh_form3.html && grep -q "$HERO_PATH" /tmp/cmsh_form3.html \
  && ok "the editor shows the stored hero image" || ng "editor shows hero image"
post_home $JA "hero_images=" >/dev/null
fetch_top
grep -q '<section class="hero">' /tmp/cmsh_top.html && ! grep -q "hero-with-image" /tmp/cmsh_top.html \
  && ok "clearing the image restores the text-only hero" || ng "hero image cleared"

echo "-- TC-13: hero images slide; per-element and whole-section emptiness"
printf "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==" | base64 -d > /tmp/cmsh_hero2.png
curl -s -b $JA $BASE/admin/content/home -o /tmp/cmsh_form4.html
HC2=$(csrf /tmp/cmsh_form4.html)
UP2=$(curl -s -b $JA -H "x-csrf-token: $HC2" -F "image=@/tmp/cmsh_hero2.png;type=image/png" $BASE/admin/products/image-upload)
HERO_PATH2=$(printf '%s' "$UP2" | sed -n 's/.*"url":"\([^"]*\)".*/\1/p')
[ -n "$HERO_PATH2" ] && ok "second hero image uploads" || ng "second hero image upload ($UP2)"
OUT=$(post_home $JA "hero_images=$HERO_PATH,$HERO_PATH2")
case "$OUT" in "303 "*) ok "two-image hero save redirects" ;; *) ng "two-image hero save ($OUT)" ;; esac
fetch_top
[ "$(grep -c '<img class="hero-slide' /tmp/cmsh_top.html)" = "2" ] \
  && [ "$(grep -c 'data-hero-dot' /tmp/cmsh_top.html)" = "2" ] \
  && ok "two images render two slides and two dots" || ng "two slides render"
post_home $JA "hero_images=$HERO_PATH" "hero_headline=" "hero_text=" >/dev/null
fetch_top
grep -q '<section class="hero hero-with-image">' /tmp/cmsh_top.html \
  && grep -q 'hero-slide' /tmp/cmsh_top.html && ! grep -q 'hero-button' /tmp/cmsh_top.html \
  && ! grep -q '<h1>' /tmp/cmsh_top.html \
  && ok "an image-only hero shows no headline, body or button" || ng "image-only hero parts"
post_home $JA "hero_images=" "hero_headline=" "hero_text=" "hero_cta_label=" "hero_cta_url=" >/dev/null
fetch_top
! grep -q '<section class="hero' /tmp/cmsh_top.html \
  && ok "an empty hero renders no section at all" || ng "empty hero hidden"
OUT=$(post_home $JA "hero_images=$HERO_PATH,$HERO_PATH2" "hero_image_links=/special," "hero_headline=重ねる見出し" "hero_text=重ねる本文")
case "$OUT" in "303 "*) ok "linked hero save redirects" ;; *) ng "linked hero save ($OUT)" ;; esac
fetch_top
grep -q 'hero-overlay' /tmp/cmsh_top.html && grep -q '重ねる見出し' /tmp/cmsh_top.html \
  && grep -q '<a class="hero-slide is-active" href="/special">' /tmp/cmsh_top.html \
  && ok "copy overlays the slides; the first slide links to /special" || ng "hero overlay and slide link"
post_home $JA >/dev/null

echo "-- TC-14: the free-space section is a CMS section (copy, visibility)"
fetch_top
grep -q 'free-space-strip' /tmp/cmsh_top.html && grep -q 'AIエージェントにおまかせ' /tmp/cmsh_top.html \
  && ok "the free-space strip renders with its default copy" || ng "free-space strip default"
post_home $JA "heading_free_space=エージェント特集" "description_free_space=説明文テスト" >/dev/null
fetch_top
grep -q '<h2>エージェント特集</h2>' /tmp/cmsh_top.html && grep -q '説明文テスト' /tmp/cmsh_top.html \
  && ok "the strip heading and body are editable" || ng "free-space copy editable"
post_home $JA "visible_free_space=__ABSENT__" >/dev/null
fetch_top
! grep -q 'free-space-strip' /tmp/cmsh_top.html \
  && ok "a hidden free-space section renders nothing" || ng "free-space section hides"
post_home $JA >/dev/null
fetch_top
grep -q 'free-space-strip' /tmp/cmsh_top.html \
  && ok "restored defaults bring the strip back" || ng "agent strip restored"

result
