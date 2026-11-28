-- Campaign segment audiences (Wave 3 C2): a campaign may
-- target one saved segment (C1) instead of all subscribers.
--
-- NULL = all subscribers (the 7B default, unchanged).
-- The foreign key deliberately has no ON DELETE action: a
-- segment a campaign points at cannot be deleted, so a
-- stored audience can never silently widen to everyone.
ALTER TABLE email_campaigns
    ADD COLUMN segment_id BIGINT REFERENCES customer_segments(id);
