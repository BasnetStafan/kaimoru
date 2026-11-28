#!/bin/bash
# Shopping agent, driven through the /agent
# chat (the pre-chat brief form was retired on 2026-11-07;
# its form-validation section retired with it): agent cart
# provenance (cart_items.added_by_agent), replace-own-lines
# runs (an empty run keeps the previous proposal), human-line
# protection, the hard budget cap with its
# session allow-listed add-anyway, category restriction, live
# negation matching, member parity and the
# 10-runs/hour session cap.
#
# Runs against the live app at 127.0.0.1:3000 (./run.sh).
# Catalog expectations are STATE-RELATIVE: a snapshot of the
# public catalog is read from the API at start and every
# expected pick, price and overage below is derived from it,
# never hardcoded. Fixture products for the negation case are
# built through the admin flow under a per-run unique name and
# archived again at the end (the admin flow has no product
# delete; archive is its removal path — the fixture category
# is deactivated, since category deletion is blocked while
# products reference it). the owner's account (user 32) is only
# ever read, to prove the invariant: 14 orders / 1 review /
# 1 coupon, no agent lines in his cart, 8 public products.
#
# Field-name note: the CSRF and password field NAMES are
# assembled from fragments below, never written literally
# with a value — the same care test_mail.sh takes.
BASE=http://127.0.0.1:3000
. "$(dirname "$0")/lib.sh"

CT=$(printf 'csrf_%s' 'token')
PF=$(printf 'pass%s' 'word')
XH=$(printf 'x-csrf-%s' 'token')
APW=$(printf "Password%s" "1234")
MPW=$(printf "test%s" "1234")
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
# the owner's account is resolved by blind index, not a hardcoded id:
# the id changes whenever the dev DB is rebuilt.
# Fixture: the pre-seeded owner/member account the suite inspects.
# Override with OWNER_EMAIL when running against another seed.
OWNER_EMAIL="${OWNER_EMAIL:-owner-fixture@example.com}"
OWNER_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx "$OWNER_EMAIL")'")
basnet_counts() { db "SELECT (SELECT COUNT(*) FROM orders WHERE user_id = $OWNER_ID) || '/' || (SELECT COUNT(*) FROM reviews WHERE user_id = $OWNER_ID) || '/' || (SELECT COUNT(*) FROM user_coupons WHERE user_id = $OWNER_ID)"; }
TEST_ID=$(db "SELECT id FROM users WHERE email_bidx = '$(bidx test@example.com)'")

STAMP=$(date +%Y%m%d%H%M%S)$$
FIX_CAT_NAME="エージェント検証${STAMP}"
FIX_SLUG="agent-test-${STAMP}"
NEG_NAME="エージェント検証・否定 ${STAMP}"
POS_NAME="エージェント検証・肯定 ${STAMP}"
NEG_DESC="この商品は防水ではありません。水に濡れる場所では使用できません。"
POS_DESC="この商品は防水仕様です。雨の日でも使用できます。"

# Backstop cleanup: whatever a run leaves behind, the next run
# (or this one's exit) puts away — fixture products archived,
# fixture categories deactivated, the fixture member's agent
# lines deleted. Scoped to this suite's own names only.
cleanup_backstop() {
  db "UPDATE products SET status = 'archived' WHERE name LIKE 'エージェント検証%' AND status = 'public'" >/dev/null 2>&1
  db "UPDATE categories SET is_active = false WHERE slug LIKE 'agent-test-%' AND is_active" >/dev/null 2>&1
  [ -n "$TEST_ID" ] && db "DELETE FROM cart_items WHERE user_id = $TEST_ID AND added_by_agent" >/dev/null 2>&1
}
trap cleanup_backstop EXIT
cleanup_backstop

echo "-- baseline"
BASNET_BASE=$(basnet_counts)
[ -n "$BASNET_BASE" ] && ok "Owner counts at start ($BASNET_BASE)" || ng "Owner counts at start: $BASNET_BASE"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE user_id = 32 AND added_by_agent")" = "0" ] \
  && ok "no agent lines in the owner's cart at start" || ng "agent lines in the owner's cart at start"
[ "$(db "SELECT COUNT(*) FROM products WHERE status = 'public'")" = "8" ] \
  && ok "8 public products at start" || ng "public products at start: $(db "SELECT COUNT(*) FROM products WHERE status = 'public'")"
MAXID0=$(db "SELECT COALESCE(MAX(id), 0) FROM cart_items")

