//! Admin list-row types: the per-entity shapes the
//! back-office list and detail pages render, grouped
//! here instead of inside the entity model files
//! (whose rows are the storefront shapes).

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::FromRow;

/// One product row as the admin screens need it: the storefront
/// [`Product`](super::Product) columns plus publication status
/// and last-update time, which the member side never reads.
#[derive(Debug, Clone, FromRow)]
pub struct AdminProduct {
    /// Primary key.
    pub id: i64,
    /// Product name.
    pub name: String,
    /// Long description shown on the detail page.
    pub description: String,
    /// Tax-inclusive price in integer yen.
    pub price_yen: i64,
    /// Total units on hand: the sum over the product variants.
    /// i32, matching the aggregate cast: sqlx will not widen
    /// INT4 into i64 when decoding.
    pub stock: i32,
    /// Category name snapshot, synced from the linked category
    /// whenever either side is edited (see `category_id`).
    pub category: String,
    /// The managed category this product belongs to; `None` only
    /// on legacy rows seeded outside the app (direct-INSERT suite
    /// fixtures), whose edit form asks for a category on save.
    pub category_id: Option<i64>,
    /// Operator-facing product code; unique when set.
    pub product_code: Option<String>,
    /// Brand name; empty when unset.
    pub brand: String,
    /// Manufacturer name; empty when unset.
    pub manufacturer: String,
    /// Country of origin; empty when unset.
    pub country_of_origin: String,
    /// Specification text (multi-line).
    pub spec_text: String,
    /// Selling-points text (multi-line).
    pub features_text: String,
    /// Smartphone description override; empty renders the
    /// default description on phones too.
    pub description_sp: String,
    /// Extra search keywords.
    pub search_keywords: String,
    /// Tax class the product is charged under.
    pub tax_class_id: i64,
    /// Cost in yen (admin-only); `None` when not recorded.
    pub cost_yen: Option<i64>,
    /// Sale price in yen while a sale window is open.
    pub sale_price_yen: Option<i64>,
    /// Sale window start (UTC); `None` = open-ended.
    pub sale_starts_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Sale window end (UTC); `None` = open-ended.
    pub sale_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The 新着 badge flag.
    pub is_new: bool,
    /// The label-only promotion percentage badge.
    pub discount_percent: i32,
    /// The 予約 (preorder) flag.
    pub is_preorder: bool,
    /// The おすすめ (featured) flag.
    pub is_staff_pick: bool,
    /// Listing order within the default sort.
    pub display_order: i32,
    /// URL slug; unique when set.
    pub slug: Option<String>,
    /// Detail-page title override; empty uses the default.
    pub seo_title: String,
    /// Detail-page meta description; empty falls back.
    pub meta_description: String,
    /// Whether stock is tracked (false = unlimited).
    pub track_stock: bool,
    /// Whether the product sells past zero stock.
    pub allow_backorder: bool,
    /// Low-stock threshold override; `None` = shop default.
    pub low_stock_threshold: Option<i32>,
    /// Points earn-rate override; `None` = shop rate,
    /// `Some(0)` = points-excluded.
    pub points_earn_percent: Option<i32>,
    /// Weight in grams, for the shipping record.
    pub weight_grams: Option<i32>,
    /// Parcel length in millimetres.
    pub size_length_mm: Option<i32>,
    /// Parcel width in millimetres.
    pub size_width_mm: Option<i32>,
    /// Parcel height in millimetres.
    pub size_height_mm: Option<i32>,
    /// Whether the product ships chilled.
    pub cool_delivery: bool,
    /// Whether the product ships by mail delivery.
    pub mail_delivery: bool,
    /// Whether the product always ships free.
    pub free_shipping: bool,
    /// Comma-separated prefectures the product cannot ship to.
    pub undeliverable_regions: String,
    /// Days until dispatch; `None` = the standard wording.
    pub lead_time_days: Option<i32>,
    /// Publication window start (UTC); `None` = open.
    pub publish_starts_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Publication window end (UTC); `None` = open.
    pub publish_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Served path of the product's main image: the first of
    /// its uploaded images, synced from the `product_images`
    /// table; empty when the product has none.
    pub image_path: String,
    /// Whether the product is a timed/preorder event product.
    pub is_event: bool,
    /// Publication status: `public`, `draft` or `archived`.
    pub status: String,
    /// When the product was registered.
    pub created_at: DateTime<Utc>,
    /// When an admin last edited it.
    pub updated_at: DateTime<Utc>,
}

impl AdminProduct {
    /// The Japanese label for the publication status.
    pub fn status_label(&self) -> &'static str {
        product_status_label(&self.status)
    }
}

