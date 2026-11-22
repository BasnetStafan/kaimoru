//! Ordered image lists for products: the `product_images`
//! rows behind the admin gallery editor and the storefront
//! detail gallery. `sort_order` 0 is the main image;
//! `products.image_path` mirrors it and is rewritten by the
//! admin product service in the same transaction as
//! [`replace_images`], so the existing render sites keep
//! reading the main image from the column they already know.

use sqlx::{PgPool, Postgres, Transaction};

use crate::error::AppError;

/// Returns a product's image paths in display order (main
/// first). An empty vec means the product has no images —
/// possible only for drafts; the publish gate in the admin
/// product service keeps public products imaged.
pub async fn list_for_product(pool: &PgPool, product_id: i64) -> Result<Vec<String>, AppError> {
    sqlx::query_scalar::<_, String>(
        "SELECT image_path FROM product_images \
         WHERE product_id = $1 ORDER BY sort_order, id",
    )
    .bind(product_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("load product images", e))
}

/// Replaces a product's whole image list inside the caller's
/// transaction: the submitted order becomes sort_order 0..n,
/// so the first path is the new main image. Detaching an image
/// removes only its row — the file itself is never deleted
/// (uploads are stored under unique names and the built-in
/// assets are tracked files).
pub async fn replace_images(
    tx: &mut Transaction<'_, Postgres>,
    product_id: i64,
    images: &[(String, String)],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM product_images WHERE product_id = $1")
        .bind(product_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("clear product images", e))?;
    for (index, (path, alt)) in images.iter().enumerate() {
        sqlx::query(
            "INSERT INTO product_images (product_id, image_path, alt_text, sort_order) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(product_id)
        .bind(path)
        .bind(alt)
        .bind(index as i32)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("save product image", e))?;
    }
    Ok(())
}

/// Returns a product's images with their alt texts, in display
/// order — the shape the detail gallery and the admin editor
/// render.
pub async fn list_detailed(
    pool: &PgPool,
    product_id: i64,
) -> Result<Vec<(String, String)>, AppError> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT image_path, alt_text FROM product_images \
         WHERE product_id = $1 ORDER BY sort_order, id",
    )
    .bind(product_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("load product images with alt", e))
}
