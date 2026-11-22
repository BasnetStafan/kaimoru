-- Gift options V1: the noshi kind (code + label snapshot,
-- the delivery_slot pattern), the optional name inscription
-- and the message-card text. No fee, no wrapping tiers --
-- paid wrapping is a business decision this migration does
-- not make. The two free-text fields hold personal content
-- and are encrypted at rest like the order's other personal
-- fields (application-side, pii.rs).
ALTER TABLE orders
    ADD COLUMN noshi_code TEXT NOT NULL DEFAULT 'none',
    ADD COLUMN noshi_label TEXT NOT NULL DEFAULT 'なし',
    ADD COLUMN noshi_name TEXT,
    ADD COLUMN gift_message TEXT,
    ADD CONSTRAINT orders_noshi_code_check
        CHECK (noshi_code IN ('none', 'celebration', 'midyear', 'year_end', 'thanks', 'plain'));
