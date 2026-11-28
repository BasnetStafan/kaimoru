//! Variant repository.
//!
//! Owns the four variant tables: axes, options, the combination
//! rows that carry stock, and the links between combinations and
//! options. Storefront reads are pool queries; the admin matrix
//! save runs inside the product service's transaction.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::variant::{AxisWithOptions, ProductVariant, VariantAxis, VariantOption};

/// SQL fragment computing one variant row's display label from
/// its linked axes and options (`サイズ: L / カラー: ブラック`),
/// empty for the default variant. Expects the variant table to
/// be aliased `v`. Composing the label in SQL keeps the cart and
/// the placement snapshot on one derivation — the same string
/// the Rust builder [`crate::models::variant::label_for`] makes
/// from pairs, from the same rows.
pub const LABEL_SQL: &str = "COALESCE((SELECT string_agg(a.name || ': ' || o.value, ' / ' \
     ORDER BY a.sort_order, a.id) \
     FROM product_variant_option_links l \
     JOIN product_variant_options o ON o.id = l.option_id \
     JOIN product_variant_axes a ON a.id = o.axis_id \
     WHERE l.variant_id = v.id), '')";

/// Returns a product's axes with their options, both in display
/// order — the shape the picker and the admin editor render.
///
/// Two queries however many axes the product has: every axis's
/// options arrive in one `ANY` lookup and are grouped in
/// memory, where the per-axis loop this replaced issued one
/// query per axis.
pub async fn axes_with_options(
    pool: &PgPool,
    product_id: i64,
) -> Result<Vec<AxisWithOptions>, sqlx::Error> {
    let axes = sqlx::query_as::<_, VariantAxis>(
        "SELECT id, product_id, name, sort_order FROM product_variant_axes \
         WHERE product_id = $1 ORDER BY sort_order, id",
    )
    .bind(product_id)
    .fetch_all(pool)
    .await?;
    if axes.is_empty() {
        return Ok(Vec::new());
    }
    let axis_ids: Vec<i64> = axes.iter().map(|axis| axis.id).collect();
    let flat_options = sqlx::query_as::<_, VariantOption>(
        "SELECT id, axis_id, value, sort_order FROM product_variant_options \
         WHERE axis_id = ANY($1) ORDER BY sort_order, id",
    )
    .bind(&axis_ids)
    .fetch_all(pool)
    .await?;
    // Rows arrive in global display order, so each axis's slice
    // of the grouping keeps its own display order.
    let mut options_by_axis: HashMap<i64, Vec<VariantOption>> = HashMap::new();
    for option in flat_options {
        options_by_axis
            .entry(option.axis_id)
            .or_default()
            .push(option);
    }
    Ok(axes
        .into_iter()
        .map(|axis| AxisWithOptions {
            options: options_by_axis.remove(&axis.id).unwrap_or_default(),
            axis,
        })
        .collect())
}

/// Several products' axes with their options:
/// the bulk sibling of [`axes_with_options`], one
/// query for the axes of every listed product and one for
/// their options.
pub async fn axes_with_options_for_products(
    pool: &PgPool,
    product_ids: &[i64],
) -> Result<Vec<AxisWithOptions>, sqlx::Error> {
    let axes = sqlx::query_as::<_, VariantAxis>(
        "SELECT id, product_id, name, sort_order FROM product_variant_axes \
         WHERE product_id = ANY($1) ORDER BY sort_order, id",
    )
    .bind(product_ids)
    .fetch_all(pool)
    .await?;
    if axes.is_empty() {
        return Ok(Vec::new());
    }
    let axis_ids: Vec<i64> = axes.iter().map(|axis| axis.id).collect();
    let flat_options = sqlx::query_as::<_, VariantOption>(
        "SELECT id, axis_id, value, sort_order FROM product_variant_options \
         WHERE axis_id = ANY($1) ORDER BY sort_order, id",
    )
    .bind(&axis_ids)
    .fetch_all(pool)
    .await?;
    let mut options_by_axis: HashMap<i64, Vec<VariantOption>> = HashMap::new();
    for option in flat_options {
        options_by_axis
            .entry(option.axis_id)
            .or_default()
            .push(option);
    }
    Ok(axes
        .into_iter()
        .map(|axis| AxisWithOptions {
            options: options_by_axis.remove(&axis.id).unwrap_or_default(),
            axis,
        })
        .collect())
}

