-- Landing pages (page types, CMS_SPEC §9): content_pages
-- gains a layout and a sitemap switch.
--
-- * `layout` separates the standard pages (the historic
--   behaviour, full site chrome) from landing pages
--   (minimal chrome at /lp/{slug}, built for one campaign
--   and one conversion). Every existing row is a standard
--   page by the default.
-- * `in_sitemap` is the per-page sitemap switch: standard
--   pages keep today's behaviour (included, by the
--   default); a landing page's operator opts in
--   explicitly, because campaign pages are usually
--   ad-landing surfaces that should not compete with the
--   catalogue in search results.
-- * Slug uniqueness becomes per-layout: /pages/{slug}
--   and /lp/{slug} are different namespaces, so a standard
--   page and a landing page may share one slug string and
--   each serves on its own route. Existing slugs are
--   globally unique, so the composite constraint holds
--   trivially for current rows.
--
-- A backup was taken and restore-proven before this
-- migration was written
-- (~/backups/agentic-commerce/agentic_commerce-20261128-141353.sql).
ALTER TABLE content_pages
    ADD COLUMN layout TEXT NOT NULL DEFAULT 'standard';

ALTER TABLE content_pages
    ADD CONSTRAINT content_pages_layout_check
    CHECK (layout IN ('standard', 'landing'));

ALTER TABLE content_pages
    ADD COLUMN in_sitemap BOOLEAN NOT NULL DEFAULT TRUE;

ALTER TABLE content_pages
    DROP CONSTRAINT content_pages_slug_key;

ALTER TABLE content_pages
    ADD CONSTRAINT content_pages_slug_layout_key UNIQUE (slug, layout);
