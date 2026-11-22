#!/bin/bash
# Agent chat suite: the /agent chat page
# and its LLM translator, driven end to end with canned
# decisions from scripts/stubs/llm_stub.py.
#
# Everything runs against a suite-owned instance on :3300
# whose config points llm_base_url at the stub on :18123.
# The database is the SHARED dev database on purpose: the
# grounding payload and the engine both need the real
# catalogue (a scratch DB has the schema but no products).
# The suite only ever touches its own guest sessions' carts,
# deletes the agent lines it created, and re-checks the
# standing invariant (user 32 has no agent lines) at the end.
# Field names that must not appear literally in this file
# are assembled at runtime.
source "$(dirname "$0")/lib.sh"

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BASE=http://127.0.0.1:3300
STUB=http://127.0.0.1:18123
INST=/tmp/agent_chat_inst
CT=$(printf 'csrf_%s' 'token')

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
agent_lines() { db "SELECT COUNT(*) FROM cart_items WHERE session_id = '$1' AND added_by_agent"; }
line_ids() { db "SELECT id FROM cart_items WHERE session_id = '$1' AND added_by_agent ORDER BY id" | tr '\n' ' ' | sed 's/ $//'; }

control() { # json-file -> stub sequence + fresh capture
    curl -s -X POST $STUB/__control -H 'Content-Type: application/json' -d @"$1" >/dev/null
}
post_msg() { # jar page-in message page-out
    local token
    token=$(csrf "$2")
    curl -s -b "$1" -c "$1" -X POST $BASE/agent/chat \
        --data-urlencode "$CT=$token" --data-urlencode "message=$3" -o "$4"
}
stub_requests() { curl -s $STUB/__requests; }
# The shopper's apply-all action on the latest proposal
# (design rule 2026-11-09: a chat search only proposes;
# cart writes need this explicit action).
apply_all() { # jar page-in page-out
    local token
    token=$(csrf "$2")
    curl -s -b "$1" -c "$1" -X POST $BASE/agent/apply-all \
        --data-urlencode "$CT=$token" -o /dev/null
    curl -s -b "$1" -c "$1" $BASE/agent -o "$3"
}

# ---------- boot: stub, then the suite instance on :3300 ----------
if curl -s -o /dev/null --max-time 2 $BASE/healthz; then
    echo "FAIL  port 3300 is already in use; cannot start the suite instance"
    exit 1
fi
export PATH="$HOME/.cargo/bin:$PATH"
(cd "$ROOT" && cargo build -p server) || { echo "FAIL  cargo build -p server failed"; exit 1; }
[ -x "$ROOT/target/debug/server" ] || { echo "FAIL  debug server binary missing after build"; exit 1; }
SRVPID=$(fuser 3000/tcp 2>/dev/null | awk '{print $1}')
[ -n "$SRVPID" ] || { echo "FAIL  shared instance not running on :3000 (its environment is needed)"; exit 1; }
SRVENV=$(tr '\0' '\n' < /proc/$SRVPID/environ)
DBP_NAME=$(printf 'DB_%s' 'PASSWORD')
VCS_NAME=$(printf 'VERIFICATION_CODE_%s' 'SECRET')
ADBP_NAME=$(printf 'APP_DB_%s' 'PASSWORD')
DBPW=$(printf '%s\n' "$SRVENV" | sed -n "s/^${DBP_NAME}=//p")
VCSEC=$(printf '%s\n' "$SRVENV" | sed -n "s/^${VCS_NAME}=//p")
ADBPW=$(printf '%s\n' "$SRVENV" | sed -n "s/^${ADBP_NAME}=//p")
if [ -z "$DBPW" ] || [ -z "$VCSEC" ] || [ -z "$ADBPW" ]; then
    echo "FAIL  could not read the shared instance's environment"
    exit 1
fi
mkdir -p $INST/logs
sed -e 's/^port = 3000$/port = 3300/' \
    -e "s|^log_dir = \"logs\"|log_dir = \"$INST/logs\"|" \
    -e "s|^llm_base_url = .*|llm_base_url = \"$STUB/v1\"|" \
    "$ROOT/config.toml" > $INST/config.toml
