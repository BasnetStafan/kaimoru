//! Storefront blog reads (CMS blog spec,
//! docs/specs/CMS_SPEC.md).
//!
//! Every public surface — the index, the post page, the
//! category archive, the RSS feed — reads through here, and
//! every read carries the one liveness predicate (see
//! [`crate::models::Post::is_live`]): a draft, a scheduled
//! post and an unknown slug are the same `None`, which is
//! what makes their 404s indistinguishable. Bodies render
//! through the content pages' cache, keyed on the post's
//! own id space — see [`render_body`].
//!
//! Embed shortcodes (`[[product:…]]`, `[[category:…]]`,
//! CMS_SPEC §8) ride that cache as inert placeholder
//! comments: the cached HTML holds no product data and no
//! form, and [`expand_embeds`] splices the per-request
//! cards in afterwards — see its own docs for why the
//! split is the CSRF-safe shape.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use askama::Template;

use crate::error::AppError;
use crate::models::{Post, PostCategory, PostView, Product, ProductCategory};
use crate::repositories::{
    category_repository, post_category_repository, post_repository, product_repository,
};
use crate::util::markdown;
use crate::util::shortcode::{self, EmbedKind};
use crate::util::xml::escape_xml;
use crate::views::storefront::blog_embed::{
    BlogEmbedCard, BlogEmbedCategoryView, BlogEmbedProductView,
};

/// How many posts the RSS feed carries: the newest
/// articles, not the archive.
pub const FEED_LIMIT: i64 = 20;

/// Returns the live post for a slug, or `None` when the
/// slug is unknown, a draft, or still scheduled.
///
/// # Errors
/// Returns a store error when the read fails.
pub async fn live_post(pool: &PgPool, slug: &str) -> Result<Option<PostView>, AppError> {
    post_repository::find_live_by_slug(pool, slug).await
}

/// Returns one page of live posts with the total;
/// `category` narrows to one category's archive.
///
/// # Errors
/// Returns a store error when either read fails.
pub async fn live_posts(
    pool: &PgPool,
    category: Option<i64>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<PostView>, i64), AppError> {
    let rows = post_repository::list_live(pool, category, limit, offset).await?;
    let total = post_repository::count_live(pool, category).await?;
    Ok((rows, total))
}

/// Returns the category for an archive slug, or `None`.
///
/// # Errors
/// Returns a store error when the read fails.
pub async fn category_by_slug(pool: &PgPool, slug: &str) -> Result<Option<PostCategory>, AppError> {
    post_category_repository::find_by_slug(pool, slug).await
}

/// Rendered bodies by post ID, each entry stamped with the
/// `updated_at` it was rendered from — the content pages'
/// render-cache discipline, in the posts' own id space: a
/// save stamps a fresh `updated_at`, so an entry matches
/// only while the row is unchanged.
type RenderEntry = (DateTime<Utc>, String);

static RENDER_CACHE: LazyLock<Mutex<HashMap<i64, RenderEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Renders a post's body to HTML, served from the render
/// cache while the row is unchanged. A Markdown body goes
/// through [`crate::util::markdown`]; an HTML body is
/// served as stored — the save path sanitised it, and that
/// invariant is what makes serving it unescaped safe.
///
/// Embed shortcodes come back as placeholder comments
/// ([`crate::util::shortcode::mark_embeds`]): the cached
/// HTML deliberately holds no product data, so a price or
/// stock change never waits on a post edit, and no form —
/// so nothing session-shaped can ever enter the cache.
/// Serving surfaces expand the placeholders per request
/// with [`expand_embeds`] (or [`render_body_expanded`]).
pub fn render_body(post: &Post) -> String {
    if let Ok(cache) = RENDER_CACHE.lock()
        && let Some((rendered_from, html)) = cache.get(&post.id)
        && *rendered_from == post.updated_at
    {
        return html.clone();
    }
    let rendered = if post.is_html() {
        post.body_md.clone()
    } else {
        markdown::render_markdown(&post.body_md)
    };
    let html = shortcode::mark_embeds(&rendered);
    if let Ok(mut cache) = RENDER_CACHE.lock() {
        cache.insert(post.id, (post.updated_at, html.clone()));
    }
    html
}

/// How many products one category embed showcases: a
/// preview of the category page, not the page itself.
pub const CATEGORY_EMBED_LIMIT: usize = 8;

