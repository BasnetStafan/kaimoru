-- Notification (transactional) mail templates: admin overrides
-- over the code defaults in models::notification_template.
-- A mail with no row renders from its code default, so this
-- table starts empty and 初期値に戻す is a DELETE. The defaults
-- themselves live in code and can never be edited or lost.
CREATE TABLE notification_templates (
    mail_key text PRIMARY KEY,
    subject text NOT NULL,
    body text NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- The shop's mail signature, edited on the shop settings
-- screen with the other identity fields. Empty means the
-- {signature} tag falls back to the shop name, which is what
-- the built-in sign-offs have always printed.
ALTER TABLE shop_settings ADD COLUMN mail_signature text NOT NULL DEFAULT '';
