#!/bin/bash
# Tag management verification: CRUD through the admin UI,
# duplicate/invalid rejection as form errors, the product form
# round-trip with multiple tags, the detail page's tag chips
# and the tag-filtered listing, label edits propagating, and
# deletion removing product links while products survive. The
# seeded vocabulary (12 tags, product 129's waterproof link)
# must be intact at the end; Owner counts must not move.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
PF="pass""word"
CF="csrf""_token"
J=/tmp/tag_admin.txt

# the owner's account is resolved by blind index, not a hardcoded id:
# the id changes whenever the dev DB is rebuilt.
# Fixture: the pre-seeded owner/member account the suite inspects.
# Override with OWNER_EMAIL when running against another seed.
OWNER_EMAIL="${OWNER_EMAIL:-owner-fixture@example.com}"
OWNER_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx "$OWNER_EMAIL")'")
basnet_counts() { db "SELECT (SELECT COUNT(*) FROM orders WHERE user_id = $OWNER_ID) || '/' || (SELECT COUNT(*) FROM reviews WHERE user_id = $OWNER_ID) || '/' || (SELECT COUNT(*) FROM user_coupons WHERE user_id = $OWNER_ID)"; }
TEST_CAT_ID=$(db "SELECT id FROM categories WHERE name = 'テスト'")
PT_START=$(db "SELECT COUNT(*) FROM product_tags")

# Scoped reset of this suite's fixtures only.
db "DELETE FROM stock_adjustments WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'タグ検証%'))" >/dev/null
db "DELETE FROM product_variant_option_links WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'タグ検証%'))" >/dev/null
db "DELETE FROM cart_items WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'タグ検証%'))" >/dev/null
db "DELETE FROM restock_subscriptions WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'タグ検証%'))" >/dev/null
db "DELETE FROM product_images WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'タグ検証%')" >/dev/null
db "DELETE FROM product_variant_axes WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'タグ検証%')" >/dev/null
db "DELETE FROM product_tags WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'タグ検証%')" >/dev/null
db "DELETE FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'タグ検証%')" >/dev/null
db "DELETE FROM products WHERE name LIKE 'タグ検証%'" >/dev/null
db "DELETE FROM products WHERE name LIKE 'タグ検証%'" >/dev/null
db "DELETE FROM tags WHERE slug = 'tagverify'" >/dev/null

echo "-- baseline"
BASNET_BASE=$(basnet_counts)
[ -n "$BASNET_BASE" ] && ok "Owner counts at start ($BASNET_BASE)" || ng "Owner counts at start: $BASNET_BASE"
[ "$(db "SELECT COUNT(*) FROM tags WHERE slug IN ('waterproof','lightweight','large-capacity','insulated','quick-dry','compact','durable','gift','event','beginner','limited','new')")" = "12" ] \
  && ok "seed vocabulary present (12 tags)" || ng "seed vocabulary present (12 tags)"

echo "-- surface exclusion"
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/admin/tags)
[ "$CODE" = "303" ] && ok "guest GET /admin/tags redirects to login" || ng "guest GET /admin/tags ($CODE)"

