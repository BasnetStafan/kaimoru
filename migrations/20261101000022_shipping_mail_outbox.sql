-- Shipping-mail outbox: bulk ship paths (bulk status change,
-- tracking-number CSV import) enqueue one row per order inside
-- the transition's own transaction; a background worker claims
-- rows atomically and sends through the same stamp-claimed
-- send the single-order path uses, so a bulk shipment never
-- sends synchronously from the request and never sends twice.
-- One row per order ever: the order's shipped_mail_sent_at
-- stamp is the delivery record; this table is only the queue.
CREATE TABLE shipping_mail_outbox (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    order_id BIGINT NOT NULL UNIQUE REFERENCES orders(id),
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'sending', 'sent', 'failed')),
    attempts INT NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at TIMESTAMPTZ,
    sent_at TIMESTAMPTZ
);

CREATE INDEX shipping_mail_outbox_status_idx
    ON shipping_mail_outbox (status, id);
