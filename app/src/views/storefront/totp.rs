//! Two-factor settings views: the settings page in its states
//! and the one-time recovery code display. (The login-time
//! challenge view lives in `login.rs`.)

use askama::Template;

use crate::views::storefront::header::Header;

/// The enrolment block of the settings page: everything the
/// member needs to register the pending secret in an
/// authenticator app.
pub struct TotpSetup {
    /// The otpauth URI as an inline SVG QR code. Generated
    /// server-side from percent-encoded parts, so it carries
    /// no unescaped member data.
    pub qr_svg: String,
    /// The pending secret in grouped base32, for manual entry.
    pub grouped_secret: String,
}

/// Renders pages/mypage/totp.html: the two-factor settings
/// page — enrolment offer, the pending setup block, or the
/// enabled state with its actions.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/totp.html")]
pub struct TotpSettingsView {
    /// Shared header state.
    pub header: Header,
    /// Whether two-factor authentication is live.
    pub enabled: bool,
    /// The enablement date label, when enabled.
    pub enabled_date: Option<String>,
    /// The pending setup block, while an enrolment awaits its
    /// confirmation code.
    pub setup: Option<TotpSetup>,
    /// Form-level error to show, if any.
    pub error: Option<String>,
    /// Success notice to show, if any.
    pub notice: Option<String>,
}

/// Renders pages/mypage/totp_recovery.html: a freshly issued
/// recovery code set. This response is the only place the
/// plaintext codes ever appear.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/totp_recovery.html")]
pub struct RecoveryCodesView {
    /// Shared header state.
    pub header: Header,
    /// The plaintext codes, in display form (`XXXXX-XXXXX`).
    pub codes: Vec<String>,
}
