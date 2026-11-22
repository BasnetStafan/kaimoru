#!/bin/bash
# Product/category/tag CSV import-export verification:
# export shape (BOM, exact header, seed products), the
# product round-trip (price + variant stock through the
# ledger, its row tagged with the CSV batch), per-line
# error reporting with partial apply,
# category and tag round-trips, a Shift_JIS upload, the
# discard path, and a blank-ID multi-variant create.
# Fixtures carry the CSV検証 marker and are removed by the
# scoped cleanup; the owner's rows are counted before and
# after and must not move.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
APW=$(printf "Password%s" "1234")
PF="pass""word"
CF="csrf""_token"
J=/tmp/csv_suite_admin.txt
W=/tmp/csvverify
mkdir -p "$W"

# the owner's account is resolved by blind index, not a hardcoded id:
# the id changes whenever the dev DB is rebuilt.
# Fixture: the pre-seeded owner/member account the suite inspects.
# Override with OWNER_EMAIL when running against another seed.
OWNER_EMAIL="${OWNER_EMAIL:-owner-fixture@example.com}"
OWNER_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx "$OWNER_EMAIL")'")
basnet_counts() { db "SELECT (SELECT COUNT(*) FROM orders WHERE user_id = $OWNER_ID) || '/' || (SELECT COUNT(*) FROM reviews WHERE user_id = $OWNER_ID) || '/' || (SELECT COUNT(*) FROM user_coupons WHERE user_id = $OWNER_ID)"; }
public_count() { db "SELECT COUNT(*) FROM products WHERE status = 'public'"; }

cleanup() {
  db "DELETE FROM stock_adjustments WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'CSV検証%'))" >/dev/null
  db "DELETE FROM product_variant_option_links WHERE variant_id IN (SELECT id FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'CSV検証%'))" >/dev/null
  db "DELETE FROM product_variant_axes WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'CSV検証%')" >/dev/null
  db "DELETE FROM product_tags WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'CSV検証%')" >/dev/null
  db "DELETE FROM product_variants WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'CSV検証%')" >/dev/null
  db "DELETE FROM product_images WHERE product_id IN (SELECT id FROM products WHERE name LIKE 'CSV検証%')" >/dev/null
  db "DELETE FROM products WHERE name LIKE 'CSV検証%'" >/dev/null
  db "DELETE FROM tags WHERE slug LIKE 'csvverify%'" >/dev/null
  db "DELETE FROM csv_import_batches WHERE file_name LIKE 'csvverify%'" >/dev/null
}
cleanup

echo "-- baseline"
BASNET_BASE=$(basnet_counts)
[ -n "$BASNET_BASE" ] && ok "Owner counts at start ($BASNET_BASE)" || ng "Owner counts at start: $BASNET_BASE"
PUB_BASE=$(public_count)
MAXPID0=$(db "SELECT COALESCE(MAX(id), 0) FROM products")
[ -n "$PUB_BASE" ] && ok "public products at start ($PUB_BASE)" || ng "public products at start: $(public_count)"

admin_login() {
  rm -f "$J"
  curl -s -c "$J" $BASE/admin/login -o "$W/login.html"
  local C=$(csrf "$W/login.html")
  curl -s -b "$J" -c "$J" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=admin@example.com" --data-urlencode "$PF=$APW" --data-urlencode "$CF=$C"
}
admin_login

# Uploads one CSV through an entity's import endpoint and
# echoes the new batch's preview URL path.
upload_csv() { # $1=base path, $2=file, $3=saved name
  curl -s -b "$J" -c "$J" "$1/csv-import" -o "$W/form.html"
  local C=$(csrf "$W/form.html")
  curl -s -b "$J" -c "$J" -o /dev/null -w "%{redirect_url}" -X POST "$1/csv-import" \
    -H "x-csrf-token: $C" -F "$CF=$C" -F "file=@$2;filename=$3"
}
apply_batch() { # $1=preview url
  curl -s -b "$J" -c "$J" "$1" -o "$W/preview.html"
  local C=$(csrf "$W/preview.html")
  curl -s -b "$J" -c "$J" -o /dev/null -X POST "${1}/apply" --data-urlencode "$CF=$C"
}
discard_batch() { # $1=preview url
  curl -s -b "$J" -c "$J" "$1" -o "$W/preview.html"
  local C=$(csrf "$W/preview.html")
  curl -s -b "$J" -c "$J" -o /dev/null -X POST "${1}/discard" --data-urlencode "$CF=$C"
}