/// Splices the embed cards into a marked body (see
/// [`render_body`]), loading every referenced catalogue in
/// one batched query per kind. A product or category that
/// is missing or not public expands to nothing — the
/// placeholder simply goes away, so a body can never leak
/// that a draft exists. The cards' add-to-cart forms carry
/// no token, exactly like the catalog cards': the CSRF
/// middleware injects the requesting session's token into
/// every form on the way out, which is what makes sharing
/// this markup across sessions safe.
///
/// A catalogue read failure costs the cards, never the
/// article: the error is logged and the body is served
/// with its placeholders stripped.
pub async fn expand_embeds(pool: &PgPool, marked_html: &str) -> String {
    let refs = shortcode::embed_refs(marked_html);
    if refs.is_empty() {
        return marked_html.to_string();
    }
    let product_ids: Vec<i64> = refs
        .iter()
        .filter(|r| r.kind == EmbedKind::Product)
        .map(|r| r.id)
        .collect();
    let category_ids: Vec<i64> = refs
        .iter()
        .filter(|r| r.kind == EmbedKind::Category)
        .map(|r| r.id)
        .collect();
    let products: HashMap<i64, Product> = if product_ids.is_empty() {
        HashMap::new()
    } else {
        match product_repository::catalog_products(pool, &product_ids).await {
            Ok(rows) => rows.into_iter().map(|p| (p.id, p)).collect(),
            Err(e) => {
                tracing::error!("blog embed product load failed: {e}");
                return shortcode::strip_placeholders(marked_html);
            }
        }
    };
    let categories: HashMap<i64, ProductCategory> =
        match category_repository::find_active_by_ids(pool, &category_ids).await {
            Ok(rows) => rows.into_iter().map(|c| (c.id, c)).collect(),
            Err(e) => {
                tracing::error!("blog embed category load failed: {e}");
                return shortcode::strip_placeholders(marked_html);
            }
        };
    // The showcases' products ride one more batched query,
    // keyed by category name — the category page's own
    // predicate — and capped per category in id order.
    let mut showcases: HashMap<String, Vec<BlogEmbedCard>> = HashMap::new();
    if !categories.is_empty() {
        let names: Vec<String> = categories.values().map(|c| c.name.clone()).collect();
        match product_repository::catalog_products_for_categories(pool, &names).await {
            Ok(rows) => {
                for product in rows {
                    let cards = showcases.entry(product.category.clone()).or_default();
                    if cards.len() < CATEGORY_EMBED_LIMIT {
                        cards.push(BlogEmbedCard::new(product));
                    }
                }
            }
            Err(e) => {
                tracing::error!("blog embed showcase load failed: {e}");
                return shortcode::strip_placeholders(marked_html);
            }
        }
    }
    let mut out = marked_html.to_string();
    for embed in &refs {
        let replacement = match embed.kind {
            EmbedKind::Product => products
                .get(&embed.id)
                .map(render_product_card)
                .unwrap_or_default(),
            EmbedKind::Category => categories
                .get(&embed.id)
                .and_then(|category| showcases.remove(&category.name))
                .filter(|cards| !cards.is_empty())
                .map(|cards| render_category_showcase(&categories[&embed.id], cards))
                .unwrap_or_default(),
        };
        out = out.replace(&shortcode::placeholder(embed.kind, embed.id), &replacement);
    }
    out
}

/// Renders a post's body and expands its embeds in one
/// call: the post page's and the admin preview's read.
pub async fn render_body_expanded(pool: &PgPool, post: &Post) -> String {
    expand_embeds(pool, &render_body(post)).await
}

/// Renders one product embed's card; a render failure
/// (never expected — the template is static) degrades to
/// no card, like a missing product.
fn render_product_card(product: &Product) -> String {
    let view = BlogEmbedProductView {
        card: BlogEmbedCard::new(product.clone()),
    };
    match view.render() {
        Ok(html) => html,
        Err(e) => {
            tracing::error!("blog embed card render failed: {e}");
            String::new()
        }
    }
}

/// Renders one category embed's showcase from its capped
/// cards; same failure posture as [`render_product_card`].
fn render_category_showcase(category: &ProductCategory, cards: Vec<BlogEmbedCard>) -> String {
    let view = BlogEmbedCategoryView {
        name: category.name.clone(),
        cards,
    };
    match view.render() {
        Ok(html) => html,
        Err(e) => {
            tracing::error!("blog embed showcase render failed: {e}");
            String::new()
        }
    }
}

/// Builds the RSS 2.0 feed document for `base_url` from
/// the newest [`FEED_LIMIT`] live posts.
///
/// # Errors
/// Returns a store error when the feed's posts or the
/// shop settings cannot be read.
pub async fn feed_xml(pool: &PgPool, base_url: &str) -> Result<String, AppError> {
    let posts = post_repository::list_live_for_feed(pool, FEED_LIMIT).await?;
    let shop = crate::services::shop_settings_service::load(pool).await;
    Ok(render_feed(base_url, &shop.shop_name, &posts, Utc::now()))
}

