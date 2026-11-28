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

## 8. Shoppable embeds (added 2026-11-28)

A post body may carry shortcodes that the app expands into
live catalogue cards at render time:

- `[[product:<id>]]` — one product card: image, the
  catalog card's badge chain, name, its tax-inclusive
  price line (sale strike-through, （税込）, the tax-class
  note), and a purchase control that mirrors the catalog
  card's purchasability exactly. A simple purchasable
  product gets the add-to-cart form (htmx POST to the
  existing `/cart/items`, quantity 1); a multi-variant
  product links to its detail page instead （詳細を見る） —
  an embed never chooses a combination for the customer;
  a sold-out product shows the disabled 売り切れ button.
- `[[category:<id>]]` — a showcase: the category name
  linking to the storefront category page, a grid of that
  category's public products (the same set the category
  page lists — the name-snapshot predicate — in catalog
  order, capped at 8) with the same per-product cards,
  and a 商品一覧を見る link to the full category page.

Semantics and invariants:

- Blog posts only. Standard content pages never expand
  shortcodes; landing pages — the one content-page type
  built to sell — do (see §9).
- Both body formats: the token is plain text, so it works
  in Markdown and HTML posts alike. Expansion happens on
  the rendered HTML, after Markdown rendering / the
  stored (already sanitised) HTML — authors still cannot
  write markup through a token, and the sanitizer story
  is unchanged.
- Grammar: `[[`, `product` or `category`, `:`, one or
  more ASCII digits, `]]`. Anything else (empty digits,
  letters, spaces, an overflowing number) is not a token
  and stays literal text.
- A well-formed token naming a missing, deleted or
  non-public product — or a missing/inactive category, or
  a category with no public products — renders NOTHING:
  the token disappears with no placeholder and no error,
  so a page never leaks that a draft exists.
- Duplicate embeds each render; several distinct products
  in one post each get their own working add control.
- Rendering is two-phase, which is what keeps it CSRF-
  and cache-safe. `render_body` rewrites tokens into
  inert placeholder comments BEFORE the HTML enters the
  render cache, so the cache holds no product data (a
  price or stock change never waits on a post edit) and
  no form. Per request, `expand_embeds` loads every
  referenced product and category in one batched query
  per kind (no N+1) and splices in the server-rendered
  cards. The cards' forms carry no token — like every
  storefront form, they rely on the CSRF middleware
  injecting the requesting session's token on the way
  out — so the same cached body serves every session
  correctly.
- Text surfaces never see tokens: excerpts (stored and
  derived) strip well-formed shortcodes, so cards and the
  RSS feed stay token-free; the feed describes the post,
  it does not sell from it.
- The admin preview is the real render: embeds in the
  unsaved body expand exactly as the published page will
  show them.
- A catalogue read failure degrades to the article
  without its cards (placeholders stripped, error
  logged), never to an error page.

Deliberate non-features:

- No "add the whole category to the cart" button:
  one-click bundle adds are a dark pattern, and stock
  edge cases (some lines capped, some sold out) would
  make the outcome unpredictable. Adds stay per product.
- No admin picker: the post form documents both
  shortcodes with examples; IDs come from the product
  and category lists.

