-- Hero images: the hero slot's single image_path becomes an
-- ordered list of up to ten images in its own table, the
-- same shape as home_section_pins. The current image (the
-- dev store carries none) moves in as the first image. The
-- home_slots.image_path column stays, retired: code stops
-- reading and writing it with this migration's release.
CREATE TABLE home_hero_images (
    slot_key text NOT NULL,
    image_path text NOT NULL,
    sort_order integer NOT NULL,
    PRIMARY KEY (slot_key, image_path)
);
CREATE INDEX home_hero_images_order_idx
    ON home_hero_images (slot_key, sort_order);

INSERT INTO home_hero_images (slot_key, image_path, sort_order)
SELECT key, image_path, 10
FROM home_slots
WHERE key = 'hero' AND image_path <> '';
