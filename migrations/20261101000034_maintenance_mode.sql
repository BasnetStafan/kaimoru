-- Maintenance mode. Off by default; the message is
-- the operator's own words for the closed page (empty
-- renders the code default).
ALTER TABLE shop_settings
    ADD COLUMN maintenance_mode BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN maintenance_message TEXT NOT NULL DEFAULT '';
