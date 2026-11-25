-- Naming alignment (2026-11-08 audit): three stored names
-- described something other than what they hold.
-- (1) coupons.stripe_featured flags the coupon shown in the
--     header band; this shop does not use Stripe (KOMOJU).
ALTER TABLE coupons RENAME COLUMN stripe_featured TO band_featured;
-- (2) notification_templates.mail_key values were the sender
--     functions' names (send_* verbs); a stored key should name
--     the mail. Overrides, if any exist, follow their mail.
UPDATE notification_templates SET mail_key = 'verification_code' WHERE mail_key = 'send_verification_code';
UPDATE notification_templates SET mail_key = 'password_reset_code' WHERE mail_key = 'send_password_reset_code';
UPDATE notification_templates SET mail_key = 'email_change_code' WHERE mail_key = 'send_email_change_code';
UPDATE notification_templates SET mail_key = 'withdrawal_code' WHERE mail_key = 'send_withdrawal_code';
UPDATE notification_templates SET mail_key = 'order_confirmation' WHERE mail_key = 'send_order_confirmation';
UPDATE notification_templates SET mail_key = 'payment_confirmation' WHERE mail_key = 'send_payment_confirmation';
UPDATE notification_templates SET mail_key = 'konbini_instructions' WHERE mail_key = 'send_konbini_instructions';
UPDATE notification_templates SET mail_key = 'shipping_notification' WHERE mail_key = 'send_shipping_notification';
UPDATE notification_templates SET mail_key = 'order_cancellation' WHERE mail_key = 'send_order_cancellation';
UPDATE notification_templates SET mail_key = 'inquiry_acknowledgement' WHERE mail_key = 'send_inquiry_acknowledgement';
UPDATE notification_templates SET mail_key = 'withdrawal_completed' WHERE mail_key = 'send_withdrawal_completed';
-- (3) products.is_featured is the staff-picks flag; the
--     featured home section is pin-driven. The name sent every
--     reader to the wrong section.
ALTER TABLE products RENAME COLUMN is_featured TO is_staff_pick;
