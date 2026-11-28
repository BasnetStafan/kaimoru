//! Admin login view.
//!
//! A standalone page — no sidebar layout — because the
//! operator is not signed in yet. The credential error is one
//! generic message for every failure (unknown email, wrong
//! password): the page never says which part was wrong. Empty
//! fields instead get their own per-field messages, and a
//! throttled attempt gets its distinct notice.

use askama::Template;

/// The admin login form.
#[derive(Template)]
#[template(path = "admin/pages/login.html")]
pub struct LoginView {
    /// Email, preserved across a failed attempt.
    pub email: String,
    /// The generic credential error, when credentials failed.
    pub error: Option<String>,
    /// The notice shown while login attempts are throttled.
    pub throttled: Option<String>,
    /// Email field error (empty submission), when present.
    pub email_error: Option<String>,
    /// Password field error (empty submission), when present.
    pub password_error: Option<String>,
    /// Asset cache-busting fingerprint, for the page's CSS URLs.
    pub asset_fingerprint: &'static str,
}

impl LoginView {
    /// A fresh form (first visit, or after logout).
    pub fn fresh() -> LoginView {
        LoginView {
            email: String::new(),
            error: None,
            throttled: None,
            email_error: None,
            password_error: None,
            asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
        }
    }
}
