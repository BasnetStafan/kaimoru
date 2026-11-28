#!/bin/bash
# Category management verification: CRUD through the admin UI,
# duplicate/invalid rejection as form errors, the parent picker
# (self/descendants excluded, crafted cycle refused), the
# product round-trip (category_id + name snapshot, rename
# propagation), delete blocked while used and allowed when
# unused, and the inactive rules on the product form and the
# storefront. Fixture rows carry the カテゴリ検証 marker and are
# removed by the scoped reset; the owner's rows are counted
# before and after and must not move.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
PF="pass""word"
CF="csrf""_token"
J=/tmp/cat_admin.txt

# the owner's account is resolved by blind index, not a hardcoded id:
# the id changes whenever the dev DB is rebuilt.
# Fixture: the pre-seeded owner/member account the suite inspects.
# Override with OWNER_EMAIL when running against another seed.
OWNER_EMAIL="${OWNER_EMAIL:-owner-fixture@example.com}"
OWNER_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx "$OWNER_EMAIL")'")
basnet_counts() { db "SELECT (SELECT COUNT(*) FROM orders WHERE user_id = $OWNER_ID) || '/' || (SELECT COUNT(*) FROM reviews WHERE user_id = $OWNER_ID) || '/' || (SELECT COUNT(*) FROM user_coupons WHERE user_id = $OWNER_ID)"; }

# Scoped reset of this suite's fixtures only.
db "DELETE FROM stock_adjustments WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'カテゴリ検証%'))" >/dev/null
db "DELETE FROM product_variant_option_links WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'カテゴリ検証%'))" >/dev/null
db "DELETE FROM cart_items WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'カテゴリ検証%'))" >/dev/null
db "DELETE FROM restock_subscriptions WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'カテゴリ検証%'))" >/dev/null
db "DELETE FROM product_images WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'カテゴリ検証%')" >/dev/null
db "DELETE FROM product_variant_axes WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'カテゴリ検証%')" >/dev/null
db "DELETE FROM product_tags WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'カテゴリ検証%')" >/dev/null
db "DELETE FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'カテゴリ検証%')" >/dev/null
db "DELETE FROM products WHERE name LIKE 'カテゴリ検証%'" >/dev/null
db "DELETE FROM categories WHERE parent_id IN (SELECT id FROM categories WHERE name LIKE 'カテゴリ検証%')" >/dev/null
db "DELETE FROM categories WHERE name LIKE 'カテゴリ検証%'" >/dev/null

echo "-- baseline"
BASNET_BASE=$(basnet_counts)
[ -n "$BASNET_BASE" ] && ok "Owner counts at start ($BASNET_BASE)" || ng "Owner counts at start: $BASNET_BASE"

echo "-- surface exclusion"
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/admin/categories)
[ "$CODE" = "303" ] && ok "guest GET /admin/categories redirects to login" || ng "guest GET /admin/categories ($CODE)"

admin_login() {
  rm -f "$J"
  curl -s -c "$J" $BASE/admin/login -o /tmp/cat_login.html
  local C=$(csrf /tmp/cat_login.html)
  curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=admin@example.com" --data-urlencode "$PF=$APW" --data-urlencode "$CF=$C"
}
admin_login
# Product posts need an attached image now (the readymade
# picker is gone): upload the 1x1 PNG fixture once and reuse
# its stored path as the image for the posts below.
printf 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==' | base64 -d > /tmp/cat_fixture.png
curl -s -b "$J" $BASE/admin/products/new -o /tmp/cat_upform.html; C=$(csrf /tmp/cat_upform.html)
IMGURL=$(curl -s -b "$J" -X POST $BASE/admin/products/image-upload -H "x-csrf-token: $C" -F "image=@/tmp/cat_fixture.png;type=image/png" | sed -n 's/.*"url": *"\([^"]*\)".*/\1/p')

CODE=$(curl -s -b "$J" -c "$J" -o /dev/null -w "%{http_code}" $BASE/admin/categories)
[ "$CODE" = "200" ] && ok "admin category list renders" || ng "admin category list ($CODE)"

cat_form_token() { curl -s -b "$J" -c "$J" $BASE/admin/categories/new -o /tmp/cat_new.html; csrf /tmp/cat_new.html; }

