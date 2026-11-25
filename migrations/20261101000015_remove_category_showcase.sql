-- Category showcase removed ("We don't need カテゴリーピックアップ"). The section row
-- and its category pins table go away; the code that
-- rendered and edited them is removed in the same change.

DELETE FROM home_sections WHERE section_key = 'category_showcase';

DROP TABLE home_category_pins;
