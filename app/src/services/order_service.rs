//! Order service.
//!
//! Owns the checkout flow state, delivery validation, coupon
//! evaluation and the one-transaction order placement, plus
//! order reads, cancellation and restock for both surfaces, and
//! the coupon band parts.
//!
//! Placement is a pure database operation: it takes no session.
//! The checkout controller owns the session bookkeeping around
//! it; the confirmation mail is spawned from this module once
//! placement returns (see `spawn_confirmation_mail`).
//!
//! This service never touches the Session at all: the storage
//! helpers live with the session keys in `crate::session` (a
//! service may receive the Session only to resolve who is acting,
//! never to carry business state between steps). This module owns
//! the flow type, its validation and the placement.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{
    ADDRESS_LINE_REQUIRED_MESSAGE, CITY_REQUIRED_MESSAGE, EMAIL_FORMAT_MESSAGE,
    EMAIL_REQUIRED_MESSAGE, FAMILY_NAME_REQUIRED_MESSAGE, GIVEN_NAME_REQUIRED_MESSAGE,
    POSTAL_CODE_FORMAT_MESSAGE, PREFECTURE_REQUIRED_MESSAGE,
};
use crate::models::order::{OrderLineDetail, PlacedVia};
use crate::models::{CartLine, Order, OrderLine, Payment, PaymentMethod, User};
use crate::repositories::{
    cart_repository, coupon_repository, order_repository, payment_repository,
};
use crate::services::cart_service::{self, CartOwner};
use crate::validation::{self, FieldError};

/// The checkout form state, held in the session between the
/// checkout POST and the confirm POST (the same pattern as the
/// sign-up flow). Delivery details are personal data: they live
/// only in the session record and in the placed order, and are
/// never logged.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CheckoutFlow {
    /// Contact email (member's account email, or the guest's).
    pub email: String,
    /// Delivery family name.
    pub family_name: String,
    /// Delivery given name.
    pub given_name: String,
    /// Delivery postal code.
    pub postal_code: String,
    /// Delivery prefecture.
    pub prefecture: String,
    /// Delivery city and ward.
    pub city: String,
    /// Delivery street address.
    pub address_line: String,
    /// Delivery building, when given.
    pub building: String,
    /// Delivery phone number.
    pub phone: String,
    /// Requested delivery date (`YYYY-MM-DD`); empty for no
    /// preference. Carried raw like the address fields and
    /// validated against the window at placement.
    #[serde(default)]
    pub delivery_date: String,
    /// Requested delivery slot code (catalogue in
    /// [`crate::models::order`]); empty deserializes as no
    /// preference for sessions saved before the feature.
    #[serde(default)]
    pub delivery_slot: String,
    /// Applied coupon codes in entry order (Coupon
    /// Shopify-Core spec: the engine slots them by class at
    /// every evaluation). Sessions saved before the pack
    /// deserialize empty — the applied coupon is simply
    /// re-entered.
    #[serde(default)]
    pub coupon_codes: Vec<String>,
    /// The payment method in effect (cash on delivery unless
    /// the customer picked a KOMOJU method on the payment step).
    /// Sessions saved before payments existed deserialize as COD.
    #[serde(default)]
    pub payment_method: PaymentMethod,
    /// Whether a method has actually been chosen on the payment
    /// step. Until then `payment_method` is only the default, and
    /// totals must not charge the cash-on-delivery fee for a
    /// choice the customer has not made yet.
    #[serde(default)]
    pub payment_method_chosen: bool,
    /// Whether the member asked to save the typed address to
    /// their address book when the order places (Address Book
    /// spec §5.3). Set only on the typed path, for members.
    #[serde(default)]
    pub save_address: bool,
    /// Gift noshi code (catalogue in [`crate::models::gift`]);
    /// empty or `none` for no noshi. Sessions saved before
    /// the feature deserialize empty — no gift options.
    #[serde(default)]
    pub noshi: String,
    /// Gift name inscription （名入れ） as typed.
    #[serde(default)]
    pub noshi_name: String,
    /// Gift message-card text as typed.
    #[serde(default)]
    pub gift_message: String,
    /// Points the member chose to spend (EC V2 core 1),
    /// picked on the payment step. Members only: a guest
    /// flow carrying a value is ignored at placement.
    /// Sessions saved before the feature deserialize as 0.
    #[serde(default)]
    pub points_to_use: i64,
}

impl CheckoutFlow {
    /// Prefills the flow from a member's registered address.
    pub fn from_user(user: &User) -> Self {
        Self {
            email: user.email.clone(),
            family_name: user.family_name.clone(),
            given_name: user.given_name.clone(),
            postal_code: user.postal_code.clone(),
            prefecture: user.prefecture.clone(),
            city: user.city.clone(),
            address_line: user.address_line.clone(),
            building: user.building.clone().unwrap_or_default(),
            phone: user.phone.clone(),
            delivery_date: String::new(),
            delivery_slot: "none".to_string(),
            coupon_codes: Vec::new(),
            payment_method: PaymentMethod::Cod,
            payment_method_chosen: false,
            save_address: false,
            noshi: String::new(),
            noshi_name: String::new(),
            gift_message: String::new(),
            points_to_use: 0,
        }
    }

    /// Returns the method totals should use: the chosen one, or
    /// `None` while the payment step has not been completed, so
    /// pre-choice totals (cart, delivery step) carry no COD fee.
    pub fn chosen_method(&self) -> Option<PaymentMethod> {
        self.payment_method_chosen.then_some(self.payment_method)
    }

    /// Returns the flow's delivery request in the shared
    /// rendering the confirm step shows, from the carried raw
    /// values. An unparseable carried date reads as no request
    /// here; placement re-validates and rejects it there.
    pub fn delivery_request_display(&self) -> String {
        let date = chrono::NaiveDate::parse_from_str(self.delivery_date.trim(), "%Y-%m-%d").ok();
        let label = crate::models::delivery::delivery_slot_label(self.delivery_slot.trim())
            .unwrap_or("指定なし");
        crate::models::delivery::format_delivery_request(date, label)
    }

    /// Returns the flow's gift choice as display lines for
    /// the confirm step, from the carried raw values. The
    /// confirm step renders only after validation passed;
    /// an invalid carried value (impossible on that path)
    /// reads as no gift options here, and placement
    /// re-validates and rejects it there.
    pub fn gift_lines(&self) -> Vec<String> {
        match crate::models::gift::validate_gift_options(
            &self.noshi,
            &self.noshi_name,
            &self.gift_message,
        ) {
            Ok(gift) => crate::models::gift::gift_display_lines(
                gift.noshi_code,
                gift.noshi_label,
                gift.noshi_name.as_deref(),
                gift.gift_message.as_deref(),
            ),
            Err(_) => Vec::new(),
        }
    }

