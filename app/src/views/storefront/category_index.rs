//! Category index view.
//! Renders the full category list the top page nav's
//! see-more link leads to.

use askama::Template;

use super::header::Header;

/// Renders categories.html: every navigation category as
/// a link to its product list.
#[derive(Template)]
#[template(path = "storefront/pages/categories.html")]
pub struct CategoryIndexView {
    /// Shared header state.
    pub header: Header,
    /// Every navigation category name, in nav order.
    pub categories: Vec<String>,
}
