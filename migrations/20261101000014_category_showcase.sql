-- Category showcase section: a
-- new top page section whose content is the operator's pinned
-- categories. Each pinned category renders as a product block
-- with the category config count of its newest products and,
-- when the category holds more than that, a view-all link to
-- its all-products page. The pins live in their own table —
-- home_section_pins pins products, these pin categories.

CREATE TABLE home_category_pins (
    category_id bigint PRIMARY KEY REFERENCES categories(id) ON DELETE CASCADE,
    sort_order integer NOT NULL
);

INSERT INTO home_sections (section_key, heading, description, is_visible, sort_order, updated_at)
VALUES ('category_showcase', '', '', true, 55, now());
