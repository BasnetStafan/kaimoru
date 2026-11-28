//! Sitemap and robots.txt generation.
//! Builds the two crawler-facing documents from the public
//! catalog: the sitemap lists every page a search engine may
//! index, and robots.txt keeps crawlers out of the private
//! surfaces and points them at the sitemap.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::repositories::{
    content_page_repository, post_category_repository, post_repository, product_repository,
};
use crate::util::xml::escape_xml;

/// One sitemap entry: a path under the store's base URL and,
/// when the record behind it carries one, its last change.
pub struct SitemapEntry {
    /// Path beginning with `/`, appended to the base URL.
    pub path: String,
    /// Last modification stamp, rendered as a date.
    pub lastmod: Option<DateTime<Utc>>,
}

/// Fixed public pages every sitemap lists: the top page, the
/// category index and the fixed information pages. Search is
/// absent on purpose: its URLs carry query parameters, so
/// there is no single canonical address to advertise.
const STATIC_PATHS: [&str; 7] = [
    "/",
    "/categories",
    "/guide",
    "/terms",
    "/privacy",
    "/legal",
    "/faq",
];

/// Path prefixes crawlers must stay out of: the account,
/// transactional and machine surfaces, whose URLs are either
/// private or endless. `/search?` blocks the parameterised
/// search URLs while leaving the bare search page crawlable.
const DISALLOWED_PREFIXES: [&str; 9] = [
    "/admin",
    "/mypage",
    "/cart",
    "/checkout",
    "/login",
    "/signup",
    "/agent",
    "/api",
    "/search?",
];

/// Builds the sitemap document for `base_url`: the fixed
/// pages, every public product inside its publish window,
/// every published custom page, and the blog — its index,
/// every live post and every category archive holding a
/// live post.
pub async fn sitemap_xml(pool: &PgPool, base_url: &str) -> Result<String, AppError> {
    let products = product_repository::sitemap_products(pool)
        .await
        .map_err(|e| AppError::db("sitemap products", e))?;
    let pages = content_page_repository::list_published_custom(pool).await?;
    let posts = post_repository::list_live_for_sitemap(pool).await?;
    let blog_categories = post_category_repository::list_with_live_posts(pool).await?;

    let mut entries: Vec<SitemapEntry> = STATIC_PATHS
        .iter()
        .map(|path| SitemapEntry {
            path: (*path).to_string(),
            lastmod: None,
        })
        .collect();
    entries.extend(products.into_iter().map(|(id, updated_at)| SitemapEntry {
        path: format!("/products/{id}"),
        lastmod: Some(updated_at),
    }));
    entries.extend(pages.into_iter().map(|(slug, updated_at)| SitemapEntry {
        path: format!("/pages/{slug}"),
        lastmod: Some(updated_at),
    }));
    entries.push(SitemapEntry {
        path: "/blog".to_string(),
        lastmod: None,
    });
    entries.extend(posts.into_iter().map(|(slug, updated_at)| SitemapEntry {
        path: format!("/blog/{slug}"),
        lastmod: Some(updated_at),
    }));
    entries.extend(blog_categories.into_iter().map(|category| SitemapEntry {
        path: category.url_path(),
        lastmod: None,
    }));
    Ok(render_sitemap(base_url, &entries))
}

/// Renders entries as a sitemap XML document under
/// `base_url` (a trailing slash on the base is trimmed).
/// Pure, so the document shape is unit-testable without a
/// database.
pub fn render_sitemap(base_url: &str, entries: &[SitemapEntry]) -> String {
    let base = base_url.trim_end_matches('/');
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for entry in entries {
        xml.push_str("  <url>\n    <loc>");
        xml.push_str(&escape_xml(&format!("{base}{}", entry.path)));
        xml.push_str("</loc>\n");
        if let Some(lastmod) = entry.lastmod {
            xml.push_str(&format!(
                "    <lastmod>{}</lastmod>\n",
                lastmod.format("%Y-%m-%d")
            ));
        }
        xml.push_str("  </url>\n");
    }
    xml.push_str("</urlset>\n");
    xml
}

/// Renders robots.txt for `base_url`: everything is open
/// except the disallowed prefixes, and the sitemap is named
/// so crawlers find it without being told out of band.
pub fn render_robots(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let mut text = String::from("User-agent: *\nAllow: /\n");
    for prefix in DISALLOWED_PREFIXES {
        text.push_str(&format!("Disallow: {prefix}\n"));
    }
    text.push_str(&format!("\nSitemap: {base}/sitemap.xml\n"));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed stamp: 2025-01-01 00:00:00 UTC.
    fn stamp() -> DateTime<Utc> {
        DateTime::from_timestamp(1_735_689_600, 0).expect("valid timestamp")
    }

    #[test]
    fn escapes_all_five_xml_specials() {
        assert_eq!(
            escape_xml("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
    }

    #[test]
    fn render_trims_base_slash_and_formats_lastmod() {
        let entries = [SitemapEntry {
            path: "/products/7".to_string(),
            lastmod: Some(stamp()),
        }];
        let xml = render_sitemap("https://shop.example.jp/", &entries);
        assert!(xml.contains("<loc>https://shop.example.jp/products/7</loc>"));
        assert!(xml.contains("<lastmod>2025-01-01</lastmod>"));
        assert!(xml.starts_with("<?xml version=\"1.0\""));
        assert!(xml.trim_end().ends_with("</urlset>"));
    }

    #[test]
    fn render_omits_lastmod_when_absent() {
        let entries = [SitemapEntry {
            path: "/".to_string(),
            lastmod: None,
        }];
        let xml = render_sitemap("https://shop.example.jp", &entries);
        assert!(xml.contains("<loc>https://shop.example.jp/</loc>"));
        assert!(!xml.contains("lastmod"));
    }

    #[test]
    fn render_escapes_specials_in_locations() {
        let entries = [SitemapEntry {
            path: "/pages/a&b".to_string(),
            lastmod: None,
        }];
        let xml = render_sitemap("https://shop.example.jp", &entries);
        assert!(xml.contains("<loc>https://shop.example.jp/pages/a&amp;b</loc>"));
    }

    #[test]
    fn robots_disallows_private_surfaces_and_names_sitemap() {
        let text = render_robots("https://shop.example.jp/");
        assert!(text.starts_with("User-agent: *\nAllow: /\n"));
        assert!(text.contains("Disallow: /admin\n"));
        assert!(text.contains("Disallow: /checkout\n"));
        assert!(text.contains("Sitemap: https://shop.example.jp/sitemap.xml\n"));
    }
}
