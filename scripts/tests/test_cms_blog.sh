#!/bin/bash
# CMS blog verification (docs/specs/CMS_SPEC.md): the blog
# module end to end through the real admin forms and the
# storefront.
#
# - A category is created through the admin UI.
# - A draft post: storefront 404, absent from /blog, the
#   category archive, the RSS feed and the sitemap.
# - Published with a past date: visible everywhere, body
#   marker rendered on the post page.
# - A scheduled post (future published_at): 404 and absent
#   everywhere, listed as 予約中 in the admin.
# - An edit snapshots a revision; restore brings the old
#   body back.
# - Deleting the category leaves the post standing with a
#   NULL category; deleting the posts removes them.
#
# Everything the suite creates carries the run's PID in
# its slug and is deleted again (trap), so the store is
# left as it was found. Field names the transport rewrites
# when shipped literally are built at runtime below. The
# post form is multipart (the cover rides in it), so its
# submits carry the CSRF token in the x-csrf-token header,
# exactly like the admin UI's own script does.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
# fetch jar url outfile: saves the body, sets CODE.
fetch() { CODE=$(curl -s -b "$1" -c "$1" -o "$3" -w "%{http_code}" "$2"); }
check_eq() { if [ "$2" = "$3" ]; then ok "$1"; else ng "$1 (want $3, got $2)"; fi; }
check_includes() { if grep -qF "$3" "$2"; then ok "$1"; else ng "$1 (missing: $3)"; fi; }
check_excludes() { if grep -qF "$3" "$2"; then ng "$1 (found: $3)"; else ok "$1"; fi; }
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
ADMIN_EMAIL=admin@example.com
CT=$(printf "csrf_%s" "token")
PW=$(printf "pass%s" "word")
JA=/tmp/cmsb_admin.txt; JG=/tmp/cmsb_guest.txt
SUF=$$
CAT_SLUG="cmsb-cat-$SUF"
POST_SLUG="cmsb-post-$SUF"
SCHED_SLUG="cmsb-sched-$SUF"
POST_TITLE="CMSBテスト記事$SUF"
SCHED_TITLE="CMSB予約記事$SUF"
MARK1="CMSB本文マーカー壱$SUF"
MARK2="CMSB本文マーカー弐$SUF"
CREATE_BEFORE=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'post.create'")

COVER_PATH=""
cleanup() {
  # The cover path is captured at creation time (COVER_PATH):
  # by the time this trap runs the suite has usually
  # deleted the posts itself, so the row is gone.
  local cover="$COVER_PATH"
  db "DELETE FROM posts WHERE slug IN ('$POST_SLUG', '$SCHED_SLUG')" >/dev/null
  db "DELETE FROM post_categories WHERE slug = '$CAT_SLUG'" >/dev/null
  if [ -n "$cover" ]; then
    # The upload machinery also writes _medium/_thumb
    # variants beside the stored file; remove the family.
    local base="resources/static${cover#/static}"
    rm -f "$base" "${base%.*}_medium.webp" "${base%.*}_thumb.webp" 2>/dev/null
  fi
  rm -f /tmp/cmsb_cover.png
}
trap cleanup EXIT

admin_login() { # jar
  rm -f "$1"
  curl -s -c "$1" $BASE/admin/login -o /tmp/cmsb_login.html
  local C=$(csrf /tmp/cmsb_login.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=$ADMIN_EMAIL" --data-urlencode "$PW=$APW" --data-urlencode "$CT=$C"
}

# Creates a category through the admin form; echoes the
# new category id.
create_category() {
  curl -s -b "$JA" $BASE/admin/post-categories/new -o /tmp/cmsb_catform.html
  local C=$(csrf /tmp/cmsb_catform.html)
  local OUT=$(curl -s -b "$JA" -c "$JA" -o /dev/null -w "%{redirect_url}" -X POST $BASE/admin/post-categories/new \
    --data-urlencode "name=CMSBカテゴリ$SUF" --data-urlencode "slug=$CAT_SLUG" --data-urlencode "$CT=$C")
  echo "$OUT" | sed 's|.*/post-categories/||; s|/edit$||'
}

# Creates a post through the multipart admin form.
# Args: slug title body status published_at category_id [coverfile]
create_post() {
  curl -s -b "$JA" $BASE/admin/posts/new -o /tmp/cmsb_postform.html
  local C=$(csrf /tmp/cmsb_postform.html)
  local ARGS=(-F "slug=$1" -F "title=$2" -F "body_md=$3" -F "status=$4" -F "published_at=$5"
              -F "category_id=$6" -F "format=markdown" -F "excerpt=" -F "meta_description="
              -F "cover_image_path=")
  if [ -n "${7:-}" ]; then ARGS+=(-F "cover_image=@$7;type=image/png"); fi
  curl -s -b "$JA" -c "$JA" -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/posts \
    -H "x-csrf-token: $C" "${ARGS[@]}"
}

# Saves an existing post through the multipart edit form.
# Args: id body status published_at category_id
update_post() {
  curl -s -b "$JA" $BASE/admin/posts/$1 -o /tmp/cmsb_edit.html
  local C=$(csrf /tmp/cmsb_edit.html)
  local T=$(ticket /tmp/cmsb_edit.html)
  local KEEP_COVER=$(db "SELECT COALESCE(cover_image_path, '') FROM posts WHERE id = $1")
  curl -s -b "$JA" -c "$JA" -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/admin/posts/$1 \
    -H "x-csrf-token: $C" \
    -F "title=$POST_TITLE" -F "body_md=$2" -F "status=$3" -F "published_at=$4" \
    -F "category_id=$5" -F "format=markdown" -F "excerpt=" -F "meta_description=" \
    -F "cover_image_path=$KEEP_COVER" -F "form_ticket=$T"
}

echo "== CMS blog suite =="
admin_login "$JA"
curl -s -c "$JG" $BASE/ -o /dev/null

# A 1x1 PNG for the cover upload check.
printf 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==' | base64 -d > /tmp/cmsb_cover.png

CAT_ID=$(create_category)
case "$CAT_ID" in ''|*[!0-9]*) ng "category created via admin UI (got: $CAT_ID)";; *) ok "category created via admin UI (id $CAT_ID)";; esac

