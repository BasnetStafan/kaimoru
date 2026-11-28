-- Back-in-stock notifications: one row per (variant, email)
-- subscription, registered from the product page while the
-- combination is sold out and mailed once by the restock
-- sweep when its stock returns. The email is PII-encrypted
-- at rest like the order snapshot's, with the blind index
-- carrying the uniqueness the ciphertext cannot.
CREATE TABLE restock_subscriptions (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    variant_id bigint NOT NULL REFERENCES product_variants (id) ON DELETE CASCADE,
    user_id bigint REFERENCES users (id) ON DELETE SET NULL,
    email text NOT NULL,
    email_bidx text NOT NULL,
    status text NOT NULL DEFAULT 'pending',
    created_at timestamptz NOT NULL DEFAULT now(),
    claimed_at timestamptz,
    notified_at timestamptz,
    CONSTRAINT restock_subscriptions_status_check
        CHECK (status IN ('pending', 'sending', 'sent', 'cancelled')),
    CONSTRAINT restock_subscriptions_variant_email_unique
        UNIQUE (variant_id, email_bidx)
);

CREATE INDEX restock_subscriptions_status_idx
    ON restock_subscriptions (status, created_at);
