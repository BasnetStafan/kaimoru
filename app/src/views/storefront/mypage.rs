//! Mypage view.
//! Data for the member page template.

use askama::Template;

use crate::models::User;
use crate::views::storefront::header::Header;

/// Renders mypage/top.html: the member hub — grouped menu
/// sections, then the recent-orders block.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/top.html")]
pub struct MypageView {
    /// Shared header state.
    pub header: Header,
    /// The logged in member.
    pub user: User,
    /// The member's most recent orders with line summaries.
    pub recent_orders: Vec<crate::models::RecentOrder>,
    /// How many of the member's coupons are still unused: the
    /// hub menu's badge.
    pub unused_coupons: i64,
    /// The member's points balance while the programme is
    /// on (EC V2 core 1): the hub's points item and its
    /// badge. `None` hides the item (programme off).
    pub points_balance: Option<i64>,
}

/// Renders pages/mypage/orders.html: the full order history,
/// paginated.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/orders.html")]
pub struct OrderHistoryView {
    /// Shared header state.
    pub header: Header,
    /// This page of the member's orders, newest first, each
    /// with its first line's name and line count.
    pub orders: Vec<crate::models::RecentOrder>,
    /// Current history page number, 1-based.
    pub page: i64,
    /// Total number of history pages (at least 1).
    pub total_pages: i64,
}

/// Renders pages/mypage/coupons.html: the member's coupon
/// list, held first, then used.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/coupons.html")]
pub struct CouponListView {
    /// Shared header state.
    pub header: Header,
    /// The member's coupon copies, held first, then used.
    pub coupons: Vec<crate::models::MemberCoupon>,
}

/// Renders pages/mypage/order.html: one order in full.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/order.html")]
pub struct OrderDetailView {
    /// Shared header state.
    pub header: Header,
    /// The order being shown (already ownership-checked).
    pub order: crate::models::Order,
    /// Its lines with current thumbnails and review buttons.
    pub lines: Vec<OrderLineRow>,
    /// The order's payment block, when a payment row exists
    /// (orders placed before payments existed render the
    /// original COD wording instead).
    pub payment: Option<crate::views::storefront::payment::PaymentBlockView>,
    /// The cancellation area: button, preparing note, or
    /// the cancelled completion block (Order Cancellation
    /// spec section 5.1/5.5).
    pub cancel: OrderCancelBlock,
    /// A one-shot notice (a refused cancellation's
    /// explanation), when one was flashed.
    pub notice: Option<String>,
    /// The order's stored per-class tax breakdown （税率ごとの
    /// 内訳）; empty for orders placed before tax classes.
    pub tax_breakdown: Vec<crate::models::tax::OrderTaxBreakdown>,
    /// The 返品・交換の申請 form's state: empty on a plain
    /// view, the typed values and errors after a refused post.
    pub return_form: ReturnFormState,
    /// The newest return request's status label, when
    /// a workflow exists for this order.
    pub return_status_label: Option<String>,
    /// The order's shipped split parcels; empty for
    /// single-shipment orders.
    pub shipments: Vec<PublicShipmentRow>,
}

/// One shipped parcel as the customer sees it.
pub struct PublicShipmentRow {
    /// Position within the order (1-based).
    pub index: usize,
    /// Carrier display label.
    pub carrier_label: String,
    /// Tracking number for this parcel.
    pub tracking_number: String,
    /// Its lines as one summary text.
    pub items_label: String,
}

/// The return/exchange request form's state on the order
/// detail page.
#[derive(Default)]
pub struct ReturnFormState {
    /// The chosen reason code, kept after a refused post.
    pub reason: String,
    /// The free-text detail, kept after a refused post.
    pub detail: String,
    /// Field errors of the refused post.
    pub errors: ReturnFormErrors,
    /// A whole-form refusal (the inquiry rate limit), when
    /// the post was refused for that reason.
    pub form_error: Option<String>,
}

/// The request form's field errors, one slot per field —
/// the shape form templates render.
#[derive(Default)]
pub struct ReturnFormErrors {
    /// Error for the reason select.
    pub reason: Option<String>,
    /// Error for the detail textarea.
    pub detail: Option<String>,
}

impl ReturnFormErrors {
    /// Maps flat field errors onto the named slots.
    pub fn from_field_errors(errors: &[crate::validation::FieldError]) -> Self {
        let mut mapped = Self::default();
        for error in errors {
            let slot = match error.field {
                "reason" => &mut mapped.reason,
                "detail" => &mut mapped.detail,
                _ => continue,
            };
            if slot.is_none() {
                *slot = Some(error.message.clone());
            }
        }
        mapped
    }
}

impl OrderDetailView {
    /// The fixed return-reason catalogue for the form's
    /// select (models::inquiry::RETURN_REASONS).
    pub fn return_reasons(&self) -> &'static [crate::models::inquiry::Category] {
        &crate::models::inquiry::RETURN_REASONS
    }

    /// The summed 内消費税 of the order's breakdown, formatted.
    pub fn formatted_tax_total(&self) -> String {
        crate::pricing::display_tax_total(&self.tax_breakdown)
    }
}

