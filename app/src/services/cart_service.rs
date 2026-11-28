//! Cart service.
//!
//! Owns cart identity, quantity/stock rules and the totals math.
//! Prices always come from the database, never from the request.
//!
//! This service does take the Session — under one rule: a service
//! may receive the Session only to resolve who is acting
//! (identity/owner resolution), never to carry business state
//! between steps. Every Session use below is owner resolution.
//! The checkout flow, which is business state, goes through
//! `crate::session`'s helpers instead, and order_service never
//! sees a Session at all.

use rand::RngCore;
use sqlx::PgPool;
use tower_sessions::Session;

use tokio::sync::watch::Receiver;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::models::{CartLine, PaymentMethod, User};
use crate::repositories::{cart_repository, product_repository, variant_repository};
use crate::session;
use crate::session::SESSION_CART_OWNER;

// The cart's money rules are defined once in [`crate::pricing`]
// and re-exported here: most consumers already look in this
// module, and the payment-method labels (in models) read the
// same values from pricing without a layering inversion.
use crate::models::FeeSettings;

/// Who owns a cart: a logged-in member, or a guest session.
#[derive(Clone, Debug)]
pub enum CartOwner {
    /// Member cart, keyed by user ID.
    User(i64),
    /// Guest cart, keyed by the session's cart owner token.
    Session(String),
}

impl CartOwner {
    /// The user ID half of the owner key, for repository calls.
    pub fn user_id(&self) -> Option<i64> {
        match self {
            Self::User(id) => Some(*id),
            Self::Session(_) => None,
        }
    }

    /// The session token half of the owner key.
    pub fn session_key(&self) -> Option<&str> {
        match self {
            Self::User(_) => None,
            Self::Session(token) => Some(token),
        }
    }
}

/// Resolves the cart owner for this request. Members use their
/// user ID. Guests get a random owner token stored in the session,
/// created on first cart use (never on plain page views, so
/// browsing alone does not mint cart identities).
pub async fn owner_for(pool: &PgPool, session: &Session) -> CartOwner {
    if let Some(user) = session::current_user(pool, session).await {
        return CartOwner::User(user.id);
    }
    CartOwner::Session(guest_token(session, true).await)
}

/// The guest cart owner token if one already exists; never creates
/// one. Used where creating state would be wrong (page headers).
pub async fn existing_guest_token(session: &Session) -> Option<String> {
    session
        .get::<String>(SESSION_CART_OWNER)
        .await
        .ok()
        .flatten()
}

/// Gets the guest cart token, optionally creating and storing one.
async fn guest_token(session: &Session, create: bool) -> String {
    if let Some(token) = existing_guest_token(session).await {
        return token;
    }
    if !create {
        return String::new();
    }
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let _ = session.insert(SESSION_CART_OWNER, token.clone()).await;
    token
}

/// Returns the header badge count for this request, without
/// minting any state: guests with no cart yet simply count 0.
///
/// The count needs only the member's ID, never the full user row:
/// when the caller already loaded the user it passes it in;
/// otherwise the ID is read straight from the session record, so
/// the badge costs no users-table query.
pub async fn count_for_header(pool: &PgPool, session: &Session, user: Option<&User>) -> i64 {
    // A failed count reads as 0 for the badge, but it is logged:
    // a silent 0 would hide a database outage behind an
    // empty-looking cart.
    let counted = if let Some(user) = user {
        cart_repository::total_quantity(pool, Some(user.id), None).await
    } else if let Some(user_id) = session
        .get::<i64>(crate::session::SESSION_USER_ID)
        .await
        .ok()
        .flatten()
    {
        cart_repository::total_quantity(pool, Some(user_id), None).await
    } else {
        match existing_guest_token(session).await {
            Some(token) => cart_repository::total_quantity(pool, None, Some(&token)).await,
            None => Ok(0),
        }
    };
    match counted {
        Ok(n) => n,
        Err(e) => {
            tracing::error!("header cart count failed: {e}");
            0
        }
    }
}

/// Returns the cart owner for read-only views: never mints a
/// guest token, so simply viewing pages creates no cart state.
/// The write-side sibling is [`owner_for`].
pub async fn owner_for_read(pool: &PgPool, session: &Session) -> CartOwner {
    if let Some(user) = session::current_user(pool, session).await {
        return CartOwner::User(user.id);
    }
    match existing_guest_token(session).await {
        Some(token) => CartOwner::Session(token),
        None => CartOwner::Session(String::new()),
    }
}

