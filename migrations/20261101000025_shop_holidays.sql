-- Shop holidays: the days the shop does not operate. The
-- delivery promise counts the earliest date over non-holiday
-- days only, and a registered holiday is never offered or
-- accepted as a delivery date. Weekends are NOT implied:
-- Japanese EC ships on Saturdays, so only registered days
-- count. An empty table reproduces the pre-feature calendar
-- exactly.
CREATE TABLE shop_holidays (
    holiday_date DATE PRIMARY KEY,
    label TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
