-- Cart-abandonment reminders (Wave 3 C3): one row per
-- member recording the cart fingerprint that was reminded.
--
-- A reminder fires only when the cart's current fingerprint
-- differs from the recorded one, so one cart state gets at
-- most one mail; changing the cart (add/remove) is a new
-- state and may be reminded again after the delay.
CREATE TABLE cart_reminders (
    user_id BIGINT PRIMARY KEY REFERENCES users(id),
    cart_fingerprint TEXT NOT NULL,
    reminded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