/// The outcome of one add-to-cart attempt.
pub enum AddOutcome {
    /// Added; carries the new header badge count.
    Added(i64),
    /// The cart already holds every unit in stock.
    Capped(i64),
    /// The product is sold out.
    SoldOut,
    /// No such product.
    NotFound,
}

/// Adds units of one combination of a product to the owner's
/// cart under the stock rules: a tracked, non-backorder product
/// never takes more than the variant's stock in total, and a
/// sold-out combination never sells; untracked and
/// backorderable products sell past stock, capped only by the
/// quantity ceiling.
///
/// The variant is resolved here, never trusted from the request:
/// an explicitly posted variant must belong to the product (a
/// forged ID from another product is answered like a missing
/// product), and a missing variant resolves only when the
/// product has exactly one combination — a card can add a plain
/// product directly, but a combination is never chosen silently
/// for a multi-variant product.
pub async fn add_to_cart(
    pool: &PgPool,
    owner: &CartOwner,
    product_id: i64,
    variant_id: Option<i64>,
    qty: i32,
) -> Result<AddOutcome, AppError> {
    add_impl(pool, owner, product_id, variant_id, qty, false).await
}

/// Adds units of one combination to the owner's cart as the
/// shopping agent: the line lands marked
/// `added_by_agent`, so a later run may replace it. The
/// variant is always explicit — the agent has already chosen
/// the combination it means — and the stock rules are the
/// same ones a person's add obeys, through the same code.
pub async fn add_to_cart_as_agent(
    pool: &PgPool,
    owner: &CartOwner,
    product_id: i64,
    variant_id: i64,
    qty: i32,
) -> Result<AddOutcome, AppError> {
    add_impl(pool, owner, product_id, Some(variant_id), qty, true).await
}

/// The one add path behind [`add_to_cart`] and
/// [`add_to_cart_as_agent`]; `added_by_agent` is the only
/// difference between a person's add and the agent's.
async fn add_impl(
    pool: &PgPool,
    owner: &CartOwner,
    product_id: i64,
    variant_id: Option<i64>,
    qty: i32,
    added_by_agent: bool,
) -> Result<AddOutcome, AppError> {
    // A non-positive quantity is refused, never reinterpreted
    // as one unit: an add asks for at least one
    // unit, or the request is malformed. The refusal answers
    // like the stock cap — nothing is added, no toast.
    if qty < 1 {
        return Ok(AddOutcome::Capped(count(pool, owner).await?));
    }
    let Some(product) = product_repository::find_by_id(pool, product_id).await? else {
        return Ok(AddOutcome::NotFound);
    };
    let variant = match variant_id {
        Some(variant_id) => match variant_repository::find_variant(pool, variant_id).await? {
            Some(variant) if variant.product_id == product.id => variant,
            _ => return Ok(AddOutcome::NotFound),
        },
        None => {
            let variants = variant_repository::list_variants(pool, product.id).await?;
            match variants.as_slice() {
                [only] => only.clone(),
                _ => return Ok(AddOutcome::NotFound),
            }
        }
    };
    // A disabled combination is not offered at all: it answers
    // exactly like a sold-out one. An untracked product never
    // sells out, and a backorderable one sells past zero; both
    // cap at the amount ceiling instead of the stock.
    if !variant.is_enabled {
        return Ok(AddOutcome::SoldOut);
    }
    let stock_cap = if !product.track_stock || product.allow_backorder {
        i32::MAX
    } else {
        variant.stock
    };
    if stock_cap <= 0 {
        return Ok(AddOutcome::SoldOut);
    }
    // The current quantity is read only to report the outcome;
    // the cap itself is enforced inside the upsert, so two adds
    // racing on this read cannot overshoot the stock — the
    // database clamps both, and the second read sees the result.
    let current =
        cart_repository::quantity_of(pool, owner.user_id(), owner.session_key(), variant.id)
            .await?;
    let wanted = current.saturating_add(qty);
    if wanted <= current {
        return Ok(AddOutcome::Capped(count(pool, owner).await?));
    }
    insert_add(pool, owner, variant.id, qty, stock_cap, added_by_agent).await?;
    let actual =
        cart_repository::quantity_of(pool, owner.user_id(), owner.session_key(), variant.id)
            .await?;
    let count = count(pool, owner).await?;
    if actual < wanted {
        Ok(AddOutcome::Capped(count))
    } else {
        Ok(AddOutcome::Added(count))
    }
}