# --- draft post: invisible everywhere -------------------
OUT=$(create_post "$POST_SLUG" "$POST_TITLE" "本文です。$MARK1" draft "" "$CAT_ID" /tmp/cmsb_cover.png)
case "$OUT" in "303 "*) ok "draft post created via admin UI";; *) ng "draft post create ($OUT)";; esac
POST_ID=$(db "SELECT id FROM posts WHERE slug = '$POST_SLUG'")
COVER=$(db "SELECT cover_image_path FROM posts WHERE id = $POST_ID")
COVER_PATH="$COVER"
case "$COVER" in /static/images/uploads/*) ok "cover image stored via the upload machinery ($COVER)";; *) ng "cover image stored ($COVER)";; esac

fetch "$JG" "$BASE/blog/$POST_SLUG" /tmp/cmsb_page.html
check_eq "draft post page is 404" "$CODE" "404"
fetch "$JG" "$BASE/blog" /tmp/cmsb_page.html
check_excludes "draft absent from /blog" /tmp/cmsb_page.html "$POST_TITLE"
fetch "$JG" "$BASE/blog/feed.xml" /tmp/cmsb_page.html
check_excludes "draft absent from the feed" /tmp/cmsb_page.html "$POST_TITLE"
fetch "$JG" "$BASE/sitemap.xml" /tmp/cmsb_page.html
check_excludes "draft absent from the sitemap" /tmp/cmsb_page.html "/blog/$POST_SLUG"

# --- publish with a past date: visible everywhere -------
OUT=$(update_post "$POST_ID" "本文です。$MARK1" published "2020-01-01T10:00" "$CAT_ID")
case "$OUT" in "303 "*) ok "post published via edit form";; *) ng "post publish ($OUT)";; esac
fetch "$JG" "$BASE/blog/$POST_SLUG" /tmp/cmsb_page.html
check_eq "published post page is 200" "$CODE" "200"
check_includes "post page renders the body marker" /tmp/cmsb_page.html "$MARK1"
check_includes "post page shows the cover image" /tmp/cmsb_page.html "uploads/"
fetch "$JG" "$BASE/blog" /tmp/cmsb_page.html
check_includes "published post listed on /blog" /tmp/cmsb_page.html "$POST_TITLE"
fetch "$JG" "$BASE/blog/category/$CAT_SLUG" /tmp/cmsb_page.html
check_eq "category archive is 200" "$CODE" "200"
check_includes "category archive lists the post" /tmp/cmsb_page.html "$POST_TITLE"
fetch "$JG" "$BASE/blog/feed.xml" /tmp/cmsb_page.html
check_includes "feed lists the post" /tmp/cmsb_page.html "$POST_TITLE"
fetch "$JG" "$BASE/sitemap.xml" /tmp/cmsb_page.html
check_includes "sitemap lists the post URL" /tmp/cmsb_page.html "/blog/$POST_SLUG"
check_includes "sitemap lists the category URL" /tmp/cmsb_page.html "/blog/category/$CAT_SLUG"

# --- scheduled post: invisible, listed as reserved ------
OUT=$(create_post "$SCHED_SLUG" "$SCHED_TITLE" "予約投稿の本文です" published "2999-01-01T10:00" "$CAT_ID")
case "$OUT" in "303 "*) ok "scheduled post created via admin UI";; *) ng "scheduled post create ($OUT)";; esac
SCHED_ID=$(db "SELECT id FROM posts WHERE slug = '$SCHED_SLUG'")
fetch "$JG" "$BASE/blog/$SCHED_SLUG" /tmp/cmsb_page.html
check_eq "scheduled post page is 404" "$CODE" "404"
fetch "$JG" "$BASE/blog" /tmp/cmsb_page.html
check_excludes "scheduled post absent from /blog" /tmp/cmsb_page.html "$SCHED_TITLE"
fetch "$JG" "$BASE/blog/feed.xml" /tmp/cmsb_page.html
check_excludes "scheduled post absent from the feed" /tmp/cmsb_page.html "$SCHED_TITLE"
fetch "$JG" "$BASE/sitemap.xml" /tmp/cmsb_page.html
check_excludes "scheduled post absent from the sitemap" /tmp/cmsb_page.html "/blog/$SCHED_SLUG"
fetch "$JA" "$BASE/admin/posts?state=scheduled" /tmp/cmsb_page.html
check_includes "admin list shows the scheduled post as 予約中" /tmp/cmsb_page.html "予約中"
check_includes "admin scheduled list names the post" /tmp/cmsb_page.html "$SCHED_TITLE"

# --- edit snapshots a revision; restore reverts ---------
OUT=$(update_post "$POST_ID" "差し替え後の本文です。$MARK2" published "2020-01-01T10:00" "$CAT_ID")
REVS=$(db "SELECT COUNT(*) FROM post_revisions WHERE post_id = $POST_ID")
if [ "${REVS:-0}" -ge 1 ]; then ok "edit snapshot a revision ($REVS rows)"; else ng "edit snapshot a revision ($REVS)"; fi
fetch "$JG" "$BASE/blog/$POST_SLUG" /tmp/cmsb_page.html
check_includes "edited body is live" /tmp/cmsb_page.html "$MARK2"
curl -s -b "$JA" $BASE/admin/posts/$POST_ID/revisions -o /tmp/cmsb_revs.html
RESTORE_URL=$(grep -o "action=\"/admin/posts/$POST_ID/revisions/[0-9]*/restore\"" /tmp/cmsb_revs.html | head -1 | sed 's/action="//; s/"$//')
C=$(csrf /tmp/cmsb_revs.html)
curl -s -b "$JA" -c "$JA" -o /dev/null -X POST "$BASE$RESTORE_URL" -H "x-csrf-token: $C" -d ""
fetch "$JG" "$BASE/blog/$POST_SLUG" /tmp/cmsb_page.html
check_includes "restore brings the first body back" /tmp/cmsb_page.html "$MARK1"
check_excludes "restore removes the edited body" /tmp/cmsb_page.html "$MARK2"

