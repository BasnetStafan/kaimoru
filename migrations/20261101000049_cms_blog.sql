-- CMS blog module (docs/specs/CMS_SPEC.md): dated, authored
-- articles on top of the pages CMS. A post is not a page —
-- it carries a publication instant, an author, an excerpt,
-- a cover image and a category — so posts get their own
-- tables, mirroring the content_pages / content_revisions
-- shapes (snapshot-before-save revisions, the set_updated_at
-- trigger every operator table carries).
--
-- Liveness is a rule, not a column: a post is live iff
-- status = 'published' AND published_at IS NOT NULL AND
-- published_at <= now(). Scheduling is a future
-- published_at; there is no worker and no third status.
--
-- Deleting a category leaves its posts uncategorised
-- (ON DELETE SET NULL); deleting a post takes its
-- revisions with it (ON DELETE CASCADE), as content pages
-- do.
--
-- A backup was taken and restore-proven before this
-- migration was written
-- (~/backups/agentic-commerce/agentic_commerce-20261128-120209.sql).
CREATE TABLE post_categories (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER trg_post_categories_set_updated_at
    BEFORE UPDATE ON post_categories
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE posts (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    body_md TEXT NOT NULL DEFAULT '',
    format TEXT NOT NULL DEFAULT 'markdown'
        CHECK (format IN ('markdown', 'html')),
    excerpt TEXT NOT NULL DEFAULT '',
    meta_description TEXT NOT NULL DEFAULT '',
    cover_image_path TEXT,
    category_id BIGINT REFERENCES post_categories(id) ON DELETE SET NULL,
    author_admin_id BIGINT NOT NULL REFERENCES admins(id),
    status TEXT NOT NULL CHECK (status IN ('draft', 'published')),
    published_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER trg_posts_set_updated_at
    BEFORE UPDATE ON posts
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE post_revisions (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    post_id BIGINT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    body_md TEXT NOT NULL,
    format TEXT NOT NULL DEFAULT 'markdown'
        CHECK (format IN ('markdown', 'html')),
    meta_description TEXT NOT NULL DEFAULT '',
    excerpt TEXT NOT NULL DEFAULT '',
    edited_by BIGINT NOT NULL REFERENCES admins(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
