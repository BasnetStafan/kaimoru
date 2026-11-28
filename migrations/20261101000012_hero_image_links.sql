-- Each hero slide carries its own link: the storefront
-- overlays the hero copy on the slides and a click follows
-- the slide's link (there is no hero button any more). An
-- empty link leaves the slide unlinked.
ALTER TABLE home_hero_images ADD COLUMN link_url text NOT NULL DEFAULT '';