echo "-- create parent + duplicate/invalid rejection"
C=$(cat_form_token)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/categories/new \
  --data-urlencode "name=カテゴリ検証親" --data-urlencode "slug=catverify-parent" \
  --data-urlencode "description=検証用の親カテゴリー" --data-urlencode "parent_id=" \
  --data-urlencode "display_order=77" --data-urlencode "is_active=1" --data-urlencode "$CF=$C"
PARENT=$(db "SELECT id FROM categories WHERE slug = 'catverify-parent'")
[ -n "$PARENT" ] && ok "category created via UI" || ng "category created via UI"
[ "$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'category.create' AND target_id = $PARENT")" = "1" ] \
  && ok "category.create audit row written" || ng "category.create audit row written"

C=$(cat_form_token)
curl -s -b "$J" -c "$J" -o /tmp/cat_dup.html -X POST $BASE/admin/categories/new \
  --data-urlencode "name=カテゴリ検証親" --data-urlencode "slug=catverify-other" \
  --data-urlencode "description=" --data-urlencode "parent_id=" --data-urlencode "display_order=" \
  --data-urlencode "is_active=1" --data-urlencode "$CF=$C"
grep -q "このカテゴリー名は既に登録されています" /tmp/cat_dup.html && ok "duplicate name is a form error" || ng "duplicate name is a form error"
[ "$(db "SELECT COUNT(*) FROM categories WHERE name = 'カテゴリ検証親'")" = "1" ] && ok "duplicate name created nothing" || ng "duplicate name created nothing"

C=$(cat_form_token)
curl -s -b "$J" -c "$J" -o /tmp/cat_dup2.html -X POST $BASE/admin/categories/new \
  --data-urlencode "name=カテゴリ検証 別名" --data-urlencode "slug=catverify-parent" \
  --data-urlencode "description=" --data-urlencode "parent_id=" --data-urlencode "display_order=" \
  --data-urlencode "is_active=1" --data-urlencode "$CF=$C"
grep -q "このスラッグは既に登録されています" /tmp/cat_dup2.html && ok "duplicate slug is a form error" || ng "duplicate slug is a form error"

C=$(cat_form_token)
curl -s -b "$J" -c "$J" -o /tmp/cat_bad.html -X POST $BASE/admin/categories/new \
  --data-urlencode "name=カテゴリ検証 不正" --data-urlencode "slug=あいうえお" \
  --data-urlencode "description=" --data-urlencode "parent_id=" --data-urlencode "display_order=" \
  --data-urlencode "is_active=1" --data-urlencode "$CF=$C"
grep -q "スラッグは半角英小文字" /tmp/cat_bad.html && ok "Japanese slug is a format error" || ng "Japanese slug is a format error"
[ -z "$(db "SELECT id FROM categories WHERE name = 'カテゴリ検証 不正'")" ] && ok "invalid slug created nothing" || ng "invalid slug created nothing"

echo "-- hierarchy: child, picker exclusions, crafted cycle"
C=$(cat_form_token)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/categories/new \
  --data-urlencode "name=カテゴリ検証子" --data-urlencode "slug=catverify-child" \
  --data-urlencode "description=" --data-urlencode "parent_id=$PARENT" \
  --data-urlencode "display_order=" --data-urlencode "is_active=1" --data-urlencode "$CF=$C"
CHILD=$(db "SELECT id FROM categories WHERE slug = 'catverify-child'")
[ "$(db "SELECT parent_id FROM categories WHERE id = $CHILD")" = "$PARENT" ] && ok "child created under parent" || ng "child created under parent"
curl -s -b "$J" -c "$J" "$BASE/admin/categories?q=catverify" -o /tmp/cat_list.html
grep -q "カテゴリ検証子" /tmp/cat_list.html && grep -q "カテゴリ検証親" /tmp/cat_list.html \
  && ok "list search finds parent and child" || ng "list search finds parent and child"