    /// Fills the delivery fields that are still empty from the
    /// member's registered profile. A saved flow can predate
    /// the login (a guest who posted the form, then signed in):
    /// its blank fields must not shadow the registered address,
    /// while anything the visitor actually typed is kept.
    pub fn fill_empty_from_user(&mut self, user: &User) {
        let fill = |slot: &mut String, value: &str| {
            if slot.trim().is_empty() {
                *slot = value.to_string();
            }
        };
        fill(&mut self.family_name, &user.family_name);
        fill(&mut self.given_name, &user.given_name);
        fill(&mut self.postal_code, &user.postal_code);
        fill(&mut self.prefecture, &user.prefecture);
        fill(&mut self.city, &user.city);
        fill(&mut self.address_line, &user.address_line);
        if self.building.trim().is_empty()
            && let Some(building) = &user.building
        {
            self.building = building.clone();
        }
        fill(&mut self.phone, &user.phone);
    }
}

/// Length caps for the delivery fields, counted in characters:
/// the same discipline the inquiry form applies (names at 50,
/// email at 254), with one cap for the address text fields.
/// The postal code and the phone are bounded by their digit
/// rules instead.
const DELIVERY_NAME_MAX_CHARS: usize = 50;
const DELIVERY_EMAIL_MAX_CHARS: usize = 254;
const DELIVERY_ADDRESS_MAX_CHARS: usize = 100;

/// Validates the delivery fields, returning every problem at once
/// (one per field) like the sign-up details step. Members do not
/// enter an email (the account's is used), so the email rules
/// apply to guests only. The shape rules are the shared ones in
/// [`crate::validation`]; this function only attaches the
/// checkout messages to them.
pub fn validate_delivery(
    flow: &CheckoutFlow,
    is_member: bool,
    settings: &crate::models::ShopSettings,
    holidays: &[chrono::NaiveDate],
) -> Vec<FieldError> {
    let mut errors: Vec<FieldError> = Vec::new();
    let mut push = |field: &'static str, message: &str| {
        errors.push(FieldError {
            field,
            message: message.to_string(),
        });
    };
    if !is_member {
        let email = flow.email.trim();
        if email.is_empty() {
            push("email", EMAIL_REQUIRED_MESSAGE);
        } else if email.chars().count() > DELIVERY_EMAIL_MAX_CHARS
            || !validation::is_valid_email(email)
        {
            push("email", EMAIL_FORMAT_MESSAGE);
        }
    }
    if flow.family_name.trim().is_empty() {
        push("family_name", FAMILY_NAME_REQUIRED_MESSAGE);
    } else if flow.family_name.chars().count() > DELIVERY_NAME_MAX_CHARS {
        push(
            "family_name",
            &format!("姓は{DELIVERY_NAME_MAX_CHARS}文字以内で入力してください。"),
        );
    }
    if flow.given_name.trim().is_empty() {
        push("given_name", GIVEN_NAME_REQUIRED_MESSAGE);
    } else if flow.given_name.chars().count() > DELIVERY_NAME_MAX_CHARS {
        push(
            "given_name",
            &format!("名は{DELIVERY_NAME_MAX_CHARS}文字以内で入力してください。"),
        );
    }
    let postal_stripped: String = flow.postal_code.chars().filter(|c| *c != '-').collect();
    if postal_stripped.is_empty() {
        push("postal_code", "郵便番号を入力してください。");
    } else if validation::postal_code_digits(&flow.postal_code).is_none() {
        push("postal_code", POSTAL_CODE_FORMAT_MESSAGE);
    }
    if flow.prefecture.trim().is_empty() {
        push("prefecture", PREFECTURE_REQUIRED_MESSAGE);
    } else if flow.prefecture.chars().count() > DELIVERY_ADDRESS_MAX_CHARS {
        push(
            "prefecture",
            &format!("都道府県は{DELIVERY_ADDRESS_MAX_CHARS}文字以内で入力してください。"),
        );
    }
    if flow.city.trim().is_empty() {
        push("city", CITY_REQUIRED_MESSAGE);
    } else if flow.city.chars().count() > DELIVERY_ADDRESS_MAX_CHARS {
        push(
            "city",
            &format!("市区町村は{DELIVERY_ADDRESS_MAX_CHARS}文字以内で入力してください。"),
        );
    }
    if flow.address_line.trim().is_empty() {
        push("address_line", ADDRESS_LINE_REQUIRED_MESSAGE);
    } else if flow.address_line.chars().count() > DELIVERY_ADDRESS_MAX_CHARS {
        push(
            "address_line",
            &format!("番地は{DELIVERY_ADDRESS_MAX_CHARS}文字以内で入力してください。"),
        );
    }
    if flow.building.chars().count() > DELIVERY_ADDRESS_MAX_CHARS {
        push(
            "building",
            &format!("建物名は{DELIVERY_ADDRESS_MAX_CHARS}文字以内で入力してください。"),
        );
    }
    let phone_stripped: String = flow.phone.chars().filter(|c| *c != '-').collect();
    if phone_stripped.is_empty() {
        push("phone", "電話番号を入力してください。");
    } else if validation::phone_digits(&flow.phone).is_none() {
        push("phone", "電話番号が正しくありません。");
    }
    // The delivery request, checked with the same catalogue
    // and window placement re-checks later: a slot needs its
    // date, and the pair must sit inside today's window, so the
    // shopper hears about a bad pair here, under its field.
    let (earliest_off, latest_off) = settings.delivery_offsets();
    if let Err(e) = crate::models::delivery::validate_delivery_request_with_holidays(
        &flow.delivery_date,
        &flow.delivery_slot,
        crate::models::jst::jst_today(),
        earliest_off,
        latest_off,
        &settings.disabled_delivery_slots,
        holidays,
    ) {
        use crate::models::delivery::DeliveryRequestError;
        let field = match e {
            DeliveryRequestError::Invalid
                if crate::models::delivery::delivery_slot_label(flow.delivery_slot.trim())
                    .is_none() =>
            {
                "delivery_slot"
            }
            _ => "delivery_date",
        };
        push(field, e.message());
    }
    // The gift options, checked with the same catalogue
    // placement re-checks later: an unknown noshi, an
    // inscription without one, or an over-cap text each
    // surface under their own field.
    for error in
        crate::models::gift::gift_option_errors(&flow.noshi, &flow.noshi_name, &flow.gift_message)
    {
        push(error.field(), &error.message());
    }
    errors
}

