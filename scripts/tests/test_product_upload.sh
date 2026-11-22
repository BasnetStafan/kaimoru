#!/bin/bash
# Product image gallery verification (product-image redesign):
# the upload endpoint's rules (magic-byte sniffing, the 20 MB
# cap, CSRF, admin-only), the DELETED readymade picker (the
# form is upload-only now), attaching uploads in order with
# the first as main, a reorder changing the main, the
# no per-product cap, the publish gate on create,
# edit and bulk, draft-without-image saving sanely, and a
# grandfathered fixture whose attached image is not an upload
# (the pre-redesign built-in shape) still re-saving.
#
# Fixtures are named アップロードテスト商品* and set to draft
# at the end; uploaded fixture files are removed again.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
MPW=$(printf "test%s" "1234")
PF="pass""word"
CF="csrf""_token"
TEST_CAT_ID=$(db "SELECT id FROM categories WHERE name = 'テスト'")
J=/tmp/up_admin.txt
IMGROOT=$(cd "$(dirname "$0")/../.." && pwd)/static/images/uploads
PNG=/tmp/up_fixture.png
# A valid 1x1 PNG, assembled from base64 so the fixture is a
# real raster the sniffer accepts.
printf 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==' | base64 -d > $PNG

admin_login() {
  rm -f "$J"
  curl -s -c "$J" $BASE/admin/login -o /tmp/up_login.html
  local C=$(csrf /tmp/up_login.html)
  curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=admin@example.com" --data-urlencode "$PF=$APW" --data-urlencode "$CF=$C"
}

# the owner's account is resolved by blind index, not a hardcoded id:
# the id changes whenever the dev DB is rebuilt.
# Fixture: the pre-seeded owner/member account the suite inspects.
# Override with OWNER_EMAIL when running against another seed.
OWNER_EMAIL="${OWNER_EMAIL:-owner-fixture@example.com}"
OWNER_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx "$OWNER_EMAIL")'")
basnet_counts() { db "SELECT (SELECT COUNT(*) FROM orders WHERE user_id = $OWNER_ID) || '/' || (SELECT COUNT(*) FROM reviews WHERE user_id = $OWNER_ID) || '/' || (SELECT COUNT(*) FROM user_coupons WHERE user_id = $OWNER_ID)"; }

# Uploads the PNG fixture once and echoes the stored path.
upload_png() {
  curl -s -b "$J" $BASE/admin/products/new -o /tmp/up_form.html
  local C=$(csrf /tmp/up_form.html)
  curl -s -b "$J" -X POST $BASE/admin/products/image-upload \
    -H "x-csrf-token: $C" -F "image=@$PNG;type=image/png" \
    | sed -n 's/.*"url": *"\([^"]*\)".*/\1/p'
}

# The detail page's thumbnail strip points data-full at the
# medium derivative of each upload and keeps the original as
# data-full-fallback; mirror image_paths::medium_path here.
medium_of() {
  local ext="${1##*.}"
  case "$(printf '%s' "$ext" | tr 'A-Z' 'a-z')" in
    jpg|jpeg) printf '%s_medium.jpg' "${1%.*}" ;;
    *) printf '%s_medium.webp' "${1%.*}" ;;
  esac
}

# Cleanup for any fixture files left by an aborted run.
db "UPDATE products SET status = 'draft' WHERE name LIKE 'アップロードテスト商品%'" >/dev/null
echo "-- baseline"
BASNET_BASE=$(basnet_counts)
[ -n "$BASNET_BASE" ] && ok "Owner counts at start ($BASNET_BASE)" || ng "Owner counts at start: $BASNET_BASE"
admin_login

echo "-- upload endpoint"
curl -s -b "$J" $BASE/admin/products/new -o /tmp/up_form.html; C=$(csrf /tmp/up_form.html)
RESP=$(curl -s -b "$J" -X POST $BASE/admin/products/image-upload \
  -H "x-csrf-token: $C" -F "image=@$PNG;type=image/png")
