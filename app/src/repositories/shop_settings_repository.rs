//! Shop settings repository: the single `shop_settings` row.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::ShopSettings;

/// The settings row's column set, bound once so the read and
/// the model can never disagree about the shape.
const COLUMNS: &str = "shop_name, tagline, shop_description, contact_email, \
    contact_url, support_note, footer_note, copyright_holder, top_title, \
    default_meta_description, mail_signature, shipping_flat_yen, \
    free_shipping_threshold_yen, cod_fee_yen, delivery_earliest_offset_days, \
    delivery_latest_offset_days, disabled_delivery_slots, disabled_carriers, \
    invoice_registration_number, maintenance_mode, maintenance_message, \
    points_enabled, points_earn_percent, points_expiry_days";

/// Returns the shop settings row, or `None` when it is missing
/// (callers fall back to [`ShopSettings::fallback`]).
pub async fn find(pool: &PgPool) -> Result<Option<ShopSettings>, AppError> {
    sqlx::query_as::<_, ShopSettings>(&format!("SELECT {COLUMNS} FROM shop_settings WHERE id = 1"))
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("find shop settings", e))
}

/// The settings row's current `updated_at`: the stamp a
/// settings form ticket is minted with at render time.
/// (The `ShopSettings` struct does not carry the stamp,
/// so it is read on its own, like the tag's.)
pub async fn updated_at(pool: &PgPool) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM shop_settings WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(value)
}

/// Returns the settings row's current `updated_at` with
/// the row locked: the settings forms' staleness guard
/// compares its form ticket's stamp against this value
/// inside the update transaction (the product guard's
/// shape).
pub async fn updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM shop_settings WHERE id = 1 FOR UPDATE",
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

/// Stores the settings inside the caller's transaction,
/// stamping `updated_at` in the statement.
pub async fn update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    settings: &ShopSettings,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE shop_settings SET shop_name = $1, tagline = $2, \
                shop_description = $3, contact_email = $4, contact_url = $5, \
                support_note = $6, footer_note = $7, copyright_holder = $8, \
                top_title = $9, default_meta_description = $10, \
                mail_signature = $11, shipping_flat_yen = $12, \
                free_shipping_threshold_yen = $13, cod_fee_yen = $14, \
                delivery_earliest_offset_days = $15, \
                delivery_latest_offset_days = $16, \
                disabled_delivery_slots = $17, disabled_carriers = $18, \
                invoice_registration_number = $19, maintenance_mode = $20, \
                maintenance_message = $21, points_enabled = $22, \
                points_earn_percent = $23, points_expiry_days = $24, \
                updated_at = now(), updated_by = $25 \
         WHERE id = 1",
    )
    .bind(&settings.shop_name)
    .bind(&settings.tagline)
    .bind(&settings.shop_description)
    .bind(&settings.contact_email)
    .bind(&settings.contact_url)
    .bind(&settings.support_note)
    .bind(&settings.footer_note)
    .bind(&settings.copyright_holder)
    .bind(&settings.top_title)
    .bind(&settings.default_meta_description)
    .bind(&settings.mail_signature)
    .bind(settings.shipping_flat_yen)
    .bind(settings.free_shipping_threshold_yen)
    .bind(settings.cod_fee_yen)
    .bind(settings.delivery_earliest_offset_days)
    .bind(settings.delivery_latest_offset_days)
    .bind(&settings.disabled_delivery_slots)
    .bind(&settings.disabled_carriers)
    .bind(&settings.invoice_registration_number)
    .bind(settings.maintenance_mode)
    .bind(&settings.maintenance_message)
    .bind(settings.points_enabled)
    .bind(settings.points_earn_percent)
    .bind(settings.points_expiry_days)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update shop settings", e))?;
    Ok(())
}