/// Routes the upsert to the statement matching the owner kind
/// (each partial unique index needs its own ON CONFLICT target).
async fn insert_add(
    pool: &PgPool,
    owner: &CartOwner,
    variant_id: i64,
    qty: i32,
    stock_cap: i32,
    added_by_agent: bool,
) -> Result<(), sqlx::Error> {
    match owner {
        CartOwner::User(user_id) => {
            cart_repository::add(
                pool,
                Some(*user_id),
                None,
                variant_id,
                qty,
                stock_cap,
                added_by_agent,
            )
            .await
        }
        CartOwner::Session(token) => {
            cart_repository::add_for_session(
                pool,
                token,
                variant_id,
                qty,
                stock_cap,
                added_by_agent,
            )
            .await
        }
    }
}

/// The owner's current badge count.
async fn count(pool: &PgPool, owner: &CartOwner) -> Result<i64, AppError> {
    Ok(cart_repository::total_quantity(pool, owner.user_id(), owner.session_key()).await?)
}

/// The owner's cart lines.
pub async fn lines(pool: &PgPool, owner: &CartOwner) -> Result<Vec<CartLine>, AppError> {
    Ok(cart_repository::list_lines(pool, owner.user_id(), owner.session_key()).await?)
}

/// Sets a line's quantity from the cart page stepper, clamped
/// to 1..= the variant's stock for a tracked, non-backorder
/// product, and to 1..= the amount ceiling otherwise. Removing
/// is a separate, explicit action.
pub async fn update_quantity(
    pool: &PgPool,
    owner: &CartOwner,
    variant_id: i64,
    qty: i32,
) -> Result<(), AppError> {
    let Some(variant) = variant_repository::find_variant(pool, variant_id).await? else {
        return Ok(());
    };
    // The cap follows the product's stock mode: tracked and not
    // backorderable clamps to stock; the other modes clamp only
    // to the amount ceiling.
    let product = product_repository::find_by_id(pool, variant.product_id).await?;
    let capped = match &product {
        Some(p) if !p.track_stock || p.allow_backorder => qty.clamp(1, 999_999),
        _ => qty.clamp(1, variant.stock.max(1)),
    };
    let target = capped;
    Ok(cart_repository::set_quantity(
        pool,
        owner.user_id(),
        owner.session_key(),
        variant_id,
        target,
    )
    .await?)
}

/// Removes a line from the owner's cart.
pub async fn remove_line(
    pool: &PgPool,
    owner: &CartOwner,
    variant_id: i64,
) -> Result<(), AppError> {
    Ok(
        cart_repository::remove_line(pool, owner.user_id(), owner.session_key(), variant_id)
            .await?,
    )
}

/// Removes every agent-added line from the owner's cart
/// a run calls this before writing its own
/// picks, so the lines it replaces are exactly its own.
/// Lines the owner added by hand are never touched.
pub async fn delete_agent_lines(pool: &PgPool, owner: &CartOwner) -> Result<(), AppError> {
    Ok(cart_repository::delete_agent_lines(pool, owner.user_id(), owner.session_key()).await?)
}

/// Money totals for a set of cart lines. Sold-out lines (stock
/// dropped to 0 after adding) are excluded. The free-shipping
/// test uses the pre-discount subtotal.
#[derive(Clone, Debug, Default)]
pub struct CartTotals {
    /// Sum of purchasable line totals.
    pub subtotal_yen: i64,
    /// Coupon discount, capped at the subtotal.
    pub discount_yen: i64,
    /// Shipping fee after the free-shipping rules, net of
    /// any shipping-class coupon.
    pub shipping_yen: i64,
    /// The part of the shipping fee a free-shipping coupon
    /// waived (0 when none applied).
    pub shipping_discount_yen: i64,
    /// Cash-on-delivery fee (0 for an empty cart).
    pub cod_fee_yen: i64,
    /// Grand total: subtotal - discount + shipping + COD fee.
    pub total_yen: i64,
    /// How much more subtotal would make shipping free (0 when
    /// already free).
    pub free_shipping_remaining_yen: i64,
    /// Per-class decomposition of the total into the tax it
    /// contains. Informational only: the total is unchanged.
    pub tax_breakdown: Vec<crate::models::tax::TaxBreakdown>,
    /// The total tax contained in `total_yen`.
    pub tax_total_yen: i64,
}