/// How long the header coupon band may be served from memory
/// before the database is asked again.
const BAND_TTL: Duration = Duration::from_secs(60);

/// The cached band parts and when they were read. Process-local,
/// like the throttles: on ECS each task refreshes its own copy,
/// and a coupon change appears everywhere within the TTL.
static BAND_CACHE: Mutex<Option<(Instant, BandParts)>> = Mutex::new(None);

/// The header coupon band's two halves, split for the layout:
/// the message on the left of the band, the discount on the right.
///
/// Both halves are formatted from the same coupon row in the same
/// build, so they can never disagree. Empty halves mean no band
/// is rendered at all.
#[derive(Clone)]
pub struct BandParts {
    /// The coupon message without its discount phrase (for
    /// example `オープン記念クーポン:ご注文時に「WELCOME500」で`).
    pub text: String,
    /// The discount in the app's canonical label form (for
    /// example `￥500割引`).
    pub amount: String,
}

impl BandParts {
    /// No band: both halves empty.
    fn empty() -> Self {
        Self {
            text: String::new(),
            amount: String::new(),
        }
    }
}

/// Returns the header coupon band's two halves, built from the
/// active store coupon.
///
/// There is deliberately no fallback text: an earlier version
/// advertised a hardcoded WELCOME500 band whenever the lookup
/// failed or no coupon was active, so deactivating the coupon in
/// the database changed nothing a visitor saw. No active coupon —
/// or a failed lookup, which is logged — means no band: both
/// halves come back empty and the layout renders no band markup.
pub async fn band_parts(pool: &PgPool) -> BandParts {
    if let Ok(cache) = BAND_CACHE.lock()
        && let Some((read_at, parts)) = cache.as_ref()
        && read_at.elapsed() < BAND_TTL
    {
        return parts.clone();
    }
    let parts = match coupon_repository::find_band_coupon(pool).await {
        Ok(Some(coupon)) => BandParts {
            // The message keeps the wording the band has always
            // carried, minus the discount phrase, which moves to
            // the band's right-hand element.
            text: if coupon.description.is_empty() {
                format!("ご注文時に「{}」で", coupon.code)
            } else {
                format!("{}:ご注文時に「{}」で", coupon.description, coupon.code)
            },
            // The same display form the member coupon labels use
            // (￥500割引 / 10%割引 / 送料無料), so the discount
            // reads identically everywhere the app shows it.
            amount: coupon.benefit_label(),
        },
        Ok(None) => BandParts::empty(),
        Err(e) => {
            // Not cached: a recovered database should bring the
            // band back on the next page, not a minute later.
            tracing::error!("coupon band lookup failed: {e}");
            return BandParts::empty();
        }
    };
    if let Ok(mut cache) = BAND_CACHE.lock() {
        *cache = Some((Instant::now(), parts.clone()));
    }
    parts
}

/// Loads one order by ID (the completion page's read).
pub async fn find_order(pool: &PgPool, order_id: i64) -> Result<Option<Order>, AppError> {
    Ok(order_repository::find_by_id(pool, order_id).await?)
}

/// Loads the completion page's secondary reads as one call
///: the tax breakdown rows and the payment
/// row. Either read failing fails the whole call, so the
/// controller can refuse to render a half-loaded order
/// instead of degrading each read on its own.
pub async fn completion_extras(
    pool: &PgPool,
    order_id: i64,
) -> Result<(Vec<crate::models::tax::OrderTaxBreakdown>, Option<Payment>), AppError> {
    let breakdowns = order_repository::tax_breakdowns(pool, order_id).await?;
    let payment = payment_repository::find_by_order_id(pool, order_id).await?;
    Ok((breakdowns, payment))
}

/// Loads an order's snapshot lines.
pub async fn order_lines(pool: &PgPool, order_id: i64) -> Result<Vec<OrderLine>, AppError> {
    Ok(order_repository::lines_of(pool, order_id).await?)
}

/// Returns a member's most recent orders with their line
/// summaries (the mypage hub's recent-orders block).
pub async fn recent(
    pool: &PgPool,
    user_id: i64,
    limit: i64,
) -> Result<Vec<crate::models::RecentOrder>, AppError> {
    Ok(order_repository::recent_for_user(pool, user_id, limit).await?)
}

/// Returns one page of a member's order history, newest first,
/// plus the total order count (mypage).
pub async fn history(
    pool: &PgPool,
    user_id: i64,
    limit: i64,
    offset: i64,
) -> Result<(Vec<crate::models::RecentOrder>, i64), AppError> {
    Ok(order_repository::history_for(pool, user_id, limit, offset).await?)
}

/// Finds one order only when it belongs to the given member;
/// ownership is enforced inside the query, so a foreign order
/// and a missing one are indistinguishable to the caller.
pub async fn find_order_for_user(
    pool: &PgPool,
    order_id: i64,
    user_id: i64,
) -> Result<Option<Order>, AppError> {
    Ok(order_repository::find_by_id_for_user(pool, order_id, user_id).await?)
}

/// Finds one order for the public guest lookup: the typed
/// number must be the order's own derived number and the
/// email must be the address stored on the order. Both checks
/// run against the loaded row and every failure returns
/// `None`, so a wrong number, a wrong date segment and a
/// wrong email are indistinguishable to the caller.
pub async fn find_order_for_lookup(
    pool: &PgPool,
    order_number: &str,
    email: &str,
) -> Result<Option<Order>, AppError> {
    let Some(id) = crate::models::order::parse_order_number(order_number) else {
        return Ok(None);
    };
    let Some(order) = order_repository::find_by_id(pool, id).await? else {
        return Ok(None);
    };
    if order.order_number() != order_number.trim().to_uppercase() {
        return Ok(None);
    }
    if !order.email.trim().eq_ignore_ascii_case(email.trim()) {
        return Ok(None);
    }
    Ok(Some(order))
}

/// An order's lines with each product's current thumbnail
/// joined in for the detail page.
/// The outcome of one reorder (もう一度購入): how many of the
/// order's lines landed in the cart, and the snapshot names
/// of the lines that could not be added.
pub struct ReorderOutcome {
    /// Display order number (AS-…).
    pub order_number: String,
    /// Lines added (a stock-capped line counts as added).
    pub added: usize,
    /// Snapshot display names of the skipped lines.
    pub skipped: Vec<String>,
}

