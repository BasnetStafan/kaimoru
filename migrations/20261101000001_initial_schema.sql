-- Initial schema (baseline).
--
-- This file replaces the original 57-migration chain
-- (20261101000001..20261101000057) with the single schema that
-- chain produced. The store had never been deployed when the
-- squash was made (2026-11-07), so no applied-migration
-- history outside disposable dev databases could be broken;
-- the old files remain in git history up to the PII
-- encryption commit. New changes are added as new migrations
-- after this baseline, never by editing it.
--
-- The shape was verified statement-for-statement against a
-- fresh install of the old chain (pg_dump --schema-only diff:
-- zero differences).

CREATE SCHEMA IF NOT EXISTS tower_sessions;

CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public;

COMMENT ON EXTENSION pg_trgm IS 'text similarity measurement and index searching based on trigrams';

CREATE FUNCTION public.admin_audit_logs_append_only() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    RAISE EXCEPTION 'admin_audit_logs is append-only';
END;
$$;

CREATE TABLE public.admin_audit_logs (
    id bigint NOT NULL,
    admin_id bigint,
    action text NOT NULL,
    target_type text NOT NULL,
    target_id bigint,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    actor_user_id bigint
);

CREATE SEQUENCE public.admin_audit_logs_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.admin_audit_logs_id_seq OWNED BY public.admin_audit_logs.id;

CREATE TABLE public.admins (
    id bigint NOT NULL,
    email text NOT NULL,
    password_hash text NOT NULL,
    display_name text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    last_login_at timestamp with time zone
);

CREATE SEQUENCE public.admins_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.admins_id_seq OWNED BY public.admins.id;

CREATE TABLE public.agent_approvals (
    id bigint NOT NULL,
    user_id bigint,
    session_id text,
    fingerprint text NOT NULL,
    snapshot jsonb NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    consumed_order_id bigint,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT agent_approvals_check CHECK (((user_id IS NOT NULL) <> (session_id IS NOT NULL))),
    CONSTRAINT agent_approvals_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'consumed'::text, 'voided'::text, 'expired'::text])))
);

ALTER TABLE public.agent_approvals ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.agent_approvals_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.cart_items (
    id bigint NOT NULL,
    user_id bigint,
    session_id text,
    quantity integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    variant_id bigint NOT NULL,
    added_by_agent boolean DEFAULT false NOT NULL,
    CONSTRAINT cart_items_check CHECK (((user_id IS NOT NULL) <> (session_id IS NOT NULL))),
    CONSTRAINT cart_items_quantity_check CHECK ((quantity > 0))
);

CREATE SEQUENCE public.cart_items_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.cart_items_id_seq OWNED BY public.cart_items.id;

