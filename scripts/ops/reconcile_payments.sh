#!/bin/bash
# Read-only payment reconciliation: the operator's three
# questions about KOMOJU money, answered from the payments
# table. Nothing here writes.
#
#   1. What is still open (and how old is it)?
#   2. What has actually been captured, per method, net of
#      refunds?
#   3. What is flagged for a human (amount mismatches, money
#      that arrived after an order closed)?
db() { docker exec agentic-db psql -U agentic -d agentic_commerce -c "$1"; }

echo "== Open KOMOJU payments, oldest first =="
db "SELECT p.id AS payment_id, p.order_id, p.method, p.status, p.amount_yen, \
           p.created_at AT TIME ZONE 'Asia/Tokyo' AS created_jst, \
           p.payment_deadline AT TIME ZONE 'Asia/Tokyo' AS deadline_jst \
    FROM payments p \
    WHERE p.provider = 'komoju' AND p.status IN ('pending_payment', 'failed') \
    ORDER BY p.created_at"

echo "== Captured totals per method (net of refunds) =="
db "SELECT method, count(*) AS payments, \
           sum(amount_yen - amount_refunded_yen) AS net_captured_yen \
    FROM payments \
    WHERE provider = 'komoju' AND status IN ('paid', 'partially_refunded') \
    GROUP BY method ORDER BY method"

echo "== Flagged for human attention =="
db "SELECT id AS payment_id, order_id, status, flag_note \
    FROM payments WHERE flag_note IS NOT NULL ORDER BY updated_at"
