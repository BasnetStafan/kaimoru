//! View structs for the product search pages.

use askama::Template;

use super::header::Header;
use super::product_card::ProductCard;

/// Renders pages/search.html, the search results page.
#[derive(Template)]
#[template(path = "storefront/pages/search.html")]
pub struct SearchView {
    /// Shared header state.
    pub header: Header,
    /// The raw query text, shown in the heading and kept in links.
    pub query: String,
    /// The active category filter; empty for all categories.
    pub category: String,
    /// The active tag filter's slug; empty when not filtering
    /// by tag.
    pub tag_slug: String,
    /// The active tag filter's display label; `None` when not
    /// filtering by tag (or the slug names no tag).
    pub tag_label: Option<String>,
    /// The active sort key (allowlisted in the search service).
    pub sort: String,
    /// Whether the in-stock-only filter is on.
    pub in_stock: bool,
    /// The lower price bound as typed (blank for none), kept
    /// for the input and the pagination links.
    pub min_price: String,
    /// The upper price bound as typed (blank for none).
    pub max_price: String,
    /// Validation error for the lower bound, when it failed.
    pub min_price_error: Option<String>,
    /// Validation error for the upper bound, when it failed.
    pub max_price_error: Option<String>,
    /// The products on this page.
    pub products: Vec<ProductCard>,
    /// Total matches across all pages.
    pub total: i64,
    /// Current page number, 1-based.
    pub page: i64,
    /// Total number of pages.
    pub total_pages: i64,
    /// All category names, for the filter select.
    pub categories: Vec<String>,
    /// A form-level message (for example when search is busy).
    pub error_message: Option<String>,
}

/// Renders parts/search_suggest.html, the suggestion fragment that
/// htmx swaps under the header search box.
#[derive(Template)]
#[template(path = "storefront/parts/search_suggest.html")]
pub struct SuggestView {
    /// Suggested product names, best sellers first.
    pub names: Vec<String>,
}
