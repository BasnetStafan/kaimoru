-- Review moderation (Wave 2 B6): a reversible hide and a
-- shop reply, alongside the existing hard delete.
--
-- hidden_at NULL = visible. The storefront's review reads
-- (list, aggregates, distribution, card ratings) exclude
-- hidden rows; the member's own review management and the
-- admin moderation list still see them. shop_reply is the
-- shop's public answer rendered under the review; empty
-- means no reply. shop_reply_at stamps the latest edit.
ALTER TABLE reviews
    ADD COLUMN hidden_at TIMESTAMPTZ,
    ADD COLUMN shop_reply TEXT NOT NULL DEFAULT '',
    ADD COLUMN shop_reply_at TIMESTAMPTZ;
