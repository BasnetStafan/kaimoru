-- CMS blog indexes (full-codebase review 2026-11-28,
-- PERF-05): the blog tables shipped with only their
-- primary keys and the slug uniqueness. The two hot
-- reads get their support:
--
-- * the storefront index / archive / feed all read
--   `WHERE status = 'published' AND published_at <= now()
--   ORDER BY published_at DESC` — one composite index
--   serves the filter and the order together;
-- * revision reads and the admin list's newest-editor
--   subquery both read one post's revisions newest-first
--   (`WHERE post_id = $1 ORDER BY id DESC`) — the same
--   shape content_revisions already carries
--   (content_revisions_page_idx).
--
-- A backup was taken and restore-proven before this
-- migration was written
-- (~/backups/agentic-commerce/agentic_commerce-20261128-131456.sql).
CREATE INDEX posts_live_idx ON posts (status, published_at DESC);

CREATE INDEX post_revisions_post_idx ON post_revisions (post_id, id DESC);
