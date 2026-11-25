-- Shipping and delivery settings join the shop_settings row:
-- the fees and the delivery window the code used to hard-code
-- in pricing.rs / delivery.rs become operator-editable, with
-- today's values as the defaults, so behaviour is identical
-- until an operator changes something. Disabled slot/carrier
-- codes are comma-joined catalogue codes; the catalogues
-- themselves stay in code (they are carrier facts, not data).
ALTER TABLE shop_settings
    ADD COLUMN shipping_flat_yen BIGINT NOT NULL DEFAULT 550,
    ADD COLUMN free_shipping_threshold_yen BIGINT NOT NULL DEFAULT 5000,
    ADD COLUMN cod_fee_yen BIGINT NOT NULL DEFAULT 330,
    ADD COLUMN delivery_earliest_offset_days INTEGER NOT NULL DEFAULT 2,
    ADD COLUMN delivery_latest_offset_days INTEGER NOT NULL DEFAULT 14,
    ADD COLUMN disabled_delivery_slots TEXT NOT NULL DEFAULT '',
    ADD COLUMN disabled_carriers TEXT NOT NULL DEFAULT '';
