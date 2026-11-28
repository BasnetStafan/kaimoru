-- Top page section pins: the products an operator has
-- pinned to lead one top page section's row. The automatic
-- selection fills the remaining slots (spec:
-- HOME_PINS_AND_STAFF_PICKS_SPEC_2026-11-08). Deleting a
-- product deletes its pins with it.
CREATE TABLE home_section_pins (
    section_key text NOT NULL,
    product_id bigint NOT NULL REFERENCES products (id) ON DELETE CASCADE,
    sort_order integer NOT NULL,
    PRIMARY KEY (section_key, product_id)
);

CREATE INDEX home_section_pins_section_order_idx
    ON home_section_pins (section_key, sort_order);
