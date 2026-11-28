#!/bin/bash
# RFM suite (EC V2 item 3, docs/specs/RFM_SPEC.md).
# Runs against the shared dev app. Expectations are computed
# INDEPENDENTLY of the app: raw measures come from a SQL
# aggregate written for this suite, bands from a small
# Python implementation of the spec's quintile rule
# (band = ceil(5k/n), ties share a band). The suite then
# pins three surfaces to those numbers: the RFM page
# (population, leaderboard bands, segment summary), the
# segment matcher (saved band conditions count the same
# members the bands say), and the customers list's RFM
# column. Segments it creates are deleted at the end.
source "$(dirname "$0")/lib.sh"

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
BASE=http://127.0.0.1:3000
JA=/tmp/rfm_admin.jar; JG=/tmp/rfm_guest.jar
APW=$(printf 'Pass%s' 'word1234')
SUFFIX=$$

admin_login() {
  rm -f "$1"
  curl -s -c "$1" $BASE/admin/login -o /tmp/rfm_login.html
  local C=$(csrf /tmp/rfm_login.html)
  curl -s -b "$1" -c "$1" -o /dev/null -X POST $BASE/admin/login \
    --data-urlencode "email=admin@example.com" --data-urlencode "password=$APW" --data-urlencode "csrf_token=$C"
}

# --- independent expectations -------------------------------------------
db "SELECT o.user_id, ((now() AT TIME ZONE 'Asia/Tokyo')::date - (MAX(o.created_at) AT TIME ZONE 'Asia/Tokyo')::date), COUNT(*), COALESCE(SUM(GREATEST(o.total_yen - COALESCE(p.refunded, 0), 0)), 0) FROM orders o JOIN users u ON u.id = o.user_id AND u.withdrawn_at IS NULL LEFT JOIN (SELECT order_id, SUM(amount_refunded_yen) AS refunded FROM payments GROUP BY order_id) p ON p.order_id = o.id WHERE o.user_id IS NOT NULL AND o.status <> 'cancelled' GROUP BY o.user_id" > /tmp/rfm_measures.csv
EXP=$(python3 - <<'PYEOF'
import math
rows = []
for line in open("/tmp/rfm_measures.csv"):
    parts = line.strip().split("|")
    if len(parts) == 4:
        rows.append(tuple(int(x) for x in parts))
n = len(rows)
def band(k):
    return (5 * k + n - 1) // n if n else 0
scored = []
for uid, r_days, f, m in rows:
    rb = band(sum(1 for r in rows if r[1] >= r_days))
    fb = band(sum(1 for r in rows if r[2] <= f))
    mb = band(sum(1 for r in rows if r[3] <= m))
    scored.append((uid, r_days, f, m, rb, fb, mb))
top = max(scored, key=lambda s: (s[3], -s[0])) if scored else (0, 0, 0, 0, 0, 0, 0)
print(f"POP={n}")
print(f"TOP_ID={top[0]} TOP_RB={top[4]} TOP_FB={top[5]} TOP_MB={top[6]}")
print(f"ALL4={sum(1 for s in scored if s[4] >= 4 and s[5] >= 4 and s[6] >= 4)}")
print(f"R5={sum(1 for s in scored if s[4] == 5)}")
PYEOF
)
eval "$EXP"
TOP_BANDS="$TOP_RB / $TOP_FB / $TOP_MB"
TOP_LABEL="R$TOP_RB F$TOP_FB M$TOP_MB"
TOP_EMAIL=$(pii_dec "$(db "SELECT email FROM users WHERE id = $TOP_ID")")

echo "-- surface exclusion"
CODE=$(curl -s -c $JG -o /dev/null -w "%{http_code}" $BASE/admin/rfm)
[ "$CODE" = "303" ] || [ "$CODE" = "302" ] && ok "guest is redirected from /admin/rfm ($CODE)" || ng "guest is redirected from /admin/rfm ($CODE)"

admin_login $JA
curl -s -b $JA $BASE/admin/rfm -o /tmp/rfm_page.html
grep -q "RFM分析" /tmp/rfm_page.html && ok "RFM page renders for the admin" || ng "RFM page renders for the admin"
grep -q "対象会員：${POP}人" /tmp/rfm_page.html && ok "population matches the independent count ($POP)" || ng "population matches the independent count ($POP)"

LEAD_BANDS=$(sed -n '/購入金額ランキング/,/会員別スコア/p' /tmp/rfm_page.html | grep -oE "[0-9] / [0-9] / [0-9]" | head -1)
[ "$LEAD_BANDS" = "$TOP_BANDS" ] && ok "leaderboard top bands match ($TOP_BANDS)" || ng "leaderboard top bands match (got '$LEAD_BANDS', want '$TOP_BANDS')"

SUM=$(sed -n '/セグメント別会員数/,/購入金額ランキング/p' /tmp/rfm_page.html | grep -oE "[0-9]+人" | grep -oE "[0-9]+" | awk '{s+=$1} END {print s+0}')
[ "$SUM" = "$POP" ] && ok "named-segment counts sum to the population" || ng "named-segment counts sum to the population (got $SUM, want $POP)"