/// Renders the feed document. Pure, so the shape is
/// unit-testable without a database. Links are absolute
/// under `base_url` (a trailing slash is trimmed), the
/// same base rule the sitemap uses.
pub fn render_feed(
    base_url: &str,
    shop_name: &str,
    posts: &[PostView],
    built_at: DateTime<Utc>,
) -> String {
    let base = base_url.trim_end_matches('/');
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rss version=\"2.0\">\n<channel>\n",
    );
    xml.push_str(&format!("<title>{}</title>\n", escape_xml(shop_name)));
    xml.push_str(&format!(
        "<link>{}</link>\n",
        escape_xml(&format!("{base}/blog"))
    ));
    xml.push_str(&format!(
        "<description>{}のブログ</description>\n",
        escape_xml(shop_name)
    ));
    xml.push_str("<language>ja</language>\n");
    xml.push_str(&format!(
        "<lastBuildDate>{}</lastBuildDate>\n",
        rfc822_label(built_at)
    ));
    for view in posts {
        let post = &view.post;
        let link = format!("{base}{}", post.url_path());
        xml.push_str("<item>\n");
        xml.push_str(&format!("<title>{}</title>\n", escape_xml(&post.title)));
        xml.push_str(&format!("<link>{}</link>\n", escape_xml(&link)));
        xml.push_str(&format!(
            "<guid isPermaLink=\"true\">{}</guid>\n",
            escape_xml(&link)
        ));
        if let Some(at) = post.published_at {
            xml.push_str(&format!("<pubDate>{}</pubDate>\n", rfc822_label(at)));
        }
        xml.push_str(&format!(
            "<description>{}</description>\n",
            escape_xml(&post.excerpt_or_derived())
        ));
        xml.push_str("</item>\n");
    }
    xml.push_str("</channel>\n</rss>\n");
    xml
}

/// Formats an instant as an RSS `pubDate` (RFC 822, GMT):
/// `Sat, 28 Nov 2026 12:00:00 GMT`.
pub fn rfc822_label(at: DateTime<Utc>) -> String {
    at.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed stamp: 2026-11-28 12:00:00 UTC (a Saturday).
    fn stamp() -> DateTime<Utc> {
        DateTime::from_timestamp(1_795_867_200, 0).expect("valid timestamp")
    }

    #[test]
    fn rfc822_label_is_gmt_english() {
        assert_eq!(rfc822_label(stamp()), "Sat, 28 Nov 2026 12:00:00 GMT");
    }

    /// One live post view for the feed renderer.
    fn view(title: &str, excerpt: &str) -> PostView {
        PostView {
            post: Post {
                id: 7,
                slug: "hello".to_string(),
                title: title.to_string(),
                body_md: "本文".to_string(),
                format: "markdown".to_string(),
                excerpt: excerpt.to_string(),
                meta_description: String::new(),
                cover_image_path: None,
                category_id: None,
                author_admin_id: 1,
                status: "published".to_string(),
                published_at: Some(stamp()),
                created_at: stamp(),
                updated_at: stamp(),
            },
            category_name: None,
            category_slug: None,
            author_name: "店長".to_string(),
        }
    }

    /// A post whose body carries embeds, in its own id
    /// space so the shared render cache cannot collide
    /// with another test's entry.
    fn embed_post() -> Post {
        Post {
            id: 987_001,
            slug: "embeds".to_string(),
            title: "埋め込み".to_string(),
            body_md: "おすすめ [[product:5]] と [[category:2]] です".to_string(),
            format: "markdown".to_string(),
            excerpt: String::new(),
            meta_description: String::new(),
            cover_image_path: None,
            category_id: None,
            author_admin_id: 1,
            status: "published".to_string(),
            published_at: Some(stamp()),
            created_at: stamp(),
            updated_at: stamp(),
        }
    }

    #[test]
    fn render_body_marks_embeds_and_caches_no_form() {
        let html = render_body(&embed_post());
        assert!(html.contains("<!--blog-embed:product:5-->"));
        assert!(html.contains("<!--blog-embed:category:2-->"));
        assert!(!html.contains("[[product:5]]"));
        // The cached markup is session-free by construction:
        // no form, no token-shaped field can ever be in it.
        assert!(!html.contains("<form"));
        assert!(!html.contains("csrf"));
        // A second read is the cache hit: identical output.
        assert_eq!(render_body(&embed_post()), html);
    }

    #[test]
    fn feed_renders_channel_and_items_with_absolute_links() {
        let xml = render_feed(
            "https://shop.example.jp/",
            "テスト店",
            &[view("初投稿", "要約です")],
            stamp(),
        );
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<rss version=\"2.0\">"));
        assert!(xml.contains("<link>https://shop.example.jp/blog</link>"));
        assert!(xml.contains("<title>初投稿</title>"));
        assert!(xml.contains("<link>https://shop.example.jp/blog/hello</link>"));
        assert!(xml.contains("<pubDate>Sat, 28 Nov 2026 12:00:00 GMT</pubDate>"));
        assert!(xml.contains("<description>要約です</description>"));
        assert!(xml.trim_end().ends_with("</rss>"));
    }

    #[test]
    fn feed_escapes_operator_text() {
        let xml = render_feed(
            "https://shop.example.jp",
            "A&B",
            &[view("<題>", "")],
            stamp(),
        );
        assert!(xml.contains("<title>A&amp;B</title>"));
        assert!(xml.contains("<title>&lt;題&gt;</title>"));
        // The empty excerpt falls back to the derived one:
        // the body 本文.
        assert!(xml.contains("<description>本文</description>"));
    }
}
