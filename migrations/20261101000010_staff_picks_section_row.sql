-- The staff picks section row: the section joined the
-- canonical set in code (2026-11-08 pins build) without a
-- stored row, so its sort position floated with the
-- canonical default — and drifted when the agent section
-- was inserted ahead of it. Pin the row at sort 50, its
-- intended place ( level with notices; the canonical
-- tie-break keeps staff picks above the notices).
INSERT INTO home_sections (section_key, heading, description, is_visible, sort_order, updated_at)
VALUES ('staff_picks', '', '', true, 50, now())
ON CONFLICT (section_key) DO NOTHING;
