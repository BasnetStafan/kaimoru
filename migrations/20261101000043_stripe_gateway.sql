-- Second payment gateway (2026-11-22; internal payment-gateway
-- design record): the payments table learns the `stripe`
-- provider word and Stripe's own id pair, mirroring the
-- KOMOJU columns so the live KOMOJU path is untouched. The
-- method CHECK is unchanged: Stripe v1 offers the same three
-- hosted methods KOMOJU does. A backup was taken and
-- restore-proven before this migration was written
-- (backup file kept outside the repository).
ALTER TABLE payments DROP CONSTRAINT payments_provider_check;
ALTER TABLE payments ADD CONSTRAINT payments_provider_check
    CHECK ((provider = ANY (ARRAY['komoju'::text, 'stripe'::text, 'cod'::text, 'agent'::text])));
ALTER TABLE payments ADD COLUMN stripe_session_id text;
ALTER TABLE payments ADD COLUMN stripe_payment_id text;
ALTER TABLE payments ADD CONSTRAINT payments_stripe_session_id_key UNIQUE (stripe_session_id);
ALTER TABLE payments ADD CONSTRAINT payments_stripe_payment_id_key UNIQUE (stripe_payment_id);