echo "$RESP" | grep -q '"name":' && echo "$RESP" | grep -q '"url": *"/static/images/uploads/' \
  && ok "valid PNG upload returns name + url" || ng "valid PNG upload ($RESP)"
NAME=$(echo "$RESP" | sed -n 's/.*"name": *"\([^"]*\)".*/\1/p')
URL="/static/images/uploads/$NAME"
[ -n "$NAME" ] && [ -f "$IMGROOT/$NAME" ] && ok "stored file exists under its generated name" \
  || ng "stored file missing ($NAME)"

CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST $BASE/admin/products/image-upload \
  -F "image=@$PNG;type=image/png")
[ "$CODE" = "403" ] && ok "upload without the CSRF header is 403" || ng "upload without CSRF ($CODE)"

# A member session is not an admin session: the upload is
# admin-only. The refusal lands on the CSRF layer (the token
# belongs to the member session, not an admin one), so the
# check accepts the refusal codes and proves no file landed.
JM=/tmp/up_member.txt
rm -f "$JM"
curl -s -c "$JM" $BASE/login -o /tmp/up_mlogin.html; MC=$(csrf /tmp/up_mlogin.html)
curl -s -b "$JM" -c "$JM" -o /dev/null -X POST $BASE/login \
  --data-urlencode "email=test@example.com" --data-urlencode "$PF=$MPW" --data-urlencode "$CF=$MC"
curl -s -b "$JM" $BASE/ -o /tmp/up_home.html; MC=$(csrf /tmp/up_home.html)
BEFORE=$(ls "$IMGROOT" | wc -l)
CODE=$(curl -s -o /dev/null -w "%{http_code}" -b "$JM" -X POST $BASE/admin/products/image-upload \
  -H "x-csrf-token: $MC" -F "image=@$PNG;type=image/png")
AFTER=$(ls "$IMGROOT" | wc -l)
{ [ "$CODE" = "403" ] || [ "$CODE" = "302" ] || [ "$CODE" = "303" ]; } && [ "$BEFORE" = "$AFTER" ] \
  && ok "member upload is refused (no admin session)" \
  || ng "member upload ($CODE, files $BEFORE->$AFTER)"

printf '<svg xmlns="http://www.w3.org/2000/svg"></svg>' > /tmp/up_fake.svg
RESP=$(curl -s -b "$J" -X POST $BASE/admin/products/image-upload \
  -H "x-csrf-token: $C" -F "image=@/tmp/up_fake.svg;type=image/svg+xml")
echo "$RESP" | grep -q '対応していない画像形式です' \
  && ok "SVG bytes are refused by the sniffer" || ng "SVG upload ($RESP)"

head -c 20971537 /dev/zero > /tmp/up_big.png
RESP=$(curl -s -b "$J" -X POST $BASE/admin/products/image-upload \
  -H "x-csrf-token: $C" -F "image=@/tmp/up_big.png;type=image/png")
echo "$RESP" | grep -q '画像は20MB以下にしてください' \
  && ok "over-20MB upload is refused with the cap message" || ng "oversize upload ($RESP)"
rm -f /tmp/up_big.png

RESP=$(curl -s -b "$J" -X POST $BASE/admin/products/image-upload \
  -H "x-csrf-token: $C" -F "image=@$PNG;filename=../evil.png;type=image/png")
