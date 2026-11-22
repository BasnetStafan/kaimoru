-- Low-stock alert record: one row per product per JST day
-- the operator was mailed about it. The alert worker checks
-- this table before mailing, so a product sitting under its
-- threshold for a week produces one mail a day, not one per
-- pass, and two worker processes can never mail twice for
-- the same day (the primary key is the claim).
CREATE TABLE low_stock_alerts (
    product_id BIGINT NOT NULL REFERENCES products(id),
    alert_date DATE NOT NULL,
    PRIMARY KEY (product_id, alert_date)
);