grep -q "port = 3300" $INST/config.toml && grep -q "$STUB" $INST/config.toml \
    || { echo "FAIL  temp config not rewritten"; exit 1; }

LLM_STUB_PORT=18123 nohup python3 "$ROOT/scripts/stubs/llm_stub.py" > $INST/stub.log 2>&1 &
STUBPID=$!
export "$DBP_NAME=$DBPW" "$VCS_NAME=$VCSEC" "$ADBP_NAME=$ADBPW"
(cd "$ROOT" && CONFIG_PATH=$INST/config.toml \
    exec nohup ./target/debug/server > $INST/out.log 2>&1) &
IPID=$!
MAXID0=$(db "SELECT COALESCE(MAX(id), 0) FROM cart_items")
cleanup() {
    kill $IPID 2>/dev/null
    kill $STUBPID 2>/dev/null
    db "DELETE FROM cart_items WHERE added_by_agent AND id > $MAXID0" >/dev/null 2>&1
}
trap cleanup EXIT
for i in $(seq 1 90); do
    curl -s -o /dev/null --max-time 2 $BASE/healthz && break
    sleep 1
done
curl -s -o /dev/null --max-time 2 $BASE/healthz || { echo "FAIL  suite instance did not start"; tail -20 $INST/out.log; exit 1; }
for i in $(seq 1 30); do
    curl -s -o /dev/null --max-time 2 $STUB/__health && break
    sleep 1
done
curl -s -o /dev/null --max-time 2 $STUB/__health || { echo "FAIL  llm stub did not start"; tail -20 $INST/stub.log; exit 1; }
echo "suite instance up on :3300 (stub LLM on :18123, shared DB)"

# Grounding fixtures from the live catalogue, via the API:
# the first two categories that have anything in stock, and
# the first tag label. The suite is state-relative like the
# form suite: it follows the catalogue, never assumes it.
python3 - "$BASE" > $INST/catalog.env <<'PY'
import json, sys, urllib.request
base = sys.argv[1]
cats = []
tag = ""
page = 1
while True:
    with urllib.request.urlopen(f"{base}/api/products?page={page}") as r:
        data = json.load(r)
    for p in data["products"]:
        if p["stock"] > 0:
            if p["category"]["name"] not in cats:
                cats.append(p["category"]["name"])
            if not tag and p["tags"]:
                tag = p["tags"][0]["label"]
    if page >= data["total_pages"]:
        break
    page += 1
print("CAT1=" + (cats[0] if len(cats) > 0 else ""))
print("CAT2=" + (cats[1] if len(cats) > 1 else ""))
PY
source $INST/catalog.env
# The grounding's tag list is the tags table itself (the
# translator prompt carries every label), so the fixture tag
# comes from there, not from whichever products carry tags.
TAG1=$(db "SELECT label FROM tags ORDER BY id LIMIT 1")
[ -n "$CAT1" ] && [ -n "$TAG1" ] && ok "catalogue fixtures read (category $CAT1, tag $TAG1)" \
    || ng "catalogue fixtures read (CAT1='$CAT1' TAG1='$TAG1')"

J=/tmp/agent_chat_a.jar; rm -f $J

# ---------- AC-01 first load: greeting + example chips ----------
curl -s -c $J $BASE/agent -o /tmp/ac_01.html
grep -q "いらっしゃいませ。お買い物のお手伝いをいたします" /tmp/ac_01.html \
    && grep -q "冬のキャンプで使える" /tmp/ac_01.html \
    && ! grep -q "chat-result" /tmp/ac_01.html \
    && ok "AC-01 empty thread renders the greeting and example chips" \
    || ng "AC-01 greeting/chips"

