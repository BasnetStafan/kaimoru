//! Section listing view.
//! Renders one top page section's full listing: the page
//! its row's see-more link leads to, paged like the search
//! page.

use askama::Template;

use super::{header::Header, product_card::ProductCard};

/// Renders section.html: one section's products, one page
/// at a time.
#[derive(Template)]
#[template(path = "storefront/pages/section.html")]
pub struct SectionView {
    /// Shared header state.
    pub header: Header,
    /// The section's display heading.
    pub heading: String,
    /// This page's product cards.
    pub products: Vec<ProductCard>,
    /// The current page number (1-based).
    pub page: i64,
    /// The number of pages at the current page size.
    pub total_pages: i64,
    /// The section key, for the pager links.
    pub key: String,
}