curl -s -b "$J" -c "$J" $BASE/admin/categories/$PARENT/edit -o /tmp/cat_pedit.html
grep -q "value=\"$CHILD\"" /tmp/cat_pedit.html && ng "parent picker excludes descendants" || ok "parent picker excludes descendants"
grep -q "value=\"$PARENT\"" /tmp/cat_pedit.html && ng "parent picker excludes itself" || ok "parent picker excludes itself"
C=$(csrf /tmp/cat_pedit.html)
curl -s -b "$J" -c "$J" -o /tmp/cat_cycle.html -X POST $BASE/admin/categories/$PARENT/edit \
  --data-urlencode "name=カテゴリ検証親" --data-urlencode "slug=catverify-parent" \
  --data-urlencode "description=" --data-urlencode "parent_id=$CHILD" \
  --data-urlencode "display_order=77" --data-urlencode "is_active=1" --data-urlencode "form_ticket=$(ticket /tmp/cat_pedit.html)" --data-urlencode "$CF=$C"
grep -q "子カテゴリーを親にはできません" /tmp/cat_cycle.html && ok "crafted cycle POST is a form error" || ng "crafted cycle POST is a form error"
[ -z "$(db "SELECT parent_id FROM categories WHERE id = $PARENT")" ] && ok "cycle POST changed nothing" || ng "cycle POST changed nothing"