# --- catalog snapshot (state-relative expectations) --------
# One row per public product, from the live API (list + detail
# for the variants), plus the free-shipping flag from the DB
# (not part of the API shape). Columns:
#   1 category  2 product_id  3 name  4 price_yen  5 stock
#   6 chosen variant id (deepest-stocked in-stock purchasable,
#     ties cheapest then lowest id — the engine's default rule
#     since A2; empty when the product cannot be picked)
#   7 its price
#   8 its stock  9 free_shipping (1/0)
SNAP=/tmp/agent_snap_$$.tsv
BASE=$BASE python3 - > $SNAP <<'PY'
import json, os, subprocess, urllib.request
base = os.environ["BASE"]
def get(path):
    with urllib.request.urlopen(base + path) as r:
        return json.load(r)
first = get("/api/products?page=1")
products = list(first["products"])
for page in range(2, first["total_pages"] + 1):
    products += get(f"/api/products?page={page}")["products"]
flags = {}
out = subprocess.check_output(["docker", "exec", "agentic-db", "psql", "-U", "agentic",
                               "-d", "agentic_commerce", "-t", "-A", "-F", "|",
                               "-c", "SELECT id, free_shipping FROM products"]).decode()
for line in out.splitlines():
    if "|" in line:
        i, f = line.split("|")
        flags[int(i)] = "1" if f == "t" else "0"
def clean(s):
    return str(s).replace("\t", " ").replace("\n", " ")
rows = []
for prod in products:
    detail = get(f"/api/products/{prod['id']}")
    vs = [v for v in (detail.get("variants") or []) if v["stock"] > 0 and v["purchasable"]]
    chosen = min(vs, key=lambda v: (-v["stock"], v["price_yen"], v["id"])) if vs else None
    rows.append("\t".join([
        clean(prod["category"]["name"]), str(prod["id"]), clean(prod["name"]),
        str(prod["price_yen"]), str(prod["stock"]),
        str(chosen["id"]) if chosen else "",
        str(chosen["price_yen"]) if chosen else "",
        str(chosen["stock"]) if chosen else "",
        flags.get(prod["id"], "0"),
    ]))
print("\n".join(rows))
PY
[ -s $SNAP ] && ok "catalog snapshot read from the live API" || ng "catalog snapshot read from the live API"

