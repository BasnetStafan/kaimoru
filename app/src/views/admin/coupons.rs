//! Coupon views: the usage list and the definition forms.
//! Definitions are created and edited from the admin (the
//! coupons controller owns the routes).

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::views::admin::Page;

/// One coupon row, display-ready.
pub struct CouponRow {
    /// Coupon code.
    pub code: String,
    /// Definition description.
    pub description: String,
    /// Holding member's name.
    pub holder: String,
    /// Issue date, `YYYY/MM/DD` JST.
    pub issued: String,
    /// 状態 label: 未使用 or 使用済み.
    pub state_label: &'static str,
    /// The using order's ID, when used (the detail link).
    pub used_order_id: Option<i64>,
    /// The using order's formatted number, when used.
    pub used_order_number: String,
}

/// The coupon usage page.
#[derive(Template)]
#[template(path = "admin/pages/coupons/list.html")]
pub struct CouponListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// This page of issued coupon copies, newest issue first.
    pub coupons: Vec<CouponRow>,
    /// Pagination state.
    pub page: Page,
}

// ---- Coupon definition management (Coupon Admin spec §6) ----

/// One status tab on the definitions list.
pub struct DefinitionTab {
    /// Tab label.
    pub label: String,
    /// Tab URL.
    pub url: String,
    /// Definitions under this tab.
    pub count: i64,
    /// Whether this tab is selected.
    pub active: bool,
}

/// One display-ready definition row.
pub struct DefinitionRow {
    /// Definition id.
    pub id: i64,
    /// Code.
    pub code: String,
    /// Discount, formatted.
    pub discount_display: String,
    /// Type + benefit summary (for example `定率 10%割引`).
    pub type_display: String,
    /// Scope summary （注文全体 or the target's name).
    pub scope_display: String,
    /// Whether the discount applies without a code.
    pub automatic: bool,
    /// Minimum subtotal, formatted (ー when none).
    pub min_display: String,
    /// Window label （常時 or the JST range).
    pub window_label: String,
    /// Message.
    pub description: String,
    /// Whether active.
    pub active: bool,
    /// Whether the band shows this coupon.
    pub band_featured: bool,
    /// Redemption count.
    pub redemptions: i64,
}

/// `GET /admin/coupons`: the definitions list.
#[derive(Template)]
#[template(path = "admin/pages/coupons/definitions.html")]
pub struct CouponDefinitionsView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Status tabs with counts.
    pub tabs: Vec<DefinitionTab>,
    /// Rows under the selected tab.
    pub rows: Vec<DefinitionRow>,
}

/// The definition form's values, as strings (posted
/// back verbatim on validation failure).
#[derive(Default)]
pub struct CouponFormValues {
    /// Code.
    pub code: String,
    /// Value kind as selected.
    pub discount_type: String,
    /// Discount as typed.
    pub discount_yen: String,
    /// Percent as typed.
    pub percent: String,
    /// Scope kind as selected.
    pub scope: String,
    /// Scoped product id as selected (empty = none).
    pub product_id: String,
    /// Scoped variant id as selected (empty = none).
    pub variant_id: String,
    /// Total cap as typed (empty = unlimited).
    pub max_uses_total: String,
    /// Per-customer cap as typed (empty = unlimited).
    pub max_uses_per_customer: String,
    /// Whether the discount applies without a code.
    pub is_automatic: bool,
    /// Minimum subtotal as typed.
    pub min_subtotal_yen: String,
    /// Message.
    pub description: String,
    /// Window start input value (JST, datetime-local).
    pub starts_at: String,
    /// Window end input value.
    pub ends_at: String,
    /// Band flag.
    pub band_featured: bool,
    /// Initial active state (create only).
    pub active: bool,
}

/// The definition form's errors, one slot per field.
#[derive(Default)]
pub struct CouponFormErrors {
    /// Code error.
    pub code: Option<String>,
    /// Value-kind error.
    pub discount_type: Option<String>,
    /// Discount error.
    pub discount_yen: Option<String>,
    /// Percent error.
    pub percent: Option<String>,
    /// Scope error.
    pub scope: Option<String>,
    /// Scoped product error.
    pub product_id: Option<String>,
    /// Scoped variant error.
    pub variant_id: Option<String>,
    /// Total cap error.
    pub max_uses_total: Option<String>,
    /// Per-customer cap error.
    pub max_uses_per_customer: Option<String>,
    /// Minimum-subtotal error.
    pub min_subtotal_yen: Option<String>,
    /// Message error.
    pub description: Option<String>,
    /// Window-start error.
    pub starts_at: Option<String>,
    /// Window-end error.
    pub ends_at: Option<String>,
}

impl CouponFormErrors {
    /// Builds the slots from the service's field errors.
    pub fn from_field_errors(errors: &[crate::validation::FieldError]) -> CouponFormErrors {
        let get = |field: &str| {
            errors
                .iter()
                .find(|e| e.field == field)
                .map(|e| e.message.clone())
        };
        CouponFormErrors {
            code: get("code"),
            discount_type: get("discount_type"),
            discount_yen: get("discount_yen"),
            percent: get("percent"),
            scope: get("scope"),
            product_id: get("product_id"),
            variant_id: get("variant_id"),
            max_uses_total: get("max_uses_total"),
            max_uses_per_customer: get("max_uses_per_customer"),
            min_subtotal_yen: get("min_subtotal_yen"),
            description: get("description"),
            starts_at: get("starts_at"),
            ends_at: get("ends_at"),
        }
    }
}

/// One scope-target option of the definition form.
pub struct CouponTargetOption {
    /// The posted value (the row id).
    pub id: String,
    /// The display label.
    pub label: String,
    /// Whether the current values select this option.
    pub selected: bool,
    /// The parent product id for a variant option; empty
    /// for product options.
    pub parent_id: String,
}

/// `GET /admin/coupons/new` and `GET /admin/coupons/{id}`:
/// the create/edit form.
#[derive(Template)]
#[template(path = "admin/pages/coupons/form.html")]
pub struct CouponFormView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Page heading.
    pub heading: String,
    /// Where the form posts.
    pub form_action: String,
    /// Preserved values.
    pub values: CouponFormValues,
    /// Field errors.
    pub errors: CouponFormErrors,
    /// Whether the create-only active checkbox shows.
    pub show_active: bool,
    /// Whether the code field is locked (§5.1).
    pub code_locked: bool,
    /// Whether the money fields are locked (§5.1).
    pub money_locked: bool,
    /// Product options for the scope picker.
    pub products: Vec<CouponTargetOption>,
    /// Variant options for the scope picker.
    pub variants: Vec<CouponTargetOption>,
    /// The definition id on the edit form (the state
    /// actions' target); absent on create.
    pub definition_id: Option<i64>,
    /// The current active state on the edit form.
    pub current_active: Option<bool>,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`); empty on
    /// the create form, which has no record to guard.
    pub form_ticket: String,
}

/// `GET /admin/coupons/{id}/deactivate`: the confirm.
#[derive(Template)]
#[template(path = "admin/pages/coupons/deactivate.html")]
pub struct CouponDeactivateView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Definition id.
    pub id: i64,
    /// Definition code.
    pub code: String,
    /// Members holding it unused (the §5.2 warning).
    pub holders: i64,
}