/// Returns the shipping fee for a set of lines before any
/// coupon: the flat fee, waived when the cart has no
/// purchasable lines, when every purchasable line is a
/// free-shipping product, or when the pre-discount subtotal
/// reaches the free-shipping threshold. Lines totalling ¥0
/// still pay the flat fee — only an empty cart ships for
/// nothing by default. The coupon engine prices the shipping
/// class from this fee; [`totals`] charges it.
pub fn shipping_fee_for(lines: &[CartLine], fees: FeeSettings) -> i64 {
    let purchasable: Vec<&CartLine> = lines.iter().filter(|l| l.purchasable()).collect();
    let subtotal: i64 = purchasable.iter().map(|l| l.line_total_yen()).sum();
    let all_free = !purchasable.is_empty() && purchasable.iter().all(|l| l.free_shipping);
    if purchasable.is_empty() || all_free || subtotal >= fees.free_shipping_threshold_yen {
        0
    } else {
        fees.shipping_flat_yen
    }
}

/// Computes totals for the given lines, coupon discount and
/// payment method. The fees come from the shop settings (the
/// caller loads them; [`FeeSettings::fallback`] carries the
/// long-standing amounts).
///
/// This is the one totals computation in the store: the cart
/// page, both checkout steps and order placement all call it,
/// so a total can never be priced two ways. The COD fee exists
/// only for cash on delivery — a KOMOJU total never contains it
/// (the payment spec's totals matrix, TC-11).
pub fn totals(
    lines: &[CartLine],
    discount_yen: i64,
    shipping_discount_yen: i64,
    method: Option<PaymentMethod>,
    tax: crate::services::tax_service::TaxParams,
    fees: FeeSettings,
) -> CartTotals {
    let purchasable: Vec<&CartLine> = lines.iter().filter(|l| l.purchasable()).collect();
    let subtotal: i64 = purchasable.iter().map(|l| l.line_total_yen()).sum();
    // Free shipping: over the threshold as before, or when every
    // purchasable line is a free-shipping product.
    let all_free = !purchasable.is_empty() && purchasable.iter().all(|l| l.free_shipping);
    let shipping = shipping_fee_for(lines, fees);
    // A free-shipping coupon waives up to the whole fee; the
    // charged shipping (and its tax base) is the net amount.
    let shipping_discount = shipping_discount_yen.clamp(0, shipping);
    let shipping_net = shipping - shipping_discount;
    let remaining =
        if !purchasable.is_empty() && !all_free && subtotal < fees.free_shipping_threshold_yen {
            fees.free_shipping_threshold_yen - subtotal
        } else {
            0
        };
    let discount = discount_yen.clamp(0, subtotal);
    let cod = if method == Some(PaymentMethod::Cod) && subtotal > 0 {
        fees.cod_fee_yen
    } else {
        0
    };
    let tax_breakdown = tax_breakdown(&purchasable, subtotal, discount, shipping_net, cod, tax);
    let tax_total_yen = tax_breakdown.iter().map(|b| b.tax_yen).sum();
    CartTotals {
        subtotal_yen: subtotal,
        discount_yen: discount,
        shipping_yen: shipping_net,
        shipping_discount_yen: shipping_discount,
        cod_fee_yen: cod,
        total_yen: subtotal - discount + shipping_net + cod,
        free_shipping_remaining_yen: remaining,
        tax_breakdown,
        tax_total_yen,
    }
}

