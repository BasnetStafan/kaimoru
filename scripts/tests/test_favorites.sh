#!/bin/bash
# Favourites (Slice B): the member-only heart toggle on cards
# and the product buybox, the /mypage/favorites list, guest
# handling, and isolation between members.
#
# Fixture accounts are suite-owned and created through the real
# sign-up flow; product 2 (Axum マグカップ) is a public seed.
BASE=http://127.0.0.1:3000
# Shared harness: ok/ng, csrf() and result() live in lib.sh (ST-6).
. "$(dirname "$0")/lib.sh"
mailcode() { sleep 1; curl -s "http://127.0.0.1:8025/api/v1/search?query=$1" | grep -o '"Snippet":"[^"]*"' | grep -o '[0-9]\{6\}' | head -1; }
clearmail() { curl -s -X DELETE http://127.0.0.1:8025/api/v1/messages >/dev/null; }
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -q -t -A -c "$1"; }
CT=$(printf "csrf_%s" "token")
PF=$(printf "pass%s" "word")

FAV_EMAIL="fav-main@example.com"
FAV2_EMAIL="fav-other@example.com"
PW="favpass1"
PID=2

cleanup_fixtures() {
  db "
    DELETE FROM favorites WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $FAV_EMAIL)','$(bidx $FAV2_EMAIL)'));
    DELETE FROM user_coupons WHERE user_id IN (SELECT id FROM users WHERE email_bidx IN ('$(bidx $FAV_EMAIL)','$(bidx $FAV2_EMAIL)'));
    DELETE FROM users WHERE email_bidx IN ('$(bidx $FAV_EMAIL)','$(bidx $FAV2_EMAIL)');" >/dev/null 2>&1
}

# Creates one fixture account through the real sign-up flow.
create_account() {
  local jar=/tmp/fav_su_$$.txt email=$1
  rm -f $jar; clearmail
  curl -s -c $jar $BASE/signup -o /tmp/fav_su.html; local t=$(csrf /tmp/fav_su.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup --data-urlencode "email=$email" --data-urlencode "email_confirm=$email" --data-urlencode "$CT=$t"
  local code=$(mailcode $email)
  [ -n "$code" ] || return 1
  curl -s -b $jar -c $jar $BASE/signup/verify -o /tmp/fav_vf.html; t=$(csrf /tmp/fav_vf.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/signup/verify --data-urlencode "code=$code" --data-urlencode "$CT=$t"
  curl -s -b $jar -c $jar $BASE/signup/details -o /tmp/fav_det.html; t=$(csrf /tmp/fav_det.html)
  curl -s -b $jar -c $jar -X POST $BASE/signup/details -o /tmp/fav_conf.html \
    --data-urlencode "$PF=$PW" --data-urlencode "family_name=検証" --data-urlencode "given_name=太郎" \
    --data-urlencode "family_name_kana=ケンショウ" --data-urlencode "given_name_kana=タロウ" \
    --data-urlencode "birth_date=" --data-urlencode "gender=" --data-urlencode "postal_code=1500001" \
    --data-urlencode "prefecture=東京都" --data-urlencode "city=渋谷区" --data-urlencode "address_line=1-1-1" \
    --data-urlencode "building=" --data-urlencode "phone=09012345678" --data-urlencode "agree_terms=true" \
    --data-urlencode "$CT=$t"
  t=$(csrf /tmp/fav_conf.html)
  local body=$(curl -s -b $jar -c $jar -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/signup/confirm --data-urlencode "$CT=$t")
  rm -f $jar
  [ "$body" = "303 $BASE/" ]
}

# Logs in with a fresh jar; leaves the jar filled.
login_as() {
  local jar=$1 email=$2
  rm -f $jar
  curl -s -c $jar $BASE/login -o /tmp/fav_login.html; local t=$(csrf /tmp/fav_login.html)
  curl -s -b $jar -c $jar -o /dev/null -X POST $BASE/login --data-urlencode "email=$email" --data-urlencode "$PF=$PW" --data-urlencode "$CT=$t"
}

fav_count() { db "SELECT count(*) FROM favorites WHERE user_id = (SELECT id FROM users WHERE email_bidx = '$(bidx $1)')"; }

cleanup_fixtures
echo "-- fixture accounts"
create_account $FAV_EMAIL && ok "favourite fixture created" || { ng "favourite fixture created"; echo "RESULT: blocked"; exit 1; }
create_account $FAV2_EMAIL && ok "second fixture created" || { ng "second fixture created"; echo "RESULT: blocked"; exit 1; }

echo "-- guest: the card toggle is a login link, and a guest POST goes to login"
curl -s $BASE/ -o /tmp/fav_home_g.html
grep -q "href=\"/login?next=/products/$PID\"" /tmp/fav_home_g.html && ok "guest card shows the login link" || ng "guest card shows the login link"
GJ=/tmp/fav_guest.txt; rm -f $GJ
curl -s -c $GJ $BASE/ -o /tmp/fav_home_g2.html; T=$(csrf /tmp/fav_home_g2.html)
R=$(curl -s -b $GJ -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/favorites/$PID/toggle --data-urlencode "$CT=$T")
case "$R" in "303 $BASE/login"*) ok "guest toggle POST redirects to login" ;; *) ng "guest toggle POST redirects to login ($R)" ;; esac
[ "$(fav_count $FAV_EMAIL)" = "0" ] && ok "guest toggle stored nothing" || ng "guest toggle stored nothing"