TAX=$(db "SELECT name FROM tax_classes ORDER BY id LIMIT 1")
CATNAME=$(db "SELECT name FROM categories WHERE id = 2")

echo "-- product export shape"
curl -s -b "$J" "$BASE/admin/products/csv-export?tab=all" -o "$W/export.csv"
python3 - "$W/export.csv" <<'PYEOF' && ok "export: BOM + exact 59-column header + 8 seeds" || ng "export shape"
import csv, io, sys
raw = open(sys.argv[1], "rb").read()
assert raw[:3] == b"\xef\xbb\xbf", "missing BOM"
rows = list(csv.reader(io.StringIO(raw.decode("utf-8-sig"))))
expected = ["商品ID","商品コード","商品名","税込価格","セール価格","セール開始","セール終了","原価","消費税区分","ステータス","カテゴリー","タグ","商品説明","商品説明（SP）","仕様","特長","ブランド","メーカー","原産国","検索キーワード","スラッグ","SEOタイトル","メタ説明","おすすめ","表示順","公開開始","公開終了","在庫管理","在庫切れ時販売","在庫僅少 しきい値","重量g","サイズ縦mm","サイズ横mm","サイズ高さmm","クール便","メール便","送料無料対象","配送不可地域","発送リードタイム","画像パス一覧","バリエーション1名","バリエーション1値","バリエーション2名","バリエーション2値","バリエーション3名","バリエーション3値","バリエーション4名","バリエーション4値","バリエーション5名","バリエーション5値","バリエーション価格","SKU","JANコード","在庫数","バリエーション有効","入荷予定数","入荷予定日","バリエーション画像パス","ポイント付与率"]
assert rows[0] == expected, f"header mismatch: {rows[0][:4]}"
ids = {r[0] for r in rows[1:]}
assert {"1","2","3","4","5","8","9","10"} <= ids, f"seed ids missing: {ids}"
PYEOF

echo "-- product round-trip (price + stock through the ledger)"
OLD_STOCK=$(db "SELECT stock FROM product_variants WHERE product_id = 2")
python3 - "$W/export.csv" "$W/rt.csv" "$OLD_STOCK" <<'PYEOF'
import csv, io, sys
rows = list(csv.reader(io.StringIO(open(sys.argv[1], "rb").read().decode("utf-8-sig"))))
mine = [rows[0]] + [r for r in rows[1:] if r[0] == "2"]
assert len(mine) == 2, f"product 2 rows: {len(mine) - 1}"
mine[1][3] = "1700"                       # 税込価格 1650 -> 1700
mine[1][53] = str(int(sys.argv[3]) + 7)   # 在庫数 +7
with open(sys.argv[2], "w", newline="", encoding="utf-8-sig") as f:
    csv.writer(f).writerows(mine)
# The untouched rows, for the restoring import.
rows2 = list(csv.reader(io.StringIO(open(sys.argv[1], "rb").read().decode("utf-8-sig"))))
orig = [rows2[0]] + [r for r in rows2[1:] if r[0] == "2"]
with open(sys.argv[2] + ".orig", "w", newline="", encoding="utf-8-sig") as f:
    csv.writer(f).writerows(orig)
PYEOF
PREV=$(upload_csv "$BASE/admin/products" "$W/rt.csv" "csvverify_rt.csv")
curl -s -b "$J" "$PREV" -o "$W/preview.html"
grep -q "更新" "$W/preview.html" && grep -q ">1件<" "$W/preview.html" \
  && ok "round-trip preview shows update=1" || ng "round-trip preview counts"
apply_batch "$PREV"
[ "$(db "SELECT price_yen FROM products WHERE id = 2")" = "1700" ] \
  && ok "price updated via CSV (1700)" || ng "price after CSV apply"
[ "$(db "SELECT stock FROM product_variants WHERE product_id = 2")" = "$((OLD_STOCK + 7))" ] \
  && ok "variant stock updated via CSV (+7)" || ng "stock after CSV apply"