/// Decomposes a total into the tax it contains, per class.
///
/// Amounts are tax-inclusive, so each class's tax is a part of
/// its base. The coupon discount is allocated across the item
/// classes proportionally to their bases (each share floored;
/// the remainder goes to the largest class, so the shares sum
/// to the discount exactly). Shipping and the COD fee join the
/// standard class's base, as the invoice rules treat delivery
/// as a standard-rated supply. Rounding is applied once per
/// class to the class base — never per line.
fn tax_breakdown(
    purchasable: &[&CartLine],
    subtotal: i64,
    discount: i64,
    shipping: i64,
    cod: i64,
    tax: crate::services::tax_service::TaxParams,
) -> Vec<crate::models::tax::TaxBreakdown> {
    use crate::models::tax::TaxBreakdown;
    let mut classes: Vec<TaxBreakdown> = Vec::new();
    for line in purchasable {
        let entry = classes
            .iter_mut()
            .find(|b| b.tax_class_code == line.tax_class_code);
        match entry {
            Some(b) => b.taxable_base_yen += line.line_total_yen(),
            None => classes.push(TaxBreakdown {
                tax_class_code: line.tax_class_code.clone(),
                tax_class_name: line.tax_class_name.clone(),
                rate_percent: line.tax_rate_percent,
                taxable_base_yen: line.line_total_yen(),
                tax_yen: 0,
            }),
        }
    }
    if discount > 0 && subtotal > 0 {
        let mut allocated = 0i64;
        let mut shares: Vec<i64> = classes
            .iter()
            .map(|b| discount * b.taxable_base_yen / subtotal)
            .collect();
        for s in &shares {
            allocated += s;
        }
        let remainder = discount - allocated;
        if remainder > 0
            && let Some((idx, _)) = classes
                .iter()
                .enumerate()
                .max_by_key(|(_, b)| b.taxable_base_yen)
        {
            shares[idx] += remainder;
        }
        for (b, s) in classes.iter_mut().zip(shares.iter()) {
            b.taxable_base_yen -= s;
        }
    }
    let fees = shipping + cod;
    if fees > 0 {
        match classes.iter_mut().find(|b| b.tax_class_code == "standard") {
            Some(b) => b.taxable_base_yen += fees,
            None => classes.push(TaxBreakdown {
                tax_class_code: "standard".to_string(),
                tax_class_name: "標準税率".to_string(),
                rate_percent: tax.standard_rate_percent,
                taxable_base_yen: fees,
                tax_yen: 0,
            }),
        }
    }
    for b in classes.iter_mut() {
        b.tax_yen = crate::models::tax::tax_from_inclusive(
            b.taxable_base_yen,
            b.rate_percent,
            tax.rounding,
        );
    }
    classes.retain(|b| b.taxable_base_yen > 0);
    classes.sort_by_key(|b| crate::models::tax::class_rank(&b.tax_class_code));
    classes
}

/// Merges a guest session's cart into the member's cart (login),
/// inside one transaction, then forgets the guest token.
pub async fn merge_guest_cart_into_user(
    pool: &PgPool,
    session: &Session,
    user_id: i64,
) -> Result<(), AppError> {
    let Some(token) = existing_guest_token(session).await else {
        return Ok(());
    };
    let mut tx = pool.begin().await?;
    cart_repository::rekey_session_to_user(&mut tx, &token, user_id).await?;
    tx.commit().await?;
    let _ = session.remove::<String>(SESSION_CART_OWNER).await;
    Ok(())
}

/// One abandoned guest-cart cleanup pass: deletes
/// guest carts untouched for longer than the configured
/// retention (`guest_cart_retention_days` in the config
/// file) and returns how many lines were removed. A
/// retention of zero or less disables the sweep — the pass
/// then removes nothing, so an operator can turn the
/// cleanup off without touching code.
pub async fn sweep_abandoned_guest_carts(pool: &PgPool) -> Result<u64, AppError> {
    let retention_days = CONFIG.guest_cart_retention_days;
    if retention_days <= 0 {
        return Ok(0);
    }
    cart_repository::delete_abandoned_guest_carts(pool, retention_days)
        .await
        .map_err(|e| AppError::db("sweep abandoned guest carts", e))
}

/// Starts the guest-cart cleanup sweep under the server's
/// supervision: a pass on the mail worker's interval,
/// draining on the shared shutdown watch.
pub fn start_worker(pool: PgPool, mut shutdown: Receiver<bool>) -> tokio::task::JoinHandle<()> {
    let interval_secs = CONFIG.mail_worker_interval_secs.max(5);
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown.changed() => break,
            }
            match sweep_abandoned_guest_carts(&pool).await {
                Ok(removed) if removed > 0 => {
                    tracing::info!(removed, "guest cart sweep removed abandoned lines");
                }
                Ok(_) => {}
                Err(e) => tracing::error!("guest cart sweep pass failed: {e}"),
            }
        }
        tracing::info!("guest cart sweep stopped");
    });
    tracing::info!(interval_secs, "guest cart sweep started");
    handle
}

