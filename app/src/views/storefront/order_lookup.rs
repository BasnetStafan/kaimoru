//! View types for the public guest order lookup pages: the
//! form, and the matched order shown read-only.

use askama::Template;

use crate::views::storefront::header::Header;

/// The lookup form's submitted values, preserved across
/// re-renders so a typo costs one field, not the whole form.
#[derive(Default)]
pub struct LookupValues {
    /// Order number as typed.
    pub order_number: String,
    /// Email as typed.
    pub email: String,
}

/// Lookup form errors, one slot per field.
#[derive(Default)]
pub struct LookupErrors {
    /// Order number error.
    pub order_number: Option<String>,
    /// Email error.
    pub email: Option<String>,
}

impl LookupErrors {
    /// Builds the slots from the service's field errors.
    pub fn from_field_errors(errors: &[crate::validation::FieldError]) -> LookupErrors {
        let get = |field: &str| {
            errors
                .iter()
                .find(|e| e.field == field)
                .map(|e| e.message.clone())
        };
        LookupErrors {
            order_number: get("order_number"),
            email: get("email"),
        }
    }
}

/// `GET /order-lookup`: the lookup form.
#[derive(Template)]
#[template(path = "storefront/pages/order_lookup/form.html")]
pub struct OrderLookupView {
    /// Header data.
    pub header: Header,
    /// Preserved values.
    pub values: LookupValues,
    /// Validation errors, per field.
    pub errors: LookupErrors,
    /// One form-level error (a failed match, or the throttle
    /// refusal), worded so it never says which half failed.
    pub form_error: Option<String>,
}

/// The matched order, read-only: the member detail page's
/// data minus every member-only action (no review buttons,
/// no cancellation — the email match proves the order is the
/// visitor's, not that they hold the account).
#[derive(Template)]
#[template(path = "storefront/pages/order_lookup/result.html")]
pub struct GuestOrderView {
    /// Header data.
    pub header: Header,
    /// The matched order.
    pub order: crate::models::Order,
    /// Its lines with current thumbnails.
    pub lines: Vec<crate::models::order::OrderLineDetail>,
    /// The order's payment block, when a payment row exists
    /// (orders placed before payments existed render the
    /// original COD wording instead).
    pub payment: Option<crate::views::storefront::payment::PaymentBlockView>,
    /// The order's stored per-class tax breakdown （税率ごとの
    /// 内訳）; empty for orders placed before tax classes.
    pub tax_breakdown: Vec<crate::models::tax::OrderTaxBreakdown>,
    /// The order's shipped split parcels; empty for
    /// single-shipment orders.
    pub shipments: Vec<crate::views::storefront::mypage::PublicShipmentRow>,
}

impl GuestOrderView {
    /// The summed 内消費税 of the order's breakdown, formatted.
    pub fn formatted_tax_total(&self) -> String {
        crate::pricing::display_tax_total(&self.tax_breakdown)
    }
}
