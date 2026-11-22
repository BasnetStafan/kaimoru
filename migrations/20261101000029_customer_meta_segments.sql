-- Customer operator metadata + saved segments (Wave 3 C1).
--
-- Operator metadata ONLY: a free-text internal memo and
-- free-form tags per member, plus saved segment definitions
-- (conditions evaluated live against orders/tags). Nothing
-- here touches member credentials or profile columns, and
-- there is deliberately no suspend/delete-member capability.
CREATE TABLE customer_notes (
    user_id BIGINT PRIMARY KEY REFERENCES users(id),
    memo TEXT NOT NULL DEFAULT '',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE customer_tags (
    user_id BIGINT NOT NULL REFERENCES users(id),
    tag TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, tag)
);

CREATE TABLE customer_segments (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name TEXT NOT NULL,
    tag TEXT NOT NULL DEFAULT '',
    min_total_spend_yen BIGINT,
    min_order_count INTEGER,
    last_order_before DATE,
    last_order_after DATE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