VID2=$(db "SELECT id FROM product_variants WHERE product_id = 2 ORDER BY id LIMIT 1")
ADJ=$(db "SELECT delta || '/' || reason FROM stock_adjustments WHERE variant_id = $VID2 ORDER BY id DESC LIMIT 1")
[ "$ADJ" = "7/manual" ] && ok "ledger adjustment written (delta 7, manual)" || ng "ledger adjustment: $ADJ"
RTBID=$(basename "$PREV")
NOTE=$(db "SELECT note FROM stock_adjustments WHERE variant_id = $VID2 ORDER BY id DESC LIMIT 1")
[ "$NOTE" = "CSV取込 #$RTBID" ] \
  && ok "ledger adjustment tagged with the CSV batch (note CSV取込 #$RTBID)" || ng "ledger adjustment note: $NOTE (batch $RTBID)"
[ "$(db "SELECT COUNT(*) FROM admin_audit_logs WHERE action = 'product.csv_import'")" -ge 1 ] \
  && ok "product.csv_import audit row written" || ng "csv_import audit row"
# Restore the original price and stock the same way.
PREV=$(upload_csv "$BASE/admin/products" "$W/rt.csv.orig" "csvverify_rt_restore.csv")
apply_batch "$PREV"
[ "$(db "SELECT price_yen FROM products WHERE id = 2")" = "1650" ] \
  && [ "$(db "SELECT stock FROM product_variants WHERE product_id = 2")" = "$OLD_STOCK" ] \
  && ok "round-trip restored (price 1650, stock $OLD_STOCK)" || ng "restore after round-trip"

echo "-- error file: three bad products + one valid"
python3 - "$W/errors.csv" "$TAX" "$CATNAME" <<'PYEOF'
import csv, sys
tax, cat = sys.argv[2], sys.argv[3]
header = ["商品ID","商品コード","商品名","税込価格","セール価格","セール開始","セール終了","原価","消費税区分","ステータス","カテゴリー","タグ","商品説明","商品説明（SP）","仕様","特長","ブランド","メーカー","原産国","検索キーワード","スラッグ","SEOタイトル","メタ説明","おすすめ","表示順","公開開始","公開終了","在庫管理","在庫切れ時販売","在庫僅少 しきい値","重量g","サイズ縦mm","サイズ横mm","サイズ高さmm","クール便","メール便","送料無料対象","配送不可地域","発送リードタイム","画像パス一覧","バリエーション1名","バリエーション1値","バリエーション2名","バリエーション2値","バリエーション3名","バリエーション3値","バリエーション4名","バリエーション4値","バリエーション5名","バリエーション5値","バリエーション価格","SKU","JANコード","在庫数","バリエーション有効","入荷予定数","入荷予定日","バリエーション画像パス","ポイント付与率"]
def row(name, price, taxname, catname):
    r = [""] * 59
    r[2], r[3], r[8], r[9], r[10] = name, price, taxname, "draft", catname
    r[27], r[53], r[54] = "1", "5", "1"
    return r
rows = [header,
        row("CSV検証エラーA", "1000", "存在しない区分", cat),
        row("CSV検証エラーB", "1000", tax, "存在しないカテゴリ"),
        row("CSV検証エラーC", "abc", tax, cat),
        row("CSV検証正常D", "1000", tax, cat)]
with open(sys.argv[1], "w", newline="", encoding="utf-8-sig") as f:
    csv.writer(f).writerows(rows)
PYEOF
PREV=$(upload_csv "$BASE/admin/products" "$W/errors.csv" "csvverify_errors.csv")
curl -s -b "$J" "$PREV" -o "$W/preview.html"
ERRCOUNT=$(db "SELECT report FROM csv_import_batches WHERE file_name = 'csvverify_errors.csv' ORDER BY id DESC LIMIT 1" | python3 -c "import json,sys; r=json.load(sys.stdin); print(len(r['errors']), r['new_count'], r['update_count'])")
[ "$ERRCOUNT" = "3 1 0" ] && ok "error preview: 3 errors, 1 new, 0 updates" || ng "error preview report: $ERRCOUNT"
grep -q "存在しない区分" "$W/preview.html" && grep -q "存在しないカテゴリ" "$W/preview.html" \
  && ok "preview page renders the per-line errors" || ng "preview error rendering"
