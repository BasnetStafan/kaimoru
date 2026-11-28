#!/bin/bash
# Landing pages + the pages hub (CMS_SPEC §9): page types
# end to end through the real admin forms and the
# storefront.
#
# - The hub at /admin/content shows the three page types
#   with type-first create entries; each type's form is
#   reached from its entry.
# - A landing page created through the hub flow serves at
#   /lp/{slug} in the minimal chrome (no search, no global
#   menu, cart present), expands its product shortcode,
#   and adds to the cart from the page.
# - A draft LP 404s; a standard page and an LP may share
#   one slug and each serves on its own route; a standard
#   page never expands shortcodes.
# - The LP stays out of the sitemap until its operator
#   toggles it in; the hub's tabs list only their type;
#   the menu editor offers no page choices at all, so the
#   LP cannot be offered there.
#
# Everything the suite creates carries the run's PID and
# is deleted again (trap). Field names the transport
# rewrites when shipped literally are built at runtime.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
fetch() { CODE=$(curl -s -b "$1" -c "$1" -o "$3" -w "%{http_code}" "$2"); }
check_eq() { if [ "$2" = "$3" ]; then ok "$1"; else ng "$1 (want $3, got $2)"; fi; }
check_includes() { if grep -qF "$3" "$2"; then ok "$1"; else ng "$1 (missing: $3)"; fi; }
check_excludes() { if grep -qF "$3" "$2"; then ng "$1 (found: $3)"; else ok "$1"; fi; }
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
CT=$(printf "csrf_%s" "token")
PW=$(printf "pass%s" "word")
JA=/tmp/lp_admin.txt; JG=/tmp/lp_guest.txt
SUF=$$
LP_SLUG="lp-page-$SUF"
DRAFT_SLUG="lp-draft-$SUF"
STD_SLUG="lp-std-$SUF"
SHARED_SLUG="lp-shared-$SUF"
LP_TITLE="LPテストページ$SUF"
DRAFT_TITLE="LP下書き$SUF"
STD_TITLE="LP通常ページ$SUF"
SHARED_STD_TITLE="LP共有スラッグ通常$SUF"
SHARED_LP_TITLE="LP共有スラッグLP$SUF"
LP_MARK="LP本文マーカー$SUF"
STD_MARK="通常本文マーカー$SUF"
SHARED_STD_MARK="共有通常マーカー$SUF"
SHARED_LP_MARK="共有LPマーカー$SUF"
PCAT_NAME="LP埋込カテゴリ$SUF"
PCAT_SLUG="lp-pcat-$SUF"
P_SIMPLE="LP埋込シンプル$SUF"

cleanup() {
  db "DELETE FROM content_pages WHERE slug IN ('$LP_SLUG', '$DRAFT_SLUG', '$STD_SLUG', '$SHARED_SLUG')" >/dev/null
  db "DELETE FROM cart_items WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'LP埋込%'))" >/dev/null
  db "DELETE FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'LP埋込%')" >/dev/null
  db "DELETE FROM products WHERE name LIKE 'LP埋込%'" >/dev/null
  db "DELETE FROM categories WHERE slug = '$PCAT_SLUG'" >/dev/null
}
trap cleanup EXIT

