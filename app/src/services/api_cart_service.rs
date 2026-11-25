//! Builds the agent API's cart responses and applies its
//! cart writes, entirely through the storefront's own cart
//! service — an agent's cart obeys the same stock rules,
//! totals and ownership as a person's, because it is the same
//! cart: the one its owner later reviews in the browser.

use serde::Serialize;
use sqlx::PgPool;
use tower_sessions::Session;

use crate::error::AppError;
use crate::repositories::{product_repository, variant_repository};
use crate::services::cart_service::{self, AddOutcome, CartOwner};

/// One cart line as exposed to agents.
#[derive(Serialize)]
pub struct ApiCartLine {
    /// Variant id: the line's identity.
    pub variant_id: i64,
    /// Product id.
    pub product_id: i64,
    /// Product name.
    pub name: String,
    /// Combination label; empty for a plain product.
    pub variant_label: String,
    /// Units in the cart.
    pub quantity: i32,
    /// Unit price in integer yen, tax included.
    pub unit_price_yen: i64,
    /// Line total in integer yen.
    pub line_total_yen: i64,
    /// Whether the line still counts toward the totals (a
    /// product taken off sale after carting does not).
    pub purchasable: bool,
}

/// The caller's cart as exposed to agents. Totals follow the
/// cart page's own rule: no payment method is chosen yet, so
/// no cash-on-delivery fee is included, and a guest cart never
/// carries a coupon discount.
#[derive(Serialize)]
pub struct ApiCart {
    /// The cart's lines, in cart order.
    pub lines: Vec<ApiCartLine>,
    /// Total units across all lines.
    pub item_count: i64,
    /// Sum of purchasable line totals, in yen.
    pub subtotal_yen: i64,
    /// Shipping fee in yen (free at the store's threshold).
    pub shipping_yen: i64,
    /// Grand total in yen: subtotal plus shipping.
    pub total_yen: i64,
}

/// Why an API cart add was refused, in the API's terms. The
/// controller maps each to a status and a stable error code.
#[derive(Debug)]
pub enum AddError {
    /// No such product, or it is not public.
    ProductNotFound,
    /// A combination must be chosen: the product has several
    /// and no variant id was given.
    VariantRequired,
    /// The variant id does not belong to the product.
    VariantNotFound,
    /// The combination is sold out (or disabled).
    SoldOut,
    /// The quantity was outside 1..=999,999.
    InvalidQuantity,
    /// A store error; the detail stays in the server log.
    Internal(AppError),
}

impl From<AppError> for AddError {
    fn from(e: AppError) -> Self {
        Self::Internal(e)
    }
}

impl From<sqlx::Error> for AddError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(AppError::Db(e))
    }
}

/// Renders the owner's cart through the cart page's own
/// totals call, so the API and the page always agree.
async fn view(pool: &PgPool, owner: &CartOwner) -> Result<ApiCart, AppError> {
    let lines = cart_service::lines(pool, owner).await?;
    let tax = crate::services::tax_service::params_today_or_fallback(pool).await;
    let fees = crate::services::shop_settings_service::load(pool)
        .await
        .fee_settings();
    let totals = cart_service::totals(&lines, 0, 0, None, tax, fees);
    Ok(ApiCart {
        lines: lines
            .iter()
            .map(|line| ApiCartLine {
                variant_id: line.variant_id,
                product_id: line.product_id,
                name: line.name.clone(),
                variant_label: line.variant_label.clone(),
                quantity: line.quantity,
                unit_price_yen: line.unit_price_yen,
                line_total_yen: line.line_total_yen(),
                purchasable: line.purchasable(),
            })
            .collect(),
        item_count: lines.iter().map(|line| i64::from(line.quantity)).sum(),
        subtotal_yen: totals.subtotal_yen,
        shipping_yen: totals.shipping_yen,
        total_yen: totals.total_yen,
    })
}

/// Ensures the caller's cart exists (creating the guest cart
/// identity on first use) and returns it.
pub async fn ensure_cart(pool: &PgPool, session: &Session) -> Result<ApiCart, AppError> {
    let owner = cart_service::owner_for(pool, session).await;
    view(pool, &owner).await
}

/// Returns the caller's cart without creating anything: a
/// caller who never added reads an empty cart.
pub async fn read_cart(pool: &PgPool, session: &Session) -> Result<ApiCart, AppError> {
    let owner = cart_service::owner_for_read(pool, session).await;
    view(pool, &owner).await
}

/// Adds units of one product (and combination, when the
/// product has several) to the caller's cart, under the cart
/// service's stock rules. The result is the cart read back
/// after the write, so a capped add shows the quantity that
/// actually landed.
pub async fn add_item(
    pool: &PgPool,
    session: &Session,
    product_id: i64,
    variant_id: Option<i64>,
    quantity: i32,
) -> Result<ApiCart, AddError> {
    if !(1..=999_999).contains(&quantity) {
        return Err(AddError::InvalidQuantity);
    }
    // The API only ever sells the public catalog: a draft or
    // archived product answers exactly like a missing one.
    let product = product_repository::catalog_product(pool, product_id)
        .await?
        .ok_or(AddError::ProductNotFound)?;
    if variant_id.is_none() && product.has_variants {
        return Err(AddError::VariantRequired);
    }
    if let Some(variant_id) = variant_id {
        match variant_repository::find_variant(pool, variant_id).await? {
            Some(variant) if variant.product_id == product_id => {}
            _ => return Err(AddError::VariantNotFound),
        }
    }
    let owner = cart_service::owner_for(pool, session).await;
    match cart_service::add_to_cart(pool, &owner, product_id, variant_id, quantity).await? {
        AddOutcome::Added(_) | AddOutcome::Capped(_) => Ok(view(pool, &owner).await?),
        AddOutcome::SoldOut => Err(AddError::SoldOut),
        // Unreachable after the pre-checks above; mapped to
        // the closest honest answer rather than a panic.
        AddOutcome::NotFound => Err(AddError::VariantNotFound),
    }
}