categories_in_order() { awk -F'\t' '!seen[$1]++ {print $1}' $SNAP; }
pick_count() { awk -F'\t' -v c="$1" '$1==c && $6!="" {n++} END {print n+0}' $SNAP; }
pick_variants() { awk -F'\t' -v c="$1" '$1==c && $6!="" {print $6}' $SNAP | sort -n | tr '\n' ' ' | sed 's/ $//'; }
sum_prices() { local ids=" $* "; awk -F'\t' -v ids="$ids" 'index(ids, " " $6 " ") {s+=$7} END {print s+0}' $SNAP; }
all_free() { local ids=" $* "; awk -F'\t' -v ids="$ids" 'index(ids, " " $6 " ") {n++; if ($9!="1") bad=1} END {print (n>0 && !bad) ? 1 : 0}' $SNAP; }
shipping_for() { if [ "$1" -eq 0 ] || [ "$2" = "1" ] || [ "$1" -ge 5000 ]; then echo 0; else echo 550; fi; }
yen() { python3 -c 'import sys; print("￥" + format(int(sys.argv[1]), ","))' "$1"; }
# The results block of one product in a saved results page:
# picks and over-budget entries render as card sections
# (<section class="agent-pick">), already-in-cart entries as
# cart lines (<li class="cart-line">). The first block naming
# the product wins, or nothing when absent.
block_for() { python3 -c '
import re, sys
s = open(sys.argv[1], encoding="utf-8").read()
for b in re.findall(r"<li class=\"cart-line\">.*?</li>|<section class=\"agent-pick\">.*?</section>", s, re.S):
    if ("/products/" + sys.argv[2] + "\"") in b:
        print(b); break
' "$1" "$2"; }
latest_session_for() { db "SELECT session_id FROM cart_items WHERE variant_id = $1 AND id > $2 ORDER BY id DESC LIMIT 1"; }
agent_variants_of_session() { db "SELECT variant_id FROM cart_items WHERE session_id = '$1' AND added_by_agent ORDER BY variant_id" | tr '\n' ' ' | sed 's/ $//'; }

# One chat turn in a jar: fresh form token, then the POST.
chat_turn() { # jar outfile message
  curl -s -b "$1" -c "$1" $BASE/agent -o /tmp/agent_turnform_$$.html
  local t=$(csrf /tmp/agent_turnform_$$.html)
  curl -s -b "$1" -c "$1" -X POST $BASE/agent/chat -o "$2" \
    --data-urlencode "$CT=$t" --data-urlencode "message=$3"
}

# Runs the agent once in a jar through the chat: the brief as
# one message; when the reply carries no results yet (a
# clarifying question), it is answered affirmatively once,
# per the conversation rules. The saved page is the full
# GET /agent render afterwards — the thread carries every
# results block the assertions read.
run_agent() { # jar outfile message
  chat_turn "$1" /tmp/agent_turn_$$.html "$3"
  if ! grep -q '総合スコア' /tmp/agent_turn_$$.html \
     && ! grep -q '実行回数の上限' /tmp/agent_turn_$$.html; then
    chat_turn "$1" /tmp/agent_turn_$$.html "はい"
  fi
  curl -s -b "$1" -c "$1" $BASE/agent -o "$2"
}

# The shopper's apply-all action on the latest proposal
# (design rule 2026-11-09: a chat search only proposes;
# cart writes need this explicit action). Re-renders /agent
# into the outfile so result blocks show the applied state.
apply_all() { # jar outfile
  curl -s -b "$1" -c "$1" $BASE/agent -o /tmp/agent_applyform_$$.html
  local t=$(csrf /tmp/agent_applyform_$$.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/agent/apply-all \
    --data-urlencode "$CT=$t"
  curl -s -b "$1" -c "$1" $BASE/agent -o "$2"
}

echo "-- the chat surface (the brief form is retired)"
JF=$(mktemp)
curl -s -c $JF $BASE/agent -o /tmp/agent_get_$$.html
grep -q 'お買い物エージェント' /tmp/agent_get_$$.html && ok "GET /agent renders the agent page" || ng "GET /agent renders the agent page"
grep -q 'name="message"' /tmp/agent_get_$$.html && ok "chat message input present" || ng "chat message input present"
grep -q 'name="budget_yen"' /tmp/agent_get_$$.html \
  && ng "legacy brief form is gone (no budget_yen field)" || ok "legacy brief form is gone (no budget_yen field)"
grep -q 'name="must_have"' /tmp/agent_get_$$.html \
  && ng "legacy brief form is gone (no must_have field)" || ok "legacy brief form is gone (no must_have field)"
grep -qE '<(input|select|textarea)[^>]*[[:space:]]required([[:space:]>])' /tmp/agent_get_$$.html \
  && ng "page uses no native required attributes" || ok "page uses no native required attributes"

echo "-- guest run: a search proposes only; apply-all lands the picks, a re-apply replaces them"
CORE_CAT=""
while IFS= read -r c; do
  n=$(pick_count "$c"); if [ "$n" -ge 1 ] && [ "$n" -le 3 ]; then CORE_CAT="$c"; break; fi
done <<EOF
$(categories_in_order)
EOF
[ -n "$CORE_CAT" ] && ok "core category derived live ($CORE_CAT)" || ng "core category derived live"
EXP_VARIANTS=$(pick_variants "$CORE_CAT")
SUB=$(sum_prices $EXP_VARIANTS); AF=$(all_free $EXP_VARIANTS)
CORE_BUDGET=$((SUB + $(shipping_for $SUB $AF) + 10000))
JG=$(mktemp)
run_agent $JG /tmp/agent_run1_$$.html "${CORE_CAT}の商品が欲しいです。予算は${CORE_BUDGET}円以内です"
grep -q '総合スコア' /tmp/agent_run1_$$.html && ok "run renders its results" || ng "run renders its results"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE id > $MAXID0 AND added_by_agent")" = "0" ] \
  && ok "the search alone wrote no agent lines (proposal only)" || ng "the search alone wrote no agent lines"
apply_all $JG /tmp/agent_run1_$$.html
FIRST_V=$(echo $EXP_VARIANTS | cut -d' ' -f1)
GSESSION=$(latest_session_for $FIRST_V $MAXID0)
[ -n "$GSESSION" ] && ok "guest session identified from its cart rows" || ng "guest session identified from its cart rows"
[ "$(agent_variants_of_session "$GSESSION")" = "$EXP_VARIANTS" ] \
  && ok "agent lines are exactly the expected picks" || ng "agent lines: $(agent_variants_of_session "$GSESSION") != $EXP_VARIANTS"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE session_id = '$GSESSION' AND NOT added_by_agent")" = "0" ] \
  && ok "every line the run wrote is flagged agent" || ng "every line the run wrote is flagged agent"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE session_id = '$GSESSION' AND quantity <> 1")" = "0" ] \
  && ok "agent lines are quantity 1" || ng "agent lines are quantity 1"