admin_login() {
  rm -f "$1"
  curl -s -c "$1" $BASE/admin/login -o /tmp/lp_login.html
  local C=$(csrf /tmp/lp_login.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "$PW=$APW" --data-urlencode "$CT=$C"
}

# Creates a content page through the admin form, arriving
# via the type's own create URL. Args: slug title body
# status layout sitemap(1/0)
create_page() {
  curl -s -b "$JA" "$BASE/admin/content/pages/new?layout=$5" -o /tmp/lp_form.html
  local C=$(csrf /tmp/lp_form.html)
  local ARGS=(--data-urlencode "slug=$1" --data-urlencode "title=$2" --data-urlencode "body_md=$3"
              --data-urlencode "status=$4" --data-urlencode "format=markdown" --data-urlencode "meta_description="
              --data-urlencode "layout=$5" --data-urlencode "$CT=$C")
  if [ "$6" = "1" ]; then ARGS+=(--data-urlencode "in_sitemap=true"); fi
  curl -s -b "$JA" -c "$JA" -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/content/pages "${ARGS[@]}"
}

# Saves an existing page through the edit form.
# Args: id title body status sitemap(1/0)
update_page() {
  curl -s -b "$JA" $BASE/admin/content/pages/$1 -o /tmp/lp_edit.html
  local C=$(csrf /tmp/lp_edit.html)
  local T=$(ticket /tmp/lp_edit.html)
  local ARGS=(--data-urlencode "title=$2" --data-urlencode "body_md=$3" --data-urlencode "status=$4"
              --data-urlencode "format=markdown" --data-urlencode "meta_description="
              --data-urlencode "form_ticket=$T" --data-urlencode "$CT=$C")
  if [ "$5" = "1" ]; then ARGS+=(--data-urlencode "in_sitemap=true"); fi
  curl -s -b "$JA" -c "$JA" -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/content/pages/$1 "${ARGS[@]}"
}

echo "== Landing pages + pages hub suite =="
admin_login "$JA"
curl -s -c "$JG" $BASE/ -o /dev/null

# --- fixture product (SQL, the suites' usual way) -------
db "INSERT INTO categories (name, slug, is_active) VALUES ('$PCAT_NAME', '$PCAT_SLUG', true)" >/dev/null
PCAT_ID=$(db "SELECT id FROM categories WHERE slug = '$PCAT_SLUG'")
db "INSERT INTO products (name, price_yen, category, category_id, image_path, status) VALUES ('$P_SIMPLE', 1980, '$PCAT_NAME', $PCAT_ID, '', 'public')" >/dev/null
PID_SIMPLE=$(db "SELECT id FROM products WHERE name = '$P_SIMPLE'")
db "INSERT INTO product_variants (product_id, option_signature, stock) VALUES ($PID_SIMPLE, '', 5)" >/dev/null

# --- the hub: three types, type-first create ------------
fetch "$JA" "$BASE/admin/content" /tmp/lp_hub.html
check_eq "hub page is 200" "$CODE" "200"
check_includes "hub names the blog type" /tmp/lp_hub.html "ブログ記事"
check_includes "hub names the landing type" /tmp/lp_hub.html "ランディングページ"
check_includes "hub names the standard type" /tmp/lp_hub.html "通常ページ"
check_includes "hub create entry: blog post form" /tmp/lp_hub.html 'href="/admin/posts/new"'
check_includes "hub create entry: landing form preset" /tmp/lp_hub.html 'href="/admin/content/pages/new?layout=landing"'
check_includes "hub create entry: standard form" /tmp/lp_hub.html 'href="/admin/content/pages/new"'

fetch "$JA" "$BASE/admin/content/pages/new?layout=landing" /tmp/lp_form.html
check_includes "landing create form is headed as such" /tmp/lp_form.html "ランディングページの登録"
check_includes "landing create form presets the type" /tmp/lp_form.html 'name="layout" value="landing"'
check_includes "landing form documents the shortcodes" /tmp/lp_form.html '[[product:商品ID]]'
fetch "$JA" "$BASE/admin/posts/new" /tmp/lp_form.html
check_eq "blog create form still reachable" "$CODE" "200"

# --- create + serve the LP -------------------------------
LP_BODY="キャンペーン本文です。$LP_MARK

[[product:$PID_SIMPLE]]"
OUT=$(create_page "$LP_SLUG" "$LP_TITLE" "$LP_BODY" published landing 0)
case "$OUT" in "303 "*) ok "LP created through the hub flow";; *) ng "LP create ($OUT)";; esac
LP_ID=$(db "SELECT id FROM content_pages WHERE slug = '$LP_SLUG' AND layout = 'landing'")
case "$LP_ID" in ''|*[!0-9]*) ng "LP row stored with the landing layout";; *) ok "LP row stored with the landing layout (id $LP_ID)";; esac
SITEMAP_FLAG=$(db "SELECT in_sitemap FROM content_pages WHERE id = $LP_ID")
check_eq "new LP starts out of the sitemap" "$SITEMAP_FLAG" "f"

fetch "$JG" "$BASE/lp/$LP_SLUG" /tmp/lp_page.html
check_eq "LP page is 200" "$CODE" "200"
check_includes "LP renders its body marker" /tmp/lp_page.html "$LP_MARK"
check_includes "LP expands the product shortcode to a card" /tmp/lp_page.html "$P_SIMPLE"
check_includes "LP card carries the add-to-cart form" /tmp/lp_page.html 'hx-post="/cart/items"'
check_excludes "LP has no search form (minimal chrome)" /tmp/lp_page.html 'class="search-form"'
check_excludes "LP has no global header menu" /tmp/lp_page.html 'class="header-menu"'
check_excludes "LP has no full footer nav" /tmp/lp_page.html 'class="footer-nav"'
check_includes "LP keeps the cart link" /tmp/lp_page.html 'href="/cart"'
check_includes "LP footer keeps the legal link" /tmp/lp_page.html 'href="/legal"'

# Add to cart from the LP.
T=$(csrf /tmp/lp_page.html)
CODE=$(curl -s -b "$JG" -c "$JG" -o /dev/null -w "%{http_code}" -X POST $BASE/cart/items --data-urlencode "product_id=$PID_SIMPLE" --data-urlencode "quantity=1" --data-urlencode "$CT=$T")
check_eq "product added from the LP page" "$CODE" "200"
fetch "$JG" "$BASE/cart" /tmp/lp_cart.html
check_includes "cart holds the LP product" /tmp/lp_cart.html "$P_SIMPLE"

# --- draft LP: the same 404 as a missing slug ------------
OUT=$(create_page "$DRAFT_SLUG" "$DRAFT_TITLE" "下書き本文" draft landing 0)
case "$OUT" in "303 "*) ok "draft LP created";; *) ng "draft LP create ($OUT)";; esac
fetch "$JG" "$BASE/lp/$DRAFT_SLUG" /tmp/lp_page.html
check_eq "draft LP page is 404" "$CODE" "404"
fetch "$JG" "$BASE/lp/no-such-$SUF" /tmp/lp_page.html
check_eq "unknown LP slug is the same 404" "$CODE" "404"

