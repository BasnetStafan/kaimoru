-- Returns workflow (Wave 3 C4): a light status flow behind
-- the returns inquiry intake.
--
-- A return request belongs to one order, optionally born
-- from a `returns` inquiry. Its status walks
-- requested -> approved -> awaiting_return -> inspected ->
-- refunded, or rejected. `refunded` records the WORKFLOW
-- outcome only: the money itself moves exclusively through
-- the existing order refund action — there is deliberately
-- no second refund path here.
CREATE TABLE return_requests (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    order_id BIGINT NOT NULL REFERENCES orders(id),
    inquiry_id BIGINT REFERENCES inquiries(id),
    reason TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'requested'
        CHECK (status IN ('requested', 'approved', 'awaiting_return',
                          'inspected', 'refunded', 'rejected')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX return_requests_order_idx ON return_requests (order_id);
