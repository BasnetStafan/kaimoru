//! Blog views (CMS blog spec, docs/specs/CMS_SPEC.md).
//!
//! The index (and its category-archive shape) and the post
//! page. Cards carry exactly what a reader scans: cover,
//! date, category, title, excerpt.

use askama::Template;

use crate::views::storefront::header::Header;

/// One card on the blog index / archive.
pub struct BlogCard {
    /// Post URL (`/blog/{slug}`).
    pub url: String,
    /// Cover image path, when the post has one.
    pub cover_image_path: Option<String>,
    /// Publication date (`YYYY年M月D日`).
    pub date_label: String,
    /// Category display name, when categorised.
    pub category_name: Option<String>,
    /// Category archive URL, when categorised.
    pub category_url: Option<String>,
    /// Post title.
    pub title: String,
    /// Stored or derived excerpt.
    pub excerpt: String,
}

/// Renders storefront/pages/blog/index.html: the post index
/// and, with `category_name` set, a category archive — one
/// template, so the two lists can never drift apart.
#[derive(Template)]
#[template(path = "storefront/pages/blog/index.html")]
pub struct BlogIndexView {
    /// Shared header state.
    pub header: Header,
    /// The archive's category name; `None` on the plain
    /// index (the heading and title differ).
    pub category_name: Option<String>,
    /// This page's cards.
    pub cards: Vec<BlogCard>,
    /// Total live posts in the list (all pages).
    pub total: i64,
    /// Current page number, 1-based.
    pub page: i64,
    /// Total page count (at least 1).
    pub total_pages: i64,
    /// Pagination link base: the path plus `?` — page links
    /// append `page=N`.
    pub page_base: String,
}

/// Renders storefront/pages/blog/post.html.
#[derive(Template)]
#[template(path = "storefront/pages/blog/post.html")]
pub struct BlogPostView {
    /// Shared header state.
    pub header: Header,
    /// Post title (the heading).
    pub title: String,
    /// Publication date (`YYYY年M月D日`).
    pub date_label: String,
    /// The author's display name (the byline).
    pub author_name: String,
    /// Category display name, when categorised.
    pub category_name: Option<String>,
    /// Category archive URL, when categorised.
    pub category_url: Option<String>,
    /// Cover image path, when the post has one.
    pub cover_image_path: Option<String>,
    /// Rendered body HTML — see the template for why this
    /// one value is allowed through unescaped.
    pub body_html: String,
    /// The resolved meta description (post → excerpt →
    /// shop default; the controller resolves the chain).
    pub meta_description: String,
}
