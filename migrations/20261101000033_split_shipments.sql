-- Split shipments. An order may be fulfilled in
-- several parcels; each parcel is one order_shipments row
-- with its own carrier, tracking number and item
-- quantities. Orders without shipment rows keep the exact
-- single-shipment behaviour on orders.carrier /
-- orders.tracking_number. The shipping-mail outbox gains
-- an optional shipment reference so each parcel notifies
-- separately (NULL keeps the order-level meaning).
CREATE TABLE order_shipments (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    order_id BIGINT NOT NULL REFERENCES orders(id),
    carrier TEXT NOT NULL DEFAULT '',
    tracking_number TEXT NOT NULL DEFAULT '',
    shipped_at TIMESTAMPTZ,
    mail_sent_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE order_shipment_items (
    shipment_id BIGINT NOT NULL REFERENCES order_shipments(id),
    order_item_id BIGINT NOT NULL REFERENCES order_items(id),
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    PRIMARY KEY (shipment_id, order_item_id)
);

CREATE INDEX order_shipments_order_idx ON order_shipments (order_id);

ALTER TABLE shipping_mail_outbox
    ADD COLUMN shipment_id BIGINT REFERENCES order_shipments(id);

-- The outbox's one-row-per-order uniqueness becomes scoped:
-- one ORDER-level row per order (shipment_id NULL) plus one
-- row per shipment.
ALTER TABLE shipping_mail_outbox
    DROP CONSTRAINT shipping_mail_outbox_order_id_key;
CREATE UNIQUE INDEX shipping_mail_outbox_order_level_idx
    ON shipping_mail_outbox (order_id) WHERE shipment_id IS NULL;
CREATE UNIQUE INDEX shipping_mail_outbox_shipment_idx
    ON shipping_mail_outbox (shipment_id) WHERE shipment_id IS NOT NULL;
