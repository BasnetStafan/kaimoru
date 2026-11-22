//! Header view.
//! The shared site header state, present in every page view because
//! every template extends the same layout.

use chrono::{Datelike, Utc};

use crate::models::{MenuItem, ShopSettings, User};

/// State the layout header needs: who is logged in, and any one-time
/// message to show under the header.
pub struct Header {
    /// Family name of the logged in member, for the greeting.
    pub user_family_name: Option<String>,
    /// One-time message shown once after a redirect, then gone.
    pub welcome_message: Option<String>,
    /// Total quantity in the visitor's cart, for the header badge.
    pub cart_count: i64,
    /// Coupon band text, built from the active store coupon.
    pub band_text: String,
    /// Coupon band discount, the band's right-hand element;
    /// empty whenever `band_text` is.
    pub band_amount: String,
    /// The announcement bar (CMS C-3) when a live one exists:
    /// the top page renders it at the head of its content.
    pub announcement: Option<AnnouncementBar>,
    /// The header menu's visible links (CMS C-4), in render
    /// order; empty renders no menu row at all.
    pub header_menu: Vec<MenuLink>,
    /// The footer menu's visible links (CMS C-4), in render
    /// order.
    pub footer_menu: Vec<MenuLink>,
    /// The shop's operator-edited settings (logo, titles, footer).
    pub shop: ShopSettings,
    /// The footer's copyright line, ready to render:
    /// `© <JST year> <holder>`.
    pub copyright_text: String,
    /// Asset cache-busting fingerprint, for the layout's CSS/JS URLs.
    pub asset_fingerprint: &'static str,
}

/// The announcement bar's render data (CMS C-3): the
/// operator's general-purpose message — holiday shipping,
/// maintenance, a campaign — shown at the head of the
/// store top page while its slot is live. Plain text,
/// escaped at render like everything else.
pub struct AnnouncementBar {
    /// The message text.
    pub text: String,
    /// Link label; empty together with the URL when the bar
    /// has no link.
    pub link_label: String,
    /// Link URL; empty together with the label when the bar
    /// has no link.
    pub link_url: String,
}

impl AnnouncementBar {
    /// Whether the bar carries its optional link: label and
    /// URL are a pair, set together or not at all.
    pub fn has_link(&self) -> bool {
        !self.link_label.is_empty() && !self.link_url.is_empty()
    }
}

/// One rendered menu link (CMS spec C-4): the operator's
/// label and target, with externality precomputed — an
/// `https:` link renders with `rel="noopener"`, an internal
/// one exactly as the layout's links always have.
pub struct MenuLink {
    /// Link text.
    pub label: String,
    /// Link target.
    pub url: String,
    /// Whether the target is an external `https:` URL.
    pub is_external: bool,
}

impl From<&MenuItem> for MenuLink {
    /// The render data for one stored menu row.
    fn from(item: &MenuItem) -> Self {
        Self {
            label: item.label.clone(),
            url: item.url.clone(),
            is_external: item.is_external(),
        }
    }
}

/// Builds the footer's copyright line for the given settings:
/// the current year on the shop's calendar (JST) and the holder
/// (the shop name when no holder is set).
fn copyright_text(shop: &ShopSettings) -> String {
    let year = Utc::now().with_timezone(&crate::models::jst::jst()).year();
    format!("© {year} {} All Rights Reserved", shop.copyright_holder())
}

impl Header {
    /// A header for a guest, with no message.
    ///
    /// The shop settings are the fallback values: the 404
    /// fallback builds this header with no database work behind
    /// it (see the router), so it cannot read the settings row.
    pub fn guest() -> Self {
        let shop = ShopSettings::fallback();
        let copyright = copyright_text(&shop);
        Self {
            user_family_name: None,
            welcome_message: None,
            cart_count: 0,
            band_text: String::new(),
            band_amount: String::new(),
            announcement: None,
            header_menu: Vec::new(),
            footer_menu: MenuItem::fallback_footer()
                .iter()
                .map(MenuLink::from)
                .collect(),
            shop,
            copyright_text: copyright,
            asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
        }
    }

    /// A header for the given user (or guest) with optional
    /// one-time messages.
    pub fn build(
        user: Option<&User>,
        welcome: Option<String>,
        cart_count: i64,
        band_text: String,
        band_amount: String,
        shop: ShopSettings,
    ) -> Self {
        let copyright = copyright_text(&shop);
        Self {
            user_family_name: user.map(|u| u.family_name.clone()),
            welcome_message: welcome,
            cart_count,
            band_text,
            band_amount,
            announcement: None,
            header_menu: Vec::new(),
            footer_menu: MenuItem::fallback_footer()
                .iter()
                .map(MenuLink::from)
                .collect(),
            shop,
            copyright_text: copyright,
            asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
        }
    }
}