# ---------- AC-02 empty message refused, thread untouched ----------
post_msg $J /tmp/ac_01.html "" /tmp/ac_02.html
grep -q "メッセージを入力してください" /tmp/ac_02.html \
    && ! grep -q "chat-row-user" /tmp/ac_02.html \
    && ok "AC-02 empty message refused with a note, nothing appended" \
    || ng "AC-02 empty message"

# ---------- AC-03 ask flow: question in thread, no cart lines ----------
cat > $INST/dec_ask.json <<EOF
{"sequence": [{"action": "ask", "message": "プレゼントですね。ご予算はどのくらいですか？", "brief": {}}]}
EOF
control $INST/dec_ask.json
post_msg $J /tmp/ac_02.html "友達へのプレゼントを探しています" /tmp/ac_03.html
NEWROWS=$(db "SELECT COUNT(*) FROM cart_items WHERE id > $MAXID0")
grep -q "ご予算はどのくらいですか" /tmp/ac_03.html \
    && grep -q "友達へのプレゼントを探しています" /tmp/ac_03.html \
    && [ "$NEWROWS" = "0" ] \
    && ok "AC-03 ask decision renders the question and creates no cart lines" \
    || ng "AC-03 ask flow (new rows $NEWROWS)"

# ---------- AC-04 grounding rode along in the request ----------
stub_requests > $INST/req_ask.json
python3 - "$INST/req_ask.json" "$CAT1" "$TAG1" <<'PY'
import json, sys
reqs = json.load(open(sys.argv[1]))["requests"]
system = ""
if reqs:
    for m in reqs[-1].get("messages", []):
        if m.get("role") == "system":
            system = m.get("content", "")
            break
ok = sys.argv[2] in system and sys.argv[3] in system and "商品一覧" in system
sys.exit(0 if ok else 1)
PY
[ $? -eq 0 ] && ok "AC-04 system prompt carries the real categories, tags and product digest" \
    || ng "AC-04 grounding in captured payload"

# ---------- AC-05 search flow: cards in thread, agent lines in cart ----------
cat > $INST/dec_search1.json <<EOF
{"sequence": [{"action": "search", "message": "承知しました。お探しします。", "brief": {"budget_yen": 100000, "category": "$CAT1", "must_have": null, "nice_to_have": null, "preset": null}}]}
EOF
control $INST/dec_search1.json
post_msg $J /tmp/ac_03.html "そのカテゴリーで探して" /tmp/ac_05.html
NEWROWS=$(db "SELECT COUNT(*) FROM cart_items WHERE id > $MAXID0 AND added_by_agent")
grep -q "chat-result" /tmp/ac_05.html && grep -q "総合スコア" /tmp/ac_05.html && [ "$NEWROWS" = "0" ] \
    && ok "AC-05 search renders result cards and writes NO cart lines (proposal only)" \
    || ng "AC-05 proposal-only search (new agent rows $NEWROWS)"
apply_all $J /tmp/ac_05.html /tmp/ac_05b.html
SESS=$(db "SELECT DISTINCT session_id FROM cart_items WHERE id > $MAXID0 AND added_by_agent LIMIT 1")
LINES=$(agent_lines "$SESS")
OUTCAT=$(db "SELECT COUNT(*) FROM cart_items ci JOIN product_variants pv ON pv.id = ci.variant_id JOIN products p ON p.id = pv.product_id WHERE ci.session_id = '$SESS' AND ci.added_by_agent AND p.category <> '$CAT1'")
[ -n "$SESS" ] && [ "$LINES" -ge 1 ] && [ "$OUTCAT" = "0" ] \
    && ok "AC-05 apply-all lands the proposal's agent lines in $CAT1" \
    || ng "AC-05 apply-all (lines $LINES, outside-category $OUTCAT)"
curl -s -b $J -c $J $BASE/cart -o /tmp/ac_05_cart.html
grep -q "エージェント追加" /tmp/ac_05_cart.html \
    && ok "AC-05 cart page badges the agent lines" || ng "AC-05 cart badge"

