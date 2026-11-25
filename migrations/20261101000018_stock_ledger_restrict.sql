-- The stock ledger is append-only - the
-- runtime role is already denied UPDATE/DELETE on it - but
-- its variant FK was ON DELETE CASCADE, so deleting a
-- variant silently erased its ledger history. RESTRICT
-- makes a variant with ledger history undeletable, matching
-- how the audit-log FKs already behave; such products are
-- archived instead. (Migration 20261101000017 was recorded
-- empty by a tooling fault and is a no-op; this migration
-- carries the change.)
ALTER TABLE stock_adjustments DROP CONSTRAINT stock_adjustments_variant_id_fkey;
ALTER TABLE stock_adjustments ADD CONSTRAINT stock_adjustments_variant_id_fkey
    FOREIGN KEY (variant_id) REFERENCES product_variants(id) ON DELETE RESTRICT;
