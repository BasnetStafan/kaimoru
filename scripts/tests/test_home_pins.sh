#!/bin/bash
# Top page section pins + staff picks verification
# (HOME_PINS_AND_STAFF_PICKS_SPEC_2026-11-08).
#
# - The settings page renders one pin editor per pinnable
#   section plus the staff picks section row.
# - Pins lead their section's row in pin order; the
#   automatic selection fills the rest. Duplicates, over-cap
#   lists and non-event pins in the event section are
#   refused with field errors.
# - A draft product's pin does not render, and the fill
#   keeps the row at its full size.
# - The featured section renders its pins as a grid, and
#   falls back to the banner when unpinned.
# - The staff picks section shows the おすすめ-flagged
#   products newest first, and renders nothing while no
#   product is flagged.
# - A save without pin fields keeps the stored pins.
# - The category showcase pins categories: a pinned
#   category renders its newest products on the top page;
#   duplicates are refused; unpinning removes the section.
# - A section row links to its /sections/<key> listing
#   only while the section holds more than its row shows.
#
# The suite restores every pin and flag it touches.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
PASS=0; FAIL=0
ok() { PASS=$((PASS+1)); echo "  OK  $1"; }
ng() { FAIL=$((FAIL+1)); echo "  NG  $1"; }
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
CT=$(printf "csrf_%s" "token")
PW=$(printf "pass%s" "word")
JA=/tmp/pins_admin.txt

FEAT2_BEFORE=$(db "SELECT CASE WHEN is_staff_pick THEN 'true' ELSE 'false' END FROM products WHERE id = 2")
FEAT4_BEFORE=$(db "SELECT CASE WHEN is_staff_pick THEN 'true' ELSE 'false' END FROM products WHERE id = 4")
# The ambient staff-pick set (scale fixtures flag their own
# products): the staff TCs take the universe over, and the
# cleanup hands it back exactly.
STAFF_BEFORE=$(db "SELECT string_agg(id::text, ',') FROM products WHERE is_staff_pick")
cleanup() {
  db "DELETE FROM home_section_pins" >/dev/null
  db "UPDATE products SET is_staff_pick = false WHERE is_staff_pick" >/dev/null
  if [ -n "$STAFF_BEFORE" ]; then
    db "UPDATE products SET is_staff_pick = true WHERE id IN ($STAFF_BEFORE)" >/dev/null
  fi
  db "UPDATE products SET status = 'public' WHERE id = 2" >/dev/null
}
trap cleanup EXIT
cleanup

admin_login() {
  rm -f "$JA"
  curl -s -c "$JA" $BASE/admin/login -o /tmp/pins_login.html
  local C=$(csrf /tmp/pins_login.html)
  curl -s -b "$JA" -c "$JA" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "$PW=$APW" --data-urlencode "$CT=$C"
}

# Posts the home settings form with the seed defaults plus
# any extra fields given as name=value pairs (repeatable).
save_home() {
  curl -s -b "$JA" $BASE/admin/content/home -o /tmp/pins_form.html
  local C=$(csrf /tmp/pins_form.html)
  local args=(
    --data-urlencode "announcement_text=" --data-urlencode "announcement_link_label="
    --data-urlencode "announcement_link_url=" --data-urlencode "announcement_starts_at="
    --data-urlencode "announcement_ends_at="
    --data-urlencode "hero_enabled=on" --data-urlencode "hero_headline=好きなものを、もっと手軽に。"
    --data-urlencode "hero_text=ふだんの買い物はそのままに。AIエージェントにも頼める、新しいECストア。"
    --data-urlencode "hero_cta_label=新着商品を見る" --data-urlencode "hero_cta_url=#new"
    --data-urlencode "hero_images=" --data-urlencode "hero_starts_at=" --data-urlencode "hero_ends_at="
    --data-urlencode "heading_categories=" --data-urlencode "heading_new_arrivals="
    --data-urlencode "heading_free_space=" --data-urlencode "heading_ranking=" --data-urlencode "heading_featured="
    --data-urlencode "heading_staff_picks=" --data-urlencode "heading_notices="
    --data-urlencode "visible_categories=on" --data-urlencode "visible_new_arrivals=on"
    --data-urlencode "visible_free_space=on" --data-urlencode "visible_ranking=on" --data-urlencode "visible_featured=on"
    --data-urlencode "visible_staff_picks=on" --data-urlencode "visible_notices=on"
    --data-urlencode "sort_categories=10" --data-urlencode "sort_new_arrivals=20"
    --data-urlencode "sort_free_space=25" --data-urlencode "sort_ranking=30" --data-urlencode "sort_featured=40"
    --data-urlencode "sort_staff_picks=50" --data-urlencode "sort_notices=50"
  )
  local kv
  for kv in "$@"; do args+=(--data-urlencode "$kv"); done
  curl -s -b "$JA" -c "$JA" -o /tmp/pins_saved.html -w "%{http_code}" -X POST $BASE/admin/content/home \
    "${args[@]}" --data-urlencode "form_ticket=$(ticket /tmp/pins_form.html)" --data-urlencode "$CT=$C"
}