/// Several products' combination rows:
/// the bulk sibling of [`list_variants`].
pub async fn list_variants_for_products(
    pool: &PgPool,
    product_ids: &[i64],
) -> Result<Vec<ProductVariant>, sqlx::Error> {
    sqlx::query_as::<_, ProductVariant>(
        "SELECT id, product_id, option_signature, stock, price_yen, sku, jan_code, image_path, is_enabled, incoming_qty, incoming_date FROM product_variants \
         WHERE product_id = ANY($1) ORDER BY id",
    )
    .bind(product_ids)
    .fetch_all(pool)
    .await
}

/// Several products' variant-option links:
/// the bulk sibling of [`links_for`], as
/// (variant_id, option_id) pairs.
pub async fn links_for_products(
    pool: &PgPool,
    product_ids: &[i64],
) -> Result<Vec<(i64, i64)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT l.variant_id, l.option_id FROM product_variant_option_links l \
         JOIN product_variants v ON v.id = l.variant_id \
         WHERE v.product_id = ANY($1)",
    )
    .bind(product_ids)
    .fetch_all(pool)
    .await
}

/// Returns a product's combination rows in ID order.
pub async fn list_variants(
    pool: &PgPool,
    product_id: i64,
) -> Result<Vec<ProductVariant>, sqlx::Error> {
    sqlx::query_as::<_, ProductVariant>(
        "SELECT id, product_id, option_signature, stock, price_yen, sku, jan_code, image_path, is_enabled, incoming_qty, incoming_date FROM product_variants \
         WHERE product_id = $1 ORDER BY id",
    )
    .bind(product_id)
    .fetch_all(pool)
    .await
}

/// Loads one combination row by ID.
pub async fn find_variant(
    pool: &PgPool,
    variant_id: i64,
) -> Result<Option<ProductVariant>, sqlx::Error> {
    sqlx::query_as::<_, ProductVariant>(
        "SELECT id, product_id, option_signature, stock, price_yen, sku, jan_code, image_path, is_enabled, incoming_qty, incoming_date FROM product_variants WHERE id = $1",
    )
    .bind(variant_id)
    .fetch_optional(pool)
    .await
}

/// Returns a product's combinations as (option values in axis
/// order, stock) pairs: the admin grid's prefill source. Matching
/// by value (not ID) is what lets a renamed or reordered option
/// keep its stock in the editor.
pub async fn combos_with_values(
    pool: &PgPool,
    product_id: i64,
) -> Result<Vec<(Vec<String>, i32)>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, i32, Option<String>)>(
        "SELECT v.id, v.stock, o.value \
         FROM product_variants v \
         LEFT JOIN product_variant_option_links l ON l.variant_id = v.id \
         LEFT JOIN product_variant_options o ON o.id = l.option_id \
         LEFT JOIN product_variant_axes a ON a.id = o.axis_id \
         WHERE v.product_id = $1 \
         ORDER BY v.id, a.sort_order, a.id",
    )
    .bind(product_id)
    .fetch_all(pool)
    .await?;
    // Rows arrive grouped by variant (v.id leads the ORDER BY);
    // a new group starts whenever the variant changes. A default
    // variant has no link rows beyond the LEFT JOIN's null row,
    // so its value list stays empty.
    let mut out: Vec<(Vec<String>, i32)> = Vec::new();
    let mut current: Option<i64> = None;
    for (variant_id, stock, value) in rows {
        if current != Some(variant_id) {
            out.push((Vec::new(), stock));
            current = Some(variant_id);
        }
        if let Some(v) = value {
            out.last_mut().expect("a group exists").0.push(v);
        }
    }
    Ok(out)
}

/// Returns the IDs of a product's variants that appear in any
/// order: the D5-protected set, whose combinations cannot be
/// removed (cancellation restocks by variant).
pub async fn ordered_variant_ids(pool: &PgPool, product_id: i64) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT variant_id FROM order_items \
         WHERE variant_id IS NOT NULL \
           AND variant_id IN (SELECT id FROM product_variants WHERE product_id = $1)",
    )
    .bind(product_id)
    .fetch_all(pool)
    .await
}

