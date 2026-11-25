//! Review moderation view.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::views::admin::Page;

/// One moderation row, display-ready. Reviews are anonymous on
/// the storefront and stay anonymous here: no reviewer name or
/// email is part of this struct at all.
pub struct ReviewRow {
    /// Review ID (the delete form's target).
    pub id: i64,
    /// Reviewed product ID for the storefront link.
    pub product_id: i64,
    /// Reviewed product's current name.
    pub product_name: String,
    /// Star fill percentage for the clipped star bar.
    pub star_pct: i64,
    /// Rating in text form, e.g. `4.5`.
    pub rating_text: String,
    /// Comment text; empty for a rating-only review.
    pub comment: String,
    /// Whether a comment exists.
    pub has_comment: bool,
    /// Posted date/time, `YYYY/MM/DD HH:mm` JST.
    pub posted: String,
    /// Whether the review is hidden from the storefront.
    pub hidden: bool,
    /// The shop reply's current text (the reply form's value).
    pub shop_reply: String,
}

/// One product in the filter select.
pub struct ProductFilterOption {
    /// Product ID as the submitted value.
    pub value: String,
    /// Product name as the visible label.
    pub name: String,
    /// Whether it is the active filter.
    pub selected: bool,
}

/// The review moderation list page.
#[derive(Template)]
#[template(path = "admin/pages/reviews/list.html")]
pub struct ReviewListView {
    /// List URL carrying the current product filter and
    /// page: row delete forms post it back as `return_url`,
    /// so moderation returns to the exact list position.
    pub return_url: String,
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The rows of this page.
    pub reviews: Vec<ReviewRow>,
    /// Product filter options (all products, archived
    /// included), with the current selection marked.
    pub product_options: Vec<ProductFilterOption>,
    /// The selected product filter, when set.
    pub selected_product: Option<i64>,
    /// Pagination state.
    pub page: Page,
}