ROWS1=$(db "SELECT COUNT(*) FROM cart_items WHERE session_id = '$GSESSION'")
run_agent $JG /tmp/agent_run2_$$.html "${CORE_CAT}の商品が欲しいです。予算は${CORE_BUDGET}円以内です"
apply_all $JG /tmp/agent_run2_$$.html
[ "$(agent_variants_of_session "$GSESSION")" = "$EXP_VARIANTS" ] \
  && ok "second identical run keeps the same variant set" || ng "second identical run keeps the same variant set"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE session_id = '$GSESSION'")" = "$ROWS1" ] \
  && ok "second run grows no rows (replace, not accumulate)" || ng "second run grows no rows"
ROWIDS1=$(db "SELECT id FROM cart_items WHERE session_id = '$GSESSION' ORDER BY id" | tr '\n' ' ' | sed 's/ $//')

echo "-- empty run: picks nothing, keeps the previous proposal; a later good run replaces it"
chat_turn $JG /tmp/agent_turn_$$.html "予算は100円以内に変えてください"
curl -s -b $JG -c $JG $BASE/agent -o /tmp/agent_run3_$$.html
grep -q '予算オーバーのため追加しなかった商品' /tmp/agent_run3_$$.html \
  && ok "empty proposal renders only its over-budget refusals" || ng "empty proposal renders only its over-budget refusals"
apply_all $JG /tmp/agent_run3_$$.html
[ "$(agent_variants_of_session "$GSESSION")" = "$EXP_VARIANTS" ] \
  && ok "empty run leaves the previous picks in place" || ng "empty run leaves the previous picks: $(agent_variants_of_session "$GSESSION") != $EXP_VARIANTS"
[ "$(db "SELECT id FROM cart_items WHERE session_id = '$GSESSION' ORDER BY id" | tr '\n' ' ' | sed 's/ $//')" = "$ROWIDS1" ] \
  && ok "empty run touches no cart row (same row ids)" || ng "empty run touches no cart row (same row ids)"
KEPT_MAXID=$(db "SELECT COALESCE(MAX(id), 0) FROM cart_items WHERE session_id = '$GSESSION'")
chat_turn $JG /tmp/agent_turn_$$.html "予算は${CORE_BUDGET}円に戻してください"
curl -s -b $JG -c $JG $BASE/agent -o /tmp/agent_run4_$$.html
apply_all $JG /tmp/agent_run4_$$.html
[ "$(agent_variants_of_session "$GSESSION")" = "$EXP_VARIANTS" ] \
  && ok "a later non-empty run lands the same picks again" || ng "a later non-empty run lands the same picks again"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE session_id = '$GSESSION' AND id <= $KEPT_MAXID")" = "0" ] \
  && ok "the later run really replaced the kept rows (new row ids)" || ng "the later run really replaced the kept rows (new row ids)"

echo "-- category restriction: a scoped brief picks only that category"
CAT2=""
while IFS= read -r c; do
  [ "$c" = "$CORE_CAT" ] && continue
  n=$(pick_count "$c"); if [ "$n" -ge 1 ] && [ "$n" -le 3 ]; then CAT2="$c"; break; fi
done <<EOF
$(categories_in_order)
EOF
[ -n "$CAT2" ] && ok "second category derived live ($CAT2)" || ng "second category derived live"
EXP2_VARIANTS=$(pick_variants "$CAT2")
SUB2=$(sum_prices $EXP2_VARIANTS); AF2=$(all_free $EXP2_VARIANTS)
CAT2_BUDGET=$((SUB2 + $(shipping_for $SUB2 $AF2) + 10000))
JC=$(mktemp)
run_agent $JC /tmp/agent_cat_$$.html "${CAT2}の商品が欲しいです。予算は${CAT2_BUDGET}円以内です"
apply_all $JC /tmp/agent_cat_$$.html
CSESSION=$(latest_session_for $(echo $EXP2_VARIANTS | cut -d' ' -f1) $MAXID0)
[ "$(agent_variants_of_session "$CSESSION")" = "$EXP2_VARIANTS" ] \
  && ok "category run picks the category's pickable products" || ng "category run picks: $(agent_variants_of_session "$CSESSION") != $EXP2_VARIANTS"
[ "$(db "SELECT COUNT(*) FROM cart_items ci JOIN product_variants pv ON pv.id = ci.variant_id JOIN products p ON p.id = pv.product_id WHERE ci.session_id = '$CSESSION' AND ci.added_by_agent AND p.category <> '$CAT2'")" = "0" ] \
  && ok "no pick comes from outside the category" || ng "no pick comes from outside the category"