/// Returns every (variant, option) link of a product: the
/// reconciliation input of the admin matrix save.
pub async fn links_for(pool: &PgPool, product_id: i64) -> Result<Vec<(i64, i64)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT l.variant_id, l.option_id FROM product_variant_option_links l \
         JOIN product_variants v ON v.id = l.variant_id \
         WHERE v.product_id = $1",
    )
    .bind(product_id)
    .fetch_all(pool)
    .await
}

/// Inserts one axis inside the caller's transaction.
pub async fn insert_axis(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    product_id: i64,
    name: &str,
    sort_order: i32,
) -> Result<i64, AppError> {
    sqlx::query_scalar(
        "INSERT INTO product_variant_axes (product_id, name, sort_order) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(product_id)
    .bind(name)
    .bind(sort_order)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert variant axis", e))
}

/// Updates one axis's name and display order.
pub async fn update_axis(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    name: &str,
    sort_order: i32,
) -> Result<(), AppError> {
    sqlx::query("UPDATE product_variant_axes SET name = $2, sort_order = $3 WHERE id = $1")
        .bind(id)
        .bind(name)
        .bind(sort_order)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("update variant axis", e))?;
    Ok(())
}

/// Deletes one axis (its options and links cascade).
pub async fn delete_axis(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM product_variant_axes WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete variant axis", e))?;
    Ok(())
}

/// Inserts one option inside the caller's transaction.
pub async fn insert_option(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    axis_id: i64,
    value: &str,
    sort_order: i32,
) -> Result<i64, AppError> {
    sqlx::query_scalar(
        "INSERT INTO product_variant_options (axis_id, value, sort_order) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(axis_id)
    .bind(value)
    .bind(sort_order)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert variant option", e))
}

/// Updates one option's value and display order. The option
/// keeps its ID, so combinations keep their identity across a
/// rename (only future order labels use the new text).
pub async fn update_option(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    value: &str,
    sort_order: i32,
) -> Result<(), AppError> {
    sqlx::query("UPDATE product_variant_options SET value = $2, sort_order = $3 WHERE id = $1")
        .bind(id)
        .bind(value)
        .bind(sort_order)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("update variant option", e))?;
    Ok(())
}

/// Deletes one option (its combination links cascade).
pub async fn delete_option(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM product_variant_options WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete variant option", e))?;
    Ok(())
}

/// The per-combination registration fields the admin grid
/// writes: stock plus the Phase 4/5 additions. `None` price
/// means the product's effective price; `None` image means the
/// product gallery.
#[derive(Clone, Debug, Default)]
pub struct VariantFields {
    /// Units in stock.
    pub stock: i32,
    /// Own price (tax included), when set.
    pub price_yen: Option<i64>,
    /// Own SKU, when registered.
    pub sku: Option<String>,
    /// Own JAN code, when registered.
    pub jan_code: Option<String>,
    /// Own image (one of the product's gallery paths), when set.
    pub image_path: Option<String>,
    /// Whether the combination is offered.
    pub is_enabled: bool,
    /// Units expected to arrive.
    pub incoming_qty: i32,
    /// Expected arrival date.
    pub incoming_date: Option<chrono::NaiveDate>,
}

/// Inserts one combination row inside the caller's transaction.
pub async fn insert_variant(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    product_id: i64,
    signature: &str,
    fields: &VariantFields,
) -> Result<i64, AppError> {
    sqlx::query_scalar(
        "INSERT INTO product_variants (product_id, option_signature, stock, price_yen, \
         sku, jan_code, image_path, is_enabled, incoming_qty, incoming_date) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
    )
    .bind(product_id)
    .bind(signature)
    .bind(fields.stock)
    .bind(fields.price_yen)
    .bind(&fields.sku)
    .bind(&fields.jan_code)
    .bind(&fields.image_path)
    .bind(fields.is_enabled)
    .bind(fields.incoming_qty)
    .bind(fields.incoming_date)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert variant", e))
}