fetch_top() { curl -s $BASE/ -o /tmp/pins_top.html; }
# Prints the top page slice between two markers.
slice() { awk -v a="$1" -v b="$2" 'index($0,a){f=1} index($0,b){f=0} f' /tmp/pins_top.html; }
# Position of the first title-card link for product $1 on stdin.
cardpos() { grep -n -m1 -F "<h3><a href=\"/products/$1\"" | cut -d: -f1; }
cardcount() { grep -c -F '<h3><a href="/products/'; }

admin_login

echo "-- TC-01: the settings page renders the pin editors and the staff section"
curl -s -b "$JA" $BASE/admin/content/home -o /tmp/pins_form.html
[ "$(grep -c 'data-pin-editor' /tmp/pins_form.html)" = "4" ] \
  && grep -q 'トップページの固定表示' /tmp/pins_form.html \
  && grep -q 'name="heading_staff_picks"' /tmp/pins_form.html \
  && grep -q 'name="pins_ranking"' /tmp/pins_form.html \
  && ok "four pin editors + staff picks section row" || ng "pin editors render"

echo "-- TC-02: ranking pins lead in pin order, the fill keeps twenty cards"
CODE=$(save_home "pins_ranking=3,1")
fetch_top
SLICE=$(slice 'ランキング' 'イベント受取商品')
P3=$(echo "$SLICE" | cardpos 3); P1=$(echo "$SLICE" | cardpos 1); N=$(echo "$SLICE" | cardcount)
[ "$CODE" = "303" ] && [ -n "$P3" ] && [ -n "$P1" ] && [ "$P3" -lt "$P1" ] && [ "$N" = "20" ] \
  && ok "pins 3,1 lead the ranking; twenty cards shown" || ng "ranking pins lead (code=$CODE p3=$P3 p1=$P1 n=$N)"

echo "-- TC-03: a duplicated pin is refused and nothing is stored"
CODE=$(save_home "pins_ranking=3,3")
[ "$CODE" = "200" ] && grep -q '固定表示に同じ商品が重複しています' /tmp/pins_saved.html \
  && [ "$(db "SELECT count(*) FROM home_section_pins WHERE section_key = 'ranking'")" = "2" ] \
  && ok "duplicate refused, stored pins untouched" || ng "duplicate refused (code=$CODE)"

echo "-- TC-04: an over-cap pin list is refused"
CODE=$(save_home "pins_ranking=1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21")
[ "$CODE" = "200" ] && grep -q '固定表示は最大20件までです' /tmp/pins_saved.html \
  && ok "twenty-one ranking pins refused (cap 20, the config count)" || ng "over-cap refused (code=$CODE)"

echo "-- TC-05: the event section refuses a non-event product, accepts an event one"
CODE=$(save_home "pins_event=1")
[ "$CODE" = "200" ] && grep -q 'イベント受取に固定できるのは、イベント商品だけです' /tmp/pins_saved.html \
  && ok "non-event pin refused for the event section" || ng "event rule (code=$CODE)"
CODE=$(save_home "pins_event=9")
fetch_top
SLICE=$(slice 'イベント受取商品' '特集')
P9=$(echo "$SLICE" | cardpos 9); P10=$(echo "$SLICE" | cardpos 10)
[ "$CODE" = "303" ] && [ -n "$P9" ] && [ -n "$P10" ] && [ "$P9" -lt "$P10" ] \
  && ok "event pin 9 leads the pickup row" || ng "event pin leads (code=$CODE p9=$P9 p10=$P10)"

echo "-- TC-06: a draft product's pin does not render; the row still fills"
CODE=$(save_home "pins_new_arrivals=2")
db "UPDATE products SET status = 'draft' WHERE id = 2" >/dev/null
fetch_top
SLICE=$(slice '新着商品' 'AIエージェントにおまかせ')
N=$(echo "$SLICE" | cardcount)
echo "$SLICE" | grep -q '<h3><a href="/products/2"' && ng "draft pin renders" \
  || { [ "$N" = "20" ] && ok "draft pin hidden; twenty cards still shown" || ng "fill after draft pin (n=$N)"; }
