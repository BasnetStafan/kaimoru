-- Audit columns: every mutable
-- table carries updated_at, maintained by the shared
-- set_updated_at() trigger so no UPDATE path can forget to
-- bump it (the manual `updated_at = now()` writes in the
-- repositories remain as documentation but the trigger is
-- the guarantee). Admin-edited tables additionally carry
-- updated_by (the admin's id, written explicitly by the
-- admin save paths only; NULL means the last change was not
-- an admin edit). coupons and tags also gain the created_at
-- they never had.
--
-- Deliberately excluded: append-only records (admin_audit_logs,
-- stock_adjustments, coupon_redemptions, email_consent_logs,
-- content_revisions, webhook_events), ephemeral token tables
-- (password_resets, pending_registrations, user_recovery_codes,
-- withdrawal_codes), pure join rows (product_tags,
-- product_variant_option_links), immutable order children
-- (order_items, order_tax_breakdowns, order_shipment_items),
-- insert/delete-only rows (favorites, review_helpful_votes,
-- inquiry_replies, cart_reminders), the postal_codes reference
-- table, and the session store.
--
-- Rule for future tables: a new mutable table adds
-- updated_at + this trigger in its creating migration.

CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$;

ALTER TABLE coupons ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE tags ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE admins ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE coupons ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE csv_import_batches ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE customer_tags ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE email_changes ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE email_deliveries ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE email_jobs ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE email_track_links ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE home_hero_images ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE home_section_pins ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE low_stock_alerts ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE menu_items ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE ops_job_runs ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE order_shipments ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE orders ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE product_images ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE product_variant_axes ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE product_variant_options ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE restock_subscriptions ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE shipping_mail_outbox ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE shop_holidays ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE tags ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE tax_classes ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE tax_rates ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE user_coupons ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE users ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE categories ADD COLUMN updated_by BIGINT;
ALTER TABLE content_pages ADD COLUMN updated_by BIGINT;
ALTER TABLE coupons ADD COLUMN updated_by BIGINT;
ALTER TABLE email_campaigns ADD COLUMN updated_by BIGINT;
ALTER TABLE email_templates ADD COLUMN updated_by BIGINT;
ALTER TABLE home_sections ADD COLUMN updated_by BIGINT;
ALTER TABLE home_slots ADD COLUMN updated_by BIGINT;
ALTER TABLE menu_items ADD COLUMN updated_by BIGINT;
ALTER TABLE notices ADD COLUMN updated_by BIGINT;
ALTER TABLE notification_templates ADD COLUMN updated_by BIGINT;
ALTER TABLE product_variants ADD COLUMN updated_by BIGINT;
ALTER TABLE products ADD COLUMN updated_by BIGINT;
ALTER TABLE shop_settings ADD COLUMN updated_by BIGINT;
ALTER TABLE tags ADD COLUMN updated_by BIGINT;
ALTER TABLE tax_classes ADD COLUMN updated_by BIGINT;
ALTER TABLE tax_rates ADD COLUMN updated_by BIGINT;
CREATE TRIGGER trg_admins_set_updated_at BEFORE UPDATE ON admins FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_agent_approvals_set_updated_at BEFORE UPDATE ON agent_approvals FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_cart_items_set_updated_at BEFORE UPDATE ON cart_items FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_categories_set_updated_at BEFORE UPDATE ON categories FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_content_pages_set_updated_at BEFORE UPDATE ON content_pages FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_coupons_set_updated_at BEFORE UPDATE ON coupons FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_csv_import_batches_set_updated_at BEFORE UPDATE ON csv_import_batches FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_customer_notes_set_updated_at BEFORE UPDATE ON customer_notes FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_customer_segments_set_updated_at BEFORE UPDATE ON customer_segments FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_customer_tags_set_updated_at BEFORE UPDATE ON customer_tags FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_email_campaigns_set_updated_at BEFORE UPDATE ON email_campaigns FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_email_changes_set_updated_at BEFORE UPDATE ON email_changes FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_email_deliveries_set_updated_at BEFORE UPDATE ON email_deliveries FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_email_jobs_set_updated_at BEFORE UPDATE ON email_jobs FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_email_subscriptions_set_updated_at BEFORE UPDATE ON email_subscriptions FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_email_templates_set_updated_at BEFORE UPDATE ON email_templates FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_email_track_links_set_updated_at BEFORE UPDATE ON email_track_links FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_guest_cart_contacts_set_updated_at BEFORE UPDATE ON guest_cart_contacts FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_home_hero_images_set_updated_at BEFORE UPDATE ON home_hero_images FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_home_section_pins_set_updated_at BEFORE UPDATE ON home_section_pins FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_home_sections_set_updated_at BEFORE UPDATE ON home_sections FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_home_slots_set_updated_at BEFORE UPDATE ON home_slots FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_inquiries_set_updated_at BEFORE UPDATE ON inquiries FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_low_stock_alerts_set_updated_at BEFORE UPDATE ON low_stock_alerts FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_member_addresses_set_updated_at BEFORE UPDATE ON member_addresses FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_menu_items_set_updated_at BEFORE UPDATE ON menu_items FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_notices_set_updated_at BEFORE UPDATE ON notices FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_notification_templates_set_updated_at BEFORE UPDATE ON notification_templates FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_ops_job_runs_set_updated_at BEFORE UPDATE ON ops_job_runs FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_order_shipments_set_updated_at BEFORE UPDATE ON order_shipments FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_orders_set_updated_at BEFORE UPDATE ON orders FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_payments_set_updated_at BEFORE UPDATE ON payments FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_product_images_set_updated_at BEFORE UPDATE ON product_images FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_product_variant_axes_set_updated_at BEFORE UPDATE ON product_variant_axes FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_product_variant_options_set_updated_at BEFORE UPDATE ON product_variant_options FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_product_variants_set_updated_at BEFORE UPDATE ON product_variants FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_products_set_updated_at BEFORE UPDATE ON products FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_restock_subscriptions_set_updated_at BEFORE UPDATE ON restock_subscriptions FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_return_requests_set_updated_at BEFORE UPDATE ON return_requests FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_reviews_set_updated_at BEFORE UPDATE ON reviews FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_shipping_mail_outbox_set_updated_at BEFORE UPDATE ON shipping_mail_outbox FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_shop_holidays_set_updated_at BEFORE UPDATE ON shop_holidays FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_shop_settings_set_updated_at BEFORE UPDATE ON shop_settings FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_tags_set_updated_at BEFORE UPDATE ON tags FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_tax_classes_set_updated_at BEFORE UPDATE ON tax_classes FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_tax_rates_set_updated_at BEFORE UPDATE ON tax_rates FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_user_coupons_set_updated_at BEFORE UPDATE ON user_coupons FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_user_identities_set_updated_at BEFORE UPDATE ON user_identities FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_users_set_updated_at BEFORE UPDATE ON users FOR EACH ROW EXECUTE FUNCTION set_updated_at();