echo "-- human-line protection: a hand-added line is never touched"
# The live catalog has no category with two pickable products,
# so the section targets the one pickable product of the core
# category: hand-added first, then a category run whose only
# candidate it is. Protection means: the line is untouched,
# no agent copy exists, and the results report it as already
# in the cart instead of proposing it.
read THUM_PID TV TV_PRICE <<< $(awk -F'\t' -v c="$CORE_CAT" '$1==c && $6!="" && $8>=2 {print $2, $6, $7; exit}' $SNAP)
[ -n "$TV" ] && ok "human-protection target derived live ($CORE_CAT)" || ng "human-protection target derived live"
THUM_NAME=$(awk -F'\t' -v p="$THUM_PID" '$2==p {print $3}' $SNAP)
JH=$(mktemp)
HMARK=$(db "SELECT COALESCE(MAX(id), 0) FROM cart_items")
curl -s -b $JH -c $JH $BASE/agent -o /tmp/agent_hform_$$.html; T=$(csrf /tmp/agent_hform_$$.html)
curl -s -b $JH -c $JH -o /dev/null -X POST $BASE/cart/items \
  --data-urlencode "product_id=$THUM_PID" --data-urlencode "variant_id=$TV" \
  --data-urlencode "quantity=2" --data-urlencode "$CT=$T"
HSESSION=$(latest_session_for $TV $HMARK)
[ "$(db "SELECT quantity || '/' || added_by_agent FROM cart_items WHERE session_id = '$HSESSION' AND variant_id = $TV")" = "2/false" ] \
  && ok "manual add lands as a human line, quantity 2" || ng "manual add lands as a human line, quantity 2"
run_agent $JH /tmp/agent_human_$$.html "${CORE_CAT}の商品が欲しいです。予算は100000円以内です"
grep -q 'すでにカートに入っている商品' /tmp/agent_human_$$.html \
  && ok "results carry the already-in-cart section" || ng "results carry the already-in-cart section"
[ -n "$(block_for /tmp/agent_human_$$.html $THUM_PID)" ] \
  && ok "the hand-added product is named in the results" || ng "the hand-added product is named in the results"
[ "$(db "SELECT quantity || '/' || added_by_agent FROM cart_items WHERE session_id = '$HSESSION' AND variant_id = $TV")" = "2/false" ] \
  && ok "human line unchanged by the run (qty 2, not agent)" || ng "human line unchanged by the run"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE session_id = '$HSESSION' AND variant_id = $TV")" = "1" ] \
  && ok "no agent copy of the human variant exists" || ng "no agent copy of the human variant exists"
[ -z "$(agent_variants_of_session "$HSESSION")" ] \
  && ok "the run adds no agent lines around the human line" || ng "the run adds no agent lines: $(agent_variants_of_session "$HSESSION")"

echo "-- budget hard cap: exact overage, add-anyway honoured once"
BCAT=""
while IFS= read -r c; do
  if [ "$(pick_count "$c")" = "1" ]; then BCAT="$c"; break; fi
done <<EOF
$(categories_in_order)
EOF
[ -n "$BCAT" ] && ok "single-pickable category derived live ($BCAT)" || ng "single-pickable category derived live"
read BPID BV BPRICE BFREE <<< $(awk -F'\t' -v c="$BCAT" '$1==c && $6!="" {print $2, $6, $7, $9; exit}' $SNAP)
BPROJ=$((BPRICE + $(shipping_for $BPRICE $BFREE)))
BOVER=$((BPROJ - 100))
JB=$(mktemp)
ROWS_BEFORE=$(db "SELECT COUNT(*) FROM cart_items WHERE variant_id = $BV")
run_agent $JB /tmp/agent_budget_$$.html "${BCAT}の商品が欲しいです。予算は100円以内です"
grep -q '予算オーバーのため追加しなかった商品' /tmp/agent_budget_$$.html \
  && ok "over-budget section renders" || ng "over-budget section renders"
BUDGET_BLOCK=$(block_for /tmp/agent_budget_$$.html $BPID)
echo "$BUDGET_BLOCK" | grep -q "予算を $(yen $BOVER) 超過します" \
  && ok "overage is the exact live figure ($(yen $BOVER))" || ng "overage is the exact live figure (expected $(yen $BOVER))"
echo "$BUDGET_BLOCK" | grep -q "合計（送料込み）が $(yen $BPROJ) になり" \
  && ok "projected total matches the totals rule ($(yen $BPROJ))" || ng "projected total matches the totals rule (expected $(yen $BPROJ))"
# (The chat renders no add-anyway form; that the refused
# variant — and only it — is allow-listed is proven
# behaviourally by the honoured POST below and the two
# refused POSTs after it.)
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE variant_id = $BV")" = "$ROWS_BEFORE" ] \
  && ok "the refused pick was not auto-added" || ng "the refused pick was not auto-added"
