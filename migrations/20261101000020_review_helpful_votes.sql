-- Review helpful votes: one row per (review, voter). The
-- voter key is 'u:<user id>' for members and 's:<guest cart
-- owner token>' for guests -- the same identity discipline
-- as cart ownership. The UNIQUE pair is the duplicate
-- protection: the endpoint inserts ON CONFLICT DO NOTHING,
-- so a repeat vote is a quiet no-op that re-reads the
-- count. Deleting a review takes its votes with it.
CREATE TABLE review_helpful_votes (
    id BIGSERIAL PRIMARY KEY,
    review_id BIGINT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
    voter_key TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (review_id, voter_key)
);

CREATE INDEX idx_review_helpful_votes_review
    ON review_helpful_votes (review_id);