echo "-- segment band conditions"
curl -s -b $JA $BASE/admin/segments -o /tmp/rfm_seg.html
C=$(csrf /tmp/rfm_seg.html)
SEG4="RFMテスト全4以上$SUFFIX"
curl -s -b $JA -c $JA -o /dev/null -w "" -X POST $BASE/admin/segments \
  --data-urlencode "name=$SEG4" --data-urlencode "tag=" \
  --data-urlencode "min_total_spend_yen=" --data-urlencode "min_order_count=" \
  --data-urlencode "last_order_before=" --data-urlencode "last_order_after=" \
  --data-urlencode "min_r_score=4" --data-urlencode "min_f_score=4" --data-urlencode "min_m_score=4" \
  --data-urlencode "csrf_token=$C"
curl -s -b $JA $BASE/admin/segments -o /tmp/rfm_seg.html
ROW=$(grep -A 3 "$SEG4" /tmp/rfm_seg.html | head -4)
echo "$ROW" | grep -q "R 4以上" && ok "conditions label shows the bands" || ng "conditions label shows the bands"
GOT4=$(echo "$ROW" | grep -oE "[0-9]+人" | head -1 | grep -oE "[0-9]+")
[ "$GOT4" = "$ALL4" ] && ok "R/F/M>=4 segment counts the banded members ($ALL4)" || ng "R/F/M>=4 segment counts the banded members (got $GOT4, want $ALL4)"

C=$(csrf /tmp/rfm_seg.html)
SEGR5="RFMテストR5$SUFFIX"
curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/segments \
  --data-urlencode "name=$SEGR5" --data-urlencode "tag=" \
  --data-urlencode "min_total_spend_yen=" --data-urlencode "min_order_count=" \
  --data-urlencode "last_order_before=" --data-urlencode "last_order_after=" \
  --data-urlencode "min_r_score=5" --data-urlencode "min_f_score=" --data-urlencode "min_m_score=" \
  --data-urlencode "csrf_token=$C"
curl -s -b $JA $BASE/admin/segments -o /tmp/rfm_seg.html
GOTR5=$(grep -A 3 "$SEGR5" /tmp/rfm_seg.html | grep -oE "[0-9]+人" | head -1 | grep -oE "[0-9]+")
[ "$GOTR5" = "$R5" ] && ok "R=5 segment counts the banded members ($R5)" || ng "R=5 segment counts the banded members (got $GOTR5, want $R5)"

C=$(csrf /tmp/rfm_seg.html)
curl -s -b $JA -c $JA -X POST $BASE/admin/segments -o /tmp/rfm_seg_bad.html \
  --data-urlencode "name=RFMテスト不正$SUFFIX" --data-urlencode "tag=" \
  --data-urlencode "min_total_spend_yen=" --data-urlencode "min_order_count=" \
  --data-urlencode "last_order_before=" --data-urlencode "last_order_after=" \
  --data-urlencode "min_r_score=9" --data-urlencode "min_f_score=" --data-urlencode "min_m_score=" \
  --data-urlencode "csrf_token=$C"
grep -q "RFMスコアは1から5の整数で入力してください" /tmp/rfm_seg_bad.html && ok "band 9 is refused with the validation message" || ng "band 9 is refused with the validation message"
[ "$(db "SELECT COUNT(*) FROM customer_segments WHERE name = 'RFMテスト不正$SUFFIX'")" = "0" ] && ok "the refused segment was not saved" || ng "the refused segment was not saved"

echo "-- customers list column"
Q=$(python3 -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1]))" "$TOP_EMAIL")
curl -s -b $JA "$BASE/admin/customers?q=$Q" -o /tmp/rfm_cust.html
grep -q "<th>RFM</th>" /tmp/rfm_cust.html && ok "customers list has the RFM column" || ng "customers list has the RFM column"
[ -n "$TOP_LABEL" ] && grep -q "$TOP_LABEL" /tmp/rfm_cust.html && ok "top member list label matches their bands ($TOP_LABEL)" || ng "top member list label matches their bands ($TOP_LABEL)"

echo "-- cleanup"
curl -s -b $JA $BASE/admin/segments -o /tmp/rfm_seg.html
C=$(csrf /tmp/rfm_seg.html)
for name in "$SEG4" "$SEGR5"; do
  SID=$(db "SELECT id FROM customer_segments WHERE name = '$name'")
  [ -n "$SID" ] && curl -s -b $JA -c $JA -o /dev/null -X POST $BASE/admin/segments/$SID/delete --data-urlencode "csrf_token=$C"
done
CNT=$(db "SELECT COUNT(*) FROM customer_segments WHERE name LIKE 'RFMテスト%'")
[ "$CNT" = "0" ] && ok "suite segments deleted" || ng "suite segments deleted"

result
