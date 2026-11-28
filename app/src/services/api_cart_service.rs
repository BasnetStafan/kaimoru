//! Builds the agent API's cart responses and applies its
//! cart writes, entirely through the storefront's own cart
//! service — an agent's cart obeys the same stock rules,
//! totals and ownership as a person's, because it is the same
//! cart: the one its owner later reviews in the browser.

use sqlx::PgPool;
use tower_sessions::Session;

use crate::error::AppError;
use crate::repositories::{product_repository, variant_repository};
use crate::services::cart_service::{self, AddOutcome, CartOwner};

pub use mcp_defs::{ApiCart, ApiCartLine};

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

/// Renders one already-resolved owner's cart. The MCP cart
/// tools resolve their owner from the cart handle instead of
/// a session; everything below that resolution is this same
/// view, so all three surfaces agree on the numbers.
pub async fn view_for_owner(pool: &PgPool, owner: &CartOwner) -> Result<ApiCart, AppError> {
    view(pool, owner).await
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
    // The pre-checks run before the owner is resolved on
    // purpose: resolving a guest owner mints a cart token,
    // and a refused add must not mint one.
    check_add(pool, product_id, variant_id, quantity).await?;
    let owner = cart_service::owner_for(pool, session).await;
    apply_add(pool, &owner, product_id, variant_id, quantity).await
}

/// Adds to an already-resolved owner's cart: the MCP cart
/// tools resolve their owner from the cart handle, then run
/// exactly this path — same public-catalog pre-checks, same
/// stock rules, same read-back — as the session-shaped
/// [`add_item`].
pub async fn add_item_for_owner(
    pool: &PgPool,
    owner: &CartOwner,
    product_id: i64,
    variant_id: Option<i64>,
    quantity: i32,
) -> Result<ApiCart, AddError> {
    check_add(pool, product_id, variant_id, quantity).await?;
    apply_add(pool, owner, product_id, variant_id, quantity).await
}

/// The add path's public-catalog pre-checks, shared by both
/// owner shapes: quantity range, the product is public, the
/// combination (when the product has several) is chosen and
/// belongs to the product.
async fn check_add(
    pool: &PgPool,
    product_id: i64,
    variant_id: Option<i64>,
    quantity: i32,
) -> Result<(), AddError> {
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
    Ok(())
}

/// Applies a pre-checked add through the cart service and
/// reads the cart back, so a capped add shows the quantity
/// that actually landed.
async fn apply_add(
    pool: &PgPool,
    owner: &CartOwner,
    product_id: i64,
    variant_id: Option<i64>,
    quantity: i32,
) -> Result<ApiCart, AddError> {
    match cart_service::add_to_cart(pool, owner, product_id, variant_id, quantity).await? {
        AddOutcome::Added(_) | AddOutcome::Capped(_) => Ok(view(pool, owner).await?),
        AddOutcome::SoldOut => Err(AddError::SoldOut),
        // Unreachable after the pre-checks above; mapped to
        // the closest honest answer rather than a panic.
        AddOutcome::NotFound => Err(AddError::VariantNotFound),
    }
}
