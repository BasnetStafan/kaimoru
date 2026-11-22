//! Shop settings view.
//!
//! One form for the whole `shop_settings` row, grouped the way
//! EC-CUBE groups its 基本設定： identity, contact, footer, SEO.
//! Every value is plain text and renders escaped — an operator
//! typing markup into a field sees the literal characters on
//! the storefront, by design (the form says so).

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::services::admin::shop_settings_service::{
    PointsSettingsInput, ShippingSettingsInput, ShopSettingsInput,
};

/// One checkbox row of the shipping section: a catalogue
/// entry and whether the shop offers it.
pub struct ShippingOption {
    /// Catalogue code the checkbox submits when enabled.
    pub code: &'static str,
    /// Japanese label beside the checkbox.
    pub label: &'static str,
    /// Whether the shop currently offers it (checked).
    pub enabled: bool,
}

/// Builds the slot checkbox rows from the catalogue and the
/// input's disabled set. `none` is always enabled and carries
/// no checkbox semantics — it renders checked and disabled.
pub fn slot_options(input: &ShippingSettingsInput) -> Vec<ShippingOption> {
    crate::models::delivery::DELIVERY_SLOTS
        .iter()
        .map(|slot| ShippingOption {
            code: slot.code,
            label: slot.label,
            enabled: !input.disabled_delivery_slots.iter().any(|c| c == slot.code),
        })
        .collect()
}

/// Builds the carrier checkbox rows from the catalogue and
/// the input's disabled set.
pub fn carrier_options(input: &ShippingSettingsInput) -> Vec<ShippingOption> {
    crate::models::carrier::CARRIERS
        .iter()
        .map(|carrier| ShippingOption {
            code: carrier.code,
            label: carrier.label,
            enabled: !input.disabled_carriers.iter().any(|c| c == carrier.code),
        })
        .collect()
}

/// The shop settings page.
#[derive(Template)]
#[template(path = "admin/pages/shop_settings.html")]
pub struct ShopSettingsView {
    /// The one-time form ticket minted for this render of
    /// the settings page (see `form_ticket_service`); both
    /// section forms on the page carry the same ticket.
    pub form_ticket: String,
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key: highlights 店舗設定 in the
    /// sidebar while this page is shown.
    pub nav: &'static str,
    /// The form values: the stored settings, or the submitted
    /// ones when validation failed (so the operator fixes them
    /// instead of retyping).
    pub values: ShopSettingsInput,
    /// Shop name field error.
    pub shop_name_error: Option<String>,
    /// Tagline field error.
    pub tagline_error: Option<String>,
    /// Shop description field error.
    pub shop_description_error: Option<String>,
    /// Contact email field error.
    pub contact_email_error: Option<String>,
    /// Contact URL field error.
    pub contact_url_error: Option<String>,
    /// Support note field error.
    pub support_note_error: Option<String>,
    /// Footer note field error.
    pub footer_note_error: Option<String>,
    /// Copyright holder field error.
    pub copyright_holder_error: Option<String>,
    /// Top title field error.
    pub top_title_error: Option<String>,
    /// Default meta description field error.
    pub default_meta_description_error: Option<String>,
    /// Mail signature field error.
    pub mail_signature_error: Option<String>,
    /// Error under the registration-number field, when set.
    pub invoice_number_error: Option<String>,
    /// The maintenance message's validation error.
    pub maintenance_message_error: Option<String>,
    /// The payment methods currently in effect, as one joined
    /// label line: a factual hint beside the meta description
    /// field, derived by the controller from the same source
    /// the checkout offers.
    pub payment_methods_label: String,
    /// The shipping section's form values (its own form, its
    /// own save — the back-office rule's per-section shape).
    pub shipping_values: ShippingSettingsInput,
    /// Delivery slot checkbox rows.
    pub slot_options: Vec<ShippingOption>,
    /// Carrier checkbox rows.
    pub carrier_options: Vec<ShippingOption>,
    /// Shipping fee field error.
    pub shipping_flat_error: Option<String>,
    /// Free-shipping threshold field error.
    pub free_shipping_threshold_error: Option<String>,
    /// COD fee field error.
    pub cod_fee_error: Option<String>,
    /// Earliest offset field error.
    pub delivery_earliest_error: Option<String>,
    /// Latest offset field error.
    pub delivery_latest_error: Option<String>,
    /// The points section's form values (its own form, its
    /// own save, like the shipping section).
    pub points_values: PointsSettingsInput,
    /// Earn percent field error.
    pub points_earn_error: Option<String>,
    /// Expiry days field error.
    pub points_expiry_error: Option<String>,
}

impl ShopSettingsView {
    /// Builds the page around one set of form values, with no
    /// field errors.
    pub fn new(
        layout: AdminLayout,
        values: ShopSettingsInput,
        shipping_values: ShippingSettingsInput,
        points_values: PointsSettingsInput,
        payment_methods_label: String,
    ) -> Self {
        let slot_options = slot_options(&shipping_values);
        let carrier_options = carrier_options(&shipping_values);
        Self {
            form_ticket: String::new(),
            layout,
            nav: "shop_settings",
            values,
            shipping_values,
            points_values,
            points_earn_error: None,
            points_expiry_error: None,
            slot_options,
            carrier_options,
            shipping_flat_error: None,
            free_shipping_threshold_error: None,
            cod_fee_error: None,
            delivery_earliest_error: None,
            delivery_latest_error: None,
            payment_methods_label,
            shop_name_error: None,
            tagline_error: None,
            shop_description_error: None,
            contact_email_error: None,
            contact_url_error: None,
            support_note_error: None,
            footer_note_error: None,
            copyright_holder_error: None,
            top_title_error: None,
            default_meta_description_error: None,
            mail_signature_error: None,
            invoice_number_error: None,
            maintenance_message_error: None,
        }
    }
}