/// Reorders one of the member's own orders into their cart
/// through the normal human add path: each line is added at
/// its ordered quantity; a line whose product or combination
/// is off sale, sold out or gone is skipped and reported by
/// its snapshot name. `Ok(None)` when the order is missing
/// or belongs to someone else — the caller answers the same
/// styled 404 as the order detail page.
pub async fn reorder(
    pool: &PgPool,
    owner: &crate::services::cart_service::CartOwner,
    user_id: i64,
    order_id: i64,
) -> Result<Option<ReorderOutcome>, AppError> {
    let Some(order) = find_order_for_user(pool, order_id, user_id).await? else {
        return Ok(None);
    };
    let rows = order_repository::reorder_rows(pool, order_id).await?;
    let mut added = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    for row in &rows {
        let name = if row.variant_label.is_empty() {
            row.product_name.clone()
        } else {
            format!("{}（{}）", row.product_name, row.variant_label)
        };
        match crate::services::cart_service::add_to_cart(
            pool,
            owner,
            row.product_id,
            row.variant_id,
            row.quantity,
        )
        .await?
        {
            crate::services::cart_service::AddOutcome::Added(_)
            | crate::services::cart_service::AddOutcome::Capped(_) => added += 1,
            crate::services::cart_service::AddOutcome::SoldOut
            | crate::services::cart_service::AddOutcome::NotFound => skipped.push(name),
        }
    }
    Ok(Some(ReorderOutcome {
        order_number: order.order_number(),
        added,
        skipped,
    }))
}

pub async fn order_lines_with_images(
    pool: &PgPool,
    order_id: i64,
) -> Result<Vec<OrderLineDetail>, AppError> {
    Ok(order_repository::lines_with_images(pool, order_id).await?)
}

/// Why placing an order did not happen.
pub enum PlaceError {
    /// The cart holds nothing purchasable.
    EmptyCart,
    /// A line asks for more than the locked stock.
    StockShortage,
    /// The flow's coupon no longer validates.
    CouponInvalid,
    /// A line's product cannot be delivered to the order's
    /// prefecture (its undeliverable-region list names it);
    /// carries the product name for the message.
    Undeliverable(String),
    /// The carried delivery request failed the placement-time
    /// re-validation (window moved, or a tampered value).
    DeliveryInvalid(String),
    /// The figures moved between the confirmation page and
    /// placement (a price, fee, coupon or line set changed
    /// under the shopper): nothing was written, and the
    /// shopper must re-confirm at the current figures.
    PriceChanged,
    /// The chosen payment method refuses this total (its
    /// published ceiling — konbini's ¥300,000). The
    /// selection-step check alone cannot be trusted: a
    /// crafted confirm POST can carry a flow whose method
    /// was never re-checked there, so placement re-checks
    /// against the final totals. Carries the shopper-facing
    /// message.
    PaymentMethodLimit(String),
    /// Database failure.    PriceChanged,
    /// Database failure.
    Db(sqlx::Error),
}

impl From<sqlx::Error> for PlaceError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

/// Places the order in one transaction: variant rows are locked
/// in ID order, stock is re-validated, the order, its snapshot
/// lines, its payment row and its per-class tax breakdown are
/// inserted, tracked lines' stock is decremented, a member's
/// coupon is marked used and the ordered cart lines are cleared
/// — or nothing happens at all.
///
/// Only about the database: the caller (the checkout controller)
/// owns the session bookkeeping around it — recording the placed
/// order for the completion page — and sends the confirmation
/// mail after it returns.
pub async fn place_order(
    pool: &PgPool,
    owner: &CartOwner,
    flow: &CheckoutFlow,
    placed_via: PlacedVia,
    expected_fingerprint: Option<&str>,
) -> Result<Order, PlaceError> {
    let settings = crate::services::shop_settings_service::load(pool).await;
    let (delivery_date, delivery_slot, delivery_slot_label, gift) =
        validated_delivery_and_gift(pool, flow, &settings).await?;

    // Tax rates of the order's own date (JST today), resolved
    // before the transaction: rates are reference data, and the
    // snapshots below must freeze today's values even if a
    // future-dated rate is already registered.
    let class_rates =
        crate::repositories::tax_repository::class_rates_on(pool, crate::models::jst::jst_today())
            .await?;
    let standard_rate = class_rates
        .iter()
        .find(|r| r.class.code == "standard")
        .map(|r| r.rate_percent)
        .unwrap_or(10);
    let mut tx = pool.begin().await?;

    let (effective, locked) = effective_lines(&mut tx, owner, flow, &class_rates).await?;

    // Coupons re-validated against the locked cart, inside the
    // transaction with every candidate definition locked (Coupon
    // Shopify-Core spec): the engine re-slots the flow's entered
    // codes plus the automatic discounts, re-checking scope,
    // minimums and usage caps against the redemption ledger.
    // Entered codes are members-only, enforced again here: a
    // guest flow carrying a code — saved before login rules
    // applied, or crafted — fails placement, as before.
    if !flow.coupon_codes.is_empty() && owner.user_id().is_none() {
        return Err(PlaceError::CouponInvalid);
    }
    let outcome = crate::services::coupon_service::evaluate_in_tx(
        &mut tx,
        owner.user_id(),
        &effective,
        &flow.coupon_codes,
        settings.fee_settings(),
    )
    .await?;
    // An entered code that no longer applies (its cap raced
    // out, its window closed) fails the placement rather than
    // silently dropping a discount the customer saw.
    for code in &flow.coupon_codes {
        let canonical = code.trim().to_uppercase();
        if !canonical.is_empty() && !outcome.contains(&canonical) {
            return Err(PlaceError::CouponInvalid);
        }
    }
    let totals = cart_service::totals(
        &effective,
        outcome.discount_yen(),
        outcome.shipping_discount_yen,
        Some(flow.payment_method),
        crate::services::tax_service::TaxParams {
            standard_rate_percent: standard_rate,
            rounding: crate::models::tax::TaxRounding::configured(),
        },
        settings.fee_settings(),
    );

    // Binding: when the caller carries the confirmation
    // page's fingerprint, the figures placement just computed
    // must equal the figures the shopper confirmed — the same
    // fingerprint the agent path binds its approvals with. A
    // price, fee, coupon or line change since the confirmation
    // refuses the placement whole (the transaction drops, so
    // nothing is written) instead of silently charging the
    // new figures.
    if let Some(expected) = expected_fingerprint {
        let actual = crate::services::agent_approval_service::fingerprint_for(&effective, &totals);
        if actual != expected {
            return Err(PlaceError::PriceChanged);
        }
    }

    // The payment method's own ceiling (when it publishes
    // one) is a money rule, so it is re-checked here against
    // the final totals — the same reason coupons are
    // re-validated in this transaction. The selection step
    // checks it for the shopper's benefit; this check is the
    // one that cannot be skipped.
    if let Some(max) = flow.payment_method.max_total_yen()
        && totals.total_yen > max
    {
        return Err(PlaceError::PaymentMethodLimit(
            crate::messages::payment_method_limit_message(flow.payment_method.label(), max),
        ));
    }
    let order_id = write_order(
        &mut tx,
        owner,
        flow,
        placed_via,
        &effective,
        &locked,
        &outcome,
        &totals,
        delivery_date,
        delivery_slot,
        delivery_slot_label,
        &gift,
        settings.points_settings(),
    )
    .await?;

    tx.commit().await?;

    let order = order_repository::find_by_id(pool, order_id)
        .await?
        .expect("the order was just inserted");
    tracing::info!(order_id = order.id, "order_placed");
    Ok(order)
}

