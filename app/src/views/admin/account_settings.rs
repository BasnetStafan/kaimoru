//! Admin account settings view.
//!
//! One page, two forms: the display name (the sidebar's name,
//! bootstrap-only until now) and the password change. Password
//! values are never carried back into the page — the inputs
//! render empty after every submit, the login form's rule.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::views::admin::totp::AdminTotpSetup;

/// The account settings page.
#[derive(Template)]
#[template(path = "admin/pages/password.html")]
pub struct SettingsView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key: highlights アカウント設定 in the
    /// sidebar while this page is shown.
    pub nav: &'static str,
    /// The admin's login email, shown read-only: it is the
    /// sign-in ID, and this page does not change it.
    pub email: String,
    /// The display name input's value: the stored name, or the
    /// submitted one when the profile form failed validation
    /// (so the operator fixes it instead of retyping).
    pub display_name: String,
    /// Display name field error, when the profile submit failed.
    pub name_error: Option<String>,
    /// The password form's form-level error (a refused current
    /// password, or the new password equalling it).
    pub pw_error: Option<String>,
    /// Current-password field error.
    pub current_error: Option<String>,
    /// New-password field error.
    pub new_error: Option<String>,
    /// Confirmation field error.
    pub confirm_error: Option<String>,
    /// Whether two-factor authentication is enabled for
    /// this admin.
    pub totp_enabled: bool,
    /// When two-factor authentication was enabled, in
    /// display form, when enabled.
    pub totp_enabled_date: Option<String>,
    /// The pending enrolment block, while a setup is in
    /// progress and two-factor is not yet enabled.
    pub totp_setup: Option<AdminTotpSetup>,
    /// The two-factor section's error, when a two-factor
    /// submit failed.
    pub totp_error: Option<String>,
}
