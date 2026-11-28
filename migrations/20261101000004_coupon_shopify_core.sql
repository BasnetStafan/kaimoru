-- Coupon Shopify-core pack (spec: COUPON_SHOPIFY_CORE_SPEC_2026-11-08):
-- value types (fixed yen / percent / free shipping), scope (whole order /
-- one product / one variant), usage caps, automatic discounts, and the
-- redemption ledger that usage counting and reporting read.

ALTER TABLE coupons ADD COLUMN discount_type text NOT NULL DEFAULT 'fixed';
ALTER TABLE coupons ADD COLUMN percent integer;
ALTER TABLE coupons ADD COLUMN scope text NOT NULL DEFAULT 'order';
ALTER TABLE coupons ADD COLUMN product_id bigint REFERENCES products (id);
ALTER TABLE coupons ADD COLUMN variant_id bigint REFERENCES product_variants (id);
ALTER TABLE coupons ADD COLUMN max_uses_total integer;
ALTER TABLE coupons ADD COLUMN max_uses_per_customer integer DEFAULT 1;
ALTER TABLE coupons ADD COLUMN is_automatic boolean NOT NULL DEFAULT false;

ALTER TABLE coupons DROP CONSTRAINT coupons_discount_yen_check;
ALTER TABLE coupons ADD CONSTRAINT coupons_value_shape CHECK (
    (discount_type = 'fixed' AND discount_yen > 0 AND percent IS NULL)
    OR (discount_type = 'percent' AND percent BETWEEN 1 AND 100 AND discount_yen = 0)
    OR (discount_type = 'free_shipping' AND discount_yen = 0 AND percent IS NULL)
);
ALTER TABLE coupons ADD CONSTRAINT coupons_scope_shape CHECK (
    (scope = 'order' AND product_id IS NULL AND variant_id IS NULL)
    OR (scope = 'product' AND product_id IS NOT NULL AND variant_id IS NULL)
    OR (scope = 'variant' AND variant_id IS NOT NULL AND product_id IS NULL)
);
ALTER TABLE coupons ADD CONSTRAINT coupons_caps_shape CHECK (
    (max_uses_total IS NULL OR max_uses_total > 0)
    AND (max_uses_per_customer IS NULL OR max_uses_per_customer > 0)
);
ALTER TABLE coupons ADD CONSTRAINT coupons_shipping_scope CHECK (
    discount_type <> 'free_shipping' OR scope = 'order'
);

ALTER TABLE orders ADD COLUMN shipping_discount_yen bigint NOT NULL DEFAULT 0;

CREATE TABLE coupon_redemptions (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    order_id bigint NOT NULL REFERENCES orders (id),
    user_id bigint,
    code text NOT NULL,
    class text NOT NULL CHECK (class IN ('product', 'order', 'shipping')),
    discount_yen bigint NOT NULL CHECK (discount_yen >= 0),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX coupon_redemptions_code_idx ON coupon_redemptions (code);
CREATE INDEX coupon_redemptions_order_idx ON coupon_redemptions (order_id);
CREATE INDEX coupon_redemptions_user_code_idx ON coupon_redemptions (user_id, code);

-- Backfill the ledger from order history: every pre-pack coupon was a
-- whole-order fixed-yen code, so the class is 'order' throughout.
INSERT INTO coupon_redemptions (order_id, user_id, code, class, discount_yen, created_at)
SELECT id, user_id, coupon_code, 'order', discount_yen, created_at
FROM orders
WHERE coupon_code IS NOT NULL;