/// The placement-time re-validation of the flow's delivery
/// request and gift options, resolved against the window
/// computed NOW: a value carried across the JST midnight
/// boundary, or a tampered one, fails the whole placement —
/// it is never clamped to a valid value, and no order is
/// written. Returns the resolved delivery date, slot and
/// slot label, and the validated gift options.
async fn validated_delivery_and_gift(
    pool: &PgPool,
    flow: &CheckoutFlow,
    settings: &crate::models::ShopSettings,
) -> Result<
    (
        Option<chrono::NaiveDate>,
        &'static str,
        &'static str,
        crate::models::gift::ValidGift,
    ),
    PlaceError,
> {
    let holidays = crate::services::holiday_service::upcoming(pool).await;
    // The delivery request is re-validated against the window
    // computed NOW, at placement: a value carried across the
    // JST midnight boundary, or a tampered one, fails the whole
    // placement — it is never clamped to a valid value, and no
    // order is written.
    let (earliest_off, latest_off) = settings.delivery_offsets();
    let (delivery_date, delivery_slot, delivery_slot_label) =
        match crate::models::delivery::validate_delivery_request_with_holidays(
            &flow.delivery_date,
            &flow.delivery_slot,
            crate::models::jst::jst_today(),
            earliest_off,
            latest_off,
            &settings.disabled_delivery_slots,
            &holidays,
        ) {
            Ok(parts) => parts,
            Err(e) => return Err(PlaceError::DeliveryInvalid(e.message().to_string())),
        };
    // The gift options are re-validated with the delivery
    // request: a tampered noshi code or an over-cap text
    // carried in the session fails the placement whole, it
    // is never clamped and no order is written.
    let gift = match crate::models::gift::validate_gift_options(
        &flow.noshi,
        &flow.noshi_name,
        &flow.gift_message,
    ) {
        Ok(gift) => gift,
        Err(e) => return Err(PlaceError::DeliveryInvalid(e.message())),
    };
    Ok((delivery_date, delivery_slot, delivery_slot_label, gift))
}

/// Locks the owner's cart rows and every involved variant
/// row inside the placement transaction and builds the
/// effective line set at locked prices, with the order
/// date's tax class snapshots. Returns the effective lines
/// and the locked variant rows (the write step reads the
/// snapshot names and stock modes from them).
async fn effective_lines(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner: &CartOwner,
    flow: &CheckoutFlow,
    class_rates: &[crate::models::tax::ClassRate],
) -> Result<(Vec<CartLine>, Vec<order_repository::LockedVariant>), PlaceError> {
    let rate_of = |class_id: i64| {
        class_rates
            .iter()
            .find(|r| r.class.id == class_id)
            .map(|r| (r.class.code.clone(), r.class.name.clone(), r.rate_percent))
            .unwrap_or_else(|| ("standard".to_string(), "標準税率".to_string(), 10))
    };
    // Lock the owner's cart rows and re-read the lines under
    // the lock (audit M1): this is the authoritative set the
    // placement orders. A placement that raced this one has
    // already cleared the cart by the time the lock is
    // granted, so it stops here as an empty cart instead of
    // placing a duplicate order.
    cart_repository::lock_owner_lines(tx, owner.user_id(), owner.session_key()).await?;
    let lines = cart_repository::list_lines_in_tx(tx, owner.user_id(), owner.session_key()).await?;
    if lines.iter().all(|l| !l.purchasable()) {
        return Err(PlaceError::EmptyCart);
    }
    // Delivery-region check, before any money moves: a product
    // that names the delivery prefecture as undeliverable blocks
    // the whole placement with the product named.
    let prefecture = flow.prefecture.trim();
    for line in lines.iter().filter(|l| l.purchasable()) {
        if line.undeliverable_to(prefecture) {
            return Err(PlaceError::Undeliverable(line.name.clone()));
        }
    }

    // Lock every involved variant row in ID order (a fixed order
    // keeps concurrent placements from deadlocking) and read the
    // authoritative stock, price and label under the lock — one
    // query for the whole cart.
    let variant_ids: Vec<i64> = lines.iter().map(|l| l.variant_id).collect();
    let locked = order_repository::lock_variants(tx, &variant_ids).await?;
    let locked_for = |variant_id: i64| locked.iter().find(|row| row.variant_id == variant_id);

    // Effective lines at locked prices; sold-out lines are skipped
    // (the cart page already told the customer to remove them),
    // a partially available line aborts the whole order.
    let mut effective: Vec<CartLine> = Vec::new();
    for line in &lines {
        let Some(row) = locked_for(line.variant_id) else {
            return Err(PlaceError::StockShortage);
        };
        // A product drafted or archived after entering the cart is
        // off sale: the line is skipped, exactly like a sold-out
        // line. The cart and confirm pages already showed it as
        // off sale and excluded it from the totals, so aborting
        // the whole order here would contradict the presented
        // price and report a stock shortage that does not exist.
        // The skipped line stays in the cart.
        if row.status != "public" || !row.publish_open || !row.is_enabled {
            continue;
        }
        // Stock rules follow the product's mode: untracked sells
        // without limit, backorderable sells past zero, tracked
        // keeps the long-standing rules (zero skips the line,
        // short aborts the order).
        if row.track_stock && !row.allow_backorder {
            if row.stock == 0 {
                continue;
            }
            if line.quantity > row.stock {
                return Err(PlaceError::StockShortage);
            }
        }
        let mut l = line.clone();
        l.unit_price_yen = row.price_yen;
        l.stock = row.stock;
        let (code, name, rate) = rate_of(row.tax_class_id);
        l.tax_class_code = code;
        l.tax_class_name = name;
        l.tax_rate_percent = rate;
        effective.push(l);
    }
    if effective.is_empty() {
        return Err(PlaceError::EmptyCart);
    }
    Ok((effective, locked))
}

