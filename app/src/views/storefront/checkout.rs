//! View structs for the checkout pages.

use askama::Template;

use super::header::Header;
use crate::models::order::OrderLineDetail;
use crate::models::{CartLine, MemberCoupon, Order};
use crate::services::cart_service::CartTotals;
use crate::services::order_service::CheckoutFlow;
use crate::validation::FieldError;

/// Per-field validation messages for the checkout form, shown
/// under their fields like the sign-up details step.
#[derive(Default)]
pub struct CheckoutErrors {
    /// Error for the email field.
    pub email: Option<String>,
    /// Error for the family name field.
    pub family_name: Option<String>,
    /// Error for the given name field.
    pub given_name: Option<String>,
    /// Error for the postal code field.
    pub postal_code: Option<String>,
    /// Error for the prefecture field.
    pub prefecture: Option<String>,
    /// Error for the city field.
    pub city: Option<String>,
    /// Error for the street address field.
    pub address_line: Option<String>,
    /// Error for the phone field.
    pub phone: Option<String>,
    /// Error for the payment method choice (the konbini limit).
    pub payment_method: Option<String>,
    /// Error for the delivery date field.
    pub delivery_date: Option<String>,
    /// Error for the delivery slot field.
    pub delivery_slot: Option<String>,
    /// Error for the gift noshi select.
    pub noshi: Option<String>,
    /// Error for the gift name inscription.
    pub noshi_name: Option<String>,
    /// Error for the gift message card text.
    pub gift_message: Option<String>,
}

impl CheckoutErrors {
    /// Builds the per-field errors from the service's list.
    pub fn from_field_errors(list: &[FieldError]) -> Self {
        let mut errors = Self::default();
        for e in list {
            let slot = match e.field {
                "email" => &mut errors.email,
                "family_name" => &mut errors.family_name,
                "given_name" => &mut errors.given_name,
                "postal_code" => &mut errors.postal_code,
                "prefecture" => &mut errors.prefecture,
                "city" => &mut errors.city,
                "address_line" => &mut errors.address_line,
                "phone" => &mut errors.phone,
                "payment_method" => &mut errors.payment_method,
                "delivery_date" => &mut errors.delivery_date,
                "delivery_slot" => &mut errors.delivery_slot,
                "noshi" => &mut errors.noshi,
                "noshi_name" => &mut errors.noshi_name,
                "gift_message" => &mut errors.gift_message,
                _ => continue,
            };
            if slot.is_none() {
                *slot = Some(e.message.clone());
            }
        }
        errors
    }
}

/// Shared money formatting for the checkout views.
pub trait TotalsDisplay {
    /// The totals these views display.
    fn totals(&self) -> &CartTotals;
    /// Product subtotal formatted.
    fn formatted_subtotal(&self) -> String {
        crate::pricing::display_price(self.totals().subtotal_yen)
    }
    /// Discount formatted.
    fn formatted_discount(&self) -> String {
        crate::pricing::display_price(self.totals().discount_yen)
    }
    /// Shipping formatted.
    fn formatted_shipping(&self) -> String {
        crate::pricing::display_price(self.totals().shipping_yen)
    }
    /// The shipping discount (a free-shipping coupon's
    /// waived fee) formatted.
    fn formatted_shipping_discount(&self) -> String {
        crate::pricing::display_price(self.totals().shipping_discount_yen)
    }
    /// COD fee formatted.
    fn formatted_cod_fee(&self) -> String {
        crate::pricing::display_price(self.totals().cod_fee_yen)
    }
    /// Grand total formatted.
    fn formatted_total(&self) -> String {
        crate::pricing::display_price(self.totals().total_yen)
    }
    /// Total tax contained in the grand total, formatted.
    fn formatted_tax_total(&self) -> String {
        crate::pricing::display_price(self.totals().tax_total_yen)
    }
}

/// One selectable delivery date of the picker: the window
/// dates generated at render, each with its weekday label.
pub struct DeliveryDateOption {
    /// Posted value (`YYYY-MM-DD`).
    pub value: String,
    /// Picker label, for example `11月7日（金）`.
    pub label: String,
    /// Whether this date is the flow's current choice.
    pub selected: bool,
}

/// One selectable delivery slot of the picker, from the model
/// catalogue, no preference first.
pub struct DeliverySlotOption {
    /// Posted value (the slot code).
    pub code: String,
    /// Japanese label.
    pub label: String,
    /// Whether this slot is the flow's current choice.
    pub selected: bool,
}

/// One selectable payment method on the payment step.
pub struct PaymentMethodOption {
    /// The posted value (the method slug).
    pub slug: &'static str,
    /// The customer-facing name.
    pub label: &'static str,
    /// The note under the name.
    pub note: String,
    /// Whether this method is the flow's current choice.
    pub checked: bool,
}

/// Renders pages/checkout/delivery.html: delivery and coupon
/// in one form (payment choice is its own step).
#[derive(Template)]
#[template(path = "storefront/pages/checkout/delivery.html")]
pub struct CheckoutView {
    /// Shared header state.
    pub header: Header,
    /// The form values (account prefill, flow state, or input).
    pub values: CheckoutFlow,
    /// Per-field validation errors.
    pub errors: CheckoutErrors,
    /// Whether the visitor is a member (email comes from the
    /// account and is not an input).
    pub is_member: bool,
    /// The cart lines being bought (purchasable only).
    pub lines: Vec<CartLine>,
    /// Totals including any applied coupon.
    pub totals: CartTotals,
    /// The applied coupons (one per class), with amounts.
    pub applied_coupons: Vec<crate::models::AppliedCoupon>,
    /// The member's held, unused coupons, offered in the
    /// coupon dropdown (empty for guests, for members
    /// without one, and once a coupon is applied).
    pub held_coupons: Vec<MemberCoupon>,
    /// Generic coupon error, when the last apply failed.
    pub coupon_error: Option<String>,
    /// A form-level message (for example a stock shortage).
    pub error_message: Option<String>,
    /// Prefecture options for the select.
    pub prefectures: &'static [&'static str],
    /// Delivery date options for the picker: the window dates
    /// generated for today (JST).
    pub delivery_dates: Vec<DeliveryDateOption>,
    /// Delivery slot options for the picker, catalogue order.
    pub delivery_slots: Vec<DeliverySlotOption>,
    /// The member's saved addresses for the delivery picker
    /// (empty for guests and empty books).
    pub saved_addresses: Vec<crate::models::MemberAddress>,
}

