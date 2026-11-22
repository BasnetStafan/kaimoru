//! Special feature page view.
//! Data for the 特集 landing page the top page banner links
//! to: the section's heading and its curated products.

use askama::Template;

use super::header::Header;
use super::product_card::ProductCard;

/// Renders `pages/special.html` inside the `base.html`
/// layout block.
#[derive(Template)]
#[template(path = "storefront/pages/special.html")]
pub struct SpecialView {
    /// Shared header state.
    pub header: Header,
    /// The feature's heading: the featured section's
    /// display heading (its override, or the built-in
    /// default).
    pub heading: String,
    /// The feature's lead sentence: the featured section's
    /// display description, shared with the top page
    /// banner.
    pub description: String,
    /// The feature's products as cards: the featured pins,
    /// or the staff picks while nothing is pinned.
    pub products: Vec<ProductCard>,
}