echo "-- member: toggle on from the card"
J=/tmp/fav_main.txt; login_as $J $FAV_EMAIL
curl -s -b $J $BASE/ -o /tmp/fav_home_m.html
grep -q "hx-post=\"/favorites/$PID/toggle\"" /tmp/fav_home_m.html && grep -q 'aria-pressed="false"' /tmp/fav_home_m.html \
  && ok "member card shows the untoggled heart" || ng "member card shows the untoggled heart"
T=$(csrf /tmp/fav_home_m.html)
curl -s -b $J -c $J -X POST $BASE/favorites/$PID/toggle -H "HX-Request: true" --data-urlencode "$CT=$T" -o /tmp/fav_t1.html
grep -q 'aria-pressed="true"' /tmp/fav_t1.html && grep -q 'is-favorite' /tmp/fav_t1.html \
  && ok "toggle response swaps in the favourited button" || ng "toggle response swaps in the favourited button"
[ "$(fav_count $FAV_EMAIL)" = "1" ] && ok "favourite row stored" || ng "favourite row stored ($(fav_count $FAV_EMAIL))"

echo "-- the state follows the member across pages"
curl -s -b $J $BASE/products/$PID -o /tmp/fav_detail.html
grep -q "お気に入り済み" /tmp/fav_detail.html && ok "detail buybox shows the favourited label" || ng "detail buybox shows the favourited label"
curl -s -b $J $BASE/ -o /tmp/fav_home_m2.html
grep -q 'aria-pressed="true"' /tmp/fav_home_m2.html && ok "home card still favourited" || ng "home card still favourited"

echo "-- /mypage/favorites lists it, the hub links it"
curl -s -b $J $BASE/mypage -o /tmp/fav_hub.html
grep -q "お気に入り商品" /tmp/fav_hub.html && grep -q 'href="/mypage/favorites"' /tmp/fav_hub.html \
  && ok "hub menu links the favourites page" || ng "hub menu links the favourites page"
curl -s -b $J $BASE/mypage/favorites -o /tmp/fav_list.html
grep -q "Axum マグカップ" /tmp/fav_list.html && grep -q "data-breadcrumb" /tmp/fav_list.html \
  && ok "favourites page lists the product with the breadcrumb mount" || ng "favourites page lists the product"

echo "-- another member does not see them"
J2=/tmp/fav_other.txt; login_as $J2 $FAV2_EMAIL
curl -s -b $J2 $BASE/mypage/favorites -o /tmp/fav_list2.html
grep -q "お気に入り商品はまだありません" /tmp/fav_list2.html && ! grep -q "Axum マグカップ" /tmp/fav_list2.html \
  && ok "second member's list is empty" || ng "second member's list is empty"
curl -s -b $J2 $BASE/products/$PID -o /tmp/fav_detail2.html
grep -q "お気に入りに追加" /tmp/fav_detail2.html && ! grep -q "お気に入り済み" /tmp/fav_detail2.html \
  && ok "second member's buybox is untoggled" || ng "second member's buybox is untoggled"

echo "-- toggle off empties the list; unknown products are refused"
T=$(csrf /tmp/fav_list.html)
curl -s -b $J -c $J -X POST $BASE/favorites/$PID/toggle -H "HX-Request: true" --data-urlencode "$CT=$T" -o /tmp/fav_t2.html
grep -q 'aria-pressed="false"' /tmp/fav_t2.html && ok "toggle off swaps back" || ng "toggle off swaps back"
curl -s -b $J $BASE/mypage/favorites -o /tmp/fav_list3.html
grep -q "お気に入り商品はまだありません" /tmp/fav_list3.html && ok "list shows the empty state" || ng "list shows the empty state"
[ "$(fav_count $FAV_EMAIL)" = "0" ] && ok "favourite row removed" || ng "favourite row removed"
R=$(curl -s -b $J -o /dev/null -w "%{http_code}" -X POST $BASE/favorites/999999/toggle --data-urlencode "$CT=$T")
[ "$R" = "404" ] && ok "unknown product toggle is 404" || ng "unknown product toggle is 404 ($R)"

result
