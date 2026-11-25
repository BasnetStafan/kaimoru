//! Admin two-factor views: the
//! standalone login challenge page and the recovery-code
//! display. The settings section itself rides on the
//! account settings view (`account_settings::SettingsView`).

use askama::Template;

use crate::controllers::admin::AdminLayout;

/// The admin login challenge: a standalone page like the
/// login form (no sidebar — the operator is not signed in
/// until the challenge completes).
#[derive(Template)]
#[template(path = "admin/pages/login_totp.html")]
pub struct TotpChallengeView {
    /// The generic code error, when a submission failed.
    pub error: Option<String>,
    /// Asset cache-busting fingerprint, for the page's CSS URLs.
    pub asset_fingerprint: &'static str,
}

/// The freshly issued recovery codes, shown exactly once.
#[derive(Template)]
#[template(path = "admin/pages/totp_recovery.html")]
pub struct AdminRecoveryCodesView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key (the account settings page).
    pub nav: &'static str,
    /// The plaintext codes, in display form (`XXXXX-XXXXX`).
    pub codes: Vec<String>,
}

/// The pending enrolment block of the settings page: the
/// QR (an inline SVG generated server-side) and the
/// grouped secret for manual entry.
pub struct AdminTotpSetup {
    /// The otpauth QR code as an inline SVG document.
    pub qr_svg: String,
    /// The secret grouped in fours for manual entry.
    pub grouped_secret: String,
}
