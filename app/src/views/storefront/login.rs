//! Login view.
//! Data for the login page template.

use askama::Template;

use crate::views::storefront::header::Header;

/// Renders login_totp.html: the two-factor challenge step of
/// the login, after the password has verified.
#[derive(Template)]
#[template(path = "storefront/pages/login_totp.html")]
pub struct LoginTotpView {
    /// Shared header state.
    pub header: Header,
    /// Error to show, if any.
    pub error: Option<String>,
}

/// Renders login.html.
#[derive(Template)]
#[template(path = "storefront/pages/login.html")]
pub struct LoginView {
    /// Shared header state.
    pub header: Header,
    /// Error to show, if any.
    pub error: Option<String>,
    /// Email entered before, kept after an error.
    pub email: String,
    /// Validated return path after login; empty for the default.
    pub next: String,
    /// The SNS button block (empty when no provider is
    /// available, which hides the block).
    pub sns: crate::views::storefront::sns::SnsBlock,
}