db "UPDATE products SET status = 'public' WHERE id = 2" >/dev/null

echo "-- TC-07: the featured section renders its pins, and the banner returns when cleared"
CODE=$(save_home "pins_featured=2")
fetch_top
SLICE=$(slice '特集:今月のピックアップ' 'お知らせ')
echo "$SLICE" | grep -q '<h3><a href="/products/2"' \
  && echo "$SLICE" | grep -q 'href="/special">特集を見る' \
  && ! echo "$SLICE" | grep -q 'スタッフが選んだ、今月のおすすめ商品をまとめました' \
  && ok "featured pins render as a product grid with the feature page link" || ng "featured pins render (code=$CODE)"
CODE=$(save_home "pins_featured=")
fetch_top
grep -q 'スタッフが選んだ、今月のおすすめ商品をまとめました' /tmp/pins_top.html \
  && grep -q 'href="/special">特集を見る' /tmp/pins_top.html \
  && ok "clearing the pins restores the banner, linking to /special" || ng "banner fallback (code=$CODE)"

echo "-- TC-08: the staff picks section shows flagged products, newest first, then vanishes"
db "UPDATE products SET is_staff_pick = false WHERE is_staff_pick" >/dev/null
db "UPDATE products SET is_staff_pick = true WHERE id IN (2, 4)" >/dev/null
fetch_top
SLICE=$(slice 'スタッフおすすめ商品' 'お知らせ')
P2=$(echo "$SLICE" | cardpos 2); P4=$(echo "$SLICE" | cardpos 4)
[ -n "$P2" ] && [ -n "$P4" ] && [ "$P2" -lt "$P4" ] \
  && ok "staff picks show products 2 and 4, newest first" || ng "staff picks order (p2=$P2 p4=$P4)"
db "UPDATE products SET is_staff_pick = false WHERE id IN (2, 4)" >/dev/null
fetch_top
grep -q 'スタッフおすすめ商品' /tmp/pins_top.html \
  && ng "staff section renders with no flagged products" || ok "staff section hidden while nothing is flagged"

echo "-- TC-09: a save without pin fields keeps the stored pins"
CODE=$(save_home)
[ "$CODE" = "303" ] && [ "$(db "SELECT count(*) FROM home_section_pins WHERE section_key = 'ranking'")" = "2" ] \
  && [ "$(db "SELECT count(*) FROM home_section_pins WHERE section_key = 'event'")" = "1" ] \
  && ok "pin-less save keeps the stored pins" || ng "pins survive a pin-less save (code=$CODE)"

echo "-- TC-10: the /special page carries the pins, falls back to staff picks, then rests empty"
CODE=$(save_home "pins_featured=2")
curl -s $BASE/special -o /tmp/pins_special.html
grep -q '<h1>特集:今月のピックアップ</h1>' /tmp/pins_special.html \
  && grep -q '<h3><a href="/products/2"' /tmp/pins_special.html \
  && ok "the feature page shows the pinned product" || ng "feature page pins (code=$CODE)"
CODE=$(save_home "pins_featured=")
db "UPDATE products SET is_staff_pick = true WHERE id = 4" >/dev/null
curl -s $BASE/special -o /tmp/pins_special.html
grep -q '<h3><a href="/products/4"' /tmp/pins_special.html \
  && ok "with no pins the feature page falls back to the staff picks" || ng "feature page staff fallback"
db "UPDATE products SET is_staff_pick = false WHERE id = 4" >/dev/null
curl -s $BASE/special -o /tmp/pins_special.html
grep -q '特集商品は準備中です' /tmp/pins_special.html \
  && ok "with neither, the feature page rests at its empty state" || ng "feature page empty state"

echo "-- TC-11: the featured description is admin-editable and shared by banner and page"
CODE=$(save_home "description_featured=今月は手仕事の品を集めました。")
curl -s $BASE/ -o /tmp/pins_top.html
curl -s $BASE/special -o /tmp/pins_special.html
grep -q '今月は手仕事の品を集めました' /tmp/pins_top.html \
  && grep -q '今月は手仕事の品を集めました' /tmp/pins_special.html \
  && ok "a saved description shows on the banner and the feature page" || ng "description renders (code=$CODE)"
