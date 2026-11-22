-- Per-product points earn-rate override (EC V2 core 1
-- extension; internal points design record, override layer).
-- A product may carry its own earn percent: NULL means
-- "the shop rate", 0 means the product earns nothing
-- (points-excluded). Placement snapshots the EFFECTIVE
-- rate onto each order line, so a later product or
-- settings edit never changes what a placed order earns;
-- lines written before this migration keep NULL and fall
-- back to the order-level computation at grant time.

ALTER TABLE products
    ADD COLUMN points_earn_percent INTEGER;

ALTER TABLE order_items
    ADD COLUMN points_earn_percent INTEGER;