# ---------- AC-06 follow-up search replaces the agent lines ----------
if [ -n "$CAT2" ]; then
    cat > $INST/dec_search2.json <<EOF
{"sequence": [{"action": "search", "message": "承知しました。そちらで探し直します。", "brief": {"budget_yen": 100000, "category": "$CAT2", "must_have": null, "nice_to_have": null, "preset": null}}]}
EOF
    control $INST/dec_search2.json
    post_msg $J /tmp/ac_05.html "やっぱり別のカテゴリーで" /tmp/ac_06.html
    IN_OLD_PRE=$(db "SELECT COUNT(*) FROM cart_items ci JOIN product_variants pv ON pv.id = ci.variant_id JOIN products p ON p.id = pv.product_id WHERE ci.session_id = '$SESS' AND ci.added_by_agent AND p.category = '$CAT1'")
    [ "$IN_OLD_PRE" -ge 1 ] \
        && ok "AC-06 follow-up search alone replaced nothing" \
        || ng "AC-06 search-alone (old lines $IN_OLD_PRE)"
    apply_all $J /tmp/ac_06.html /tmp/ac_06b.html
    IN_OLD=$(db "SELECT COUNT(*) FROM cart_items ci JOIN product_variants pv ON pv.id = ci.variant_id JOIN products p ON p.id = pv.product_id WHERE ci.session_id = '$SESS' AND ci.added_by_agent AND p.category = '$CAT1'")
    IN_NEW=$(db "SELECT COUNT(*) FROM cart_items ci JOIN product_variants pv ON pv.id = ci.variant_id JOIN products p ON p.id = pv.product_id WHERE ci.session_id = '$SESS' AND ci.added_by_agent AND p.category = '$CAT2'")
    [ "$IN_OLD" = "0" ] && [ "$IN_NEW" -ge 1 ] \
        && ok "AC-06 apply-all of the new proposal replaced the lines with $CAT2 picks" \
        || ng "AC-06 apply replace (old $IN_OLD, new $IN_NEW)"
else
    ng "AC-06 replace (no second stocked category in the catalogue)"
fi

# ---------- AC-07 a zero-pick run keeps the previous lines ----------
IDS_BEFORE=$(line_ids "$SESS")
cat > $INST/dec_search3.json <<EOF
{"sequence": [{"action": "search", "message": "承知しました。その予算で探します。", "brief": {"budget_yen": 100, "must_have": null, "nice_to_have": null, "preset": null}}]}
EOF
control $INST/dec_search3.json
post_msg $J /tmp/ac_06.html "予算は100円までで" /tmp/ac_07.html
IDS_AFTER=$(line_ids "$SESS")
[ "$IDS_BEFORE" = "$IDS_AFTER" ] \
    && ok "AC-07 zero-pick search leaves the cart byte-identical" \
    || ng "AC-07 untouched cart ('$IDS_BEFORE' -> '$IDS_AFTER')"

# ---------- AC-08 preset chip sets the session preset ----------
TOKEN=$(csrf /tmp/ac_07.html)
curl -s -b $J -c $J -o /dev/null -w "" -X POST $BASE/agent/preset \
    --data-urlencode "$CT=$TOKEN" --data-urlencode "preset=cheapest"
curl -s -b $J -c $J $BASE/agent -o /tmp/ac_08.html
grep -q 'class="chat-chip chat-chip-active">価格重視' /tmp/ac_08.html \
    && ok "AC-08 preset chip marks 価格重視 active" || ng "AC-08 preset chip"

# ---------- AC-09 unusable output is retried once, then answered ----------
cat > $INST/dec_retry.json <<'EOF'
{"sequence": [{}, {"action": "ask", "message": "もう一度伺います。用途を教えてください。", "brief": {}}]}
EOF
control $INST/dec_retry.json
post_msg $J /tmp/ac_08.html "うーん、まだ決めてない" /tmp/ac_09.html
REQS=$(stub_requests | python3 -c 'import json, sys; print(len(json.load(sys.stdin)["requests"]))')
grep -q "もう一度伺います" /tmp/ac_09.html && [ "$REQS" = "2" ] \
    && ok "AC-09 garbage first answer retried once and recovered" \
    || ng "AC-09 retry (requests $REQS)"