CODE=$(save_home "description_featured=")
curl -s $BASE/special -o /tmp/pins_special.html
grep -q 'スタッフが選んだ、今月のおすすめ商品をまとめました' /tmp/pins_special.html \
  && ok "clearing the description restores the built-in sentence" || ng "description default (code=$CODE)"

echo "-- TC-12: the featured section takes pins without a cap (its page lists them all)"
CODE=$(save_home "pins_featured=1,2,3,4,5,8,9,10,12")
CNT=$(db "SELECT count(*) FROM home_section_pins WHERE section_key = 'featured'")
[ "$CODE" = "303" ] && [ "$CNT" = "9" ] \
  && ok "nine featured pins are accepted and stored" || ng "featured uncapped (code=$CODE stored=$CNT)"
CODE=$(save_home "pins_featured=")

echo "-- TC-16: the section listing pages render, pins lead there too, unknown keys 404"
curl -s $BASE/sections/ranking -o /tmp/pins_section.html -w "%{http_code}" > /tmp/pins_section_code.txt
SCODE=$(cat /tmp/pins_section_code.txt)
grep -q '<h3><a href="/products/' /tmp/pins_section.html && CARD=yes || CARD=no
BCODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/sections/unknown)
[ "$SCODE" = "200" ] && [ "$CARD" = "yes" ] && [ "$BCODE" = "404" ] \
  && ok "ranking listing renders products; unknown section is a 404" || ng "section pages (code=$SCODE card=$CARD bogus=$BCODE)"
SP3=$(cardpos 3 < /tmp/pins_section.html); SP1=$(cardpos 1 < /tmp/pins_section.html)
[ -n "$SP3" ] && [ -n "$SP1" ] && [ "$SP3" -lt "$SP1" ] \
  && ok "the ranking listing leads with pins 3,1 (the general rule)" || ng "listing pins lead (p3=$SP3 p1=$SP1)"

echo "-- TC-17: the see-more rule on the top page (link only past the row's count)"
fetch_top
EVSLICE=$(slice 'イベント受取商品' '特集')
EVT=$(db "SELECT count(*) FROM products WHERE status = 'public' AND is_event")
PUB=$(db "SELECT count(*) FROM products WHERE status = 'public'")
if [ "$EVT" -gt 20 ]; then
  echo "$EVSLICE" | grep -q 'href="/sections/event"' \
    && ok "event row links to its listing past twenty products" || ng "event see-more missing (event=$EVT)"
else
  echo "$EVSLICE" | grep -q 'href="/sections/event"' \
    && ng "event row links out at twenty or fewer" || ok "event row has no see-more link at twenty or fewer"
fi
if [ "$PUB" -gt 20 ]; then
  grep -q 'href="/sections/new_arrivals"' /tmp/pins_top.html \
    && ok "new arrivals link to their listing past twenty products" || ng "new arrivals see-more missing (public=$PUB)"
else
  grep -q 'href="/sections/new_arrivals"' /tmp/pins_top.html \
    && ng "new arrivals link out at twenty or fewer" || ok "new arrivals have no see-more link at twenty or fewer"
fi

echo "-- TC-18: the category nav shows the config count, the rest behind もっと見る"
fetch_top
NAVSLICE=$(slice 'カテゴリーから探す' '新着商品')
NAVN=$(echo "$NAVSLICE" | grep -c 'href="/search?category=')
TOTAL=$(db "SELECT count(*) FROM categories c WHERE c.is_active AND EXISTS (SELECT 1 FROM products p WHERE p.category_id = c.id AND p.status = 'public')")
if [ "$TOTAL" -gt 6 ]; then
  [ "$NAVN" = "6" ] && echo "$NAVSLICE" | grep -q 'href="/categories">全てのカテゴリー' \
    && ok "nav shows six categories plus the all-categories item (total $TOTAL)" || ng "nav count (shown=$NAVN total=$TOTAL)"
  IDXN=$(curl -s $BASE/categories | grep -c 'href="/search?category=')
  [ "$IDXN" = "$TOTAL" ] \
    && ok "the category index lists all $TOTAL categories" || ng "category index count (shown=$IDXN total=$TOTAL)"
else
  [ "$NAVN" = "$TOTAL" ] && ! echo "$NAVSLICE" | grep -q 'href="/categories"' \
    && ok "nav shows all $TOTAL categories, no see-more" || ng "nav at six or fewer (shown=$NAVN total=$TOTAL)"
fi

echo "RESULT: PASS=$PASS FAIL=$FAIL"