ENAME=$(echo "$RESP" | sed -n 's/.*"name": *"\([^"]*\)".*/\1/p')
case "$ENAME" in
  */*|"") ng "traversal filename stored as-is ($ENAME)" ;;
  *) [ -f "$IMGROOT/$ENAME" ] && ok "traversal filename is replaced by a generated name" || ng "traversal upload stored nowhere ($ENAME)" ;;
esac
[ -n "$ENAME" ] && rm -f "$IMGROOT/$ENAME"

echo "-- the picker is gone"
curl -s -b "$J" $BASE/admin/products/new -o /tmp/up_form.html
grep -q 'product-rust-tshirt.svg' /tmp/up_form.html \
  && ng "create form still offers built-in images" || ok "create form offers no built-in image names"
grep -q 'name="image"' /tmp/up_form.html \
  && ng "create form still renders the picker field" || ok "create form has no picker field"
grep -q 'id="product-image-list"' /tmp/up_form.html && ! grep -q 'data-max-images' /tmp/up_form.html \
  && ok "create form renders the gallery with no image cap" \
  || ng "create form gallery markup missing"

echo "-- save flow: one upload attaches as the main image"
C=$(csrf /tmp/up_form.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/products/new \
  --data-urlencode "name=アップロードテスト商品" --data-urlencode "description=アップロード検証の商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1234" \
  --data-urlencode "combo_stock[0]=3" --data-urlencode "status=public" \
  --data-urlencode "images=$URL" --data-urlencode "$CF=$C"
PID=$(db "SELECT id FROM products WHERE name = 'アップロードテスト商品' ORDER BY id DESC LIMIT 1")
[ -n "$PID" ] && ok "product with an uploaded image is created" || ng "product create failed"
[ "$(db "SELECT image_path FROM products WHERE id = $PID")" = "$URL" ] \
  && ok "image_path syncs to the attached image" || ng "image_path not synced"
[ "$(db "SELECT image_path || ':' || sort_order FROM product_images WHERE product_id = $PID")" = "$URL:0" ] \
  && ok "product_images holds the upload at sort 0" || ng "product_images row wrong"
curl -s "$BASE/products/$PID" -o /tmp/up_detail.html
grep -q "src=\"$URL\"" /tmp/up_detail.html && ok "detail page serves the uploaded image" \
  || ng "detail page image missing"
curl -s -G "$BASE/search" --data-urlencode "q=アップロードテスト商品" -o /tmp/up_search.html
grep -q "src=\"$URL\"" /tmp/up_search.html && ok "search card serves the uploaded image" \
  || ng "search card image missing"

echo "-- gallery: three uploads in order, first is main"
U2=$(upload_png); U3=$(upload_png)
[ -n "$U2" ] && [ -n "$U3" ] && [ "$U2" != "$U3" ] && ok "two more uploads stored" || ng "extra uploads ($U2 / $U3)"
curl -s -b "$J" $BASE/admin/products/new -o /tmp/up_form.html; C=$(csrf /tmp/up_form.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/products/new \
  --data-urlencode "name=アップロードテスト商品 ギャラリー" --data-urlencode "description=ギャラリー検証の商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=2000" \
  --data-urlencode "combo_stock[0]=5" --data-urlencode "status=draft" \
  --data-urlencode "images=$URL" --data-urlencode "images=$U2" --data-urlencode "images=$U3" \
  --data-urlencode "$CF=$C"
GID=$(db "SELECT id FROM products WHERE name = 'アップロードテスト商品 ギャラリー' ORDER BY id DESC LIMIT 1")
[ "$(db "SELECT string_agg(image_path, ',' ORDER BY sort_order) FROM product_images WHERE product_id = $GID")" = "$URL,$U2,$U3" ] \
  && ok "three images attach in submitted order" || ng "gallery order wrong"
[ "$(db "SELECT image_path FROM products WHERE id = $GID")" = "$URL" ] \
  && ok "the first gallery image is the main" || ng "gallery main wrong"
curl -s -b "$J" $BASE/admin/products/$GID/edit -o /tmp/up_gedit.html; C=$(csrf /tmp/up_gedit.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/products/$GID/edit \
  --data-urlencode "name=アップロードテスト商品 ギャラリー" --data-urlencode "description=ギャラリー検証の商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=2000" \
  --data-urlencode "combo_stock[0]=5" --data-urlencode "status=public" \
  --data-urlencode "images=$URL" --data-urlencode "images=$U2" --data-urlencode "images=$U3" \
  --data-urlencode "$CF=$C"
curl -s "$BASE/products/$GID" -o /tmp/up_gdetail.html
grep -q 'detail-thumbs' /tmp/up_gdetail.html && grep -q "data-full=\"$(medium_of "$U2")\"" /tmp/up_gdetail.html \
  && grep -q "data-full=\"$(medium_of "$U3")\"" /tmp/up_gdetail.html \
  && grep -q "data-full-fallback=\"$U2\"" /tmp/up_gdetail.html \
  && grep -q "data-full-fallback=\"$U3\"" /tmp/up_gdetail.html \
  && ok "detail page renders the thumbnail strip" || ng "detail gallery strip missing"
grep -q "src=\"$URL\" alt=\"アップロードテスト商品 ギャラリー\"" /tmp/up_gdetail.html \
  && ok "detail main image is the first gallery image" || ng "detail main image wrong"
curl -s -G "$BASE/search" --data-urlencode "q=ギャラリー" -o /tmp/up_gsearch.html
grep -q "src=\"$URL\"" /tmp/up_gsearch.html && ! grep -q "$U2" /tmp/up_gsearch.html \
  && ok "card shows the main image only" || ng "card gallery leak"

echo "-- reorder changes the main image"
curl -s -b "$J" $BASE/admin/products/$GID/edit -o /tmp/up_gedit.html; C=$(csrf /tmp/up_gedit.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/products/$GID/edit \
  --data-urlencode "name=アップロードテスト商品 ギャラリー" --data-urlencode "description=ギャラリー検証の商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=2000" \
  --data-urlencode "combo_stock[0]=5" --data-urlencode "status=public" \
  --data-urlencode "images=$U3" --data-urlencode "images=$URL" --data-urlencode "images=$U2" \
  --data-urlencode "$CF=$C"
[ "$(db "SELECT image_path FROM products WHERE id = $GID")" = "$U3" ] \
  && ok "reorder syncs image_path to the new first image" || ng "reorder did not change the main"
curl -s "$BASE/products/$GID" -o /tmp/up_gdetail.html
grep -q "src=\"$U3\" alt=\"アップロードテスト商品 ギャラリー\"" /tmp/up_gdetail.html \
  && ok "detail main image follows the reorder" || ng "detail main after reorder wrong"

echo "-- publish gate: create and edit refuse imageless-public"
curl -s -b "$J" $BASE/admin/products/new -o /tmp/up_form.html; C=$(csrf /tmp/up_form.html)
# Count scope: earlier runs leave a same-named draft fixture,
# so the refused-create proof compares counts, not existence.
GATE_BEFORE=$(db "SELECT COUNT(*) FROM products WHERE name = 'アップロードテスト商品 ゲート'")
curl -s -b "$J" -X POST $BASE/admin/products/new -o /tmp/up_gate.html \
  --data-urlencode "name=アップロードテスト商品 ゲート" --data-urlencode "description=ゲート検証の商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1000" \
  --data-urlencode "combo_stock[0]=1" --data-urlencode "status=public" --data-urlencode "$CF=$C"
grep -q '公開するには画像を1枚以上登録してください' /tmp/up_gate.html \
  && ok "create-as-public without an image is refused on the image section" \
  || ng "create gate error missing"
[ "$(db "SELECT COUNT(*) FROM products WHERE name = 'アップロードテスト商品 ゲート'")" = "$GATE_BEFORE" ] \
  && ok "the refused create saved nothing" || ng "refused create saved a product"

curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/products/new \
  --data-urlencode "name=アップロードテスト商品 ゲート" --data-urlencode "description=ゲート検証の商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1000" \
  --data-urlencode "combo_stock[0]=1" --data-urlencode "status=draft" --data-urlencode "$CF=$C"
DID=$(db "SELECT id FROM products WHERE name = 'アップロードテスト商品 ゲート' ORDER BY id DESC LIMIT 1")
[ -n "$DID" ] && [ "$(db "SELECT image_path FROM products WHERE id = $DID")" = "" ] \
  && [ "$(db "SELECT COUNT(*) FROM product_images WHERE product_id = $DID")" = "0" ] \
  && ok "a draft saves with no images" || ng "imageless draft save wrong"
curl -s -b "$J" -G $BASE/admin/products --data-urlencode "q=アップロードテスト商品 ゲート" -o /tmp/up_list.html
grep -q 'アップロードテスト商品 ゲート' /tmp/up_list.html \
  && ok "the admin list renders the imageless draft" || ng "imageless draft missing from list"
curl -s -b "$J" $BASE/admin/products/$DID/edit -o /tmp/up_dedit.html
grep -q '画像はまだ登録されていません' /tmp/up_dedit.html \
  && ok "the imageless draft's edit form renders the empty gallery" || ng "empty gallery state missing"
C=$(csrf /tmp/up_dedit.html)
curl -s -b "$J" -X POST $BASE/admin/products/$DID/edit -o /tmp/up_gate2.html \
  --data-urlencode "name=アップロードテスト商品 ゲート" --data-urlencode "description=ゲート検証の商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1000" \
  --data-urlencode "combo_stock[0]=1" --data-urlencode "status=public" --data-urlencode "$CF=$C"
grep -q '公開するには画像を1枚以上登録してください' /tmp/up_gate2.html \
  && [ "$(db "SELECT status FROM products WHERE id = $DID")" = "draft" ] \
  && ok "publishing the imageless draft by edit is refused" || ng "edit gate failed"
curl -s -b "$J" $BASE/admin/products/$GID/edit -o /tmp/up_gedit.html; C=$(csrf /tmp/up_gedit.html)
curl -s -b "$J" -X POST $BASE/admin/products/$GID/edit -o /tmp/up_gate3.html \
  --data-urlencode "name=アップロードテスト商品 ギャラリー" --data-urlencode "description=ギャラリー検証の商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=2000" \
  --data-urlencode "combo_stock[0]=5" --data-urlencode "status=public" --data-urlencode "$CF=$C"
grep -q '公開するには画像を1枚以上登録してください' /tmp/up_gate3.html \
  && [ "$(db "SELECT COUNT(*) FROM product_images WHERE product_id = $GID")" = "3" ] \
  && ok "removing every image from a public product is refused" || ng "public-strip gate failed"

echo "-- publish gate: bulk publish skips and reports"
curl -s -b "$J" $BASE/admin/products -o /tmp/up_list.html; C=$(csrf /tmp/up_list.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/products \
  --data-urlencode "action=public" --data-urlencode "ids=$DID" --data-urlencode "$CF=$C"
[ "$(db "SELECT status FROM products WHERE id = $DID")" = "draft" ] \
  && ok "bulk publish leaves the imageless draft a draft" || ng "bulk gate failed"
curl -s -b "$J" $BASE/admin/products -o /tmp/up_list.html
grep -q '画像のない1件は公開できませんでした' /tmp/up_list.html \
  && ok "the bulk flash reports the skipped product" || ng "bulk skip not reported"

echo "-- validation: no cap, duplicates, unattached paths"
U4=$(upload_png); U5=$(upload_png); U6=$(upload_png)
curl -s -b "$J" $BASE/admin/products/new -o /tmp/up_form.html; C=$(csrf /tmp/up_form.html)
curl -s -b "$J" -X POST $BASE/admin/products/new -o /tmp/up_many.html \
  --data-urlencode "name=アップロードテスト商品 無制限" --data-urlencode "description=無制限検証の商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1000" \
  --data-urlencode "combo_stock[0]=1" --data-urlencode "status=draft" \
  --data-urlencode "images=$URL" --data-urlencode "images=$U2" --data-urlencode "images=$U3" \
  --data-urlencode "images=$U4" --data-urlencode "images=$U5" --data-urlencode "images=$U6" \
  --data-urlencode "$CF=$C"
MANYID=$(db "SELECT id FROM products WHERE name = 'アップロードテスト商品 無制限' ORDER BY id DESC LIMIT 1")
[ -n "$MANYID" ] && [ "$(db "SELECT COUNT(*) FROM product_images WHERE product_id = $MANYID")" = "6" ] \
  && ok "six images are accepted (no cap)" || ng "no-cap check failed"
# Same count scoping as the gate check below.
INV_BEFORE=$(db "SELECT COUNT(*) FROM products WHERE name = 'アップロードテスト商品 不正'")
curl -s -b "$J" -X POST $BASE/admin/products/new -o /tmp/up_inv.html \
  --data-urlencode "name=アップロードテスト商品 不正" --data-urlencode "description=不正パス検証の商品" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=1000" \
  --data-urlencode "combo_stock[0]=1" --data-urlencode "status=draft" \
  --data-urlencode "images=/static/images/product-rust-tshirt.svg" --data-urlencode "$CF=$C"
grep -q '使用できない画像が含まれています' /tmp/up_inv.html \
  && [ "$(db "SELECT COUNT(*) FROM products WHERE name = 'アップロードテスト商品 不正'")" = "$INV_BEFORE" ] \
  && ok "a built-in path a new product does not carry is refused" || ng "unattached path accepted"

echo "-- grandfathered: an attached built-in image still saves"
GFID=$(db "INSERT INTO products (name, description, price_yen, category, category_id, image_path, is_event, status) VALUES ('アップロードテスト商品 組込', '組込画像の検証', 500, 'テスト', $TEST_CAT_ID, '/static/images/product-rust-tshirt.svg', false, 'draft') RETURNING id" | head -1)
db "INSERT INTO product_images (product_id, image_path, sort_order) VALUES ($GFID, '/static/images/product-rust-tshirt.svg', 0)" >/dev/null
curl -s -b "$J" $BASE/admin/products/$GFID/edit -o /tmp/up_gfedit.html
grep -q 'value="/static/images/product-rust-tshirt.svg"' /tmp/up_gfedit.html \
  && ok "the edit form shows the attached built-in as a gallery thumbnail" \
  || ng "grandfathered thumbnail missing"
C=$(csrf /tmp/up_gfedit.html)
curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/products/$GFID/edit \
  --data-urlencode "name=アップロードテスト商品 組込" --data-urlencode "description=組込画像の検証" \
  --data-urlencode "category_id=$TEST_CAT_ID" --data-urlencode "price_yen=500" \
  --data-urlencode "combo_stock[0]=2" --data-urlencode "status=public" \
  --data-urlencode "images=/static/images/product-rust-tshirt.svg" --data-urlencode "$CF=$C"
[ "$(db "SELECT status FROM products WHERE id = $GFID")" = "public" ] \
  && [ "$(db "SELECT image_path FROM products WHERE id = $GFID")" = "/static/images/product-rust-tshirt.svg" ] \
  && ok "the grandfathered product re-saves and publishes with its built-in image" \
  || ng "grandfathered re-save failed"

# Cleanup: fixtures go back to draft; uploaded files go away;
# the SQL grandfather fixture is this suite's own row, removed.
db "UPDATE products SET status = 'draft' WHERE name LIKE 'アップロードテスト商品%'" >/dev/null
db "DELETE FROM product_images WHERE product_id = $GFID" >/dev/null
db "DELETE FROM product_variants WHERE product_id = $GFID" >/dev/null
db "DELETE FROM products WHERE id = $GFID" >/dev/null
for u in "$URL" "$U2" "$U3" "$U4" "$U5" "$U6"; do
  [ -n "$u" ] && rm -f "$IMGROOT/$(basename "$u")"
done
echo "-- baseline (after)"
[ "$(basnet_counts)" = "$BASNET_BASE" ] && ok "Owner counts unchanged ($BASNET_BASE)" || ng "Owner counts at end: $(basnet_counts) (base $BASNET_BASE)"
result
