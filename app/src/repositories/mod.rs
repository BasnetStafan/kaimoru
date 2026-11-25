//! Repositories for Kaimoru: the only layer that talks
//! to the database. Every query binds its values as parameters.

pub mod admin;
pub mod agent_approval_repository;
pub mod cart_reminder_repository;
pub mod cart_repository;
pub mod category_repository;
pub mod challenge_repository;
pub mod content_page_repository;
pub mod coupon_repository;
pub mod csv_import_batch_repository;
pub mod favorite_repository;
pub mod holiday_repository;
pub mod home_content_repository;
pub mod inquiry_repository;
pub mod low_stock_alert_repository;
pub mod mail_repository;
pub mod member_address_repository;
pub mod menu_repository;
pub mod notice_repository;
pub mod notification_template_repository;
pub mod ops_repository;
pub mod order_repository;
pub mod payment_repository;
pub mod pending_registration_repository;
pub mod point_repository;
pub mod postal_code_repository;
pub mod product_image_repository;
pub mod product_repository;
pub mod restock_repository;
pub mod review_repository;
pub mod scenario_repository;
pub mod search_repository;
pub mod session_revocation_repository;
pub mod shipping_mail_repository;
pub mod shop_settings_repository;
pub mod stock_ledger_repository;
pub mod tag_repository;
pub mod tax_repository;
pub mod totp_repository;
pub mod user_identity_repository;
pub mod user_repository;
pub mod variant_repository;

use sqlx::PgPool;

use crate::error::AppError;

/// Whether `value` is already present in `table.column`,
/// compared case-insensitively, ignoring the row `exclude_id`.
///
/// The table and column names are interpolated into the SQL
/// because identifiers cannot be bound parameters. They are
/// `&'static str` and must only ever be literals naming a fixed
/// table and column from inside this crate — never request
/// data. Only `value` is caller-influenced, and it stays a
/// bound parameter. `context` names the check in the error, in
/// the `AppError::db` style of the calling repository.
pub async fn value_taken(
    pool: &PgPool,
    table: &'static str,
    column: &'static str,
    context: &'static str,
    value: &str,
    exclude_id: Option<i64>,
) -> Result<bool, AppError> {
    sqlx::query_scalar::<_, bool>(&format!(
        "SELECT EXISTS (SELECT 1 FROM {table} \
         WHERE lower({column}) = lower($1) AND ($2::bigint IS NULL OR id <> $2))"
    ))
    .bind(value)
    .bind(exclude_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db(context, e))
}

/// The SQL twin of [`crate::pricing::effective_price_yen`] over the products
/// columns, for the cart and placement queries that price lines
/// in the database: `COALESCE(variant price, this)`.
pub const EFFECTIVE_PRICE_SQL: &str = "CASE WHEN p.sale_price_yen IS NOT NULL \
    AND p.sale_price_yen >= 1 AND p.sale_price_yen < p.price_yen \
    AND (p.sale_starts_at IS NULL OR p.sale_starts_at <= now()) \
    AND (p.sale_ends_at IS NULL OR p.sale_ends_at > now()) \
    THEN p.sale_price_yen ELSE p.price_yen END";

/// [`EFFECTIVE_PRICE_SQL`] rewritten for a query that names
/// the products table something other than `p` (the search
/// query spells it out as `products`). The expression stays
/// defined exactly once, in the constant above.
pub fn effective_price_for(alias: &str) -> String {
    EFFECTIVE_PRICE_SQL.replace("p.", &format!("{alias}."))
}

/// The publication-window predicate over the products columns:
/// a public product inside its optional window. Appended to the
/// storefront's `status = 'public'` filters.
pub const PUBLISH_WINDOW_SQL: &str = "(p.publish_starts_at IS NULL OR p.publish_starts_at <= now()) \
    AND (p.publish_ends_at IS NULL OR p.publish_ends_at > now())";
