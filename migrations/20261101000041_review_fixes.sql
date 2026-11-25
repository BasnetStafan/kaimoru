-- Review fix programme, schema half: compensation marker,
-- admin TOTP tables, indexes and CHECK constraints.

-- A full refund's compensation (restock + points
-- reversal) becomes re-drivable. The marker is set in the
-- transaction that applies the refund and cleared by the
-- compensation itself; the payment sweep retries whatever
-- still stands.
ALTER TABLE payments
    ADD COLUMN compensation_pending BOOLEAN NOT NULL DEFAULT FALSE;

-- Indexes for the lookups the V2 code actually runs.
-- The points grant/reversal path filters the ledger by
-- order on every shipment and refund.
CREATE INDEX point_ledger_order_idx ON point_ledger (order_id);
-- The admin scenario list aggregates the send log per
-- scenario; member deletion cascades through enrollments.
CREATE INDEX mail_scenario_sends_scenario_idx
    ON mail_scenario_sends (scenario_id);
CREATE INDEX mail_scenario_sends_enrollment_idx
    ON mail_scenario_sends (enrollment_id);
-- Consent history is read (and deleted) per member.
CREATE INDEX email_consent_logs_user_idx
    ON email_consent_logs (user_id);
-- Tag-first lookups (per-tag counts) cannot use the
-- (product_id, tag_id) primary key.
CREATE INDEX product_tags_tag_idx ON product_tags (tag_id);

-- CHECK constraints on the step-mail state machines,
-- matching the literals the code writes (verified against
-- the live rows before adding them).
ALTER TABLE mail_scenarios
    ADD CONSTRAINT mail_scenarios_trigger_kind_check
    CHECK (trigger_kind IN ('signup', 'order_delivered'));
ALTER TABLE mail_scenario_steps
    ADD CONSTRAINT mail_scenario_steps_delay_check
    CHECK (delay_days >= 0);
ALTER TABLE mail_scenario_enrollments
    ADD CONSTRAINT mail_scenario_enrollments_status_check
    CHECK (status IN ('active', 'completed', 'stopped', 'failed'));
ALTER TABLE mail_scenario_sends
    ADD CONSTRAINT mail_scenario_sends_status_check
    CHECK (status IN ('sent', 'skipped_consent', 'failed'));

-- Administrator TOTP, mirroring the member shape
-- (users.totp_secret / totp_enabled_at / totp_last_step
-- and the user_recovery_codes table).
ALTER TABLE admins ADD COLUMN totp_secret TEXT;
ALTER TABLE admins ADD COLUMN totp_enabled_at TIMESTAMPTZ;
ALTER TABLE admins ADD COLUMN totp_last_step BIGINT;

CREATE TABLE admin_recovery_codes (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    admin_id BIGINT NOT NULL REFERENCES admins (id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX admin_recovery_codes_admin_idx
    ON admin_recovery_codes (admin_id);