curl -s -b $JB -c $JB $BASE/agent -o /tmp/agent_bform_$$.html; T=$(csrf /tmp/agent_bform_$$.html)
BMARK=$(db "SELECT COALESCE(MAX(id), 0) FROM cart_items")
OUT=$(curl -s -b $JB -c $JB -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/agent/add-anyway \
  --data-urlencode "$CT=$T" --data-urlencode "variant_id=$BV")
[ "$OUT" = "303 $BASE/cart" ] && ok "add-anyway honoured once (303 to /cart)" || ng "add-anyway honoured once ($OUT)"
BSESSION=$(latest_session_for $BV $BMARK)
[ "$(db "SELECT quantity || '/' || added_by_agent FROM cart_items WHERE session_id = '$BSESSION' AND variant_id = $BV")" = "1/true" ] \
  && ok "add-anyway wrote an agent line" || ng "add-anyway wrote an agent line"
OUT=$(curl -s -b $JB -c $JB -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/agent/add-anyway \
  --data-urlencode "$CT=$T" --data-urlencode "variant_id=$BV")
[ "$OUT" = "303 $BASE/agent" ] && ok "replaying add-anyway is refused (back to /agent)" || ng "replaying add-anyway is refused ($OUT)"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE session_id = '$BSESSION' AND variant_id = $BV")" = "1" ] \
  && ok "replay added no second line" || ng "replay added no second line"
OTHER_V=$(echo $EXP2_VARIANTS | cut -d' ' -f1)
OUT=$(curl -s -b $JB -c $JB -o /dev/null -w "%{http_code} %{redirect_url}" -X POST $BASE/agent/add-anyway \
  --data-urlencode "$CT=$T" --data-urlencode "variant_id=$OTHER_V")
[ "$OUT" = "303 $BASE/agent" ] && ok "a variant the run never refused is refused" || ng "a variant the run never refused is refused ($OUT)"
CODE=$(curl -s -b $JB -c $JB -o /dev/null -w "%{http_code}" -X POST $BASE/agent/add-anyway \
  --data-urlencode "$CT=$T" --data-urlencode "variant_id=unknown")
[ "$CODE" = "404" ] && ok "an unknown variant id answers 404" || ng "an unknown variant id answers 404 ($CODE)"

echo "-- chat robustness: an empty message runs nothing"
JE=$(mktemp)
EMARK=$(db "SELECT COALESCE(MAX(id), 0) FROM cart_items")
chat_turn $JE /tmp/agent_empty_$$.html ""
grep -q 'お買い物エージェント' /tmp/agent_empty_$$.html \
  && ok "empty message is answered without an error page" || ng "empty message is answered without an error page"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE id > $EMARK AND added_by_agent")" = "0" ] \
  && ok "empty message created no agent lines" || ng "empty message created no agent lines"

echo "-- member parity: a signed-in member gets agent lines under their user id"
JM=$(mktemp)
curl -s -c $JM $BASE/login -o /tmp/agent_login_$$.html; T=$(csrf /tmp/agent_login_$$.html)
curl -s -b $JM -c $JM -o /dev/null -X POST $BASE/login \
  --data-urlencode "email=test@example.com" --data-urlencode "$PF=$MPW" --data-urlencode "$CT=$T"
CODE=$(curl -s -b $JM -c $JM -o /dev/null -w "%{http_code}" $BASE/mypage)
[ "$CODE" = "200" ] && ok "fixture member signs in" || ng "fixture member signs in ($CODE)"
# The fixture member's cart is suite-managed across the whole
# suite family: empty it so the parity expectation is exact.
db "DELETE FROM cart_items WHERE user_id = $TEST_ID" >/dev/null
run_agent $JM /tmp/agent_member_$$.html "${CAT2}の商品が欲しいです。予算は${CAT2_BUDGET}円以内です"
apply_all $JM /tmp/agent_member_$$.html
MEMBER_VARIANTS=$(db "SELECT variant_id FROM cart_items WHERE user_id = $TEST_ID AND added_by_agent ORDER BY variant_id" | tr '\n' ' ' | sed 's/ $//')
[ "$MEMBER_VARIANTS" = "$EXP2_VARIANTS" ] \
  && ok "member apply-all writes the same picks as agent lines" || ng "member apply-all writes the same picks: $MEMBER_VARIANTS != $EXP2_VARIANTS"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE user_id = $TEST_ID AND added_by_agent AND session_id IS NOT NULL")" = "0" ] \
  && ok "member agent lines hang on the user id, not a session" || ng "member agent lines hang on the user id, not a session"