echo "-- product round-trip + rename propagation"
curl -s -b "$J" -c "$J" $BASE/admin/products/new -o /tmp/cat_pnew.html; C=$(csrf /tmp/cat_pnew.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/products/new \
  --data-urlencode "name=カテゴリ検証商品" --data-urlencode "description=カテゴリ検証の商品" \
  --data-urlencode "category_id=$PARENT" --data-urlencode "price_yen=1000" --data-urlencode "combo_stock[0]=5" \
  --data-urlencode "status=public" --data-urlencode "images=$IMGURL" --data-urlencode "$CF=$C"
PID=$(db "SELECT id FROM products WHERE name = 'カテゴリ検証商品' ORDER BY id DESC LIMIT 1")
[ "$(db "SELECT category_id FROM products WHERE id = $PID")" = "$PARENT" ] && ok "product stores category_id" || ng "product stores category_id"
[ "$(db "SELECT category FROM products WHERE id = $PID")" = "カテゴリ検証親" ] && ok "product stores the name snapshot" || ng "product stores the name snapshot"

curl -s -b "$J" -c "$J" $BASE/admin/categories/$PARENT/edit -o /tmp/cat_pedit2.html; C=$(csrf /tmp/cat_pedit2.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/categories/$PARENT/edit \
  --data-urlencode "name=カテゴリ検証親改" --data-urlencode "slug=catverify-parent" \
  --data-urlencode "description=" --data-urlencode "parent_id=" \
  --data-urlencode "display_order=77" --data-urlencode "is_active=1" --data-urlencode "form_ticket=$(ticket /tmp/cat_pedit2.html)" --data-urlencode "$CF=$C"
[ "$(db "SELECT category FROM products WHERE id = $PID")" = "カテゴリ検証親改" ] && ok "rename rewrites the product snapshot" || ng "rename rewrites the product snapshot"
curl -s -G "$BASE/search" --data-urlencode "category=カテゴリ検証親改" -o /tmp/cat_search.html
grep -q "/products/$PID" /tmp/cat_search.html && ok "search by the new category name lists the product" || ng "search by the new category name lists the product"
curl -s $BASE/ -o /tmp/cat_home.html
grep -q "カテゴリ検証親改" /tmp/cat_home.html && ok "home nav shows the active category" || ng "home nav shows the active category"
curl -s $BASE/products/$PID -o /tmp/cat_detail.html
grep -q "search?category=" /tmp/cat_detail.html && ok "detail links the active category" || ng "detail links the active category"

echo "-- delete: blocked while used, allowed when unused"
curl -s -b "$J" -c "$J" $BASE/admin/categories -o /tmp/cat_list2.html; C=$(csrf /tmp/cat_list2.html)
curl -s -b "$J" -c "$J" -L -o /tmp/cat_delchild.html $BASE/admin/categories/$CHILD/delete --data-urlencode "$CF=$C"
[ -z "$(db "SELECT id FROM categories WHERE id = $CHILD")" ] && ok "unused category deleted" || ng "unused category deleted"
[ "$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'category.delete' AND target_id = $CHILD")" = "1" ] \
  && ok "category.delete audit row written" || ng "category.delete audit row written"
curl -s -b "$J" -c "$J" $BASE/admin/categories -o /tmp/cat_list3.html; C=$(csrf /tmp/cat_list3.html)
curl -s -b "$J" -c "$J" -L -o /tmp/cat_delparent.html $BASE/admin/categories/$PARENT/delete --data-urlencode "$CF=$C"
grep -q "削除できません" /tmp/cat_delparent.html && ok "used category delete is blocked with a reason" || ng "used category delete is blocked with a reason"
[ -n "$(db "SELECT id FROM categories WHERE id = $PARENT")" ] && ok "used category survives the delete attempt" || ng "used category survives the delete attempt"
[ -n "$(db "SELECT id FROM products WHERE id = $PID")" ] && ok "its product is untouched" || ng "its product is untouched"

echo "-- deactivation: nav, product form, storefront detail"
curl -s -b "$J" -c "$J" $BASE/admin/categories/$PARENT/edit -o /tmp/cat_pedit3.html; C=$(csrf /tmp/cat_pedit3.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/categories/$PARENT/edit \
  --data-urlencode "name=カテゴリ検証親改" --data-urlencode "slug=catverify-parent" \
  --data-urlencode "description=" --data-urlencode "parent_id=" \
  --data-urlencode "display_order=77" --data-urlencode "form_ticket=$(ticket /tmp/cat_pedit3.html)" --data-urlencode "$CF=$C"
[ "$(db "SELECT is_active FROM categories WHERE id = $PARENT")" = "f" ] && ok "category deactivated" || ng "category deactivated"
curl -s $BASE/ -o /tmp/cat_home2.html
grep -q "カテゴリ検証親改" /tmp/cat_home2.html && ng "home nav hides the inactive category" || ok "home nav hides the inactive category"
curl -s -b "$J" -c "$J" $BASE/admin/products/new -o /tmp/cat_pnew2.html
grep -q "<option value=\"$PARENT\"" /tmp/cat_pnew2.html && ng "new-product form hides the inactive category" || ok "new-product form hides the inactive category"
curl -s -b "$J" -c "$J" $BASE/admin/products/$PID/edit -o /tmp/cat_pedit4.html
grep -q "value=\"$PARENT\" selected" /tmp/cat_pedit4.html && grep -q "（無効）" /tmp/cat_pedit4.html \
  && ok "edit form shows its own inactive category, marked and selected" || ng "edit form shows its own inactive category, marked and selected"
curl -s $BASE/products/$PID -o /tmp/cat_detail2.html
grep -q "search?category=" /tmp/cat_detail2.html && ng "detail renders the inactive category unlinked" || ok "detail renders the inactive category unlinked"
grep -q "カテゴリ検証親改" /tmp/cat_detail2.html && ok "detail still prints the category name" || ng "detail still prints the category name"
CODE=$(curl -s -o /dev/null -w "%{http_code}" $BASE/products/$PID)
[ "$CODE" = "200" ] && ok "product of an inactive category stays reachable" || ng "product of an inactive category stays reachable ($CODE)"

echo "-- cleanup + final counts"
db "DELETE FROM stock_adjustments WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id = $PID)" >/dev/null
db "DELETE FROM product_variant_option_links WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id = $PID)" >/dev/null
db "DELETE FROM product_tags WHERE product_id = $PID" >/dev/null
db "DELETE FROM product_images WHERE product_id = $PID" >/dev/null
db "DELETE FROM product_variants WHERE product_id = $PID" >/dev/null
db "DELETE FROM products WHERE id = $PID" >/dev/null
curl -s -b "$J" -c "$J" $BASE/admin/categories -o /tmp/cat_list4.html; C=$(csrf /tmp/cat_list4.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/categories/$PARENT/delete --data-urlencode "$CF=$C"
[ -z "$(db "SELECT id FROM categories WHERE id = $PARENT")" ] && ok "category deleted once unused" || ng "category deleted once unused"
[ "$(basnet_counts)" = "$BASNET_BASE" ] && ok "Owner counts unchanged ($BASNET_BASE)" || ng "Owner counts at end: $(basnet_counts) (base $BASNET_BASE)"

result
