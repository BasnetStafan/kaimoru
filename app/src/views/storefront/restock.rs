//! View types for the back-in-stock registration card on
//! the product page: the card state the detail page renders
//! and the fragment the registration endpoint answers with.

use askama::Template;

/// One sold-out combination offered in the card's select.
pub struct RestockOption {
    /// The combination's variant ID.
    pub variant_id: i64,
    /// Its display label (axis names and option values).
    pub label: String,
}

/// The card's errors: one slot for the email field, one for
/// the form as a whole (an unregistrable combination, or
/// the rate limit).
#[derive(Default)]
pub struct RestockErrors {
    /// Email field error.
    pub email: Option<String>,
    /// Form-level error.
    pub form: Option<String>,
}

/// The registration card's full state. The detail page
/// renders it as a part; the registration endpoint answers
/// with the same part alone, so the two can never drift.
pub struct RestockCard {
    /// Product the card belongs to.
    pub product_id: i64,
    /// The sold-out combinations offered, in picker order.
    pub options: Vec<RestockOption>,
    /// The only offered combination's ID when exactly one
    /// is offered: the form then carries it as a hidden
    /// input instead of a one-entry select.
    pub single_variant_id: Option<i64>,
    /// The email field's current value (a signed-in
    /// member's address is prefilled).
    pub email: String,
    /// Errors from the last submission.
    pub errors: RestockErrors,
    /// The just-registered state replaces the form.
    pub done: bool,
    /// The already-registered state replaces the form.
    pub already: bool,
}

/// Renders parts/restock_form.html on its own, for the
/// registration endpoint's htmx swap.
#[derive(Template)]
#[template(path = "storefront/parts/restock_form.html")]
pub struct RestockFormFragment {
    /// The card state.
    pub restock: RestockCard,
}
