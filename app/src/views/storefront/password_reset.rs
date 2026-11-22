//! Password reset views.
//! Data for the password reset flow templates (request, verify,
//! new password steps). Unlike the sign-up views, none of them
//! carries the submitted email — masked or otherwise: whether an
//! address has an account must not be inferable from anything
//! these pages show (anti-enumeration).

use askama::Template;

use crate::views::storefront::header::Header;

/// Renders pages/password_reset/request.html, the email entry step.
#[derive(Template)]
#[template(path = "storefront/pages/password_reset/request.html")]
pub struct ResetRequestView {
    /// Shared header state.
    pub header: Header,
    /// Error to show, if any.
    pub error: Option<String>,
    /// Email entered before, kept after an error.
    pub email: String,
}

/// Renders pages/password_reset/verify.html, the code entry step.
#[derive(Template)]
#[template(path = "storefront/pages/password_reset/verify.html")]
pub struct ResetVerifyView {
    /// Shared header state.
    pub header: Header,
    /// Error to show, if any.
    pub error: Option<String>,
    /// Notice to show, for example after a resend.
    pub notice: Option<String>,
}

/// Renders pages/password_reset/password.html, the new password step.
#[derive(Template)]
#[template(path = "storefront/pages/password_reset/password.html")]
pub struct ResetPasswordView {
    /// Shared header state.
    pub header: Header,
    /// Error to show, if any.
    pub error: Option<String>,
}
