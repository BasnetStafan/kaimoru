-- Points programme (EC V2 core 1; internal points design record).
-- The ledger is the only record of a member's points: earn
-- rows carry the spendable remainder and the expiry, every
-- other row is a signed movement against them. Orders record
-- what they spent and earned so a cancellation can reverse
-- exactly its own movements. Points settings join the single
-- shop_settings row (operator-tunable, never constants).

ALTER TABLE shop_settings
    ADD COLUMN points_enabled BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE shop_settings
    ADD COLUMN points_earn_percent INTEGER NOT NULL DEFAULT 1;
ALTER TABLE shop_settings
    ADD COLUMN points_expiry_days INTEGER NOT NULL DEFAULT 180;

ALTER TABLE orders
    ADD COLUMN points_used_yen BIGINT NOT NULL DEFAULT 0;
ALTER TABLE orders
    ADD COLUMN points_earned BIGINT NOT NULL DEFAULT 0;

CREATE TABLE point_ledger (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users (id),
    kind TEXT NOT NULL,
    points BIGINT NOT NULL,
    remaining BIGINT,
    order_id BIGINT REFERENCES orders (id),
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX point_ledger_user_grants_idx
    ON point_ledger (user_id, expires_at)
    WHERE kind = 'earn';

CREATE INDEX point_ledger_user_history_idx
    ON point_ledger (user_id, id DESC);

CREATE TABLE point_spend_allocations (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    spend_ledger_id BIGINT NOT NULL REFERENCES point_ledger (id),
    grant_ledger_id BIGINT NOT NULL REFERENCES point_ledger (id),
    points BIGINT NOT NULL,
    reversed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX point_spend_allocations_spend_idx
    ON point_spend_allocations (spend_ledger_id);
