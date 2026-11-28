//! View types for the contact pages (Contact spec §3.2,
//! §5.1). The form renders outside the auth-card idiom's
//! account context but reuses its classes, badges and
//! per-field errors.

use askama::Template;

use crate::models;
use crate::views::storefront::header::Header;

/// Form errors, one slot per field.
#[derive(Default)]
pub struct ContactErrors {
    /// Name error.
    pub name: Option<String>,
    /// Email error.
    pub email: Option<String>,
    /// Category error.
    pub category: Option<String>,
    /// Order-number error.
    pub order_number: Option<String>,
    /// Body error.
    pub body: Option<String>,
}

impl ContactErrors {
    /// Builds the slots from the service's field errors.
    pub fn from_field_errors(errors: &[crate::validation::FieldError]) -> ContactErrors {
        let get = |field: &str| {
            errors
                .iter()
                .find(|e| e.field == field)
                .map(|e| e.message.clone())
        };
        ContactErrors {
            name: get("name"),
            email: get("email"),
            category: get("category"),
            order_number: get("order_number"),
            body: get("body"),
        }
    }
}

/// The submitted (or prefilled) values, preserved across
/// validation re-renders.
#[derive(Default)]
pub struct ContactValues {
    /// Name.
    pub name: String,
    /// Email.
    pub email: String,
    /// Selected category code.
    pub category: String,
    /// Order number.
    pub order_number: String,
    /// Body.
    pub body: String,
}

/// `GET /contact`: the inquiry form.
#[derive(Template)]
#[template(path = "storefront/pages/contact/contact.html")]
pub struct ContactView {
    /// Header data.
    pub header: Header,
    /// The support note from shop settings, when set.
    pub support_note: Option<String>,
    /// Preserved values.
    pub values: ContactValues,
    /// Validation errors, per field.
    pub errors: ContactErrors,
    /// One form-level error (the rate-limit message).
    pub form_error: Option<String>,
}

impl ContactView {
    /// The category catalogue for the select.
    pub fn categories(&self) -> &[models::Category] {
        &models::CATEGORIES
    }
}

/// `GET /contact/complete`: the acceptance screen with
/// the 受付番号.
#[derive(Template)]
#[template(path = "storefront/pages/contact/contact_complete.html")]
pub struct ContactCompleteView {
    /// Header data.
    pub header: Header,
    /// The reference, e.g. `INQ-000123`.
    pub reference: String,
}
