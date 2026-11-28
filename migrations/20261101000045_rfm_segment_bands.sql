-- RFM band conditions for customer segments (RFM spec section 6):
-- a segment may require a minimum R, F and/or M band (1-5, the
-- quintile scores the RFM service computes live). NULL means the
-- band does not filter, exactly like the existing conditions.
-- A backup was taken and restore-proven before this migration
-- was written (~/backups/agentic-commerce/agentic_commerce-20261127-235023.sql).
ALTER TABLE customer_segments
    ADD COLUMN min_r_score INTEGER,
    ADD COLUMN min_f_score INTEGER,
    ADD COLUMN min_m_score INTEGER,
    ADD CONSTRAINT customer_segments_rfm_bands_range CHECK (
        (min_r_score IS NULL OR min_r_score BETWEEN 1 AND 5)
        AND (min_f_score IS NULL OR min_f_score BETWEEN 1 AND 5)
        AND (min_m_score IS NULL OR min_m_score BETWEEN 1 AND 5)
    );
