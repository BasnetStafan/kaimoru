//! Content page view.
//! Data for the one generic long-form page (CMS spec C-2):
//! the system pages and the custom pages render through this
//! single template, so there is exactly one storefront shape
//! for operator-written text.

use askama::Template;

use crate::views::storefront::header::Header;

/// Renders storefront/pages/content.html.
#[derive(Template)]
#[template(path = "storefront/pages/content.html")]
pub struct ContentView {
    /// Shared header state.
    pub header: Header,
    /// Page title (the heading).
    pub title: String,
    /// Rendered body HTML — see the template for why this one
    /// value is allowed through unescaped.
    pub body_html: String,
    /// 最終更新日 label (`YYYY年M月D日`); `None` on the admin
    /// preview, where the body has never been saved.
    pub updated_label: Option<String>,
}