/// One product row as the admin product LIST renders it:
/// the narrow shape — the long description, spec and SEO
/// texts stay in the database instead of crossing the wire
/// for every row of every page. The full [`AdminProduct`]
/// remains the shape of the edit screen and the CSV export.
#[derive(Debug, Clone, FromRow)]
pub struct AdminProductListRow {
    /// Primary key.
    pub id: i64,
    /// Product name.
    pub name: String,
    /// Tax-inclusive price in integer yen.
    pub price_yen: i64,
    /// Total units on hand: the sum over the product variants.
    /// i32, matching the aggregate cast.
    pub stock: i32,
    /// Category name snapshot (see [`AdminProduct::category`]).
    pub category: String,
    /// Served path of the product's main image; empty when
    /// the product has none.
    pub image_path: String,
    /// Publication status: `public`, `draft` or `archived`.
    pub status: String,
    /// When an admin last edited it.
    pub updated_at: DateTime<Utc>,
}

impl AdminProductListRow {
    /// The Japanese label for the publication status.
    pub fn status_label(&self) -> &'static str {
        product_status_label(&self.status)
    }
}

/// The Japanese label for a product publication status value.
pub fn product_status_label(status: &str) -> &'static str {
    match status {
        "draft" => "下書き",
        "archived" => "アーカイブ",
        _ => "公開",
    }
}

/// One row of the `tags` controlled vocabulary.
#[derive(Debug, Clone, FromRow)]
pub struct Tag {
    /// Primary key.
    pub id: i64,
    /// Stable ASCII identifier (the agent's needs matching reads this).
    pub slug: String,
    /// Japanese label shown in the admin form.
    pub label: String,
    /// Operator-facing description from the tag management page;
    /// empty when never filled in.
    pub description: String,
}

/// One row of the `categories` table: the managed category
/// vocabulary. Products link here by `products.category_id` and
/// keep the category's name as a snapshot in `products.category`,
/// which is what the storefront's search reads.
#[derive(Debug, Clone, FromRow)]
pub struct ProductCategory {
    /// Primary key.
    pub id: i64,
    /// Display name, unique case-insensitively.
    pub name: String,
    /// Stable ASCII identifier, unique case-insensitively.
    pub slug: String,
    /// Operator-facing description; empty when never filled in.
    pub description: String,
    /// Parent category for the hierarchy; `None` at the top level.
    pub parent_id: Option<i64>,
    /// Nav and list ordering weight (smaller first).
    pub display_order: i32,
    /// Inactive categories vanish from the storefront navigation
    /// and the product form's choices, but keep their products.
    pub is_active: bool,
    /// When the category was registered.
    pub created_at: DateTime<Utc>,
    /// When the category was last edited.
    pub updated_at: DateTime<Utc>,
}

/// One order as the admin order list shows it: the order row's
/// display columns plus the buying member's name, resolved in
/// the query so guest orders show ゲスト.
#[derive(Debug, Clone, FromRow)]
pub struct AdminOrderSummary {
    /// Primary key.
    pub id: i64,
    /// Buying member's name; `None` for a guest order.
    pub member_name: Option<String>,
    /// Total charged in yen, snapshots included.
    pub total_yen: i64,
    /// Fulfilment status value.
    pub status: String,
    /// When the order was placed.
    pub created_at: DateTime<Utc>,
}

/// One order as the admin order list shows it: [`AdminOrderSummary`]
/// plus the buying member's ID (the 会員 cell links to the customer
/// detail) and the total item quantity. The dashboard keeps using
/// the summary — it renders neither extra column.
#[derive(Debug, Clone, FromRow)]
pub struct AdminOrderListRow {
    /// Primary key.
    pub id: i64,
    /// Buying member's ID; `None` for a guest order.
    pub user_id: Option<i64>,
    /// Buying member's name; `None` for a guest order.
    pub member_name: Option<String>,
    /// Total charged in yen, snapshots included.
    pub total_yen: i64,
    /// Fulfilment status value.
    pub status: String,
    /// When the order was placed.
    pub created_at: DateTime<Utc>,
    /// Which door the order entered through (`storefront` /
    /// `agent`), stamped at placement.
    pub placed_via: String,
    /// Total quantity across the order's lines.
    pub item_count: i64,
    /// Payment provider (`komoju` / `cod`); `None` for orders
    /// placed before payment rows existed.
    pub payment_provider: Option<String>,
    /// Payment status; `None` with the provider.
    pub payment_status: Option<String>,
}

/// One member as the admin customers list shows it. Profile
/// columns only — the password hash is never selected here.
#[derive(Debug, Clone, FromRow)]
pub struct AdminCustomer {
    /// Primary key (the 会員ID）.
    pub id: i64,
    /// Family name.
    pub family_name: String,
    /// Given name.
    pub given_name: String,
    /// Account email.
    pub email: String,
    /// When the member registered.
    pub created_at: DateTime<Utc>,
    /// Orders placed, excluding cancelled ones: a cancelled COD
    /// order collected no money and fulfilled nothing.
    pub order_count: i64,
    /// Total spent over the same non-cancelled set, in yen.
    pub total_spent_yen: i64,
    /// When the most recent non-cancelled order was placed, if any.
    pub last_order_at: Option<DateTime<Utc>>,
    /// When the member withdrew, when they have: the row
    /// is then an anonymised tombstone (Account Deletion
    /// spec D7's marker).
    pub withdrawn_at: Option<DateTime<Utc>>,
}

