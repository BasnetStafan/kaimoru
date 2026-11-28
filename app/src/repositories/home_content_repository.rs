//! Home content repository: `home_slots` and
//! `home_sections` (CMS spec C-3).

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{HeroImage, HomePin, HomeSection, HomeSlot, ProductOption};

/// The slot column set, bound once so every read agrees
/// about the shape.
const SLOT_COLUMNS: &str = "key, headline, body, cta_label, cta_url, \
    is_enabled, starts_at, ends_at, updated_at";

/// The section column set, bound once so every read agrees
/// about the shape.
const SECTION_COLUMNS: &str =
    "section_key, heading, description, is_visible, sort_order, updated_at";

/// Returns every slot row. There are exactly two (the
/// announcement and the hero); callers pick theirs by key.
pub async fn list_slots(pool: &PgPool) -> Result<Vec<HomeSlot>, AppError> {
    sqlx::query_as::<_, HomeSlot>(&format!(
        "SELECT {SLOT_COLUMNS} FROM home_slots ORDER BY key"
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list home slots", e))
}

/// Returns every section row, in no promised order: the
/// service merges them over the canonical key set and sorts
/// there.
pub async fn list_sections(pool: &PgPool) -> Result<Vec<HomeSection>, AppError> {
    sqlx::query_as::<_, HomeSection>(&format!("SELECT {SECTION_COLUMNS} FROM home_sections"))
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list home sections", e))
}

/// Stores one slot row's content fields and stamps its
/// `updated_at`, inside the caller's transaction.
pub async fn update_slot(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    slot: &HomeSlot,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE home_slots SET headline = $2, body = $3, cta_label = $4, \
         cta_url = $5, is_enabled = $6, starts_at = $7, ends_at = $8, \
         updated_at = now(), updated_by = $9 WHERE key = $1",
    )
    .bind(&slot.key)
    .bind(&slot.headline)
    .bind(&slot.body)
    .bind(&slot.cta_label)
    .bind(&slot.cta_url)
    .bind(slot.is_enabled)
    .bind(slot.starts_at)
    .bind(slot.ends_at)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update home slot", e))?;
    Ok(())
}

/// Stores one section row's fields and stamps its
/// `updated_at`, inside the caller's transaction.
pub async fn update_section(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    section: &HomeSection,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE home_sections SET heading = $2, description = $3, is_visible = $4, \
         sort_order = $5, updated_at = now(), updated_by = $6 WHERE section_key = $1",
    )
    .bind(&section.section_key)
    .bind(&section.heading)
    .bind(&section.description)
    .bind(section.is_visible)
    .bind(section.sort_order)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update home section", e))?;
    Ok(())
}

/// Returns the hero's slides, in display order.
pub async fn list_hero_images(pool: &PgPool) -> Result<Vec<HeroImage>, AppError> {
    sqlx::query_as::<_, HeroImage>(
        "SELECT image_path, link_url FROM home_hero_images \
         WHERE slot_key = 'hero' ORDER BY sort_order, image_path",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list hero images", e))
}

/// Replaces the hero's slide list with `images`, in the
/// given order, inside the caller's transaction.
pub async fn replace_hero_images(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    images: &[HeroImage],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM home_hero_images WHERE slot_key = 'hero'")
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("clear hero images", e))?;
    for (index, image) in images.iter().enumerate() {
        sqlx::query(
            "INSERT INTO home_hero_images (slot_key, image_path, link_url, sort_order) \
             VALUES ('hero', $1, $2, $3)",
        )
        .bind(&image.image_path)
        .bind(&image.link_url)
        .bind(((index + 1) * 10) as i32)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("store hero image", e))?;
    }
    Ok(())
}

/// Returns every stored pin, in section and list order.
pub async fn list_pins(pool: &PgPool) -> Result<Vec<HomePin>, AppError> {
    sqlx::query_as::<_, HomePin>(
        "SELECT section_key, product_id, sort_order FROM home_section_pins \
         ORDER BY section_key, sort_order, product_id",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list home pins", e))
}

/// Returns every product as a pin editor option, in id
/// order: drafts included, because a pin placed ahead of a
/// launch renders once the product is public.
pub async fn list_product_options(pool: &PgPool) -> Result<Vec<ProductOption>, AppError> {
    sqlx::query_as::<_, ProductOption>("SELECT id, name, status FROM products ORDER BY id")
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list pin product options", e))
}

/// Replaces one section's whole pin list inside the
/// caller's transaction: the editor submits the full list,
/// so the stored set is exactly the submitted order.
pub async fn replace_pins(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    section_key: &str,
    product_ids: &[i64],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM home_section_pins WHERE section_key = $1")
        .bind(section_key)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("clear home pins", e))?;
    for (index, product_id) in product_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO home_section_pins (section_key, product_id, sort_order) \
             VALUES ($1, $2, $3)",
        )
        .bind(section_key)
        .bind(product_id)
        .bind((index as i32 + 1) * 10)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("store home pin", e))?;
    }
    Ok(())
}
