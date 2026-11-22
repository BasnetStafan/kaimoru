#!/bin/bash
# Guest-cart cleanup sweep suite (EC V2 ride-along).
# Seeds four carts straight into the database — an old guest
# cart, a mixed-age guest cart, a fresh guest cart and an
# old MEMBER cart — and waits for the running app's sweep
# (mail-worker interval) to remove only the first. The unit
# under test is the cart: a guest cart with any fresh line
# survives whole, and member carts are never swept.
source "$(dirname "$0")/lib.sh"

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
RUN=$(date +%s)
OLD="sweep-old-$RUN"; MIXED="sweep-mixed-$RUN"; FRESH="sweep-fresh-$RUN"
V1=$(db "SELECT id FROM product_variants WHERE product_id=5 ORDER BY id LIMIT 1")
V2=$(db "SELECT id FROM product_variants WHERE product_id=5 ORDER BY id LIMIT 1 OFFSET 1")
[ -z "$V2" ] && V2=$(db "SELECT id FROM product_variants WHERE product_id=4 ORDER BY id LIMIT 1")
M_UID=$(db "SELECT id FROM users WHERE email_bidx='$(bidx test@example.com)'")
MV=$(db "SELECT v.id FROM product_variants v WHERE NOT EXISTS (SELECT 1 FROM cart_items c WHERE c.user_id=$M_UID AND c.variant_id=v.id) ORDER BY v.id LIMIT 1")

seed() { # session variant age_days
    db "INSERT INTO cart_items (session_id, variant_id, quantity, created_at, updated_at) VALUES ('$1', $2, 1, now() - interval '$3 days', now() - interval '$3 days')" >/dev/null
}
seed "$OLD" "$V1" 40
seed "$OLD" "$V2" 40
seed "$MIXED" "$V1" 40
seed "$MIXED" "$V2" 0
seed "$FRESH" "$V1" 0
db "INSERT INTO cart_items (user_id, variant_id, quantity, created_at, updated_at) VALUES ($M_UID, $MV, 1, now() - interval '40 days', now() - interval '40 days')" >/dev/null

count() { db "SELECT count(*) FROM cart_items WHERE $1"; }

GONE=0
for i in $(seq 1 20); do
    [ "$(count "session_id='$OLD'")" = "0" ] && { GONE=1; break; }
    sleep 5
done
[ "$GONE" = "1" ] && ok "S-01 sweep removes a guest cart untouched past the retention" || ng "S-01 old guest cart"
[ "$(count "session_id='$MIXED'")" = "2" ] && ok "S-02 a guest cart with a fresh line survives whole" || ng "S-02 mixed cart ($(count "session_id='$MIXED'"))"
[ "$(count "session_id='$FRESH'")" = "1" ] && ok "S-03 a fresh guest cart survives" || ng "S-03 fresh cart"
[ "$(count "user_id=$M_UID AND variant_id=$MV")" = "1" ] && ok "S-04 an old member cart is never swept" || ng "S-04 member cart"

db "DELETE FROM cart_items WHERE session_id IN ('$OLD','$MIXED','$FRESH')" >/dev/null
db "DELETE FROM cart_items WHERE user_id=$M_UID AND variant_id=$MV" >/dev/null

result