#[cfg(test)]
mod tests {
    use super::totals;
    use crate::models::{CartLine, PaymentMethod};

    fn line(price: i64, qty: i32, stock: i32) -> CartLine {
        CartLine {
            variant_id: 1,
            product_id: 1,
            name: "テスト".to_string(),
            variant_label: String::new(),
            unit_price_yen: price,
            stock,
            image_path: String::new(),
            status: "public".to_string(),
            quantity: qty,
            added_by_agent: false,
            variant_enabled: true,
            publish_open: true,
            track_stock: true,
            allow_backorder: false,
            free_shipping: false,
            tax_class_code: "standard".to_string(),
            tax_class_name: "標準税率".to_string(),
            tax_rate_percent: 10,
            undeliverable_regions: String::new(),
        }
    }

    fn tax() -> crate::services::tax_service::TaxParams {
        crate::services::tax_service::TaxParams::fallback()
    }

    fn fees() -> crate::models::FeeSettings {
        crate::models::FeeSettings::fallback()
    }

    #[test]
    fn physical_below_threshold_pays_shipping_and_cod() {
        let t = totals(
            &[line(2_980, 1, 10)],
            0,
            0,
            Some(PaymentMethod::Cod),
            tax(),
            fees(),
        );
        assert_eq!(t.subtotal_yen, 2_980);
        assert_eq!(t.shipping_yen, 550);
        assert_eq!(t.cod_fee_yen, 330);
        assert_eq!(t.total_yen, 3_860);
        assert_eq!(t.free_shipping_remaining_yen, 2_020);
    }

    #[test]
    fn shipping_is_free_exactly_at_the_threshold() {
        let t = totals(
            &[line(2_500, 2, 10)],
            0,
            0,
            Some(PaymentMethod::Cod),
            tax(),
            fees(),
        );
        assert_eq!(t.subtotal_yen, 5_000);
        assert_eq!(t.shipping_yen, 0);
        assert_eq!(t.free_shipping_remaining_yen, 0);
    }

    #[test]
    fn off_sale_lines_are_excluded_from_totals() {
        // A product drafted after entering the cart counts exactly
        // like a sold-out line — nothing.
        let mut drafted = line(1_000, 2, 5);
        drafted.status = "draft".to_string();
        let t = totals(
            &[drafted, line(1_000, 1, 5)],
            0,
            0,
            Some(PaymentMethod::Cod),
            tax(),
            fees(),
        );
        assert_eq!(t.subtotal_yen, 1_000);
    }

    #[test]
    fn sold_out_lines_are_excluded_from_totals() {
        let t = totals(
            &[line(1_000, 2, 0), line(1_000, 1, 5)],
            0,
            0,
            Some(PaymentMethod::Cod),
            tax(),
            fees(),
        );
        assert_eq!(t.subtotal_yen, 1_000);
    }

    #[test]
    fn discount_is_capped_at_the_subtotal() {
        let t = totals(
            &[line(1_000, 1, 5)],
            9_999,
            0,
            Some(PaymentMethod::Cod),
            tax(),
            fees(),
        );
        assert_eq!(t.discount_yen, 1_000);
        assert_eq!(t.total_yen, 550 + 330);
    }

    #[test]
    fn komoju_methods_never_carry_the_cod_fee() {
        // TC-11's core: on an identical cart, the 330 yen exists
        // only in the COD total — with paid shipping and with
        // free shipping, with a coupon and without.
        for method in [
            PaymentMethod::CreditCard,
            PaymentMethod::Konbini,
            PaymentMethod::PayPay,
        ] {
            let t = totals(&[line(2_980, 1, 10)], 0, 0, Some(method), tax(), fees());
            assert_eq!(t.cod_fee_yen, 0);
            assert_eq!(t.total_yen, 2_980 + 550);
            let t = totals(&[line(2_500, 2, 10)], 500, 0, Some(method), tax(), fees());
            assert_eq!(t.cod_fee_yen, 0);
            assert_eq!(t.discount_yen, 500);
            assert_eq!(t.total_yen, 5_000 - 500);
        }
        // The COD twin of the second case keeps the fee.
        let t = totals(
            &[line(2_500, 2, 10)],
            500,
            0,
            Some(PaymentMethod::Cod),
            tax(),
            fees(),
        );
        assert_eq!(t.total_yen, 5_000 - 500 + 330);
    }

