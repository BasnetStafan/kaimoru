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

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{Post, PostCategory, PostView};
use crate::repositories::{post_category_repository, post_repository};
use crate::util::markdown;
use crate::util::xml::escape_xml;

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
pub fn render_body(post: &Post) -> String {
    if let Ok(cache) = RENDER_CACHE.lock()
        && let Some((rendered_from, html)) = cache.get(&post.id)
        && *rendered_from == post.updated_at
    {
        return html.clone();
    }
    let html = if post.is_html() {
        post.body_md.clone()
    } else {
        markdown::render_markdown(&post.body_md)
    };
    if let Ok(mut cache) = RENDER_CACHE.lock() {
        cache.insert(post.id, (post.updated_at, html.clone()));
    }
    html
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