/// One combination's new state for [`update_variants`]: the
/// row it lands on, the signature it takes, and the grid fields
/// it is stored with.
pub struct VariantUpdate<'a> {
    /// The stored combination row.
    pub id: i64,
    /// The option signature the row takes.
    pub signature: &'a str,
    /// The registration fields the row is stored with.
    pub fields: &'a VariantFields,
}

/// Updates many combinations' signatures and registration
/// fields in ONE statement: a `VALUES` list joined onto the
/// variant rows, the same shape as the order placement stock
/// decrement. An empty batch runs no statement.
pub async fn update_variants(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    updates: &[VariantUpdate<'_>],
) -> Result<(), AppError> {
    if updates.is_empty() {
        return Ok(());
    }
    let mut values = String::new();
    for i in 0..updates.len() {
        if i > 0 {
            values.push_str(", ");
        }
        let b = i * 10;
        values.push_str(&format!(
            "(${}::bigint, ${}::text, ${}::int, ${}::bigint, ${}::text, ${}::text, \
             ${}::text, ${}::boolean, ${}::int, ${}::date)",
            b + 1,
            b + 2,
            b + 3,
            b + 4,
            b + 5,
            b + 6,
            b + 7,
            b + 8,
            b + 9,
            b + 10
        ));
    }
    let sql = format!(
        "UPDATE product_variants v SET option_signature = x.option_signature, \
         stock = x.stock, price_yen = x.price_yen, sku = x.sku, jan_code = x.jan_code, \
         image_path = x.image_path, is_enabled = x.is_enabled, \
         incoming_qty = x.incoming_qty, incoming_date = x.incoming_date, \
         updated_at = now() \
         FROM (VALUES {values}) AS x(id, option_signature, stock, price_yen, sku, \
         jan_code, image_path, is_enabled, incoming_qty, incoming_date) \
         WHERE v.id = x.id"
    );
    let mut query = sqlx::query(&sql);
    for update in updates {
        query = query
            .bind(update.id)
            .bind(update.signature)
            .bind(update.fields.stock)
            .bind(update.fields.price_yen)
            .bind(&update.fields.sku)
            .bind(&update.fields.jan_code)
            .bind(&update.fields.image_path)
            .bind(update.fields.is_enabled)
            .bind(update.fields.incoming_qty)
            .bind(update.fields.incoming_date);
    }
    query
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("update variants", e))?;
    Ok(())
}

/// Whether any of the variants carries stock-ledger
/// history: history makes a variant
/// undeletable - the ledger is append-only and its FK is
/// RESTRICT - so the matrix save refuses up front.
pub async fn variants_with_stock_history(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    variant_ids: &[i64],
) -> Result<bool, AppError> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM stock_adjustments WHERE variant_id = ANY($1))")
        .bind(variant_ids)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| AppError::db("variant stock history", e))
}

/// Deletes one combination row; its cart rows cascade away with
/// it, as the spec requires for never-ordered combinations.
pub async fn delete_variant(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM product_variants WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete variant", e))?;
    Ok(())
}

/// Replaces the option links of many combinations at once:
/// ONE delete clears every listed combination's links, then
/// ONE multi-row insert writes all the (combination, option)
/// pairs. A combination listed with no pair ends unlinked —
/// the default combination of an axis-less product.
pub async fn replace_links(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    variant_ids: &[i64],
    pairs: &[(i64, i64)],
) -> Result<(), AppError> {
    if !variant_ids.is_empty() {
        sqlx::query("DELETE FROM product_variant_option_links WHERE variant_id = ANY($1)")
            .bind(variant_ids)
            .execute(&mut **tx)
            .await
            .map_err(|e| AppError::db("clear variant links", e))?;
    }
    if pairs.is_empty() {
        return Ok(());
    }
    let mut values = String::new();
    for i in 0..pairs.len() {
        if i > 0 {
            values.push_str(", ");
        }
        let b = i * 2;
        values.push_str(&format!("(${}::bigint, ${}::bigint)", b + 1, b + 2));
    }
    let sql =
        format!("INSERT INTO product_variant_option_links (variant_id, option_id) VALUES {values}");
    let mut query = sqlx::query(&sql);
    for (variant_id, option_id) in pairs {
        query = query.bind(variant_id).bind(option_id);
    }
    query
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("insert variant links", e))?;
    Ok(())
}
