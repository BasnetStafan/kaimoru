-- The agent strip joins the CMS sections: it used to be a
-- fixed block anchored after new arrivals with hard-coded
-- copy. Its row starts visible at sort 25, its historical
-- position between new arrivals (20) and ranking (30); the
-- heading and body fall back to the built-in defaults while
-- the overrides are empty.
INSERT INTO home_sections (section_key, heading, description, is_visible, sort_order, updated_at)
VALUES ('agent', '', '', true, 25, now())
ON CONFLICT (section_key) DO NOTHING;
