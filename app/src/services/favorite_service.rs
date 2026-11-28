//! Favourite rules: the toggle, the per-page card state and
//! the favourites page's product list.
//!
//! Favourites are member-only: the controller's `CurrentUser`
//! extractor has already bounced guests to the login page
//! before any function here runs.

use std::collections::HashSet;

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::Product;
use crate::repositories::{favorite_repository, product_repository};

/// Flips the member's mark on a product and returns the state
/// after the flip. Only a product the storefront itself would
/// show can be marked: the toggle re-reads through the public
/// product lookup, so a draft or archived product id is a 404
/// here, never a mark on an invisible product.
pub async fn toggle(pool: &PgPool, user_id: i64, product_id: i64) -> Result<bool, AppError> {
    if product_repository::find_by_id(pool, product_id)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound);
    }
    if favorite_repository::is_favorite(pool, user_id, product_id).await? {
        favorite_repository::remove(pool, user_id, product_id).await?;
        Ok(false)
    } else {
        favorite_repository::add(pool, user_id, product_id).await?;
        Ok(true)
    }
}

/// Whether the member has favourited the product (the detail
/// page's toggle state).
pub async fn is_favorite(pool: &PgPool, user_id: i64, product_id: i64) -> bool {
    favorite_repository::is_favorite(pool, user_id, product_id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(user_id, product_id, "favourite state failed: {e}");
            false
        })
}

/// The favourited subset of a page of products, for card
/// state: one query per page render, and guests (no user id)
/// get an empty set without touching the database.
pub async fn favorite_id_set(
    pool: &PgPool,
    user_id: Option<i64>,
    product_ids: &[i64],
) -> HashSet<i64> {
    let Some(user_id) = user_id else {
        return HashSet::new();
    };
    if product_ids.is_empty() {
        return HashSet::new();
    }
    match favorite_repository::favorite_ids_among(pool, user_id, product_ids).await {
        Ok(ids) => ids.into_iter().collect(),
        Err(e) => {
            // A failed read degrades to "nothing marked" with a
            // log line: the listing is the page, the hearts only
            // its state.
            tracing::error!(user_id, "favourite card state failed: {e}");
            HashSet::new()
        }
    }
}

/// The member's favourited products as card data, newest mark
/// first. Marks on products that have left the public catalogue
/// are skipped: their card would link to a page that no longer
/// exists.
pub async fn favorite_products(pool: &PgPool, user_id: i64) -> Result<Vec<Product>, AppError> {
    let marks = favorite_repository::list_for_user(pool, user_id).await?;
    let ids: Vec<i64> = marks.iter().map(|mark| mark.product_id).collect();
    Ok(product_repository::find_cards_by_ids(pool, &ids).await?)
}
