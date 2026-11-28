//! Info views.
//! Data for the one info page that stays a static template:
//! the withdrawal farewell page. The long-form shop pages
//! (guide, terms, privacy, legal) are database content now and
//! render through `crate::views::storefront::content_page`.

use askama::Template;

use crate::views::storefront::header::Header;

/// Renders pages/info/withdrawn.html: the public farewell
/// page after a withdrawal (Account Deletion spec 3.5).
/// Static by design — it carries no per-member data.
#[derive(Template)]
#[template(path = "storefront/pages/info/withdrawn.html")]
pub struct WithdrawnView {
    /// Shared header state.
    pub header: Header,
}