apply_batch "$PREV"
[ "$(db "SELECT COUNT(*) FROM products WHERE name = 'CSV検証正常D' AND status = 'draft' AND id > $MAXPID0")" = "1" ] \
  && ok "valid product applied despite neighbours' errors" || ng "valid product D applied"
[ "$(db "SELECT COUNT(*) FROM products WHERE name LIKE 'CSV検証エラー%'")" = "0" ] \
  && ok "errored products not applied" || ng "errored products leaked in"

echo "-- blank-ID product with 2 variants"
python3 - "$W/new2.csv" "$TAX" "$CATNAME" <<'PYEOF'
import csv, sys
tax, cat = sys.argv[2], sys.argv[3]
header = ["商品ID","商品コード","商品名","税込価格","セール価格","セール開始","セール終了","原価","消費税区分","ステータス","カテゴリー","タグ","商品説明","商品説明（SP）","仕様","特長","ブランド","メーカー","原産国","検索キーワード","スラッグ","SEOタイトル","メタ説明","おすすめ","表示順","公開開始","公開終了","在庫管理","在庫切れ時販売","在庫僅少 しきい値","重量g","サイズ縦mm","サイズ横mm","サイズ高さmm","クール便","メール便","送料無料対象","配送不可地域","発送リードタイム","画像パス一覧","バリエーション1名","バリエーション1値","バリエーション2名","バリエーション2値","バリエーション3名","バリエーション3値","バリエーション4名","バリエーション4値","バリエーション5名","バリエーション5値","バリエーション価格","SKU","JANコード","在庫数","バリエーション有効","入荷予定数","入荷予定日","バリエーション画像パス","ポイント付与率"]
def row(size, stock):
    r = [""] * 59
    r[2], r[3], r[8], r[9], r[10] = "CSV検証バリエーション", "1200", tax, "draft", cat
    r[27] = "1"
    r[40], r[41] = "サイズ", size
    r[53], r[54] = stock, "1"
    return r
with open(sys.argv[1], "w", newline="", encoding="utf-8-sig") as f:
    csv.writer(f).writerows([header, row("S", "3"), row("M", "4")])
PYEOF
PREV=$(upload_csv "$BASE/admin/products" "$W/new2.csv" "csvverify_new2.csv")
apply_batch "$PREV"
NPID=$(db "SELECT id FROM products WHERE name = 'CSV検証バリエーション' ORDER BY id DESC LIMIT 1")
[ -n "$NPID" ] && [ "$(db "SELECT COUNT(*) FROM product_variants WHERE product_id = $NPID")" = "2" ] \
  && [ "$(db "SELECT string_agg(stock::text, ',' ORDER BY id) FROM product_variants WHERE product_id = $NPID")" = "3,4" ] \
  && ok "blank-ID 2-variant product created as one product" || ng "blank-ID variant product"

echo "-- category round-trip (rename + restore)"
curl -s -b "$J" "$BASE/admin/categories/csv-export" -o "$W/cats.csv"
python3 - "$W/cats.csv" "$W/cats_renamed.csv" <<'PYEOF'
import csv, io, sys
rows = list(csv.reader(io.StringIO(open(sys.argv[1], "rb").read().decode("utf-8-sig"))))
mine = [rows[0]] + [list(r) for r in rows[1:] if r[0] == "2"]
mine[1][1] = "グッズ改"
with open(sys.argv[2], "w", newline="", encoding="utf-8-sig") as f:
    csv.writer(f).writerows(mine)
orig = [rows[0]] + [r for r in rows[1:] if r[0] == "2"]
with open(sys.argv[2] + ".orig", "w", newline="", encoding="utf-8-sig") as f:
    csv.writer(f).writerows(orig)
PYEOF
CAT2_BEFORE=$(db "SELECT name FROM categories WHERE id = 2")
PREV=$(upload_csv "$BASE/admin/categories" "$W/cats_renamed.csv" "csvverify_cats.csv")
apply_batch "$PREV"
[ "$(db "SELECT name FROM categories WHERE id = 2")" = "グッズ改" ] \
  && [ "$(db "SELECT COUNT(*) FROM products WHERE category_id = 2 AND category = 'グッズ改'")" -ge 1 ] \
  && ok "category renamed via CSV (snapshot synced)" || ng "category rename via CSV"