    #[test]
    fn no_method_chosen_yet_carries_no_cod_fee() {
        // Before the payment step no method has been chosen:
        // cart and delivery totals must not pre-charge the
        // cash-on-delivery fee for an unmade choice.
        let t = totals(&[line(2_980, 1, 10)], 0, 0, None, tax(), fees());
        assert_eq!(t.cod_fee_yen, 0);
        assert_eq!(t.total_yen, 2_980 + 550);
    }

    #[test]
    fn tax_breakdown_decomposes_the_same_total() {
        // 2,980 standard + shipping 550 + COD 330 = 3,860 total;
        // base 3,860 at 10%: tax = floor(3,860 × 10 / 110) = 350
        // (the floor-of-tax rule). The total itself
        // is unchanged.
        let t = totals(
            &[line(2_980, 1, 10)],
            0,
            0,
            Some(PaymentMethod::Cod),
            tax(),
            fees(),
        );
        assert_eq!(t.total_yen, 3_860);
        assert_eq!(t.tax_breakdown.len(), 1);
        assert_eq!(t.tax_breakdown[0].taxable_base_yen, 3_860);
        assert_eq!(t.tax_breakdown[0].tax_yen, 350);
        assert_eq!(t.tax_total_yen, 350);
    }

    #[test]
    fn tax_breakdown_mixed_classes_floor_of_tax() {
        // 2,980 standard (10%) + 1,080 reduced (8%) = 4,060
        // subtotal; shipping 550 and the COD fee 330 join the
        // standard base (4,060 is below the 5,000 threshold):
        // standard base 3,860 -> floor(3,860 x 10 / 110) = 350;
        // reduced base 1,080 -> floor(1,080 x 8 / 108) = 80.
        // The total itself (4,060 + 550 + 330 = 4,940) does not
        // depend on the tax convention.
        let mut reduced = line(1_080, 1, 10);
        reduced.tax_class_code = "reduced".to_string();
        reduced.tax_class_name = "軽減税率".to_string();
        reduced.tax_rate_percent = 8;
        let t = totals(
            &[line(2_980, 1, 10), reduced],
            0,
            0,
            Some(PaymentMethod::Cod),
            tax(),
            fees(),
        );
        assert_eq!(t.total_yen, 4_940);
        assert_eq!(t.tax_breakdown.len(), 2);
        assert_eq!(t.tax_breakdown[0].tax_class_code, "standard");
        assert_eq!(t.tax_breakdown[0].taxable_base_yen, 3_860);
        assert_eq!(t.tax_breakdown[0].tax_yen, 350);
        assert_eq!(t.tax_breakdown[1].tax_class_code, "reduced");
        assert_eq!(t.tax_breakdown[1].taxable_base_yen, 1_080);
        assert_eq!(t.tax_breakdown[1].tax_yen, 80);
        assert_eq!(t.tax_total_yen, 430);
    }

    #[test]
    fn all_free_shipping_lines_pay_no_shipping() {
        let mut l = line(1_000, 1, 5);
        l.free_shipping = true;
        let t = totals(&[l], 0, 0, Some(PaymentMethod::Cod), tax(), fees());
        assert_eq!(t.shipping_yen, 0);
        assert_eq!(t.free_shipping_remaining_yen, 0);
    }

    #[test]
    fn empty_cart_is_all_zero() {
        let t = totals(&[], 0, 0, Some(PaymentMethod::Cod), tax(), fees());
        assert_eq!(t.total_yen, 0);
        assert_eq!(t.cod_fee_yen, 0);
    }

    #[test]
    fn zero_priced_lines_still_pay_the_flat_shipping_fee() {
        // A ¥0 line that is not a free-shipping product is not
        // an empty cart: the flat fee applies exactly as for
        // any subtotal under the threshold, and the cart is the
        // full threshold away from free shipping.
        let t = totals(
            &[line(0, 1, 5)],
            0,
            0,
            Some(PaymentMethod::CreditCard),
            tax(),
            fees(),
        );
        assert_eq!(t.subtotal_yen, 0);
        assert_eq!(t.shipping_yen, 550);
        assert_eq!(t.total_yen, 550);
        assert_eq!(t.free_shipping_remaining_yen, 5_000);
    }
}
