//! Storefront error pages: the styled 404 and 500 views.

use crate::views::storefront::header::Header;
use askama::Template;

/// Renders pagenotfound.html, the styled 404 for unmatched URLs.
#[derive(Template)]
#[template(path = "storefront/pages/pagenotfound.html")]
pub struct PageNotFoundView {
    /// Shared header state.
    pub header: Header,
}

/// Renders error500.html, the styled page a panicking
/// handler degrades to: the visitor gets a
/// recoverable error page instead of a dropped connection.
#[derive(Template)]
#[template(path = "storefront/pages/error500.html")]
pub struct ServerErrorView {
    /// Shared header state.
    pub header: Header,
}