/// The order detail's cancellation area, by order state.
pub struct OrderCancelBlock {
    /// `received`: the cancel button renders.
    pub can_cancel: bool,
    /// `preparing`: the fixed note replaces the button.
    pub preparing_note: bool,
    /// `cancelled`: the completion block renders.
    pub cancelled: Option<CancelledBlock>,
}

/// A cancelled order's completion block.
pub struct CancelledBlock {
    /// The refund line for the payment state, when money
    /// needs a sentence.
    pub refund_line: Option<String>,
}

/// Renders pages/mypage/order_cancel.html: the member
/// order cancellation confirmation — the contract page
/// of spec section 5.2.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/order_cancel.html")]
pub struct OrderCancelConfirmView {
    /// Shared header state.
    pub header: Header,
    /// The order being cancelled (ownership-checked).
    pub order: crate::models::Order,
    /// Its lines, for the summary list.
    pub lines: Vec<crate::models::order::OrderLineDetail>,
    /// The money consequence paragraph.
    pub consequence: String,
    /// The coupon consequence, when a coupon was used.
    pub coupon_line: Option<String>,
    /// The shop contact line (captured konbini only).
    pub contact_line: Option<String>,
}

/// One order line on the detail page, with the member's own
/// review of that product when it exists: Some(id) renders
/// レビューを編集する, None renders レビューを書く.
pub struct OrderLineRow {
    /// The order line itself.
    pub line: crate::models::order::OrderLineDetail,
    /// The member's review id for this product, when written.
    pub own_review_id: Option<i64>,
}

/// Renders pages/mypage/withdraw.html: the withdrawal
/// explanation, blockers, or confirmation ceremony
/// (Account Deletion spec section 3).
#[derive(Template)]
#[template(path = "storefront/pages/mypage/withdraw.html")]
pub struct WithdrawView {
    /// Shared header state.
    pub header: Header,
    /// Whether the account has a password: the ceremony asks
    /// for it when it does, and for the email code proof when
    /// it does not (SNS-registered accounts).
    pub has_password: bool,
    /// Whether the email code proof has already verified:
    /// the ceremony then asks only for the confirmation.
    pub code_verified: bool,
    /// Blocking orders, when in-flight business prevents
    /// the withdrawal (empty = the ceremony form
    /// renders). Each carries its id (the detail link)
    /// and its display number.
    pub blockers: Vec<(i64, String)>,
    /// The password field's error, when any.
    pub password_error: Option<String>,
    /// The confirmation checkbox's error, when any.
    pub confirm_error: Option<String>,
}

/// Renders pages/mypage/order_missing.html: the indistinguishable
/// 404 for a missing or foreign order ID.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/order_missing.html")]
pub struct OrderMissingView {
    /// Shared header state.
    pub header: Header,
}

/// Renders pages/mypage/edit.html: the profile edit form.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/edit.html")]
pub struct ProfileEditView {
    /// Shared header state.
    pub header: Header,
    /// The member's email, shown read-only (it is the login ID
    /// and cannot be changed here).
    pub email: String,
    /// Current form values.
    pub values: crate::views::storefront::signup::DetailsValues,
    /// Per-field validation errors.
    pub errors: crate::views::storefront::signup::DetailsErrors,
    /// Prefecture options for the select.
    pub prefectures: &'static [&'static str],
    /// Form-level error to show, if any.
    pub error: Option<String>,
    /// Success notice to show, if any.
    pub notice: Option<String>,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`): the hidden
    /// field the submit spends to prove the form was
    /// rendered from the current record.
    pub form_ticket: String,
}

/// Renders pages/mypage/password.html: the password change form.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/password.html")]
pub struct PasswordChangeView {
    /// Shared header state.
    pub header: Header,
    /// Error to show, if any.
    pub error: Option<String>,
    /// Success notice to show, if any.
    pub notice: Option<String>,
    /// Whether the account already has a password: the form
    /// is a change when it does and a first-time set when it
    /// does not (SNS-registered accounts).
    pub has_password: bool,
}

/// Renders pages/mypage/withdraw_verify.html: the email code
/// step of the withdrawal for passwordless accounts.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/withdraw_verify.html")]
pub struct WithdrawVerifyView {
    /// Shared header state.
    pub header: Header,
    /// Error to show, if any.
    pub error: Option<String>,
    /// Notice to show, for example after a resend.
    pub notice: Option<String>,
    /// The account email, masked for display.
    pub email_masked: String,
}

/// Renders pages/mypage/email.html: the email change start form
/// (current password re-confirmation + the new address).
#[derive(Template)]
#[template(path = "storefront/pages/mypage/email.html")]
pub struct EmailChangeView {
    /// Shared header state.
    pub header: Header,
    /// The member's current email, shown read-only.
    pub current_email: String,
    /// The new address entered before, kept after an error.
    pub new_email: String,
    /// Error to show, if any.
    pub error: Option<String>,
}

/// Renders pages/mypage/email_verify.html: the code entry step
/// of the email change, and after completion the success state.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/email_verify.html")]
pub struct EmailChangeVerifyView {
    /// Shared header state.
    pub header: Header,
    /// Error to show, if any.
    pub error: Option<String>,
    /// Notice to show, for example after a resend or completion.
    pub notice: Option<String>,
    /// The destination (new) email, masked for display.
    pub email_masked: String,
    /// Whether the change has completed: the form is replaced
    /// by the success message and a link back to mypage.
    pub completed: bool,
}