CREATE TABLE public.categories (
    id bigint NOT NULL,
    name text NOT NULL,
    slug text NOT NULL,
    description text DEFAULT ''::text NOT NULL,
    parent_id bigint,
    display_order integer DEFAULT 0 NOT NULL,
    is_active boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE public.categories_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.categories_id_seq OWNED BY public.categories.id;

CREATE TABLE public.content_pages (
    id bigint NOT NULL,
    slug text NOT NULL,
    kind text NOT NULL,
    title text NOT NULL,
    body_md text DEFAULT ''::text NOT NULL,
    meta_description text DEFAULT ''::text NOT NULL,
    status text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    format text DEFAULT 'markdown'::text NOT NULL,
    CONSTRAINT content_pages_format_check CHECK ((format = ANY (ARRAY['markdown'::text, 'html'::text]))),
    CONSTRAINT content_pages_kind_check CHECK ((kind = ANY (ARRAY['system'::text, 'custom'::text]))),
    CONSTRAINT content_pages_status_check CHECK ((status = ANY (ARRAY['draft'::text, 'published'::text])))
);

ALTER TABLE public.content_pages ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.content_pages_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.content_revisions (
    id bigint NOT NULL,
    page_id bigint NOT NULL,
    title text NOT NULL,
    body_md text NOT NULL,
    meta_description text DEFAULT ''::text NOT NULL,
    edited_by bigint NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    format text DEFAULT 'markdown'::text NOT NULL,
    CONSTRAINT content_revisions_format_check CHECK ((format = ANY (ARRAY['markdown'::text, 'html'::text])))
);

ALTER TABLE public.content_revisions ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.content_revisions_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.coupons (
    id bigint NOT NULL,
    code text NOT NULL,
    discount_yen bigint NOT NULL,
    min_subtotal_yen bigint DEFAULT 0 NOT NULL,
    active boolean DEFAULT true NOT NULL,
    description text DEFAULT ''::text NOT NULL,
    starts_at timestamp with time zone,
    ends_at timestamp with time zone,
    stripe_featured boolean DEFAULT false NOT NULL,
    CONSTRAINT coupons_discount_yen_check CHECK ((discount_yen > 0))
);

CREATE SEQUENCE public.coupons_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.coupons_id_seq OWNED BY public.coupons.id;

CREATE TABLE public.csv_import_batches (
    id bigint NOT NULL,
    entity text NOT NULL,
    admin_id bigint NOT NULL,
    file_name text NOT NULL,
    raw_text text NOT NULL,
    report text DEFAULT '{}'::text NOT NULL,
    status text DEFAULT 'preview'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    applied_at timestamp with time zone,
    CONSTRAINT csv_import_batches_entity_check CHECK ((entity = ANY (ARRAY['product'::text, 'category'::text, 'tag'::text]))),
    CONSTRAINT csv_import_batches_status_check CHECK ((status = ANY (ARRAY['preview'::text, 'applied'::text, 'discarded'::text])))
);

CREATE SEQUENCE public.csv_import_batches_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.csv_import_batches_id_seq OWNED BY public.csv_import_batches.id;

CREATE TABLE public.email_campaigns (
    id bigint NOT NULL,
    name text NOT NULL,
    template_id bigint,
    subject text NOT NULL,
    body_html text NOT NULL,
    body_text text NOT NULL,
    audience text DEFAULT 'subscribed'::text NOT NULL,
    status text DEFAULT 'draft'::text NOT NULL,
    scheduled_at timestamp with time zone,
    sent_at timestamp with time zone,
    created_by bigint NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT email_campaigns_status_check CHECK ((status = ANY (ARRAY['draft'::text, 'scheduled'::text, 'sending'::text, 'sent'::text])))
);

CREATE SEQUENCE public.email_campaigns_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.email_campaigns_id_seq OWNED BY public.email_campaigns.id;

CREATE TABLE public.email_changes (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    email text NOT NULL,
    code_hash text NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    resend_count integer DEFAULT 0 NOT NULL,
    code_sent_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    verified boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE public.email_changes_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.email_changes_id_seq OWNED BY public.email_changes.id;

CREATE TABLE public.email_consent_logs (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    action text NOT NULL,
    source text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT email_consent_logs_action_check CHECK ((action = ANY (ARRAY['opt_in'::text, 'opt_out'::text])))
);

CREATE SEQUENCE public.email_consent_logs_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.email_consent_logs_id_seq OWNED BY public.email_consent_logs.id;

CREATE TABLE public.email_deliveries (
    id bigint NOT NULL,
    job_id bigint NOT NULL,
    campaign_id bigint NOT NULL,
    user_id bigint NOT NULL,
    email text NOT NULL,
    status text DEFAULT 'queued'::text NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    last_error text,
    sent_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    claimed_at timestamp with time zone,
    CONSTRAINT email_deliveries_status_check CHECK ((status = ANY (ARRAY['queued'::text, 'sending'::text, 'sent'::text, 'failed'::text, 'skipped'::text])))
);

CREATE SEQUENCE public.email_deliveries_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.email_deliveries_id_seq OWNED BY public.email_deliveries.id;

CREATE TABLE public.email_jobs (
    id bigint NOT NULL,
    campaign_id bigint NOT NULL,
    status text DEFAULT 'queued'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    started_at timestamp with time zone,
    finished_at timestamp with time zone,
    CONSTRAINT email_jobs_status_check CHECK ((status = ANY (ARRAY['queued'::text, 'running'::text, 'done'::text, 'failed'::text])))
);

CREATE SEQUENCE public.email_jobs_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.email_jobs_id_seq OWNED BY public.email_jobs.id;

CREATE TABLE public.email_subscriptions (
    user_id bigint NOT NULL,
    status text DEFAULT 'unsubscribed'::text NOT NULL,
    consented_at timestamp with time zone,
    source text,
    unsubscribe_token text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT email_subscriptions_status_check CHECK ((status = ANY (ARRAY['subscribed'::text, 'unsubscribed'::text])))
);

CREATE TABLE public.email_templates (
    id bigint NOT NULL,
    name text NOT NULL,
    subject text NOT NULL,
    body_html text NOT NULL,
    body_text text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE public.email_templates_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.email_templates_id_seq OWNED BY public.email_templates.id;

CREATE TABLE public.favorites (
    user_id bigint NOT NULL,
    product_id bigint NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.home_sections (
    section_key text NOT NULL,
    heading text DEFAULT ''::text NOT NULL,
    is_visible boolean DEFAULT true NOT NULL,
    sort_order integer NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.home_slots (
    key text NOT NULL,
    headline text DEFAULT ''::text NOT NULL,
    body text DEFAULT ''::text NOT NULL,
    cta_label text DEFAULT ''::text NOT NULL,
    cta_url text DEFAULT ''::text NOT NULL,
    is_enabled boolean DEFAULT false NOT NULL,
    starts_at timestamp with time zone,
    ends_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    image_path text DEFAULT ''::text NOT NULL
);

CREATE TABLE public.inquiries (
    id bigint NOT NULL,
    user_id bigint,
    name text NOT NULL,
    email text NOT NULL,
    category text NOT NULL,
    order_number text,
    body text NOT NULL,
    status text DEFAULT 'open'::text NOT NULL,
    admin_memo text DEFAULT ''::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT inquiries_category_check CHECK ((category = ANY (ARRAY['product'::text, 'order'::text, 'returns'::text, 'payment'::text, 'other'::text]))),
    CONSTRAINT inquiries_status_check CHECK ((status = ANY (ARRAY['open'::text, 'in_progress'::text, 'closed'::text])))
);

ALTER TABLE public.inquiries ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.inquiries_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.inquiry_replies (
    id bigint NOT NULL,
    inquiry_id bigint NOT NULL,
    admin_id bigint NOT NULL,
    subject text NOT NULL,
    body text NOT NULL,
    sent_at timestamp with time zone DEFAULT now() NOT NULL
);

ALTER TABLE public.inquiry_replies ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.inquiry_replies_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.member_addresses (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    label text DEFAULT ''::text NOT NULL,
    family_name text NOT NULL,
    given_name text NOT NULL,
    postal_code text NOT NULL,
    prefecture text NOT NULL,
    city text NOT NULL,
    address_line text NOT NULL,
    building text,
    phone text NOT NULL,
    is_default boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

ALTER TABLE public.member_addresses ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.member_addresses_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.menu_items (
    id bigint NOT NULL,
    menu_key text NOT NULL,
    label text NOT NULL,
    url text NOT NULL,
    is_visible boolean DEFAULT true NOT NULL,
    sort_order integer DEFAULT 0 NOT NULL,
    CONSTRAINT menu_items_menu_key_check CHECK ((menu_key = ANY (ARRAY['header'::text, 'footer'::text])))
);

ALTER TABLE public.menu_items ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.menu_items_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.notices (
    id bigint NOT NULL,
    body text NOT NULL,
    title text DEFAULT ''::text NOT NULL,
    display_from date,
    display_to date,
    is_visible boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT notices_display_period_order CHECK (((display_from IS NULL) OR (display_to IS NULL) OR (display_to >= display_from)))
);

CREATE SEQUENCE public.notices_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.notices_id_seq OWNED BY public.notices.id;

CREATE TABLE public.order_items (
    id bigint NOT NULL,
    order_id bigint NOT NULL,
    product_id bigint NOT NULL,
    product_name text NOT NULL,
    unit_price_yen bigint NOT NULL,
    quantity integer NOT NULL,
    variant_id bigint,
    variant_label text DEFAULT ''::text NOT NULL,
    tax_class_code text,
    rate_percent integer,
    CONSTRAINT order_items_quantity_check CHECK ((quantity > 0))
);

CREATE SEQUENCE public.order_items_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.order_items_id_seq OWNED BY public.order_items.id;

CREATE TABLE public.order_tax_breakdowns (
    id bigint NOT NULL,
    order_id bigint NOT NULL,
    tax_class_code text NOT NULL,
    tax_class_name text NOT NULL,
    rate_percent integer NOT NULL,
    taxable_base_yen bigint NOT NULL,
    tax_yen bigint NOT NULL
);

ALTER TABLE public.order_tax_breakdowns ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.order_tax_breakdowns_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.orders (
    id bigint NOT NULL,
    user_id bigint,
    email text NOT NULL,
    family_name text NOT NULL,
    given_name text NOT NULL,
    postal_code text NOT NULL,
    prefecture text NOT NULL,
    city text NOT NULL,
    address_line text NOT NULL,
    building text,
    phone text NOT NULL,
    subtotal_yen bigint NOT NULL,
    discount_yen bigint DEFAULT 0 NOT NULL,
    shipping_yen bigint NOT NULL,
    cod_fee_yen bigint NOT NULL,
    total_yen bigint NOT NULL,
    coupon_code text,
    status text DEFAULT 'received'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    delivery_date date,
    delivery_slot text DEFAULT 'none'::text NOT NULL,
    delivery_slot_label text DEFAULT '指定なし'::text NOT NULL,
    carrier text,
    tracking_number text,
    shipped_mail_sent_at timestamp with time zone,
    stock_restocked_at timestamp with time zone,
    placed_via text DEFAULT 'storefront'::text NOT NULL,
    CONSTRAINT orders_carrier_check CHECK ((carrier = ANY (ARRAY['yamato'::text, 'sagawa'::text, 'japan_post'::text, 'other'::text]))),
    CONSTRAINT orders_delivery_slot_check CHECK ((delivery_slot = ANY (ARRAY['none'::text, 'am'::text, 't1416'::text, 't1618'::text, 't1820'::text, 't1921'::text]))),
    CONSTRAINT orders_placed_via_check CHECK ((placed_via = ANY (ARRAY['storefront'::text, 'agent'::text]))),
    CONSTRAINT orders_status_check CHECK ((status = ANY (ARRAY['received'::text, 'preparing'::text, 'shipped'::text, 'delivered'::text, 'cancelled'::text])))
);

CREATE SEQUENCE public.orders_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.orders_id_seq OWNED BY public.orders.id;

CREATE TABLE public.password_resets (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    email text NOT NULL,
    code_hash text NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    resend_count integer DEFAULT 0 NOT NULL,
    code_sent_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    verified boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE public.password_resets_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.password_resets_id_seq OWNED BY public.password_resets.id;

CREATE TABLE public.payments (
    id bigint NOT NULL,
    order_id bigint NOT NULL,
    provider text NOT NULL,
    method text NOT NULL,
    status text NOT NULL,
    amount_yen bigint NOT NULL,
    amount_refunded_yen bigint DEFAULT 0 NOT NULL,
    currency text DEFAULT 'JPY'::text NOT NULL,
    komoju_session_id text,
    komoju_payment_id text,
    external_order_num text,
    payment_deadline timestamp with time zone,
    instructions_url text,
    payment_details jsonb,
    flag_note text,
    captured_at timestamp with time zone,
    confirmation_mail_sent_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT payments_method_check CHECK ((method = ANY (ARRAY['credit_card'::text, 'konbini'::text, 'paypay'::text, 'cod'::text, 'agent_test'::text]))),
    CONSTRAINT payments_provider_check CHECK ((provider = ANY (ARRAY['komoju'::text, 'cod'::text, 'agent'::text]))),
    CONSTRAINT payments_status_check CHECK ((status = ANY (ARRAY['pending_payment'::text, 'paid'::text, 'failed'::text, 'expired'::text, 'cancelled'::text, 'refunded'::text, 'partially_refunded'::text])))
);

CREATE SEQUENCE public.payments_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.payments_id_seq OWNED BY public.payments.id;

CREATE TABLE public.pending_registrations (
    id bigint NOT NULL,
    email text NOT NULL,
    code_hash text NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    resend_count integer DEFAULT 0 NOT NULL,
    code_sent_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    verified boolean DEFAULT false NOT NULL,
    password_hash text,
    family_name text,
    given_name text,
    family_name_kana text,
    given_name_kana text,
    birth_date date,
    gender text,
    postal_code text,
    prefecture text,
    city text,
    address_line text,
    building text,
    phone text,
    mail_magazine boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    email_bidx text
);

CREATE SEQUENCE public.pending_registrations_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.pending_registrations_id_seq OWNED BY public.pending_registrations.id;

CREATE TABLE public.postal_codes (
    postal_code text NOT NULL,
    prefecture text NOT NULL,
    city text NOT NULL,
    town text DEFAULT ''::text NOT NULL
);

CREATE TABLE public.product_images (
    id bigint NOT NULL,
    product_id bigint NOT NULL,
    image_path text NOT NULL,
    sort_order integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    alt_text text DEFAULT ''::text NOT NULL
);

CREATE SEQUENCE public.product_images_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.product_images_id_seq OWNED BY public.product_images.id;

CREATE TABLE public.product_tags (
    product_id bigint NOT NULL,
    tag_id bigint NOT NULL
);

CREATE TABLE public.product_variant_axes (
    id bigint NOT NULL,
    product_id bigint NOT NULL,
    name text NOT NULL,
    sort_order integer DEFAULT 0 NOT NULL
);

ALTER TABLE public.product_variant_axes ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.product_variant_axes_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.product_variant_option_links (
    variant_id bigint NOT NULL,
    option_id bigint NOT NULL
);

CREATE TABLE public.product_variant_options (
    id bigint NOT NULL,
    axis_id bigint NOT NULL,
    value text NOT NULL,
    sort_order integer DEFAULT 0 NOT NULL
);

ALTER TABLE public.product_variant_options ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.product_variant_options_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.product_variants (
    id bigint NOT NULL,
    product_id bigint NOT NULL,
    option_signature text NOT NULL,
    stock integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    price_yen bigint,
    sku text,
    jan_code text,
    image_path text,
    is_enabled boolean DEFAULT true NOT NULL,
    incoming_qty integer DEFAULT 0 NOT NULL,
    incoming_date date
);

ALTER TABLE public.product_variants ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.product_variants_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.products (
    id bigint NOT NULL,
    name text NOT NULL,
    price_yen bigint NOT NULL,
    rating double precision DEFAULT 0 NOT NULL,
    image_path text NOT NULL,
    category text NOT NULL,
    is_new boolean DEFAULT false NOT NULL,
    discount_percent integer DEFAULT 0 NOT NULL,
    is_preorder boolean DEFAULT false NOT NULL,
    is_event boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    description text DEFAULT ''::text NOT NULL,
    status text DEFAULT 'public'::text NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    category_id bigint,
    tax_class_id bigint DEFAULT 1 NOT NULL,
    cost_yen bigint,
    sale_price_yen bigint,
    sale_starts_at timestamp with time zone,
    sale_ends_at timestamp with time zone,
    product_code text,
    brand text DEFAULT ''::text NOT NULL,
    manufacturer text DEFAULT ''::text NOT NULL,
    country_of_origin text DEFAULT ''::text NOT NULL,
    spec_text text DEFAULT ''::text NOT NULL,
    features_text text DEFAULT ''::text NOT NULL,
    description_sp text DEFAULT ''::text NOT NULL,
    search_keywords text DEFAULT ''::text NOT NULL,
    track_stock boolean DEFAULT true NOT NULL,
    allow_backorder boolean DEFAULT false NOT NULL,
    low_stock_threshold integer,
    weight_grams integer,
    size_length_mm integer,
    size_width_mm integer,
    size_height_mm integer,
    cool_delivery boolean DEFAULT false NOT NULL,
    mail_delivery boolean DEFAULT false NOT NULL,
    free_shipping boolean DEFAULT false NOT NULL,
    undeliverable_regions text DEFAULT ''::text NOT NULL,
    lead_time_days integer,
    publish_starts_at timestamp with time zone,
    publish_ends_at timestamp with time zone,
    is_featured boolean DEFAULT false NOT NULL,
    display_order integer DEFAULT 0 NOT NULL,
    slug text,
    seo_title text DEFAULT ''::text NOT NULL,
    meta_description text DEFAULT ''::text NOT NULL,
    CONSTRAINT products_status_check CHECK ((status = ANY (ARRAY['public'::text, 'draft'::text, 'archived'::text])))
);

CREATE SEQUENCE public.products_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.products_id_seq OWNED BY public.products.id;

CREATE TABLE public.reviews (
    id bigint NOT NULL,
    product_id bigint NOT NULL,
    user_id bigint NOT NULL,
    order_id bigint NOT NULL,
    rating double precision NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    comment text DEFAULT ''::text NOT NULL,
    CONSTRAINT reviews_rating_check CHECK (((rating >= (0.5)::double precision) AND (rating <= (5)::double precision) AND ((rating * (2)::double precision) = floor((rating * (2)::double precision)))))
);

CREATE SEQUENCE public.reviews_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.reviews_id_seq OWNED BY public.reviews.id;

CREATE TABLE public.shop_settings (
    id integer NOT NULL,
    shop_name text NOT NULL,
    tagline text DEFAULT ''::text NOT NULL,
    shop_description text DEFAULT ''::text NOT NULL,
    contact_email text DEFAULT ''::text NOT NULL,
    contact_url text DEFAULT ''::text NOT NULL,
    support_note text DEFAULT ''::text NOT NULL,
    footer_note text DEFAULT ''::text NOT NULL,
    copyright_holder text DEFAULT ''::text NOT NULL,
    top_title text DEFAULT ''::text NOT NULL,
    default_meta_description text DEFAULT ''::text NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT shop_settings_id_check CHECK ((id = 1))
);

CREATE TABLE public.stock_adjustments (
    id bigint NOT NULL,
    variant_id bigint NOT NULL,
    delta integer NOT NULL,
    reason text NOT NULL,
    order_id bigint,
    admin_id bigint,
    note text DEFAULT ''::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT stock_adjustments_reason_check CHECK ((reason = ANY (ARRAY['order'::text, 'cancel'::text, 'refund'::text, 'manual'::text, 'initial'::text])))
);

ALTER TABLE public.stock_adjustments ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.stock_adjustments_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.tags (
    id bigint NOT NULL,
    slug text NOT NULL,
    label text NOT NULL,
    description text DEFAULT ''::text NOT NULL
);

CREATE SEQUENCE public.tags_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.tags_id_seq OWNED BY public.tags.id;

CREATE TABLE public.tax_classes (
    id bigint NOT NULL,
    code text NOT NULL,
    name text NOT NULL
);

ALTER TABLE public.tax_classes ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.tax_classes_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.tax_rates (
    id bigint NOT NULL,
    tax_class_id bigint NOT NULL,
    rate_percent integer NOT NULL,
    effective_from date NOT NULL,
    effective_to date,
    CONSTRAINT tax_rates_check CHECK (((effective_to IS NULL) OR (effective_to >= effective_from))),
    CONSTRAINT tax_rates_rate_percent_check CHECK ((rate_percent >= 0))
);

ALTER TABLE public.tax_rates ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.tax_rates_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.user_coupons (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    code text NOT NULL,
    issued_at timestamp with time zone DEFAULT now() NOT NULL,
    used_at timestamp with time zone,
    voided_at timestamp with time zone
);

CREATE SEQUENCE public.user_coupons_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.user_coupons_id_seq OWNED BY public.user_coupons.id;

CREATE TABLE public.user_identities (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    provider text NOT NULL,
    provider_user_id text NOT NULL,
    provider_email text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT user_identities_provider_check CHECK ((provider = ANY (ARRAY['line'::text, 'google'::text, 'yahoo'::text])))
);

CREATE SEQUENCE public.user_identities_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.user_identities_id_seq OWNED BY public.user_identities.id;

CREATE TABLE public.user_recovery_codes (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    code_hash text NOT NULL,
    used_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

ALTER TABLE public.user_recovery_codes ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.user_recovery_codes_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE public.users (
    id bigint NOT NULL,
    email text NOT NULL,
    password_hash text NOT NULL,
    family_name text NOT NULL,
    given_name text NOT NULL,
    family_name_kana text NOT NULL,
    given_name_kana text NOT NULL,
    birth_date date,
    gender text,
    postal_code text NOT NULL,
    prefecture text NOT NULL,
    city text NOT NULL,
    address_line text NOT NULL,
    building text,
    phone text NOT NULL,
    mail_magazine boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    withdrawn_at timestamp with time zone,
    welcome_email_hash text,
    totp_secret text,
    totp_enabled_at timestamp with time zone,
    has_password boolean DEFAULT true NOT NULL,
    email_bidx text
);

CREATE SEQUENCE public.users_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.users_id_seq OWNED BY public.users.id;

CREATE TABLE public.webhook_events (
    event_id text NOT NULL,
    delivery_id text,
    event_type text NOT NULL,
    payment_id bigint,
    received_at timestamp with time zone DEFAULT now() NOT NULL,
    applied boolean DEFAULT false NOT NULL,
    raw_body jsonb NOT NULL
);

CREATE TABLE public.withdrawal_codes (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    email text NOT NULL,
    code_hash text NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    resend_count integer DEFAULT 0 NOT NULL,
    code_sent_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    verified boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE public.withdrawal_codes_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.withdrawal_codes_id_seq OWNED BY public.withdrawal_codes.id;

CREATE TABLE IF NOT EXISTS tower_sessions.session (
    id text PRIMARY KEY NOT NULL,
    data bytea NOT NULL,
    expiry_date timestamptz NOT NULL
);

ALTER TABLE ONLY public.admin_audit_logs ALTER COLUMN id SET DEFAULT nextval('public.admin_audit_logs_id_seq'::regclass);

ALTER TABLE ONLY public.admins ALTER COLUMN id SET DEFAULT nextval('public.admins_id_seq'::regclass);

ALTER TABLE ONLY public.cart_items ALTER COLUMN id SET DEFAULT nextval('public.cart_items_id_seq'::regclass);

ALTER TABLE ONLY public.categories ALTER COLUMN id SET DEFAULT nextval('public.categories_id_seq'::regclass);

ALTER TABLE ONLY public.coupons ALTER COLUMN id SET DEFAULT nextval('public.coupons_id_seq'::regclass);

ALTER TABLE ONLY public.csv_import_batches ALTER COLUMN id SET DEFAULT nextval('public.csv_import_batches_id_seq'::regclass);

ALTER TABLE ONLY public.email_campaigns ALTER COLUMN id SET DEFAULT nextval('public.email_campaigns_id_seq'::regclass);

ALTER TABLE ONLY public.email_changes ALTER COLUMN id SET DEFAULT nextval('public.email_changes_id_seq'::regclass);

ALTER TABLE ONLY public.email_consent_logs ALTER COLUMN id SET DEFAULT nextval('public.email_consent_logs_id_seq'::regclass);

ALTER TABLE ONLY public.email_deliveries ALTER COLUMN id SET DEFAULT nextval('public.email_deliveries_id_seq'::regclass);

ALTER TABLE ONLY public.email_jobs ALTER COLUMN id SET DEFAULT nextval('public.email_jobs_id_seq'::regclass);

ALTER TABLE ONLY public.email_templates ALTER COLUMN id SET DEFAULT nextval('public.email_templates_id_seq'::regclass);

ALTER TABLE ONLY public.notices ALTER COLUMN id SET DEFAULT nextval('public.notices_id_seq'::regclass);

ALTER TABLE ONLY public.order_items ALTER COLUMN id SET DEFAULT nextval('public.order_items_id_seq'::regclass);

ALTER TABLE ONLY public.orders ALTER COLUMN id SET DEFAULT nextval('public.orders_id_seq'::regclass);

ALTER TABLE ONLY public.password_resets ALTER COLUMN id SET DEFAULT nextval('public.password_resets_id_seq'::regclass);

ALTER TABLE ONLY public.payments ALTER COLUMN id SET DEFAULT nextval('public.payments_id_seq'::regclass);

ALTER TABLE ONLY public.pending_registrations ALTER COLUMN id SET DEFAULT nextval('public.pending_registrations_id_seq'::regclass);

ALTER TABLE ONLY public.product_images ALTER COLUMN id SET DEFAULT nextval('public.product_images_id_seq'::regclass);

ALTER TABLE ONLY public.products ALTER COLUMN id SET DEFAULT nextval('public.products_id_seq'::regclass);

ALTER TABLE ONLY public.reviews ALTER COLUMN id SET DEFAULT nextval('public.reviews_id_seq'::regclass);

ALTER TABLE ONLY public.tags ALTER COLUMN id SET DEFAULT nextval('public.tags_id_seq'::regclass);

ALTER TABLE ONLY public.user_coupons ALTER COLUMN id SET DEFAULT nextval('public.user_coupons_id_seq'::regclass);

ALTER TABLE ONLY public.user_identities ALTER COLUMN id SET DEFAULT nextval('public.user_identities_id_seq'::regclass);

ALTER TABLE ONLY public.users ALTER COLUMN id SET DEFAULT nextval('public.users_id_seq'::regclass);

ALTER TABLE ONLY public.withdrawal_codes ALTER COLUMN id SET DEFAULT nextval('public.withdrawal_codes_id_seq'::regclass);

ALTER TABLE ONLY public.admin_audit_logs
    ADD CONSTRAINT admin_audit_logs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.admins
    ADD CONSTRAINT admins_email_key UNIQUE (email);

ALTER TABLE ONLY public.admins
    ADD CONSTRAINT admins_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.agent_approvals
    ADD CONSTRAINT agent_approvals_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.cart_items
    ADD CONSTRAINT cart_items_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.categories
    ADD CONSTRAINT categories_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.content_pages
    ADD CONSTRAINT content_pages_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.content_pages
    ADD CONSTRAINT content_pages_slug_key UNIQUE (slug);

ALTER TABLE ONLY public.content_revisions
    ADD CONSTRAINT content_revisions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.coupons
    ADD CONSTRAINT coupons_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.csv_import_batches
    ADD CONSTRAINT csv_import_batches_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.email_campaigns
    ADD CONSTRAINT email_campaigns_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.email_changes
    ADD CONSTRAINT email_changes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.email_consent_logs
    ADD CONSTRAINT email_consent_logs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.email_deliveries
    ADD CONSTRAINT email_deliveries_job_id_user_id_key UNIQUE (job_id, user_id);

ALTER TABLE ONLY public.email_deliveries
    ADD CONSTRAINT email_deliveries_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.email_jobs
    ADD CONSTRAINT email_jobs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.email_subscriptions
    ADD CONSTRAINT email_subscriptions_pkey PRIMARY KEY (user_id);

ALTER TABLE ONLY public.email_subscriptions
    ADD CONSTRAINT email_subscriptions_unsubscribe_token_key UNIQUE (unsubscribe_token);

ALTER TABLE ONLY public.email_templates
    ADD CONSTRAINT email_templates_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.favorites
    ADD CONSTRAINT favorites_pkey PRIMARY KEY (user_id, product_id);

ALTER TABLE ONLY public.home_sections
    ADD CONSTRAINT home_sections_pkey PRIMARY KEY (section_key);

ALTER TABLE ONLY public.home_slots
    ADD CONSTRAINT home_slots_pkey PRIMARY KEY (key);

ALTER TABLE ONLY public.inquiries
    ADD CONSTRAINT inquiries_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.inquiry_replies
    ADD CONSTRAINT inquiry_replies_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.member_addresses
    ADD CONSTRAINT member_addresses_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.menu_items
    ADD CONSTRAINT menu_items_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.notices
    ADD CONSTRAINT notices_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.order_items
    ADD CONSTRAINT order_items_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.order_tax_breakdowns
    ADD CONSTRAINT order_tax_breakdowns_order_id_tax_class_code_key UNIQUE (order_id, tax_class_code);

ALTER TABLE ONLY public.order_tax_breakdowns
    ADD CONSTRAINT order_tax_breakdowns_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.orders
    ADD CONSTRAINT orders_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.password_resets
    ADD CONSTRAINT password_resets_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.payments
    ADD CONSTRAINT payments_komoju_payment_id_key UNIQUE (komoju_payment_id);

ALTER TABLE ONLY public.payments
    ADD CONSTRAINT payments_komoju_session_id_key UNIQUE (komoju_session_id);

ALTER TABLE ONLY public.payments
    ADD CONSTRAINT payments_order_id_key UNIQUE (order_id);

ALTER TABLE ONLY public.payments
    ADD CONSTRAINT payments_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.pending_registrations
    ADD CONSTRAINT pending_registrations_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.postal_codes
    ADD CONSTRAINT postal_codes_pkey PRIMARY KEY (postal_code, town);

ALTER TABLE ONLY public.product_images
    ADD CONSTRAINT product_images_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.product_tags
    ADD CONSTRAINT product_tags_pkey PRIMARY KEY (product_id, tag_id);

ALTER TABLE ONLY public.product_variant_axes
    ADD CONSTRAINT product_variant_axes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.product_variant_option_links
    ADD CONSTRAINT product_variant_option_links_pkey PRIMARY KEY (variant_id, option_id);

ALTER TABLE ONLY public.product_variant_options
    ADD CONSTRAINT product_variant_options_axis_id_value_key UNIQUE (axis_id, value);

ALTER TABLE ONLY public.product_variant_options
    ADD CONSTRAINT product_variant_options_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.product_variants
    ADD CONSTRAINT product_variants_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.product_variants
    ADD CONSTRAINT product_variants_product_id_option_signature_key UNIQUE (product_id, option_signature);

ALTER TABLE ONLY public.products
    ADD CONSTRAINT products_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.reviews
    ADD CONSTRAINT reviews_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.reviews
    ADD CONSTRAINT reviews_product_id_user_id_key UNIQUE (product_id, user_id);

ALTER TABLE ONLY public.shop_settings
    ADD CONSTRAINT shop_settings_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.stock_adjustments
    ADD CONSTRAINT stock_adjustments_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.tags
    ADD CONSTRAINT tags_label_key UNIQUE (label);

ALTER TABLE ONLY public.tags
    ADD CONSTRAINT tags_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.tags
    ADD CONSTRAINT tags_slug_key UNIQUE (slug);

ALTER TABLE ONLY public.tax_classes
    ADD CONSTRAINT tax_classes_code_key UNIQUE (code);

ALTER TABLE ONLY public.tax_classes
    ADD CONSTRAINT tax_classes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.tax_rates
    ADD CONSTRAINT tax_rates_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.user_coupons
    ADD CONSTRAINT user_coupons_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.user_identities
    ADD CONSTRAINT user_identities_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.user_identities
    ADD CONSTRAINT user_identities_provider_provider_user_id_key UNIQUE (provider, provider_user_id);

ALTER TABLE ONLY public.user_identities
    ADD CONSTRAINT user_identities_user_id_provider_key UNIQUE (user_id, provider);

ALTER TABLE ONLY public.user_recovery_codes
    ADD CONSTRAINT user_recovery_codes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.webhook_events
    ADD CONSTRAINT webhook_events_pkey PRIMARY KEY (event_id);

ALTER TABLE ONLY public.withdrawal_codes
    ADD CONSTRAINT withdrawal_codes_pkey PRIMARY KEY (id);

CREATE INDEX admin_audit_logs_created_idx ON public.admin_audit_logs USING btree (created_at DESC, id DESC);

CREATE INDEX admin_audit_logs_target_idx ON public.admin_audit_logs USING btree (target_type, created_at DESC, id DESC);

CREATE INDEX agent_approvals_owner_idx ON public.agent_approvals USING btree (user_id, session_id, id DESC);

CREATE UNIQUE INDEX cart_items_session_variant ON public.cart_items USING btree (session_id, variant_id) WHERE (session_id IS NOT NULL);

CREATE UNIQUE INDEX cart_items_user_variant ON public.cart_items USING btree (user_id, variant_id) WHERE (user_id IS NOT NULL);

CREATE UNIQUE INDEX categories_name_unique ON public.categories USING btree (lower(name));

CREATE INDEX categories_parent_idx ON public.categories USING btree (parent_id);

CREATE UNIQUE INDEX categories_slug_unique ON public.categories USING btree (lower(slug));

CREATE INDEX content_revisions_page_idx ON public.content_revisions USING btree (page_id, id DESC);

CREATE UNIQUE INDEX coupons_code_upper_idx ON public.coupons USING btree (upper(code));

CREATE UNIQUE INDEX coupons_one_stripe_featured_idx ON public.coupons USING btree (stripe_featured) WHERE stripe_featured;

CREATE INDEX email_changes_user_idx ON public.email_changes USING btree (user_id);

CREATE INDEX email_deliveries_campaign ON public.email_deliveries USING btree (campaign_id, status);

CREATE INDEX email_deliveries_job_status ON public.email_deliveries USING btree (job_id, status);

CREATE INDEX email_jobs_status ON public.email_jobs USING btree (status, id);

CREATE INDEX favorites_user_created_idx ON public.favorites USING btree (user_id, created_at DESC);

CREATE INDEX idx_user_identities_user ON public.user_identities USING btree (user_id);

CREATE INDEX idx_withdrawal_codes_user ON public.withdrawal_codes USING btree (user_id);

CREATE INDEX inquiries_email_idx ON public.inquiries USING btree (email, created_at DESC);

CREATE INDEX inquiries_status_idx ON public.inquiries USING btree (status, created_at DESC);

CREATE INDEX inquiry_replies_inquiry_idx ON public.inquiry_replies USING btree (inquiry_id, id);

CREATE UNIQUE INDEX member_addresses_one_default_idx ON public.member_addresses USING btree (user_id) WHERE is_default;

CREATE INDEX member_addresses_user_idx ON public.member_addresses USING btree (user_id, created_at);

CREATE INDEX menu_items_menu_idx ON public.menu_items USING btree (menu_key, sort_order);

CREATE INDEX order_items_order_idx ON public.order_items USING btree (order_id);

CREATE INDEX order_items_product_idx ON public.order_items USING btree (product_id);

CREATE INDEX order_items_variant_idx ON public.order_items USING btree (variant_id);

CREATE INDEX orders_created_idx ON public.orders USING btree (created_at DESC, id DESC);

CREATE INDEX orders_status_created_idx ON public.orders USING btree (status, created_at DESC);

CREATE INDEX orders_user_idx ON public.orders USING btree (user_id, created_at DESC);

CREATE INDEX password_resets_user_idx ON public.password_resets USING btree (user_id);

CREATE INDEX payments_open_komoju_idx ON public.payments USING btree (created_at) WHERE ((provider = 'komoju'::text) AND (status = ANY (ARRAY['pending_payment'::text, 'failed'::text])));

CREATE INDEX pending_registrations_email_bidx_idx ON public.pending_registrations USING btree (email_bidx);

CREATE INDEX pending_registrations_email_idx ON public.pending_registrations USING btree (email);

CREATE INDEX product_images_product_idx ON public.product_images USING btree (product_id, sort_order);

CREATE INDEX product_variant_axes_product_idx ON public.product_variant_axes USING btree (product_id);

CREATE UNIQUE INDEX product_variants_jan_unique ON public.product_variants USING btree (jan_code) WHERE ((jan_code IS NOT NULL) AND (jan_code <> ''::text));

CREATE UNIQUE INDEX product_variants_sku_unique ON public.product_variants USING btree (sku) WHERE ((sku IS NOT NULL) AND (sku <> ''::text));

CREATE INDEX products_brand_trgm_idx ON public.products USING gin (brand public.gin_trgm_ops);

CREATE INDEX products_category_id_idx ON public.products USING btree (category_id);

CREATE INDEX products_category_status_idx ON public.products USING btree (category, status);

CREATE INDEX products_category_trgm_idx ON public.products USING gin (category public.gin_trgm_ops);

CREATE INDEX products_name_trgm_idx ON public.products USING gin (name public.gin_trgm_ops);

CREATE UNIQUE INDEX products_product_code_unique ON public.products USING btree (product_code) WHERE ((product_code IS NOT NULL) AND (product_code <> ''::text));

CREATE INDEX products_search_keywords_trgm_idx ON public.products USING gin (search_keywords public.gin_trgm_ops);

CREATE UNIQUE INDEX products_slug_unique ON public.products USING btree (slug) WHERE (slug IS NOT NULL);

CREATE INDEX products_status_created_idx ON public.products USING btree (status, created_at DESC, id DESC);

CREATE INDEX reviews_order_idx ON public.reviews USING btree (order_id);

CREATE INDEX reviews_product_created_idx ON public.reviews USING btree (product_id, created_at DESC);

CREATE INDEX stock_adjustments_variant_idx ON public.stock_adjustments USING btree (variant_id, id DESC);

CREATE UNIQUE INDEX tags_label_unique ON public.tags USING btree (lower(label));

CREATE UNIQUE INDEX tags_slug_unique ON public.tags USING btree (lower(slug));

CREATE INDEX tax_rates_class_from_idx ON public.tax_rates USING btree (tax_class_id, effective_from DESC);

CREATE UNIQUE INDEX user_coupons_user_code ON public.user_coupons USING btree (user_id, code);

CREATE INDEX user_recovery_codes_user_idx ON public.user_recovery_codes USING btree (user_id);

CREATE UNIQUE INDEX users_email_bidx_key ON public.users USING btree (email_bidx) WHERE (email_bidx IS NOT NULL);

CREATE INDEX users_welcome_email_hash_idx ON public.users USING btree (welcome_email_hash) WHERE (welcome_email_hash IS NOT NULL);

CREATE INDEX webhook_events_payment_idx ON public.webhook_events USING btree (payment_id);

CREATE INDEX session_expiry_date_idx ON tower_sessions.session USING btree (expiry_date);

CREATE TRIGGER admin_audit_logs_no_update_or_delete BEFORE DELETE OR UPDATE ON public.admin_audit_logs FOR EACH ROW EXECUTE FUNCTION public.admin_audit_logs_append_only();

ALTER TABLE ONLY public.admin_audit_logs
    ADD CONSTRAINT admin_audit_logs_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.admin_audit_logs
    ADD CONSTRAINT admin_audit_logs_admin_id_fkey FOREIGN KEY (admin_id) REFERENCES public.admins(id);

ALTER TABLE ONLY public.agent_approvals
    ADD CONSTRAINT agent_approvals_consumed_order_id_fkey FOREIGN KEY (consumed_order_id) REFERENCES public.orders(id);

ALTER TABLE ONLY public.agent_approvals
    ADD CONSTRAINT agent_approvals_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.cart_items
    ADD CONSTRAINT cart_items_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.cart_items
    ADD CONSTRAINT cart_items_variant_id_fkey FOREIGN KEY (variant_id) REFERENCES public.product_variants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.categories
    ADD CONSTRAINT categories_parent_id_fkey FOREIGN KEY (parent_id) REFERENCES public.categories(id) ON DELETE RESTRICT;

ALTER TABLE ONLY public.content_revisions
    ADD CONSTRAINT content_revisions_edited_by_fkey FOREIGN KEY (edited_by) REFERENCES public.admins(id);

ALTER TABLE ONLY public.content_revisions
    ADD CONSTRAINT content_revisions_page_id_fkey FOREIGN KEY (page_id) REFERENCES public.content_pages(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.email_campaigns
    ADD CONSTRAINT email_campaigns_template_id_fkey FOREIGN KEY (template_id) REFERENCES public.email_templates(id);

ALTER TABLE ONLY public.email_changes
    ADD CONSTRAINT email_changes_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.email_consent_logs
    ADD CONSTRAINT email_consent_logs_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.email_deliveries
    ADD CONSTRAINT email_deliveries_campaign_id_fkey FOREIGN KEY (campaign_id) REFERENCES public.email_campaigns(id);

ALTER TABLE ONLY public.email_deliveries
    ADD CONSTRAINT email_deliveries_job_id_fkey FOREIGN KEY (job_id) REFERENCES public.email_jobs(id);

ALTER TABLE ONLY public.email_deliveries
    ADD CONSTRAINT email_deliveries_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.email_jobs
    ADD CONSTRAINT email_jobs_campaign_id_fkey FOREIGN KEY (campaign_id) REFERENCES public.email_campaigns(id);

ALTER TABLE ONLY public.email_subscriptions
    ADD CONSTRAINT email_subscriptions_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.favorites
    ADD CONSTRAINT favorites_product_id_fkey FOREIGN KEY (product_id) REFERENCES public.products(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.favorites
    ADD CONSTRAINT favorites_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.inquiries
    ADD CONSTRAINT inquiries_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.inquiry_replies
    ADD CONSTRAINT inquiry_replies_admin_id_fkey FOREIGN KEY (admin_id) REFERENCES public.admins(id);

ALTER TABLE ONLY public.inquiry_replies
    ADD CONSTRAINT inquiry_replies_inquiry_id_fkey FOREIGN KEY (inquiry_id) REFERENCES public.inquiries(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.member_addresses
    ADD CONSTRAINT member_addresses_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.order_items
    ADD CONSTRAINT order_items_order_id_fkey FOREIGN KEY (order_id) REFERENCES public.orders(id);

ALTER TABLE ONLY public.order_items
    ADD CONSTRAINT order_items_product_id_fkey FOREIGN KEY (product_id) REFERENCES public.products(id);

ALTER TABLE ONLY public.order_items
    ADD CONSTRAINT order_items_variant_id_fkey FOREIGN KEY (variant_id) REFERENCES public.product_variants(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.order_tax_breakdowns
    ADD CONSTRAINT order_tax_breakdowns_order_id_fkey FOREIGN KEY (order_id) REFERENCES public.orders(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.orders
    ADD CONSTRAINT orders_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.password_resets
    ADD CONSTRAINT password_resets_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.payments
    ADD CONSTRAINT payments_order_id_fkey FOREIGN KEY (order_id) REFERENCES public.orders(id);

ALTER TABLE ONLY public.product_images
    ADD CONSTRAINT product_images_product_id_fkey FOREIGN KEY (product_id) REFERENCES public.products(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.product_tags
    ADD CONSTRAINT product_tags_product_id_fkey FOREIGN KEY (product_id) REFERENCES public.products(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.product_tags
    ADD CONSTRAINT product_tags_tag_id_fkey FOREIGN KEY (tag_id) REFERENCES public.tags(id);

ALTER TABLE ONLY public.product_variant_axes
    ADD CONSTRAINT product_variant_axes_product_id_fkey FOREIGN KEY (product_id) REFERENCES public.products(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.product_variant_option_links
    ADD CONSTRAINT product_variant_option_links_option_id_fkey FOREIGN KEY (option_id) REFERENCES public.product_variant_options(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.product_variant_option_links
    ADD CONSTRAINT product_variant_option_links_variant_id_fkey FOREIGN KEY (variant_id) REFERENCES public.product_variants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.product_variant_options
    ADD CONSTRAINT product_variant_options_axis_id_fkey FOREIGN KEY (axis_id) REFERENCES public.product_variant_axes(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.product_variants
    ADD CONSTRAINT product_variants_product_id_fkey FOREIGN KEY (product_id) REFERENCES public.products(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.products
    ADD CONSTRAINT products_category_id_fkey FOREIGN KEY (category_id) REFERENCES public.categories(id) ON DELETE RESTRICT;

ALTER TABLE ONLY public.products
    ADD CONSTRAINT products_tax_class_id_fkey FOREIGN KEY (tax_class_id) REFERENCES public.tax_classes(id);

ALTER TABLE ONLY public.reviews
    ADD CONSTRAINT reviews_order_id_fkey FOREIGN KEY (order_id) REFERENCES public.orders(id);

ALTER TABLE ONLY public.reviews
    ADD CONSTRAINT reviews_product_id_fkey FOREIGN KEY (product_id) REFERENCES public.products(id);

ALTER TABLE ONLY public.reviews
    ADD CONSTRAINT reviews_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.stock_adjustments
    ADD CONSTRAINT stock_adjustments_admin_id_fkey FOREIGN KEY (admin_id) REFERENCES public.admins(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.stock_adjustments
    ADD CONSTRAINT stock_adjustments_order_id_fkey FOREIGN KEY (order_id) REFERENCES public.orders(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.stock_adjustments
    ADD CONSTRAINT stock_adjustments_variant_id_fkey FOREIGN KEY (variant_id) REFERENCES public.product_variants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.tax_rates
    ADD CONSTRAINT tax_rates_tax_class_id_fkey FOREIGN KEY (tax_class_id) REFERENCES public.tax_classes(id);

ALTER TABLE ONLY public.user_coupons
    ADD CONSTRAINT user_coupons_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.user_identities
    ADD CONSTRAINT user_identities_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.user_recovery_codes
    ADD CONSTRAINT user_recovery_codes_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.webhook_events
    ADD CONSTRAINT webhook_events_payment_id_fkey FOREIGN KEY (payment_id) REFERENCES public.payments(id);

ALTER TABLE ONLY public.withdrawal_codes
    ADD CONSTRAINT withdrawal_codes_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);
