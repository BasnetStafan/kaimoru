# CMS blog module — specification

Extends the existing pages CMS (`content_pages`, CMS spec
C-2) with the one content shape it does not have: dated,
authored articles. A blog post is not a page — it carries a
publication instant, an author, an excerpt, a cover image
and a category — so posts get their own tables, built on the
pages machinery's patterns: Markdown or sanitised-HTML
bodies, snapshot-before-save revisions, audited writes.

Status: APPROVED 2026-11-28 — the owner took every
recommendation in section 7 (D1 blog + scheduling + RSS,
D2 own post categories and no tags, D3 no comments, D4
optional cover image). Built the same day.

## 1. Data model (migration 20261101000049)

- `post_categories`: id, name, slug UNIQUE, created_at,
  updated_at (+ the `set_updated_at` trigger every operator
  table carries).
- `posts`: id, slug UNIQUE, title, body_md, format
  (`markdown` / `html`), excerpt, meta_description,
  cover_image_path (nullable), category_id (nullable,
  references post_categories ON DELETE SET NULL),
  author_admin_id (references admins), status (`draft` /
  `published`), published_at (nullable), created_at,
  updated_at (+ trigger).
- `post_revisions`: the content_revisions shape plus the
  excerpt column; post_id references posts ON DELETE
  CASCADE. A save snapshots the state it replaces; the
  newest 50 per post are kept.

## 2. Liveness — one definition

A post is **live** iff `status = 'published'` AND
`published_at IS NOT NULL` AND `published_at <= now()`.
The rule is defined once (the post model) and every
storefront query — index, post page, category archive,
feed, sitemap — uses the same predicate, so the surfaces
can never disagree about what is public.

Derived admin states: 下書き (draft), 予約中 (published with
a future — or missing — published_at), 公開中 (live).
Scheduling is a future published_at; there is no worker
and no third status. Publishing with an empty published_at
stamps it with the save instant.

## 3. Admin

Guarded by the content area (AREA_CONTENT), listed in the
sidebar's コンテンツ group beside the pages entries.

- `/admin/posts`: list with a state filter and pagination;
  title, category, state, published_at, updated, author and
  last editor columns.
- `/admin/posts/new`, `/admin/posts/{id}`: the editor —
  title, slug (creation only, immutable after), category
  select, excerpt, body with the pages editor's format
  choice, meta description, cover image upload (the product
  image machinery; clearable), status, published_at as a
  JST `datetime-local`.
- Preview renders the unsaved post through the storefront
  post template, admin session only, `no-store` — the pages
  preview discipline.
- Revisions list + restore, mirroring content pages: the
  restore snapshots the state it replaces, so it is itself
  undoable. Restore never touches status, category, cover
  or published_at — a snapshot holds words, not scheduling.
- Delete: any post may be deleted (posts are all
  operator-made); the form confirms natively, revisions
  cascade. Audit: post.create / post.update / post.delete /
  post.restore.
- `/admin/post-categories`: plain CRUD (name + slug).
  Deleting a category leaves its posts uncategorised via
  the foreign key's SET NULL. Audit: post_category.create /
  .update / .delete.

Validation mirrors the pages service: slug 3–60 of
lowercase ASCII / digits / hyphens (plus `category` and
`feed.xml` reserved — the blog's own subroutes), title
required (80), body 50,000, meta 160, the shared link
allowlist refused at save naming the link, HTML bodies
sanitised before storage.

## 4. Storefront

- `GET /blog`: live posts, published_at descending,
  paginated; cards show the cover image when set, the JST
  date, the category link, the title and the excerpt —
  the stored excerpt, or one derived from the body text
  (markup stripped, whitespace collapsed, truncated) when
  the excerpt is empty.
- `GET /blog/{slug}`: the post — title, date, author
  display name, category link, body rendered exactly as
  content pages render, meta description with the fallback
  chain post → excerpt → shop default. Draft, scheduled
  and unknown slugs answer the same styled 404.
- `GET /blog/category/{slug}`: the same index filtered to
  one category; an unknown category 404s.
- `GET /blog/feed.xml`: RSS 2.0 with the latest 20 live
  posts (title, absolute link, guid, RFC 822 pubDate,
  excerpt-or-derived description), mounted with the
  crawler documents — no session, no CSRF — under the
  maintenance gate.

## 5. Sitemap

`/blog`, every live post (lastmod = updated_at) and every
category page holding at least one live post join the
sitemap's entries.

## 6. Out of scope

Comments (D3), post tags (D2), a media library, a block
page builder, multi-author bylines beyond the creating
admin. The cover image reuses the product upload path —
no new image pipeline.

## 7. Decisions

- D1: blog + scheduled publishing + RSS in one build.
- D2: post categories are their own small table (a post
  about shipping is not a product category); no post tags.
- D3: no comments — moderation burden, and reviews already
  cover product-scoped voices.
- D4: optional cover image via the existing upload
  machinery.