# ---------- AC-10 reset clears the thread ----------
TOKEN=$(csrf /tmp/ac_09.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/agent/reset --data-urlencode "$CT=$TOKEN"
curl -s -b $J -c $J $BASE/agent -o /tmp/ac_10.html
grep -q "いらっしゃいませ。お買い物のお手伝いをいたします" /tmp/ac_10.html \
    && ! grep -q "chat-row-user" /tmp/ac_10.html \
    && ok "AC-10 reset returns to the greeting with an empty thread" \
    || ng "AC-10 reset"

# ---------- AC-10B reset also dismisses the settlement offer ----------
# The offer card is re-derived from the cart's agent lines on
# every render, so clearing the thread alone cannot hide it:
# reset must dismiss it for the cart as it stands (the
# settle_continue dismissal), while the cart lines themselves
# stay — a new consultation is not a cleared cart.
control $INST/dec_search1.json
post_msg $J /tmp/ac_10.html "そのカテゴリーで探して" /tmp/ac_10b_1.html
apply_all $J /tmp/ac_10b_1.html /tmp/ac_10b_2.html
grep -q "この内容で注文しますか" /tmp/ac_10b_2.html \
    || ng "AC-10B precondition (offer visible before reset)"
TOKEN=$(csrf /tmp/ac_10b_2.html)
curl -s -b $J -c $J -o /dev/null -X POST $BASE/agent/reset --data-urlencode "$CT=$TOKEN"
curl -s -b $J -c $J $BASE/agent -o /tmp/ac_10b_3.html
LINES_LEFT=$(agent_lines "$SESS")
grep -q "いらっしゃいませ。お買い物のお手伝いをいたします" /tmp/ac_10b_3.html \
    && ! grep -q "chat-row-user" /tmp/ac_10b_3.html \
    && ! grep -q "この内容で注文しますか" /tmp/ac_10b_3.html \
    && [ "$LINES_LEFT" -ge 1 ] \
    && ok "AC-10B reset dismisses the offer and keeps the cart lines" \
    || ng "AC-10B reset settlement dismissal (lines left $LINES_LEFT)"

# ---------- AC-11 the run cap counts searches only ----------
JB=/tmp/agent_chat_b.jar; rm -f $JB
curl -s -c $JB $BASE/agent -o /tmp/ac_11_0.html
python3 - "$INST/dec_cap.json" "$CAT1" <<'PY'
import json, sys
ask = {"action": "ask", "message": "ご予算はどのくらいですか？", "brief": {}}
search = {"action": "search", "message": "承知しました。お探しします。",
          "brief": {"budget_yen": 100000, "category": sys.argv[2],
                    "must_have": None, "nice_to_have": None, "preset": None}}
seq = []
for _ in range(10):
    seq.append(ask)
    seq.append(search)
seq.append(search)
json.dump({"sequence": seq}, open(sys.argv[1], "w"), ensure_ascii=False)
PY
control $INST/dec_cap.json
PAGE=/tmp/ac_11_0.html
for i in $(seq 1 21); do
    post_msg $JB $PAGE "探して" /tmp/ac_11_$i.html
    PAGE=/tmp/ac_11_$i.html
done
RESULTS10=$(grep -o "chat-result" /tmp/ac_11_20.html | wc -l)
grep -q "エージェントの実行回数の上限" /tmp/ac_11_21.html \
    && [ "$RESULTS10" = "10" ] \
    && ok "AC-11 ten searches ran (asks free), the eleventh was refused" \
    || ng "AC-11 run cap (result blocks after 20 posts: $RESULTS10)"

# ---------- AC-14 confirmation after a question forces the search ----------
JC=/tmp/agent_chat_c.jar; rm -f $JC
curl -s -c $JC $BASE/agent -o /tmp/ac_14_0.html
cat > $INST/dec_confirm.json <<'EOF'
{"sequence": [
  {"action": "ask", "message": "安くて評価の高いものをお探しですか？", "brief": {"preset": "best_rated"}},
  {"action": "reply", "message": "かしこまりました。", "brief": {}}
]}
EOF
control $INST/dec_confirm.json
post_msg $JC /tmp/ac_14_0.html "安くて評価の高いものを教えてください" /tmp/ac_14_1.html
M14=$(db "SELECT COALESCE(MAX(id), 0) FROM cart_items")
post_msg $JC /tmp/ac_14_1.html "はい、そういうものほしいです。" /tmp/ac_14_2.html
NONE14=$(db "SELECT COUNT(*) FROM cart_items WHERE id > $M14 AND added_by_agent")
grep -q "chat-result" /tmp/ac_14_2.html && [ "$NONE14" = "0" ] \
    && ok "AC-14 a はい after the question runs the search the model only acknowledged (proposal only)" \
    || ng "AC-14 confirmation rule (agent rows $NONE14)"
apply_all $JC /tmp/ac_14_2.html /tmp/ac_14_3.html
SESS14=$(db "SELECT DISTINCT session_id FROM cart_items WHERE id > $M14 AND added_by_agent LIMIT 1")
LINES14=$(agent_lines "$SESS14")
[ -n "$SESS14" ] && [ "$LINES14" -ge 1 ] \
    && ok "AC-14 apply-all lands the confirmed search's picks" \
    || ng "AC-14 apply-all (lines $LINES14)"

# ---------- AC-15 a promised search is rescued into a real one ----------
JD=/tmp/agent_chat_d.jar; rm -f $JD
curl -s -c $JD $BASE/agent -o /tmp/ac_15_0.html
cat > $INST/dec_promise.json <<'EOF'
{"sequence": [{"action": "reply", "message": "軽くて評価の高いものを探しています。", "brief": {"preset": "cheapest"}}]}
EOF
control $INST/dec_promise.json
M15=$(db "SELECT COALESCE(MAX(id), 0) FROM cart_items")
post_msg $JD /tmp/ac_15_0.html "安いものを探して" /tmp/ac_15.html
NONE15=$(db "SELECT COUNT(*) FROM cart_items WHERE id > $M15 AND added_by_agent")
grep -q "chat-result" /tmp/ac_15.html && [ "$NONE15" = "0" ] \
    && ok "AC-15 the promise message with a preset signal runs the search anyway (proposal only)" \
    || ng "AC-15 promise rescue (agent rows $NONE15)"
apply_all $JD /tmp/ac_15.html /tmp/ac_15b.html
SESS15=$(db "SELECT DISTINCT session_id FROM cart_items WHERE id > $M15 AND added_by_agent LIMIT 1")
LINES15=$(agent_lines "$SESS15")
[ -n "$SESS15" ] && [ "$LINES15" -ge 1 ] \
    && ok "AC-15 apply-all lands the rescued search's picks" \
    || ng "AC-15 apply-all (lines $LINES15)"

# ---------- AC-16 a promise with no signal stays a reply ----------
JE=/tmp/agent_chat_e.jar; rm -f $JE
curl -s -c $JE $BASE/agent -o /tmp/ac_16_0.html
cat > $INST/dec_promise_bare.json <<'EOF'
{"sequence": [{"action": "reply", "message": "商品を探しています。", "brief": {}}]}
EOF
control $INST/dec_promise_bare.json
M16=$(db "SELECT COALESCE(MAX(id), 0) FROM cart_items")
post_msg $JE /tmp/ac_16_0.html "何かいいものある？" /tmp/ac_16.html
NEW16=$(db "SELECT COUNT(*) FROM cart_items WHERE id > $M16")
grep -q "商品を探しています" /tmp/ac_16.html && ! grep -q "chat-result" /tmp/ac_16.html \
    && [ "$NEW16" = "0" ] \
    && ok "AC-16 the promise with no signal in the brief stays a plain reply" \
    || ng "AC-16 bare promise (new rows $NEW16)"

# ---------- AC-17 follow-up words re-search; thanks never does ----------
IDS15=$(line_ids "$SESS15")
cat > $INST/dec_follow.json <<'EOF'
{"sequence": [{"action": "ask", "message": "他に条件はありますか？", "brief": {}}]}
EOF
control $INST/dec_follow.json
post_msg $JD /tmp/ac_15.html "もっと安いのありますか？" /tmp/ac_17_1.html
IDS17_PRE=$(line_ids "$SESS15")
RESULTS17=$(grep -o "chat-result" /tmp/ac_17_1.html | wc -l)
[ "$RESULTS17" -ge 2 ] && [ "$IDS17_PRE" = "$IDS15" ] \
    && ok "AC-17 もっと安い re-searched; the search alone replaced nothing" \
    || ng "AC-17 follow-up search-alone (results $RESULTS17)"
apply_all $JD /tmp/ac_17_1.html /tmp/ac_17_1b.html
IDS17=$(line_ids "$SESS15")
[ -n "$IDS17" ] && [ "$IDS17" != "$IDS15" ] \
    && ok "AC-17 apply-all of the follow-up proposal replaced the agent lines" \
    || ng "AC-17 follow-up apply (ids '$IDS17')"
cat > $INST/dec_thanks.json <<'EOF'
{"sequence": [{"action": "search", "message": "お探しします。", "brief": {"preset": "cheapest"}}]}
EOF
control $INST/dec_thanks.json
post_msg $JD /tmp/ac_17_1.html "ありがとう" /tmp/ac_17_2.html
IDS17B=$(line_ids "$SESS15")
RESULTS17B=$(grep -o "chat-result" /tmp/ac_17_2.html | wc -l)
grep -q "またお探しのものがあれば" /tmp/ac_17_2.html \
    && [ "$RESULTS17B" = "$RESULTS17" ] && [ "$IDS17B" = "$IDS17" ] \
    && ok "AC-17 thanks closes politely: no search, cart untouched" \
    || ng "AC-17 closing rule (results $RESULTS17B)"

# ---------- AC-18 the question budget forces a search ----------
JF=/tmp/agent_chat_f.jar; rm -f $JF
curl -s -c $JF $BASE/agent -o /tmp/ac_18_0.html
cat > $INST/dec_asks.json <<'EOF'
{"sequence": [
  {"action": "ask", "message": "どなたへのプレゼントですか？", "brief": {"preset": "cheapest"}},
  {"action": "ask", "message": "ご予算はどのくらいですか？", "brief": {}},
  {"action": "ask", "message": "色の好みはありますか？", "brief": {}}
]}
EOF
control $INST/dec_asks.json
post_msg $JF /tmp/ac_18_0.html "プレゼントを探しています" /tmp/ac_18_1.html
post_msg $JF /tmp/ac_18_1.html "友達にあげます" /tmp/ac_18_2.html
M18=$(db "SELECT COALESCE(MAX(id), 0) FROM cart_items")
post_msg $JF /tmp/ac_18_2.html "まだ決めていません" /tmp/ac_18_3.html
NEW18=$(db "SELECT COUNT(*) FROM cart_items WHERE id > $M18 AND added_by_agent")
grep -q "chat-result" /tmp/ac_18_3.html && [ "$NEW18" = "0" ] \
    && ok "AC-18 two questions without a search is the cap: the third turn searches (proposal only)" \
    || ng "AC-18 question budget (agent rows $NEW18)"
apply_all $JF /tmp/ac_18_3.html /tmp/ac_18_4.html
SESS18=$(db "SELECT DISTINCT session_id FROM cart_items WHERE id > $M18 AND added_by_agent LIMIT 1")
[ -n "$SESS18" ] \
    && ok "AC-18 apply-all lands the forced search's picks" \
    || ng "AC-18 apply-all"

# ---------- AC-19 an htmx post answers with a fragment ----------
JG=/tmp/agent_chat_g.jar; rm -f $JG
curl -s -c $JG $BASE/agent -o /tmp/ac_19_0.html
cat > $INST/dec_hx_ask.json <<'EOF'
{"sequence": [{"action": "ask", "message": "ご用途を教えてください。", "brief": {}}]}
EOF
control $INST/dec_hx_ask.json
curl -s -b $JG -c $JG -X POST $BASE/agent/chat -H "HX-Request: true" \
    --data-urlencode "$CT=$(csrf /tmp/ac_19_0.html)" \
    --data-urlencode "message=プレゼントを探しています" -o /tmp/ac_19.html
! grep -q "<html" /tmp/ac_19.html \
    && grep -q "ご用途を教えてください" /tmp/ac_19.html \
    && grep -q "chat-row-user" /tmp/ac_19.html \
    && grep -q 'hx-swap-oob="true"' /tmp/ac_19.html \
    && grep -q 'id="chat-note"' /tmp/ac_19.html \
    && ok "AC-19 htmx post returns the entries fragment plus OOB updates" \
    || ng "AC-19 htmx fragment"

# ---------- AC-20 a plain post still answers the full page ----------
post_msg $JG /tmp/ac_19_0.html "まだ決めていません" /tmp/ac_20.html
grep -q "<html" /tmp/ac_20.html && grep -q "chat-thread" /tmp/ac_20.html \
    && ok "AC-20 plain post returns the full chat page" || ng "AC-20 plain page"

# ---------- AC-21 an htmx search appends result cards ----------
cat > $INST/dec_hx_search.json <<EOF
{"sequence": [{"action": "search", "message": "承知しました。お探しします。", "brief": {"budget_yen": 100000, "category": "$CAT1", "must_have": null, "nice_to_have": null, "preset": null}}]}
EOF
control $INST/dec_hx_search.json
curl -s -b $JG -c $JG -X POST $BASE/agent/chat -H "HX-Request: true" \
    --data-urlencode "$CT=$(csrf /tmp/ac_20.html)" \
    --data-urlencode "message=そのカテゴリーで探して" -o /tmp/ac_21.html
grep -q "chat-result" /tmp/ac_21.html && grep -q "product-card" /tmp/ac_21.html \
    && grep -q "総合スコア" /tmp/ac_21.html && ! grep -q "<html" /tmp/ac_21.html \
    && ok "AC-21 htmx search fragment carries the result cards" \
    || ng "AC-21 htmx search"

# ---------- AC-22 an htmx preset tap answers the chips ----------
curl -s -b $JG -c $JG -X POST $BASE/agent/preset -H "HX-Request: true" \
    --data-urlencode "$CT=$(csrf /tmp/ac_20.html)" \
    --data-urlencode "preset=best_rated" -o /tmp/ac_22.html
! grep -q "<html" /tmp/ac_22.html \
    && grep -q 'class="chat-chip chat-chip-active">評価重視' /tmp/ac_22.html \
    && ok "AC-22 htmx preset returns the chips with the new active preset" \
    || ng "AC-22 htmx preset"

# ---------- AC-12 LLM down: Japanese fallback in the thread ----------
kill $STUBPID 2>/dev/null; sleep 1
CODE=$(curl -s -b $J -c $J -o /tmp/ac_12.html -w "%{http_code}" -X POST $BASE/agent/chat \
    --data-urlencode "$CT=$(csrf /tmp/ac_10.html)" --data-urlencode "message=何かありますか")
[ "$CODE" = "200" ] && grep -q "少し調子が悪いようです" /tmp/ac_12.html \
    && ok "AC-12 unreachable LLM degrades to the Japanese fallback message" \
    || ng "AC-12 LLM down ($CODE)"

# ---------- invariant: the owner's account untouched ----------
[ "$(db "SELECT COUNT(*) FROM cart_items WHERE user_id = 32 AND added_by_agent")" = "0" ] \
    && [ "$(db "SELECT COUNT(*) FROM orders WHERE user_id = 32")" = "14" ] \
    && ok "AC-13 invariant holds (user 32: 14 orders, no agent lines)" \
    || ng "AC-13 invariant"

result