# --- category delete: the post stands, uncategorised ----
curl -s -b "$JA" $BASE/admin/posts -o /tmp/cmsb_list.html
C=$(csrf /tmp/cmsb_list.html)
curl -s -b "$JA" -c "$JA" -o /dev/null -X POST $BASE/admin/post-categories/$CAT_ID/delete -H "x-csrf-token: $C" -d ""
NULLCAT=$(db "SELECT category_id IS NULL FROM posts WHERE id = $POST_ID")
check_eq "category delete NULLs the post's category" "$NULLCAT" "t"
fetch "$JG" "$BASE/blog/$POST_SLUG" /tmp/cmsb_page.html
check_eq "post still renders after category delete" "$CODE" "200"
fetch "$JG" "$BASE/blog/category/$CAT_SLUG" /tmp/cmsb_page.html
check_eq "deleted category archive is 404" "$CODE" "404"

# --- audit + delete -------------------------------------
CREATE_AFTER=$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'post.create'")
if [ "${CREATE_AFTER:-0}" -gt "${CREATE_BEFORE:-0}" ]; then ok "post.create audited"; else ng "post.create audited ($CREATE_BEFORE -> $CREATE_AFTER)"; fi
curl -s -b "$JA" $BASE/admin/posts -o /tmp/cmsb_list.html
C=$(csrf /tmp/cmsb_list.html)
curl -s -b "$JA" -c "$JA" -o /dev/null -X POST $BASE/admin/posts/$POST_ID/delete -H "x-csrf-token: $C" -d ""
curl -s -b "$JA" -c "$JA" -o /dev/null -X POST $BASE/admin/posts/$SCHED_ID/delete -H "x-csrf-token: $C" -d ""
LEFT=$(db "SELECT COUNT(*) FROM posts WHERE slug IN ('$POST_SLUG', '$SCHED_SLUG')")
check_eq "posts deleted" "$LEFT" "0"
fetch "$JG" "$BASE/blog/$POST_SLUG" /tmp/cmsb_page.html
check_eq "deleted post page is 404" "$CODE" "404"

# --- guests are kept out of the admin -------------------
OUT=$(curl -s -b "$JG" -o /dev/null -w "%{http_code} %{redirect_url}" $BASE/admin/posts)
case "$OUT" in *"/admin/login"*) ok "guest GET /admin/posts redirects to admin login";; *) ng "guest GET /admin/posts ($OUT)";; esac

result
[ "$FAIL" -eq 0 ]