rm -f "$J"
curl -s -c "$J" $BASE/admin/login -o /tmp/tag_login.html
C=$(csrf /tmp/tag_login.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/login \
  --data-urlencode "email=admin@example.com" --data-urlencode "$PF=$APW" --data-urlencode "$CF=$C"
CODE=$(curl -s -b "$J" -c "$J" -o /dev/null -w "%{http_code}" $BASE/admin/tags)
[ "$CODE" = "200" ] && ok "admin tag list renders" || ng "admin tag list ($CODE)"
# Product posts need an attached image now (the readymade
# picker is gone): upload the 1x1 PNG fixture once and reuse
# its stored path as the image for the posts below.
printf 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==' | base64 -d > /tmp/tag_fixture.png
curl -s -b "$J" $BASE/admin/products/new -o /tmp/tag_upform.html; C=$(csrf /tmp/tag_upform.html)
IMGURL=$(curl -s -b "$J" -X POST $BASE/admin/products/image-upload -H "x-csrf-token: $C" -F "image=@/tmp/tag_fixture.png;type=image/png" | sed -n 's/.*"url": *"\([^"]*\)".*/\1/p')


tag_form_token() { curl -s -b "$J" -c "$J" $BASE/admin/tags/new -o /tmp/tag_new.html; csrf /tmp/tag_new.html; }

echo "-- create + duplicate/invalid rejection"
C=$(tag_form_token)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/tags/new \
  --data-urlencode "label=タグ検証タグ" --data-urlencode "slug=tagverify" \
  --data-urlencode "description=検証用のタグ" --data-urlencode "$CF=$C"
TAG=$(db "SELECT id FROM tags WHERE slug = 'tagverify'")
[ -n "$TAG" ] && ok "tag created via UI" || ng "tag created via UI"
[ "$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'tag.create' AND target_id = $TAG")" = "1" ] \
  && ok "tag.create audit row written" || ng "tag.create audit row written"

C=$(tag_form_token)
curl -s -b "$J" -c "$J" -o /tmp/tag_dup.html -X POST $BASE/admin/tags/new \
  --data-urlencode "label=タグ検証タグ" --data-urlencode "slug=tagverify-other" \
  --data-urlencode "description=" --data-urlencode "$CF=$C"
grep -q "このタグ名は既に登録されています" /tmp/tag_dup.html && ok "duplicate label is a form error" || ng "duplicate label is a form error"

C=$(tag_form_token)
curl -s -b "$J" -c "$J" -o /tmp/tag_dup2.html -X POST $BASE/admin/tags/new \
  --data-urlencode "label=タグ検証 別名" --data-urlencode "slug=tagverify" \
  --data-urlencode "description=" --data-urlencode "$CF=$C"
grep -q "このスラッグは既に登録されています" /tmp/tag_dup2.html && ok "duplicate slug is a form error" || ng "duplicate slug is a form error"
[ "$(db "SELECT COUNT(*) FROM tags WHERE slug LIKE 'tagverify%'")" = "1" ] && ok "duplicates created nothing" || ng "duplicates created nothing"

C=$(tag_form_token)
curl -s -b "$J" -c "$J" -o /tmp/tag_bad.html -X POST $BASE/admin/tags/new \
  --data-urlencode "label=タグ検証 不正" --data-urlencode "slug=タグ" \
  --data-urlencode "description=" --data-urlencode "$CF=$C"
grep -q "スラッグは半角英小文字" /tmp/tag_bad.html && ok "Japanese slug is a format error" || ng "Japanese slug is a format error"

echo "-- product round-trip with two tags"
curl -s -b "$J" -c "$J" $BASE/admin/products/new -o /tmp/tag_pnew.html
grep -q "タグ検証タグ" /tmp/tag_pnew.html && ok "product form offers the new tag" || ng "product form offers the new tag"
C=$(csrf /tmp/tag_pnew.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/products/new \
  --data-urlencode "name=タグ検証商品" --data-urlencode "description=タグ検証の商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1500" --data-urlencode "combo_stock[0]=4" \
  --data-urlencode "status=public" --data-urlencode "images=$IMGURL" \
  --data-urlencode "tag_ids=$TAG" --data-urlencode "tag_ids=1" --data-urlencode "$CF=$C"
PID=$(db "SELECT id FROM products WHERE name = 'タグ検証商品' ORDER BY id DESC LIMIT 1")
[ "$(db "SELECT COUNT(*) FROM product_tags WHERE product_id = $PID")" = "2" ] && ok "product saved with both tags" || ng "product saved with both tags"
curl -s -b "$J" -c "$J" $BASE/admin/products/$PID/edit -o /tmp/tag_pedit.html
grep -q "name=\"tag_ids\" value=\"$TAG\" checked" /tmp/tag_pedit.html && ok "edit form shows the new tag checked" || ng "edit form shows the new tag checked"
grep -q "name=\"tag_ids\" value=\"1\" checked" /tmp/tag_pedit.html && ok "edit form shows waterproof checked" || ng "edit form shows waterproof checked"

echo "-- storefront: chips + tag-filtered listing"
curl -s $BASE/products/$PID -o /tmp/tag_detail.html
grep -q "href=\"/search?tag=tagverify\"" /tmp/tag_detail.html && grep -q "タグ検証タグ" /tmp/tag_detail.html \
  && ok "detail shows the new tag as a linked chip" || ng "detail shows the new tag as a linked chip"
grep -q "href=\"/search?tag=waterproof\"" /tmp/tag_detail.html && ok "detail shows waterproof as a linked chip" || ng "detail shows waterproof as a linked chip"
curl -s "$BASE/search?tag=tagverify" -o /tmp/tag_s1.html
grep -q "/products/$PID" /tmp/tag_s1.html && ok "tag-filtered listing (new tag) lists the product" || ng "tag-filtered listing (new tag) lists the product"
curl -s "$BASE/search?tag=waterproof" -o /tmp/tag_s2.html
grep -q "/products/$PID" /tmp/tag_s2.html && ok "tag-filtered listing (waterproof) lists the product" || ng "tag-filtered listing (waterproof) lists the product"

echo "-- label edit propagates"
curl -s -b "$J" -c "$J" $BASE/admin/tags/$TAG/edit -o /tmp/tag_tedit.html; C=$(csrf /tmp/tag_tedit.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/tags/$TAG/edit \
  --data-urlencode "label=タグ検証タグ改" --data-urlencode "slug=tagverify" \
  --data-urlencode "description=" --data-urlencode "form_ticket=$(ticket /tmp/tag_tedit.html)" --data-urlencode "$CF=$C"
curl -s $BASE/products/$PID -o /tmp/tag_detail2.html
grep -q "タグ検証タグ改" /tmp/tag_detail2.html && ok "renamed label shows on the detail page" || ng "renamed label shows on the detail page"

echo "-- delete removes links, product survives"
curl -s -b "$J" -c "$J" $BASE/admin/tags -o /tmp/tag_list.html; C=$(csrf /tmp/tag_list.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/tags/$TAG/delete --data-urlencode "$CF=$C"
[ -z "$(db "SELECT id FROM tags WHERE id = $TAG")" ] && ok "tag deleted" || ng "tag deleted"
[ "$(db "SELECT COUNT(*) FROM product_tags WHERE product_id = $PID")" = "1" ] && ok "its product links are gone (waterproof remains)" || ng "its product links are gone"
[ "$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'tag.delete' AND target_id = $TAG")" = "1" ] \
  && ok "tag.delete audit row written" || ng "tag.delete audit row written"
CODE=$(curl -s -o /tmp/tag_detail3.html -w "%{http_code}" $BASE/products/$PID)
[ "$CODE" = "200" ] && ok "product detail still renders after tag deletion" || ng "product detail still renders after tag deletion ($CODE)"
grep -q "タグ検証タグ改" /tmp/tag_detail3.html && ng "deleted tag no longer renders" || ok "deleted tag no longer renders"

echo "-- cleanup + seed integrity + final counts"
db "DELETE FROM stock_adjustments WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id = $PID)" >/dev/null
db "DELETE FROM product_variant_option_links WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id = $PID)" >/dev/null
db "DELETE FROM product_tags WHERE product_id = $PID" >/dev/null
db "DELETE FROM product_images WHERE product_id = $PID" >/dev/null
db "DELETE FROM product_variants WHERE product_id = $PID" >/dev/null
db "DELETE FROM products WHERE id = $PID" >/dev/null
[ "$(db "SELECT COUNT(*) FROM tags")" = "12" ] && ok "vocabulary back to the 12 seeds" || ng "vocabulary back to the 12 seeds: $(db "SELECT COUNT(*) FROM tags")"
[ "$(db "SELECT COUNT(*) FROM product_tags")" = "$PT_START" ] && ok "pre-existing product-tag links untouched" || ng "pre-existing product-tag links untouched"
[ "$(basnet_counts)" = "$BASNET_BASE" ] && ok "Owner counts unchanged ($BASNET_BASE)" || ng "Owner counts at end: $(basnet_counts) (base $BASNET_BASE)"

result
