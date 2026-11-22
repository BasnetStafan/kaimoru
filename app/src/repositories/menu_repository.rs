//! Menu repository: `menu_items` (CMS spec C-4).

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::MenuItem;

/// The item column set, bound once so every read agrees
/// about the shape.
const ITEM_COLUMNS: &str = "id, menu_key, label, url, is_visible, sort_order";

/// Returns every item of every menu, each menu's items in
/// render order: sort position, then id — the order the
/// rows were inserted, so equal positions keep the
/// operator's sequence.
pub async fn list_all(pool: &PgPool) -> Result<Vec<MenuItem>, AppError> {
    sqlx::query_as::<_, MenuItem>(&format!(
        "SELECT {ITEM_COLUMNS} FROM menu_items ORDER BY menu_key, sort_order, id"
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list menu items", e))
}

/// Deletes one menu's rows, inside the caller's
/// transaction: one half of the replace-all save. The
/// admin menu service re-inserts the menu's kept rows in
/// the same transaction, so a menu is never half-written.
pub async fn delete_menu(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    menu_key: &str,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM menu_items WHERE menu_key = $1")
        .bind(menu_key)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete menu items", e))?;
    Ok(())
}

/// Inserts one item row, inside the caller's transaction:
/// the other half of the replace-all save. The id is
/// generated — a re-inserted row is a new row, and nothing
/// references menu items, so identity carries no meaning
/// across a save.
pub async fn insert_item(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    item: &MenuItem,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO menu_items (menu_key, label, url, is_visible, sort_order) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&item.menu_key)
    .bind(&item.label)
    .bind(&item.url)
    .bind(item.is_visible)
    .bind(item.sort_order)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert menu item", e))?;
    Ok(())
}
