-- Orders gain a real delivery stamp. The step-mail review-request scan
-- guarded on orders.created_at, which is the wrong event
-- twice over: an order created before the scenario
-- existed but delivered after it was never enrolled, and
-- with no recency bound a re-enabled scenario mailed
-- every historical buyer.
-- Existing rows keep NULL: they predate the stamp and
-- must never enroll retroactively.
ALTER TABLE orders ADD COLUMN delivered_at TIMESTAMPTZ;