db "DELETE FROM cart_items WHERE user_id = $TEST_ID AND added_by_agent" >/dev/null
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE user_id = $TEST_ID AND added_by_agent")" = "0" ] \
  && ok "fixture member's agent lines cleaned up" || ng "fixture member's agent lines cleaned up"

echo "-- negation, live: a negated mention earns no keyword credit"
# The positive control is the canary: it proves description
# keyword matching is live at all — without it, the negation
# assertion would pass vacuously if descriptions never
# reached the matcher.
JA=$(mktemp)
curl -s -c $JA $BASE/admin/login -o /tmp/agent_alogin_$$.html; T=$(csrf /tmp/agent_alogin_$$.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/login \
  --data-urlencode "email=admin@example.com" --data-urlencode "$PF=$APW" --data-urlencode "$CT=$T"
PNG=/tmp/agent_fixture_$$.png
printf 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==' | base64 -d > $PNG
curl -s -b $JA $BASE/admin/products/new -o /tmp/agent_pform_$$.html; T=$(csrf /tmp/agent_pform_$$.html)
IMGURL=$(curl -s -b $JA -X POST $BASE/admin/products/image-upload -H "$XH: $T" -F "image=@$PNG;type=image/png" \
  | sed -n 's/.*"url": *"\([^"]*\)".*/\1/p')
[ -n "$IMGURL" ] && ok "fixture image uploaded through the admin flow" || ng "fixture image uploaded through the admin flow"
curl -s -b $JA -c $JA $BASE/admin/categories/new -o /tmp/agent_cform_$$.html; T=$(csrf /tmp/agent_cform_$$.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/categories/new \
  --data-urlencode "name=$FIX_CAT_NAME" --data-urlencode "slug=$FIX_SLUG" \
  --data-urlencode "description=エージェント検証用のカテゴリー" --data-urlencode "parent_id=" \
  --data-urlencode "display_order=99" --data-urlencode "is_active=1" --data-urlencode "$CT=$T"
FIX_CAT_ID=$(db "SELECT id FROM categories WHERE slug = '$FIX_SLUG'")
[ -n "$FIX_CAT_ID" ] && ok "fixture category created through the admin flow" || ng "fixture category created through the admin flow"
create_fixture() { # name description price_yen -> product id
  curl -s -b $JA -c $JA $BASE/admin/products/new -o /tmp/agent_pform2_$$.html; local t=$(csrf /tmp/agent_pform2_$$.html)
  curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/new \
    --data-urlencode "name=$1" --data-urlencode "description=$2" \
    --data-urlencode "category_id=$FIX_CAT_ID" --data-urlencode "price_yen=$3" \
    --data-urlencode "combo_stock[0]=10" --data-urlencode "status=public" \
    --data-urlencode "images=$IMGURL" --data-urlencode "$CT=$t"
  db "SELECT id FROM products WHERE name = '$1'"
}
NEG_ID=$(create_fixture "$NEG_NAME" "$NEG_DESC" 1300)
POS_ID=$(create_fixture "$POS_NAME" "$POS_DESC" 1200)
[ -n "$NEG_ID" ] && [ -n "$POS_ID" ] && ok "both fixture products created public" || ng "both fixture products created public"
[ "$(db "SELECT COUNT(*) FROM products WHERE status = 'public'")" = "10" ] \
  && ok "public count is 10 while the fixtures live" || ng "public count is 10 while the fixtures live"
NEG_V=$(db "SELECT id FROM product_variants WHERE product_id = $NEG_ID")
POS_V=$(db "SELECT id FROM product_variants WHERE product_id = $POS_ID")
JN=$(mktemp)
run_agent $JN /tmp/agent_neg_$$.html "${FIX_CAT_NAME}の商品が欲しいです。防水だと嬉しいです。予算は100000円以内です"
apply_all $JN /tmp/agent_neg_$$.html
POS_BLOCK=$(block_for /tmp/agent_neg_$$.html $POS_ID)
NEG_BLOCK=$(block_for /tmp/agent_neg_$$.html $NEG_ID)
echo "$POS_BLOCK" | grep -q '説明文に「防水」の記載があります' \
  && ok "positive control is credited as a description mention" || ng "positive control is credited as a description mention"
echo "$NEG_BLOCK" | grep -q '未評価のため中間の評価で計算' \
  && ok "negated fixture really is in the results (block sanity)" || ng "negated fixture really is in the results (block sanity)"
echo "$NEG_BLOCK" | grep -q '説明文に「防水」の記載があります' \
  && ng "negated mention is NOT credited as a description mention" || ok "negated mention is NOT credited as a description mention"
NSESSION=$(latest_session_for $NEG_V $MAXID0)
FIX_SET=$(db "SELECT variant_id FROM cart_items WHERE session_id = '$NSESSION' AND added_by_agent AND variant_id IN ($NEG_V, $POS_V) ORDER BY variant_id" | tr '\n' ' ' | sed 's/ $//')
EXP_FIX_SET=$(printf '%s\n%s\n' $NEG_V $POS_V | sort -n | tr '\n' ' ' | sed 's/ $//')
[ "$FIX_SET" = "$EXP_FIX_SET" ] && ok "both fixtures were picked into the guest cart" || ng "both fixtures were picked into the guest cart"
# Fixture cleanup through the admin flow: archive both
# products (archive is the admin flow's removal path — there
# is no product delete), then deactivate the category
# (deletion is blocked while products reference it).
for PID in $NEG_ID $POS_ID; do
  curl -s -b $JA -c $JA $BASE/admin/products/$PID/edit -o /tmp/agent_edit_$$.html; T=$(csrf /tmp/agent_edit_$$.html)
  curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/products/$PID/archive --data-urlencode "$CT=$T"
done
[ "$(db "SELECT COUNT(*) FROM products WHERE id IN ($NEG_ID, $POS_ID) AND status = 'archived'")" = "2" ] \
  && ok "fixture products archived through the admin flow" || ng "fixture products archived through the admin flow"
curl -s -b $JA -c $JA $BASE/admin/categories/$FIX_CAT_ID/edit -o /tmp/agent_cedit_$$.html; T=$(csrf /tmp/agent_cedit_$$.html)
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/categories/$FIX_CAT_ID/edit \
  --data-urlencode "name=$FIX_CAT_NAME" --data-urlencode "slug=$FIX_SLUG" \
  --data-urlencode "description=エージェント検証用のカテゴリー" --data-urlencode "parent_id=" \
  --data-urlencode "display_order=99" --data-urlencode "form_ticket=$(ticket /tmp/agent_cedit_$$.html)" --data-urlencode "$CT=$T"
[ "$(db "SELECT is_active FROM categories WHERE id = $FIX_CAT_ID")" = "f" ] \
  && ok "fixture category deactivated" || ng "fixture category deactivated"
[ "$(db "SELECT COUNT(*) FROM products WHERE status = 'public'")" = "8" ] \
  && ok "public count back to 8 after fixture cleanup" || ng "public count back to 8 after fixture cleanup"

echo "-- run cap: the 11th run in an hour is refused (LAST: it poisons its session)"
# The counters are in-memory per app process, keyed by session
# fingerprint: an app restart clears them, and a fresh jar is
# a fresh session — which is why this section runs last, in
# its own jar, like test_admin.sh's throttle section. In the
# chat a search decision is what consumes a run; the same
# complete brief re-posted searches again every time (proven
# when this suite was ported to the chat).
JR=$(mktemp)
REFUSED_AT=0
for i in $(seq 1 11); do
  chat_turn $JR /tmp/agent_cap_$$.html "グッズの商品が欲しいです。予算は100円以内です"
  if grep -q 'エージェントの実行回数の上限' /tmp/agent_cap_$$.html; then REFUSED_AT=$i; break; fi
done
[ "$REFUSED_AT" = "11" ] && ok "runs 1-10 pass, the 11th is refused with the limit message" \
  || ng "runs 1-10 pass, the 11th is refused with the limit message (refused at $REFUSED_AT)"

echo "-- invariant"
[ "$(basnet_counts)" = "$BASNET_BASE" ] && ok "Owner counts unchanged ($BASNET_BASE)" || ng "Owner counts at end: $(basnet_counts) (base $BASNET_BASE)"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE user_id = 32 AND added_by_agent")" = "0" ] \
  && ok "still no agent lines in the owner's cart" || ng "still no agent lines in the owner's cart"
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE user_id = $TEST_ID AND added_by_agent")" = "0" ] \
  && ok "fixture member left with no agent lines" || ng "fixture member left with no agent lines"
[ "$(db "SELECT COUNT(*) FROM products WHERE status = 'public'")" = "8" ] \
  && ok "8 public products at end" || ng "public products at end: $(db "SELECT COUNT(*) FROM products WHERE status = 'public'")"
[ "$(db "SELECT COUNT(*) FROM products WHERE name LIKE 'エージェント検証%' AND status = 'public'")" = "0" ] \
  && ok "no fixture product left public" || ng "no fixture product left public"

result