/// Finds one locked variant row by variant id.
fn locked_row(
    locked: &[order_repository::LockedVariant],
    variant_id: i64,
) -> Option<&order_repository::LockedVariant> {
    locked.iter().find(|row| row.variant_id == variant_id)
}

/// Writes the order and everything attached to it inside
/// the placement transaction: the order row, its snapshot
/// lines and per-class tax breakdown, the tracked stock
/// decrements with their ledger rows, the coupon redemption
/// ledger and used stamps, the cart clear, the guest
/// reminder-contact retirement and the payment row. Returns
/// the new order's id; the caller commits.
#[allow(clippy::too_many_arguments)]
async fn write_order(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner: &CartOwner,
    flow: &CheckoutFlow,
    placed_via: PlacedVia,
    effective: &[CartLine],
    locked: &[order_repository::LockedVariant],
    outcome: &crate::services::coupon_service::CouponOutcome,
    totals: &cart_service::CartTotals,
    delivery_date: Option<chrono::NaiveDate>,
    delivery_slot: &'static str,
    delivery_slot_label: &'static str,
    gift: &crate::models::gift::ValidGift,
    points: crate::models::shop_settings::PointsSettings,
) -> Result<i64, PlaceError> {
    // Points the member spends on this order (EC V2 core 1):
    // members only, programme on, clamped to the total. The
    // ledger spend itself happens after the order row exists
    // (the movements carry its id), still inside this
    // transaction — a balance that moved since the confirm
    // screen refuses the whole placement.
    let points_to_use = if points.enabled && owner.user_id().is_some() {
        flow.points_to_use.clamp(0, totals.total_yen)
    } else {
        0
    };
    let order_id = order_repository::insert_order(
        &mut **tx,
        &order_repository::NewOrder {
            user_id: owner.user_id(),
            email: flow.email.trim().to_string(),
            family_name: flow.family_name.trim().to_string(),
            given_name: flow.given_name.trim().to_string(),
            postal_code: flow.postal_code.trim().to_string(),
            prefecture: flow.prefecture.trim().to_string(),
            city: flow.city.trim().to_string(),
            address_line: flow.address_line.trim().to_string(),
            building: {
                let b = flow.building.trim();
                if b.is_empty() {
                    None
                } else {
                    Some(b.to_string())
                }
            },
            phone: flow.phone.trim().to_string(),
            subtotal_yen: totals.subtotal_yen,
            discount_yen: totals.discount_yen,
            shipping_yen: totals.shipping_yen,
            shipping_discount_yen: totals.shipping_discount_yen,
            cod_fee_yen: totals.cod_fee_yen,
            total_yen: totals.total_yen,
            coupon_code: outcome.primary_code(),
            delivery_date,
            delivery_slot: delivery_slot.to_string(),
            delivery_slot_label: delivery_slot_label.to_string(),
            placed_via,
            noshi_code: gift.noshi_code.to_string(),
            noshi_label: gift.noshi_label.to_string(),
            noshi_name: gift.noshi_name.clone(),
            gift_message: gift.gift_message.clone(),
            points_used_yen: points_to_use,
        },
    )
    .await?;
    // Snapshot lines carry the locked name and price, inserted
    // and stock-decremented as one statement each.
    let new_lines: Vec<order_repository::NewOrderLine> = effective
        .iter()
        .map(|line| {
            let row = locked_row(locked, line.variant_id).expect("every line was locked");
            order_repository::NewOrderLine {
                product_id: line.product_id,
                variant_id: line.variant_id,
                product_name: row.name.clone(),
                variant_label: row.variant_label.clone(),
                unit_price_yen: row.price_yen,
                quantity: line.quantity,
                tax_class_code: line.tax_class_code.clone(),
                rate_percent: line.tax_rate_percent,
                points_earn_percent: row.points_earn_percent.unwrap_or(points.earn_percent),
            }
        })
        .collect();
    order_repository::insert_lines(tx, order_id, &new_lines).await?;
    order_repository::insert_tax_breakdowns(tx, order_id, &totals.tax_breakdown).await?;
    // Only tracked products move stock: an untracked product's
    // counter is bookkeeping the shop opted out of. Decrements
    // and their ledger rows commit together.
    let tracked_lines: Vec<order_repository::NewOrderLine> = new_lines
        .iter()
        .filter(|line| {
            locked_row(locked, line.variant_id)
                .map(|row| row.track_stock)
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    order_repository::decrement_stocks(tx, &tracked_lines).await?;
    for line in &tracked_lines {
        crate::repositories::stock_ledger_repository::insert_adjustment(
            tx,
            line.variant_id,
            -line.quantity,
            "order",
            Some(order_id),
            None,
        )
        .await
        .map_err(|e| match e {
            AppError::Db(db) => PlaceError::Db(db),
            other => PlaceError::Db(sqlx::Error::Protocol(other.to_string())),
        })?;
    }
    // Record the applied coupons in the redemption ledger and
    // refresh each held copy's used stamp from the new counts:
    // the ledger is what the caps count, and the stamp is the
    // member surface's "this copy is exhausted" signal. The
    // definition locks the engine took make the counts exact,
    // so no unique-violation race remains here.
    for applied in &outcome.applied {
        coupon_repository::insert_redemption(&mut **tx, order_id, owner.user_id(), applied).await?;
        if let Some(uid) = owner.user_id() {
            coupon_repository::sync_used_stamp(&mut **tx, uid, &applied.code).await?;
        }
    }
    let ordered_variant_ids: Vec<i64> = effective.iter().map(|l| l.variant_id).collect();
    cart_repository::clear_lines(
        &mut **tx,
        owner.user_id(),
        owner.session_key(),
        &ordered_variant_ids,
    )
    .await?;
    // A placed order retires the guest reminder contact
    // with it (C3 completion): the address must not
    // outlive the cart it was captured from.
    if owner.user_id().is_none()
        && let Some(session_key) = owner.session_key()
    {
        crate::repositories::cart_reminder_repository::delete_guest_contact(&mut **tx, session_key)
            .await
            .map_err(PlaceError::Db)?;
    }

    if points_to_use > 0 {
        let user_id = owner.user_id().expect("points spend is member-only");
        crate::services::point_service::spend_in_tx(tx, user_id, order_id, points_to_use)
            .await
            .map_err(|e| match e {
                AppError::Conflict(_) => PlaceError::PriceChanged,
                AppError::Db(db) => PlaceError::Db(db),
                other => PlaceError::Db(sqlx::Error::Protocol(other.to_string())),
            })?;
    }

    // Every order gets its payment row in the same transaction:
    // an order exists only with its payment record. A KOMOJU row
    // starts unsettled (the session is created after this commit
    // — a network call never runs inside the placement
    // transaction); a COD row starts pending_payment with the
    // confirmation mail claimed, because the checkout controller
    // sends it at placement, as always. The agent dummy's row
    // starts settled (see PaymentMethod::initial_status) with
    // the mail claimed the same way, so it takes the COD
    // completion path in the controller. A KOMOJU row on a
    // ¥0 order starts settled with the mail claimed the same
    // way: no session can be created for ¥0, so the order
    // completes in place (see
    // PaymentMethod::settles_without_session).
    let method = flow.payment_method;
    // The payment method charges only what points did not
    // cover; a fully points-covered order takes the same
    // settled-in-place path as a ¥0 order.
    let payable_yen = totals.total_yen - points_to_use;
    payment_repository::insert_payment(
        &mut **tx,
        &payment_repository::NewPayment {
            order_id,
            provider: crate::services::payment_service::provider_for_method(method),
            method: method.slug(),
            status: method.initial_status_for(payable_yen),
            amount_yen: payable_yen,
            confirmation_mail_claimed: method.settles_without_session(payable_yen),
        },
    )
    .await?;
    Ok(order_id)
}

/// Returns an order's stock inside the caller's transaction,
/// exactly once: the `stock_restocked_at` guard is read and set
/// under the caller's order-row lock, so cancellation and a full
/// refund can never both restore the same units. Untracked
/// products never decremented, so they never restore. Returns
/// whether this call did the restock.
pub async fn restock_order_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: i64,
    reason: &str,
) -> Result<bool, AppError> {
    use crate::repositories::admin::order_repository as admin_repo;
    let restocked_at: Option<Option<chrono::DateTime<chrono::Utc>>> =
        sqlx::query_scalar("SELECT stock_restocked_at FROM orders WHERE id = $1 FOR UPDATE")
            .bind(order_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| AppError::db("read restock guard", e))?;
    match restocked_at {
        Some(None) => {}
        // Already restocked (or no such order): nothing to do.
        _ => return Ok(false),
    }
    for (variant_id, product_id, quantity) in
        admin_repo::order_line_quantities(tx, order_id).await?
    {
        let track_stock: bool =
            sqlx::query_scalar("SELECT track_stock FROM products WHERE id = $1")
                .bind(product_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(|e| AppError::db("read stock mode", e))?;
        if !track_stock {
            continue;
        }
        admin_repo::restore_stock(tx, variant_id, product_id, i64::from(quantity)).await?;
        // Ledger rows name the variant; a pre-variant legacy line
        // restores to the default variant, whose row the ledger
        // cannot name with certainty, so it writes none.
        if let Some(variant_id) = variant_id {
            crate::repositories::stock_ledger_repository::insert_adjustment(
                tx,
                variant_id,
                quantity,
                reason,
                Some(order_id),
                None,
            )
            .await?;
        }
    }
    sqlx::query("UPDATE orders SET stock_restocked_at = now() WHERE id = $1")
        .bind(order_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("set restock guard", e))?;
    Ok(true)
}

/// Cancels an order inside the caller's transaction: status,
/// stock, and (when asked) the releases that belong to an order
/// whose money never settled.
///
/// This is the one cancellation compensation in the store: the
/// admin status change, the payment state machine (expiry,
/// pre-capture cancellation) and the member cancellation path
/// all call it, so they can never compensate differently. The
/// caller holds the order row lock
/// (the lock order everywhere is order, then payment) and owns
/// the audit row.
///
/// Returns `false` when the order was no longer cancellable
/// (already shipped, delivered or cancelled) and nothing changed.
pub async fn cancel_order_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order: &Order,
    release_coupon: bool,
    points_expiry_days: i32,
) -> Result<bool, AppError> {
    use crate::repositories::admin::order_repository as admin_repo;
    if !matches!(order.status.as_str(), "received" | "preparing") {
        return Ok(false);
    }
    admin_repo::set_order_status(tx, order.id, "cancelled").await?;
    // The stock return goes through the guarded restock: the
    // order's stock comes back exactly once in its life, here
    // or at a full refund, never both.
    restock_order_in_tx(tx, order.id, "cancel").await?;
    // The points reversal rides the same compensation
    // (EC V2 core 1): the member's spent points return to
    // their grant lots. Pre-shipment cancels never carry an
    // earn, so the revocation half finds nothing. The
    // points expiry window comes from the caller, read
    // before its transaction opened (the A6 pattern).
    if let Some(user_id) = order.user_id {
        crate::services::point_service::reverse_order_in_tx(
            tx,
            user_id,
            order.id,
            points_expiry_days,
        )
        .await?;
    }
    // The coupon release is the caller's decision, and every
    // caller applies the same rule: release when no
    // money settled on the order — the customer never got the
    // order, so the discount was never really spent; a settled
    // or refunded order keeps its coupons consumed.
    if release_coupon {
        // The ledger release (Coupon Shopify-Core spec): the
        // order's redemption rows are deleted and each affected
        // held copy's used stamp is recomputed from what
        // remains, so caps and copies return together.
        let codes = coupon_repository::redemption_codes_for_order(&mut **tx, order.id).await?;
        coupon_repository::delete_redemptions_for_order(&mut **tx, order.id).await?;
        if let Some(user_id) = order.user_id {
            for code in &codes {
                coupon_repository::sync_used_stamp(&mut **tx, user_id, code).await?;
            }
        }
    }
    // The payment record follows: a KOMOJU payment that never
    // settled is cancelled with the order (nothing to refund);
    // a COD row flips to cancelled so the back office reads one
    // truth. A settled payment is deliberately untouched here —
    // its money moves through refunds, not cancellation.
    if let Some(payment) = payment_repository::find_by_order_id(&mut **tx, order.id).await? {
        let settled = payment.is_paid()
            || matches!(
                payment.status.as_str(),
                crate::models::payment::STATUS_REFUNDED
                    | crate::models::payment::STATUS_PARTIALLY_REFUNDED
            );
        if !settled {
            payment_repository::set_status(
                &mut **tx,
                payment.id,
                crate::models::payment::STATUS_CANCELLED,
            )
            .await?;
        }
    }
    Ok(true)
}

/// Mails an order's buyer its confirmation, off the request
/// path: spawned, so a slow or failing mail server never blocks
/// checkout, an admin click or a webhook answer. The payment
/// row's claim column is what makes KOMOJU confirmations
/// exactly-once; this helper only sends.
///
/// Lives in the service layer rather than the checkout
/// controller that calls it, beside the placement it follows.
/// This is the cash-on-delivery wording (pay at the door); a
/// captured KOMOJU order gets the paid wording from the payment
/// service's own spawn, claimed through the payment row.
pub fn spawn_confirmation_mail(pool: PgPool, order_id: i64) {
    tokio::spawn(async move {
        let sent: Result<(), String> = async {
            let order = find_order(&pool, order_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("order vanished before confirmation mail".to_string())?;
            let lines = order_lines(&pool, order_id)
                .await
                .map_err(|e| e.to_string())?;
            let breakdown = crate::repositories::order_repository::tax_breakdowns(&pool, order_id)
                .await
                .unwrap_or_default();
            crate::mail::send_order_confirmation(&pool, &order, &lines, &breakdown)
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        if let Err(message) = sent {
            tracing::error!(order_id, "confirmation mail failed: {message}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{CheckoutFlow, PaymentMethod, validate_delivery};

    use crate::models::User;

    fn member() -> User {
        User {
            id: 1,
            email: "member@example.jp".to_string(),
            family_name: "山田".to_string(),
            given_name: "太郎".to_string(),
            family_name_kana: "ヤマダ".to_string(),
            given_name_kana: "タロウ".to_string(),
            phone: "09012345678".to_string(),
            postal_code: "1500001".to_string(),
            prefecture: "東京都".to_string(),
            city: "渋谷区".to_string(),
            address_line: "神南1-1-1".to_string(),
            building: None,
            mail_magazine: false,
            has_password: true,
            withdrawn_at: None,
        }
    }

    #[test]
    fn empty_flow_fields_are_filled_from_the_profile() {
        let mut flow = CheckoutFlow::default();
        flow.fill_empty_from_user(&member());
        assert_eq!(flow.family_name, "山田");
        assert_eq!(flow.postal_code, "1500001");
        assert_eq!(flow.prefecture, "東京都");
        assert_eq!(flow.phone, "09012345678");
    }

    #[test]
    fn entered_fields_survive_the_fill() {
        let mut flow = CheckoutFlow {
            city: "ゲスト市".to_string(),
            ..Default::default()
        };
        flow.fill_empty_from_user(&member());
        assert_eq!(flow.city, "ゲスト市");
        assert_eq!(flow.family_name, "山田");
    }

    fn valid_guest_flow() -> CheckoutFlow {
        CheckoutFlow {
            email: "guest@example.com".to_string(),
            family_name: "山田".to_string(),
            given_name: "太郎".to_string(),
            postal_code: "1500001".to_string(),
            prefecture: "東京都".to_string(),
            city: "渋谷区".to_string(),
            address_line: "道玄坂1-1-1".to_string(),
            building: String::new(),
            phone: "09012345678".to_string(),
            delivery_date: String::new(),
            delivery_slot: "none".to_string(),
            coupon_codes: Vec::new(),
            payment_method: PaymentMethod::Cod,
            payment_method_chosen: false,
            save_address: false,
            noshi: String::new(),
            noshi_name: String::new(),
            gift_message: String::new(),
            points_to_use: 0,
        }
    }

    #[test]
    fn valid_guest_flow_passes() {
        assert!(
            validate_delivery(
                &valid_guest_flow(),
                false,
                &crate::models::ShopSettings::fallback(),
                &[]
            )
            .is_empty()
        );
    }

    #[test]
    fn guest_without_email_fails_on_email_only() {
        let mut flow = valid_guest_flow();
        flow.email = String::new();
        let errors = validate_delivery(&flow, false, &crate::models::ShopSettings::fallback(), &[]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "email");
    }

    #[test]
    fn member_needs_no_email_field() {
        let mut flow = valid_guest_flow();
        flow.email = String::new();
        assert!(
            validate_delivery(&flow, true, &crate::models::ShopSettings::fallback(), &[],)
                .is_empty()
        );
    }

    #[test]
    fn bad_postal_and_phone_are_both_reported() {
        let mut flow = valid_guest_flow();
        flow.postal_code = "123".to_string();
        flow.phone = "123".to_string();
        let fields: Vec<_> =
            validate_delivery(&flow, false, &crate::models::ShopSettings::fallback(), &[])
                .iter()
                .map(|e| e.field)
                .collect();
        assert!(fields.contains(&"postal_code"));
        assert!(fields.contains(&"phone"));
    }

    #[test]
    fn overlong_delivery_fields_fail_per_field() {
        let mut flow = valid_guest_flow();
        flow.family_name = "あ".repeat(51);
        flow.city = "い".repeat(101);
        flow.building = "う".repeat(101);
        let fields: Vec<_> =
            validate_delivery(&flow, false, &crate::models::ShopSettings::fallback(), &[])
                .iter()
                .map(|e| e.field)
                .collect();
        assert!(fields.contains(&"family_name"));
        assert!(fields.contains(&"city"));
        assert!(fields.contains(&"building"));
    }

    #[test]
    fn slot_without_a_date_fails_on_the_date_field() {
        let mut flow = valid_guest_flow();
        flow.delivery_slot = "am".to_string();
        let errors = validate_delivery(&flow, false, &crate::models::ShopSettings::fallback(), &[]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "delivery_date");
    }

    #[test]
    fn unknown_slot_code_fails_on_the_slot_field() {
        let mut flow = valid_guest_flow();
        flow.delivery_slot = "bogus".to_string();
        let errors = validate_delivery(&flow, false, &crate::models::ShopSettings::fallback(), &[]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "delivery_slot");
    }

    #[test]
    fn guest_with_a_second_at_sign_fails_on_email() {
        // The strict shared email rule: what sign-up refuses,
        // checkout refuses too.
        let mut flow = valid_guest_flow();
        flow.email = "a@b@c.d".to_string();
        let errors = validate_delivery(&flow, false, &crate::models::ShopSettings::fallback(), &[]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "email");
    }
}
