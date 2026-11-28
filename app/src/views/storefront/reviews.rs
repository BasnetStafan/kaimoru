//! View structs for the reviews feature.

use askama::Template;

use super::header::Header;
use crate::models::Product;
use crate::models::review::ReviewFieldErrors;

/// Renders pages/reviews/form.html: one form for writing a
/// review and for editing it, told apart by `editing_id`.
#[derive(Template)]
#[template(path = "storefront/pages/reviews/form.html")]
pub struct ReviewFormView {
    /// Shared header state.
    pub header: Header,
    /// The product being reviewed, named and pictured on the
    /// form so the member never guesses which product it is for.
    pub product: Product,
    /// The review id when editing; None when writing a new one.
    pub editing_id: Option<i64>,
    /// The rating to pre-select (a half-step value), from the
    /// stored review or from a re-rendered submission; None
    /// leaves every star empty.
    pub rating: Option<f64>,
    /// Comment input, preserved across validation failures.
    pub comment: String,
    /// Every field error at once, each under its own field.
    pub errors: ReviewFieldErrors,
    /// A form-level error (duplicate review, rate limit).
    pub form_error: Option<String>,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`); empty on
    /// the create form, which has no record to guard.
    pub form_ticket: String,
}

/// Renders pages/reviews/notfound.html: the styled 404 a
/// member who never bought the product gets on the create
/// form. It does not explain which purchase would qualify.
#[derive(Template)]
#[template(path = "storefront/pages/reviews/notfound.html")]
pub struct ReviewNotFoundView {
    /// Shared header state.
    pub header: Header,
}

/// Renders parts/review_helpful.html: the 参考になった
/// control of one review — the vote form, or the voted
/// state once this reader has voted. The product page
/// includes the same partial per card (binding the same
/// three fields), and the vote endpoint returns it as the
/// htmx swap fragment, so both renderings are one template.
#[derive(Template)]
#[template(path = "storefront/parts/review_helpful.html")]
pub struct HelpfulView {
    /// The vote endpoint for this review.
    pub helpful_url: String,
    /// The current helpful count.
    pub helpful_count: i64,
    /// Whether this reader already voted.
    pub voted: bool,
}