# --- one slug, two namespaces ----------------------------
OUT=$(create_page "$SHARED_SLUG" "$SHARED_STD_TITLE" "通常ページの本文。$SHARED_STD_MARK" published standard 1)
case "$OUT" in "303 "*) ok "standard page created";; *) ng "standard page create ($OUT)";; esac
fetch "$JG" "$BASE/pages/$SHARED_SLUG" /tmp/lp_page.html
check_eq "standard page serves under /pages" "$CODE" "200"
check_includes "standard page renders its marker" /tmp/lp_page.html "$SHARED_STD_MARK"
fetch "$JG" "$BASE/lp/$SHARED_SLUG" /tmp/lp_page.html
check_eq "the standard slug under /lp is 404 before the LP exists" "$CODE" "404"
OUT=$(create_page "$SHARED_SLUG" "$SHARED_LP_TITLE" "LPの本文。$SHARED_LP_MARK" published landing 0)
case "$OUT" in "303 "*) ok "LP created with the same slug as the standard page";; *) ng "shared-slug LP create ($OUT)";; esac
fetch "$JG" "$BASE/lp/$SHARED_SLUG" /tmp/lp_page.html
check_eq "shared-slug LP serves under /lp" "$CODE" "200"
check_includes "shared-slug LP renders its own marker" /tmp/lp_page.html "$SHARED_LP_MARK"
fetch "$JG" "$BASE/pages/$SHARED_SLUG" /tmp/lp_page.html
check_eq "shared-slug standard page still serves under /pages" "$CODE" "200"
check_includes "standard page keeps its own body" /tmp/lp_page.html "$SHARED_STD_MARK"
check_excludes "standard page never shows the LP body" /tmp/lp_page.html "$SHARED_LP_MARK"

# --- standard pages never expand shortcodes --------------
OUT=$(create_page "$STD_SLUG" "$STD_TITLE" "通常ページ本文。$STD_MARK [[product:$PID_SIMPLE]]" published standard 1)
case "$OUT" in "303 "*) ok "shortcode-bearing standard page created";; *) ng "standard page create ($OUT)";; esac
fetch "$JG" "$BASE/pages/$STD_SLUG" /tmp/lp_page.html
check_eq "standard page is 200" "$CODE" "200"
check_includes "standard page keeps the shortcode literal" /tmp/lp_page.html "[[product:$PID_SIMPLE]]"
check_excludes "standard page renders no embed card" /tmp/lp_page.html 'hx-post="/cart/items"'

# --- sitemap: opt-in only --------------------------------
fetch "$JG" "$BASE/sitemap.xml" /tmp/lp_sitemap.html
check_excludes "LP absent from the sitemap by default" /tmp/lp_sitemap.html "/lp/$LP_SLUG"
check_includes "standard page present in the sitemap" /tmp/lp_sitemap.html "/pages/$SHARED_SLUG"
curl -s -b "$JA" $BASE/admin/content/pages/$LP_ID -o /tmp/lp_edit.html
check_includes "LP edit form carries the sitemap switch" /tmp/lp_edit.html 'name="in_sitemap"'
check_excludes "LP sitemap switch starts off" /tmp/lp_edit.html 'name="in_sitemap" value="true" checked'
OUT=$(update_page "$LP_ID" "$LP_TITLE" "$LP_BODY" published 1)
case "$OUT" in "303 "*) ok "LP saved with the sitemap switch on";; *) ng "LP sitemap toggle save ($OUT)";; esac
fetch "$JG" "$BASE/sitemap.xml" /tmp/lp_sitemap.html
check_includes "LP joins the sitemap after opt-in" /tmp/lp_sitemap.html "/lp/$LP_SLUG"

# --- hub tabs list only their type ------------------------
fetch "$JA" "$BASE/admin/content/pages?layout=landing" /tmp/lp_list.html
check_includes "landing tab lists the LP" /tmp/lp_list.html "$LP_TITLE"
check_excludes "landing tab hides standard pages" /tmp/lp_list.html "$STD_TITLE"
fetch "$JA" "$BASE/admin/content/pages?layout=standard" /tmp/lp_list.html
check_includes "standard tab lists the standard page" /tmp/lp_list.html "$STD_TITLE"
check_excludes "standard tab hides the LP" /tmp/lp_list.html "$LP_TITLE"
fetch "$JA" "$BASE/admin/posts" /tmp/lp_list.html
check_eq "blog tab (posts list) is 200" "$CODE" "200"
check_includes "posts list carries the type tabs" /tmp/lp_list.html 'href="/admin/content/pages?layout=landing"'

# --- the menu editor offers no pages ----------------------
fetch "$JA" "$BASE/admin/content/menus" /tmp/lp_menus.html
check_eq "menu editor is 200" "$CODE" "200"
check_excludes "menu editor does not list the LP" /tmp/lp_menus.html "$LP_TITLE"
check_excludes "menu editor does not offer the LP URL" /tmp/lp_menus.html "/lp/$LP_SLUG"

result
[ "$FAIL" -eq 0 ]