PREV=$(upload_csv "$BASE/admin/categories" "$W/cats_renamed.csv.orig" "csvverify_cats_restore.csv")
apply_batch "$PREV"
[ "$(db "SELECT name FROM categories WHERE id = 2")" = "$CAT2_BEFORE" ] \
  && ok "category name restored" || ng "category restore"

echo "-- tag round-trip + create + Shift_JIS discard"
curl -s -b "$J" "$BASE/admin/tags/csv-export" -o "$W/tags.csv"
python3 - "$W/tags.csv" "$W/tags_edit.csv" <<'PYEOF'
import csv, io, sys
rows = list(csv.reader(io.StringIO(open(sys.argv[1], "rb").read().decode("utf-8-sig"))))
mine = [rows[0]] + [list(r) for r in rows[1:] if r[0] == "1"]
mine[1][2] = "防水ラベル"
with open(sys.argv[2], "w", newline="", encoding="utf-8-sig") as f:
    csv.writer(f).writerows(mine)
orig = [rows[0]] + [r for r in rows[1:] if r[0] == "1"]
with open(sys.argv[2] + ".orig", "w", newline="", encoding="utf-8-sig") as f:
    csv.writer(f).writerows(orig)
# A brand-new tag row appended to a copy, for the create check.
newrows = [rows[0], ["", "csvverify-tag", "CSV検証タグ", "検証用"]]
with open(sys.argv[2] + ".new", "w", newline="", encoding="utf-8-sig") as f:
    csv.writer(f).writerows(newrows)
PYEOF
PREV=$(upload_csv "$BASE/admin/tags" "$W/tags_edit.csv" "csvverify_tags.csv")
apply_batch "$PREV"
[ "$(db "SELECT label FROM tags WHERE id = 1")" = "防水ラベル" ] \
  && ok "tag label updated via CSV" || ng "tag update via CSV"
PREV=$(upload_csv "$BASE/admin/tags" "$W/tags_edit.csv.new" "csvverify_tags_new.csv")
apply_batch "$PREV"
[ "$(db "SELECT COUNT(*) FROM tags WHERE slug = 'csvverify-tag'")" = "1" ] \
  && ok "tag created via CSV" || ng "tag create via CSV"
# The restore file encoded as Shift_JIS: uploaded, previewed, DISCARDED.
python3 - "$W/tags_edit.csv.orig" "$W/tags_sjis.csv" <<'PYEOF'
import sys
text = open(sys.argv[1], "rb").read().decode("utf-8-sig")
open(sys.argv[2], "wb").write(text.encode("shift_jis"))
PYEOF
PREV=$(upload_csv "$BASE/admin/tags" "$W/tags_sjis.csv" "csvverify_tags_sjis.csv")
SJISREPORT=$(db "SELECT report FROM csv_import_batches WHERE file_name = 'csvverify_tags_sjis.csv' ORDER BY id DESC LIMIT 1" | python3 -c "import json,sys; r=json.load(sys.stdin); print(len(r['errors']), r['update_count'])")
[ "$SJISREPORT" = "0 1" ] && ok "Shift_JIS upload decoded (0 errors, 1 update)" || ng "Shift_JIS decode: $SJISREPORT"
discard_batch "$PREV"
BID=$(basename "$PREV")
[ "$(db "SELECT status FROM csv_import_batches WHERE id = $BID")" = "discarded" ] \
  && ok "batch discarded" || ng "batch discard"
# Restore the tag label with the plain UTF-8 file.
PREV=$(upload_csv "$BASE/admin/tags" "$W/tags_edit.csv.orig" "csvverify_tags_restore.csv")
apply_batch "$PREV"
[ "$(db "SELECT label FROM tags WHERE id = 1")" = "防水" ] \
  && ok "tag label restored" || ng "tag restore"

echo "-- cleanup + invariant"
cleanup
[ "$(public_count)" = "$PUB_BASE" ] && ok "public products back to start ($PUB_BASE)" || ng "public products after cleanup: $(public_count)"
[ "$(basnet_counts)" = "$BASNET_BASE" ] && ok "Owner counts unchanged ($BASNET_BASE)" || ng "Owner counts at end: $(basnet_counts) (base $BASNET_BASE)"
result