/// One member's full profile for the admin detail screen.
///
/// A dedicated type, not the shared [`User`](super::User): it also
/// shows birth date, gender and registration date, and — like the
/// member model — has no password-hash field, so the hash cannot
/// leak into an admin template.
#[derive(Debug, Clone, FromRow)]
pub struct AdminCustomerDetail {
    /// Primary key (the 会員ID）.
    pub id: i64,
    /// Account email.
    pub email: String,
    /// Family name.
    pub family_name: String,
    /// Given name.
    pub given_name: String,
    /// Family name in katakana.
    pub family_name_kana: String,
    /// Given name in katakana.
    pub given_name_kana: String,
    /// Phone number, digits only.
    pub phone: String,
    /// Postal code, 7 digits.
    pub postal_code: String,
    /// Prefecture name.
    pub prefecture: String,
    /// City and ward.
    pub city: String,
    /// Street address.
    pub address_line: String,
    /// Building name, when given.
    pub building: Option<String>,
    /// Date of birth, when given.
    pub birth_date: Option<NaiveDate>,
    /// Gender, when given.
    pub gender: Option<String>,
    /// Whether the member accepts the mail magazine.
    pub mail_magazine: bool,
    /// When the member registered.
    pub created_at: DateTime<Utc>,
}

impl AdminCustomer {
    /// The purchase total formatted for display.
    pub fn total_spent_display(&self) -> String {
        crate::pricing::display_price(self.total_spent_yen)
    }

    /// Returns the last-order date (`YYYY/MM/DD`, JST), or `—`
    /// when the member has no non-cancelled order.
    pub fn last_order_display(&self) -> String {
        match self.last_order_at {
            Some(at) => super::jst::jst_date_label(at),
            None => "—".to_string(),
        }
    }

    /// Returns the registration date (`YYYY/MM/DD`, JST).
    pub fn registered_display(&self) -> String {
        super::jst::jst_date_label(self.created_at)
    }
}

/// One review as the moderation list shows it: the review row
/// plus the product name it belongs to.
#[derive(Debug, Clone, FromRow)]
pub struct AdminReviewRow {
    /// Primary key.
    pub id: i64,
    /// The reviewed product's ID.
    pub product_id: i64,
    /// The reviewed product's current name.
    pub product_name: String,
    /// Star rating, 0.5–5.0 in 0.5 steps.
    pub rating: f64,
    /// Optional comment text; `None` for a rating-only review.
    pub comment: Option<String>,
    /// When the review was posted.
    pub created_at: DateTime<Utc>,
    /// Whether the review is hidden from the storefront.
    pub hidden: bool,
    /// The shop's public reply; empty = none.
    pub shop_reply: String,
}

/// One coupon as the read-only coupon screen shows it: the
/// master definition joined with one member's copy of it.
#[derive(Debug, Clone, FromRow)]
pub struct AdminCouponRow {
    /// Coupon code.
    pub code: String,
    /// What the coupon is, from the master definition.
    pub description: String,
    /// The member holding this copy; `None` cannot happen for
    /// issued coupons but keeps the join total.
    pub holder_name: Option<String>,
    /// When this copy was issued to the member.
    pub issued_at: DateTime<Utc>,
    /// When this copy was used; `None` while unused.
    pub used_at: Option<DateTime<Utc>>,
    /// The order that used this copy, resolved by member + code
    /// (user_coupons carries no order FK); `None` while unused.
    pub used_order_id: Option<i64>,
    /// When the using order was placed; `None` while unused.
    /// Carried so the order number computes from the ID and this
    /// instant, without an extra lookup per coupon.
    pub used_order_created_at: Option<DateTime<Utc>>,
}

/// One notice as the admin notices screen manages it: the raw
/// row, unlike the storefront [`Notice`](super::Notice), which
/// carries only display text. Display state （表示中 / 非表示 /
/// 期間外） is derived from the flag and period, never stored —
/// see `notice_service::display_state`.
#[derive(Debug, Clone, FromRow)]
pub struct AdminNotice {
    /// Primary key.
    pub id: i64,
    /// Headline; empty only on rows written before the title
    /// column existed (migration 0019).
    pub title: String,
    /// Announcement text.
    pub body: String,
    /// Display period start; `None` shows immediately.
    pub display_from: Option<NaiveDate>,
    /// Display period end; `None` never expires.
    pub display_to: Option<NaiveDate>,
    /// The visibility flag: off hides the notice regardless of
    /// its period.
    pub is_visible: bool,
    /// When the notice was last edited.
    pub updated_at: DateTime<Utc>,
}

/// The dashboard's per-day sales aggregate for the CSS bar
/// chart: one Japan-calendar day of non-cancelled orders.
#[derive(Debug, Clone, FromRow)]
pub struct SalesDay {
    /// The day, in Asia/Tokyo.
    pub day: NaiveDate,
    /// Sales total in yen, cancelled orders excluded.
    pub total_yen: i64,
}
