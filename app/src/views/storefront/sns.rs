//! SNS login views.
//! Data for the SNS button block, the link-required and email
//! steps of SNS registration, and the My Page SNS settings.

use askama::Template;

use crate::services::sns_providers::SnsProvider;
use crate::services::sns_service;
use crate::views::storefront::header::Header;

/// One provider button of the SNS block.
pub struct SnsButton {
    /// Provider slug, used for the brand CSS class.
    pub provider: &'static str,
    /// The button label.
    pub label: String,
    /// The flow start URL (GET), carrying the validated
    /// return path when there is one.
    pub start_url: String,
}

/// The SNS button block shared by the login and sign-up
/// pages: rendered only when at least one provider is
/// available, which the empty check in the templates reads.
pub struct SnsBlock {
    /// The available providers' buttons, in display order.
    pub buttons: Vec<SnsButton>,
}

/// Builds the SNS button block for a page: one button per
/// available provider, each starting the flow with the
/// page's validated return path.
#[must_use]
pub fn sns_block(next: &str, signup: bool) -> SnsBlock {
    let buttons = sns_service::available_providers()
        .into_iter()
        .map(|provider| {
            let start_url = if next.is_empty() {
                format!("/auth/{}/start", provider.slug())
            } else {
                format!("/auth/{}/start?next={next}", provider.slug())
            };
            SnsButton {
                provider: provider.slug(),
                label: button_label(provider, signup),
                start_url,
            }
        })
        .collect();
    SnsBlock { buttons }
}

/// The button label for one provider: the Yahoo label carries
/// the full service name, the others follow the same shape.
fn button_label(provider: SnsProvider, signup: bool) -> String {
    match (provider, signup) {
        (SnsProvider::Yahoo, false) => "Yahoo! JAPAN IDでログイン".to_string(),
        (SnsProvider::Yahoo, true) => "Yahoo! JAPAN IDで登録".to_string(),
        (_, false) => format!("{}でログイン", provider.display_name()),
        (_, true) => format!("{}で登録", provider.display_name()),
    }
}

/// Renders pages/sns_link_required.html: shown when a
/// provider-verified email already belongs to an account -
/// nothing is created or merged; the owner links the identity
/// after a password sign-in.
#[derive(Template)]
#[template(path = "storefront/pages/sns_link_required.html")]
pub struct SnsLinkRequiredView {
    /// Shared header state.
    pub header: Header,
    /// The account email the provider reported.
    pub email: String,
}

/// Renders pages/signup/sns_email.html: the email entry step
/// for providers that reported no verified email.
#[derive(Template)]
#[template(path = "storefront/pages/signup/sns_email.html")]
pub struct SnsEmailView {
    /// Shared header state.
    pub header: Header,
    /// Error to show, if any.
    pub error: Option<String>,
    /// Email entered before, kept after an error.
    pub email: String,
    /// The provider the visitor arrived from.
    pub provider_name: &'static str,
}

/// One provider row of the SNS settings page.
pub struct SnsRow {
    /// Provider slug, used for the brand CSS class.
    pub provider: &'static str,
    /// Display name.
    pub display_name: &'static str,
    /// Whether this provider is linked to the member.
    pub linked: bool,
    /// The provider email recorded at link time, when one was
    /// reported.
    pub linked_email: Option<String>,
}

/// The link confirmation waiting on the SNS settings page:
/// the provider identity that matched this account's email.
pub struct SnsLinkPendingInfo {
    /// Display name of the provider to link.
    pub provider_name: &'static str,
}

/// Renders pages/mypage/sns.html: the member's SNS links.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/sns.html")]
pub struct SnsSettingsView {
    /// Shared header state.
    pub header: Header,
    /// One row per linked or available provider.
    pub rows: Vec<SnsRow>,
    /// The pending link confirmation, when one is waiting.
    pub link_pending: Option<SnsLinkPendingInfo>,
}