impl CheckoutView {
    /// The noshi catalogue for the gift select, in display
    /// order (the same fixed list the delivery slots use).
    pub fn noshi_kinds(&self) -> &'static [crate::models::gift::NoshiKind] {
        crate::models::gift::NOSHI_KINDS
    }
}

impl TotalsDisplay for CheckoutView {
    fn totals(&self) -> &CartTotals {
        &self.totals
    }
}

/// Renders pages/checkout/confirm.html: the read-only review of
/// everything the order will contain.
#[derive(Template)]
#[template(path = "storefront/pages/checkout/confirm.html")]
pub struct CheckoutConfirmView {
    /// Shared header state.
    pub header: Header,
    /// The delivery and payment details from the flow state.
    pub flow: CheckoutFlow,
    /// The lines being bought.
    pub lines: Vec<CartLine>,
    /// Final totals.
    pub totals: CartTotals,
    /// The applied coupons (one per class), with amounts.
    pub applied_coupons: Vec<crate::models::AppliedCoupon>,
    /// The shop's fees, for the payment method's note (the
    /// COD fee it names must be the settings' value).
    pub fees: crate::models::FeeSettings,
    /// The active hosted gateway's display name, for the
    /// copy that names the provider the shopper pays through.
    pub gateway_name: &'static str,
    /// Points the member is spending on this order (0 for
    /// none); shown as its own row under the total.
    pub points_used_yen: i64,
    /// The amount the payment method will actually charge
    /// (total minus points), formatted.
    pub payable_display: String,
}

impl TotalsDisplay for CheckoutConfirmView {
    fn totals(&self) -> &CartTotals {
        &self.totals
    }
}

/// Renders pages/checkout/payment.html: the dedicated payment
/// step — the method choice with the live totals that follow it.
#[derive(Template)]
#[template(path = "storefront/pages/checkout/payment.html")]
pub struct CheckoutPaymentView {
    /// Shared header state.
    pub header: Header,
    /// The flow state: the delivery details shown for review
    /// and the method currently chosen.
    pub flow: CheckoutFlow,
    /// The lines being bought.
    pub lines: Vec<CartLine>,
    /// Totals for the flow's coupon and current method.
    pub totals: CartTotals,
    /// The payment methods offered (more than one only when
    /// KOMOJU is available; the template renders the static
    /// COD box for a single option).
    pub payment_options: Vec<PaymentMethodOption>,
    /// Error for the method choice (the konbini limit).
    pub payment_error: Option<String>,
    /// Grand total as cash on delivery would pay it, for the
    /// summary's method-switch script.
    pub grand_total_cod: String,
    /// Grand total as an online method would pay it (no COD fee).
    pub grand_total_online: String,
    /// The cash-on-delivery fee amount itself, for the fee row
    /// the method-switch script shows and hides: the row must
    /// read the fee whenever it is visible, even when the
    /// current totals were computed for an online method.
    pub cod_fee_amount: String,
    /// The member's points balance, when the points
    /// programme is on and the visitor is a member (EC V2
    /// core 1); `None` hides the points section entirely.
    pub points_balance: Option<i64>,
    /// Error for the points input (unparsable, or over the
    /// balance).
    pub points_error: Option<String>,
}

impl TotalsDisplay for CheckoutPaymentView {
    fn totals(&self) -> &CartTotals {
        &self.totals
    }
}

/// Renders pages/checkout/complete.html: the placed order.
#[derive(Template)]
#[template(path = "storefront/pages/checkout/complete.html")]
pub struct CheckoutCompleteView {
    /// Shared header state.
    pub header: Header,
    /// The placed order.
    pub order: Order,
    /// Its snapshot lines, with each product's current
    /// thumbnail joined in for display.
    pub lines: Vec<OrderLineDetail>,
    /// Whether the buyer is a member (shows the history link).
    pub is_member: bool,
    /// The payment note beside the total: COD wording unchanged
    /// for cash orders, the method and its state for KOMOJU ones.
    pub payment_note: String,
    /// The address-book save note (the at-cap skip message),
    /// when the member asked to save the address and it was
    /// skipped. Shown once.
    pub address_note: Option<String>,
    /// The order's stored per-class tax breakdown （税率ごとの
    /// 内訳）; empty for orders placed before tax classes.
    pub tax_breakdown: Vec<crate::models::tax::OrderTaxBreakdown>,
}

impl CheckoutCompleteView {
    /// Grand total formatted for display.
    pub fn formatted_total(&self) -> String {
        crate::pricing::display_price(self.order.total_yen)
    }

    /// The summed 内消費税 of the stored breakdown, formatted.
    pub fn formatted_tax_total(&self) -> String {
        crate::pricing::display_tax_total(&self.tax_breakdown)
    }
}

/// The prefecture list lives in [`crate::views::storefront`]: one shared
/// constant for the sign-up and checkout forms.
pub use crate::views::storefront::PREFECTURES;
