-- Guest cart reminder contacts (Wave 3 C3 completion):
-- the email a guest typed at checkout, stored against the
-- guest cart's session owner so the reminder worker can
-- reach an abandoned guest cart.
--
-- The address is PII-encrypted (email column) with a blind
-- index for account matching, exactly like orders. It is
-- retained only while it can still be used: the row's
-- email is cleared when the reminder is sent (the row then
-- remains as a fingerprint tombstone, so the same cart
-- state is never reminded twice), and the row is deleted
-- when the order is placed or the cart disappears.
CREATE TABLE guest_cart_contacts (
    session_id TEXT PRIMARY KEY,
    email TEXT NOT NULL DEFAULT '',
    email_bidx TEXT NOT NULL DEFAULT '',
    cart_fingerprint TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
