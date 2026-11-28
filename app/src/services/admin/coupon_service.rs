//! Admin coupon reads and definition writes.
//!
//! The read side of the coupon screens: every issued copy with
//! its holder and use state, and the definition list/form
//! reads. The write side is coupon definition management
//! (Coupon Admin spec): create/update with the §5.1
//! redemption locks, and activate/deactivate, all audited.

use chrono::TimeZone as _;
use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{
    COUPON_AMOUNT_LOCKED_MESSAGE, COUPON_TARGET_PRODUCT_REQUIRED_MESSAGE,
    COUPON_TARGET_VARIANT_REQUIRED_MESSAGE, DATETIME_FORMAT_MESSAGE,
};
use crate::models::AdminCouponRow;
use crate::models::order::order_number_for;
use crate::repositories::coupon_repository;

/// Returns one page of issued coupon copies with definitions
/// and holders, newest issue first, plus the total count.
pub async fn list_admin(
    pool: &PgPool,
    limit: i64,
    offset: i64,
    code: Option<&str>,
) -> Result<(Vec<AdminCouponRow>, i64), AppError> {
    coupon_repository::list_admin(pool, limit, offset, code).await
}

/// Returns the displayed number of the order that used one coupon
/// copy, computed from the row itself: the number derives from the
/// order's ID and placement instant, both carried by the list
/// query — no extra order query, no failure to degrade from.
pub fn used_order_number(row: &AdminCouponRow) -> String {
    match (row.used_order_id, row.used_order_created_at) {
        (Some(id), Some(created_at)) => order_number_for(id, created_at),
        _ => String::new(),
    }
}

/// Every coupon definition with its redemption counts, for
/// the definitions list.
///
/// # Errors
///
/// [`AppError::Db`] when the read fails.
pub async fn list_definitions(
    pool: &PgPool,
) -> Result<Vec<crate::models::CouponDefinitionRow>, AppError> {
    Ok(coupon_repository::list_definitions(pool).await?)
}

/// One definition by id. A read failure logs and reads as
/// missing: the screens answer their not-found page either
/// way, but the failure stays visible in the log.
pub async fn definition(pool: &PgPool, id: i64) -> Option<crate::models::CouponDefinition> {
    coupon_repository::find_definition(pool, id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("coupon definition lookup failed: {e}");
            None
        })
}

/// One definition's current form-ticket stamp (epoch
/// microseconds). A failed stamp read is logged and reads
/// as 0, a value no real stamp equals, so the ticket fails
/// closed (the rule `profile_stamp` documents on the member
/// side).
pub async fn stamp(pool: &PgPool, id: i64) -> i64 {
    match coupon_repository::updated_at(pool, id).await {
        Ok(stamp) => stamp.map(|t| t.timestamp_micros()).unwrap_or(0),
        Err(e) => {
            tracing::error!("coupon stamp read failed: {e}");
            0
        }
    }
}

/// The coupon form's target pickers: products as (id,
/// label) and variants as (id, label, parent product id).
/// A failed read logs and reads as an empty picker.
pub async fn form_options(pool: &PgPool) -> (Vec<(i64, String)>, Vec<(i64, String, i64)>) {
    let products = coupon_repository::product_options(pool)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("coupon product options read failed: {e}");
            Vec::new()
        });
    let variants = coupon_repository::variant_options(pool)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("coupon variant options read failed: {e}");
            Vec::new()
        });
    (products, variants)
}

/// Whether a definition's fields are locked, from its
/// redemption state (§5.1): once a code has redemptions,
/// its code and money fields are frozen. Returns (code
/// locked, money locked); a failed read logs and reads as
/// unlocked, as the screens have always degraded.
pub async fn field_locks(
    pool: &PgPool,
    definition: &crate::models::CouponDefinition,
) -> (bool, bool) {
    let redeemed = coupon_repository::has_redemptions(pool, &definition.code)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("coupon redemption check failed: {e}");
            false
        });
    (redeemed, redeemed)
}

/// How many members hold an unredeemed copy of the code —
/// the deactivate confirm page's §5.2 count. A failed read
/// logs and reads as zero.
pub async fn unredeemed_holders(pool: &PgPool, code: &str) -> i64 {
    coupon_repository::unredeemed_holders(pool, code)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("coupon holder count failed: {e}");
            0
        })
}

// ---- Coupon definition writes (Coupon Admin spec §5) ----

/// The raw definition form, before parsing/validation.
/// Numbers and window ends arrive as strings, as posted.
pub struct CouponDefinitionInput {
    /// Code as typed.
    pub code: String,
    /// Value kind as selected (`fixed` when absent).
    pub discount_type: String,
    /// Discount as typed.
    pub discount_yen: String,
    /// Percent as typed.
    pub percent: String,
    /// Scope kind as selected (`order` when absent).
    pub scope: String,
    /// Scoped product id as selected.
    pub product_id: String,
    /// Scoped variant id as selected.
    pub variant_id: String,
    /// Total cap as typed (empty = unlimited).
    pub max_uses_total: String,
    /// Per-customer cap as typed (empty = unlimited).
    pub max_uses_per_customer: String,
    /// Whether the discount applies without a code.
    pub is_automatic: bool,
    /// Minimum subtotal as typed.
    pub min_subtotal_yen: String,
    /// Message as typed.
    pub description: String,
    /// Window start as typed (`datetime-local`, JST).
    pub starts_at: String,
    /// Window end as typed.
    pub ends_at: String,
    /// Whether the band should show this coupon.
    pub band_featured: bool,
    /// Initial active state (create only).
    pub active: bool,
}

/// A validated definition, ready to store.
type ParsedDefinition = crate::models::CouponDefinitionWrite;

/// Why a definition write did not happen.
pub enum CouponWriteError {
    /// Field errors to show on the form.
    Validation(Vec<crate::validation::FieldError>),
    /// The definition does not exist.
    NotFound,
    /// The definition was updated after the edit form was
    /// rendered (the form-ticket guard): the save is refused
    /// so a stale form cannot silently revert newer values.
    Stale,
    /// A database failure.
    Db(AppError),
}

/// Canonicalises a code: trimmed and uppercased.
fn canonical_code(raw: &str) -> String {
    raw.trim().to_uppercase()
}

/// Whether a canonical code matches the D1 charset and
/// length: 4–20 of `A-Z 0-9 - _`.
fn code_shape_ok(code: &str) -> bool {
    (4..=20).contains(&code.chars().count())
        && code
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Parses one `datetime-local` value as JST. Empty means
/// no bound.
fn parse_window(
    raw: &str,
    field: &'static str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, crate::validation::FieldError> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let make_err = || crate::validation::FieldError::new(field, DATETIME_FORMAT_MESSAGE);
    let naive =
        chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").map_err(|_| make_err())?;
    let jst = crate::models::jst::jst();
    match jst.from_local_datetime(&naive).single() {
        Some(dt) => Ok(Some(dt.with_timezone(&chrono::Utc))),
        None => Err(make_err()),
    }
}

/// Parses and validates the form. `existing` is the
/// definition being edited (for the duplicate check's
/// exclusion); redemption locks are checked separately
/// inside the write transaction.
async fn parse_definition(
    pool: &PgPool,
    input: &CouponDefinitionInput,
    existing_id: Option<i64>,
) -> Result<ParsedDefinition, Vec<crate::validation::FieldError>> {
    let mut errors: Vec<crate::validation::FieldError> = Vec::new();
    let code = parse_code(pool, input, existing_id, &mut errors).await;
    let (discount_type, discount, percent, min_subtotal) = parse_value(input, &mut errors);
    let (scope, product_id, variant_id) =
        parse_scope(pool, input, discount_type, &mut errors).await;
    let max_uses_total = parse_cap(&input.max_uses_total, "max_uses_total", &mut errors);
    let max_uses_per_customer = parse_cap(
        &input.max_uses_per_customer,
        "max_uses_per_customer",
        &mut errors,
    );
    let description = parse_description(input, &mut errors);
    let (starts_at, ends_at) = parse_window_pair(input, &mut errors);

    if errors.is_empty() {
        Ok(ParsedDefinition {
            code,
            discount_type: discount_type.to_string(),
            discount_yen: discount,
            percent,
            scope: scope.to_string(),
            product_id,
            variant_id,
            max_uses_total,
            max_uses_per_customer,
            is_automatic: input.is_automatic,
            min_subtotal_yen: min_subtotal,
            description,
            starts_at,
            ends_at,
            band_featured: input.band_featured,
        })
    } else {
        Err(errors)
    }
}

/// The canonical code: shape-checked, and uniqueness-
/// checked against the other definitions (excluding the
/// one being edited).
async fn parse_code(
    pool: &PgPool,
    input: &CouponDefinitionInput,
    existing_id: Option<i64>,
    errors: &mut Vec<crate::validation::FieldError>,
) -> String {
    use crate::validation::{FieldError, push_uniqueness_check};
    let code = canonical_code(&input.code);
    if !code_shape_ok(&code) {
        errors.push(FieldError::new(
            "code",
            "コードは半角英数・ハイフン・アンダースコアで4〜20文字で入力してください。",
        ));
    } else {
        push_uniqueness_check(
            errors,
            "code",
            coupon_repository::code_taken(pool, &code, existing_id).await,
            "このコードは既に使われています。",
            "コードを確認できませんでした。",
        );
    }
    code
}

/// The value kind and its amount: the fixed yen discount
/// or the percent, plus the minimum subtotal, cross-checked
/// (a fixed discount may not exceed the minimum subtotal).
fn parse_value(
    input: &CouponDefinitionInput,
    errors: &mut Vec<crate::validation::FieldError>,
) -> (&'static str, i64, Option<i32>, i64) {
    use crate::validation::FieldError;
    // Value kind (Coupon Shopify-Core spec): an absent value
    // is the pre-pack fixed type, so older posts keep working.
    let discount_type = match input.discount_type.trim() {
        "" | "fixed" => "fixed",
        "percent" => "percent",
        "free_shipping" => "free_shipping",
        _ => {
            errors.push(FieldError::new(
                "discount_type",
                "割引タイプを選択してください。",
            ));
            "fixed"
        }
    };
    let discount = if discount_type == "fixed" {
        let parsed = input.discount_yen.trim().parse::<i64>().unwrap_or(-1);
        if parsed < 1 {
            errors.push(FieldError::new(
                "discount_yen",
                "割引額は1円以上の整数で入力してください。",
            ));
        }
        parsed.max(0)
    } else {
        0
    };
    let percent = if discount_type == "percent" {
        match input.percent.trim().parse::<i32>() {
            Ok(v) if (1..=100).contains(&v) => Some(v),
            _ => {
                errors.push(FieldError::new(
                    "percent",
                    "割引率は1〜100の整数で入力してください。",
                ));
                None
            }
        }
    } else {
        None
    };
    let min_subtotal = input.min_subtotal_yen.trim().parse::<i64>().unwrap_or(-1);
    if min_subtotal < 0 {
        errors.push(FieldError::new(
            "min_subtotal_yen",
            "最低注文金額は0円以上の整数で入力してください。",
        ));
    }
    if discount_type == "fixed" && discount >= 1 && min_subtotal > 0 && discount > min_subtotal {
        errors.push(FieldError::new(
            "discount_yen",
            "割引額は最低注文金額以下にしてください。",
        ));
    }
    (discount_type, discount, percent, min_subtotal)
}

/// The scope and its target: a product/variant scope's
/// target must exist; free shipping is order-scope only.
async fn parse_scope(
    pool: &PgPool,
    input: &CouponDefinitionInput,
    discount_type: &str,
    errors: &mut Vec<crate::validation::FieldError>,
) -> (&'static str, Option<i64>, Option<i64>) {
    use crate::validation::FieldError;
    // Scope: an absent value is the whole order, as before.
    let scope = match input.scope.trim() {
        "" | "order" => "order",
        "product" => "product",
        "variant" => "variant",
        _ => {
            errors.push(FieldError::new("scope", "対象を選択してください。"));
            "order"
        }
    };
    if discount_type == "free_shipping" && scope != "order" {
        errors.push(FieldError::new(
            "scope",
            "送料無料クーポンの対象は注文全体のみです。",
        ));
    }
    let mut product_id = None;
    let mut variant_id = None;
    if scope == "product" {
        match input.product_id.trim().parse::<i64>() {
            Ok(id) => {
                product_id = Some(id);
                match coupon_repository::product_exists(pool, id).await {
                    Ok(true) => {}
                    Ok(false) => errors.push(FieldError::new(
                        "product_id",
                        COUPON_TARGET_PRODUCT_REQUIRED_MESSAGE,
                    )),
                    Err(_) => errors.push(FieldError::new(
                        "product_id",
                        "対象商品を確認できませんでした。",
                    )),
                }
            }
            Err(_) => errors.push(FieldError::new(
                "product_id",
                COUPON_TARGET_PRODUCT_REQUIRED_MESSAGE,
            )),
        }
    }
    if scope == "variant" {
        match input.variant_id.trim().parse::<i64>() {
            Ok(id) => {
                variant_id = Some(id);
                match coupon_repository::variant_exists(pool, id).await {
                    Ok(true) => {}
                    Ok(false) => errors.push(FieldError::new(
                        "variant_id",
                        COUPON_TARGET_VARIANT_REQUIRED_MESSAGE,
                    )),
                    Err(_) => errors.push(FieldError::new(
                        "variant_id",
                        "対象バリエーションを確認できませんでした。",
                    )),
                }
            }
            Err(_) => errors.push(FieldError::new(
                "variant_id",
                COUPON_TARGET_VARIANT_REQUIRED_MESSAGE,
            )),
        }
    }
    (scope, product_id, variant_id)
}

/// One usage cap: empty means unlimited; a present value
/// must be a positive integer.
fn parse_cap(
    raw: &str,
    field: &'static str,
    errors: &mut Vec<crate::validation::FieldError>,
) -> Option<i32> {
    use crate::validation::FieldError;
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    match value.parse::<i32>() {
        Ok(v) if v > 0 => Some(v),
        _ => {
            errors.push(FieldError::new(
                field,
                "利用回数の上限は1以上の整数で入力してください（空欄で無制限）。",
            ));
            None
        }
    }
}

/// The display message: required, at most 100 characters.
fn parse_description(
    input: &CouponDefinitionInput,
    errors: &mut Vec<crate::validation::FieldError>,
) -> String {
    use crate::validation::FieldError;
    let description = input.description.trim().to_string();
    if description.is_empty() {
        errors.push(FieldError::new(
            "description",
            "表示メッセージを入力してください。",
        ));
    } else if description.chars().count() > 100 {
        errors.push(FieldError::new(
            "description",
            "表示メッセージは100文字以内で入力してください。",
        ));
    }
    description
}

/// The active window: both bounds optional; when both are
/// given the end must be after the start.
fn parse_window_pair(
    input: &CouponDefinitionInput,
    errors: &mut Vec<crate::validation::FieldError>,
) -> (
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
) {
    use crate::validation::FieldError;
    let starts_at = match parse_window(&input.starts_at, "starts_at") {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            None
        }
    };
    let ends_at = match parse_window(&input.ends_at, "ends_at") {
        Ok(v) => v,
        Err(e) => {
            errors.push(e);
            None
        }
    };
    if let (Some(s), Some(e)) = (starts_at, ends_at)
        && e <= s
    {
        errors.push(FieldError::new(
            "ends_at",
            "利用終了は利用開始より後にしてください。",
        ));
    }
    (starts_at, ends_at)
}

/// Creates one definition and audits it, atomically.
///
/// # Errors
///
/// Returns [`CouponWriteError::Validation`] with every
/// field error, or [`CouponWriteError::Db`].
pub async fn create_definition(
    pool: &PgPool,
    admin_id: i64,
    input: &CouponDefinitionInput,
) -> Result<i64, CouponWriteError> {
    let parsed = parse_definition(pool, input, None)
        .await
        .map_err(CouponWriteError::Validation)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("begin coupon create", e)))?;
    if input.band_featured {
        coupon_repository::clear_featured(&mut tx, admin_id)
            .await
            .map_err(|e| CouponWriteError::Db(AppError::db("clear band flag", e)))?;
    }
    let id = coupon_repository::insert_definition(&mut tx, &parsed, input.active)
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("insert coupon", e)))?;
    crate::services::admin::audit_service::record(
        &mut tx,
        admin_id,
        "coupon.create",
        "coupon",
        Some(id),
    )
    .await
    .map_err(CouponWriteError::Db)?;
    tx.commit()
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("commit coupon create", e)))?;
    tracing::info!(
        action = "coupon.create",
        coupon_id = id,
        "coupon definition created"
    );
    Ok(id)
}

/// Updates one definition. The §5.1 locks are re-checked
/// inside the transaction against the redemption state
/// at write time: with redemptions, code/discount/
/// minimum may not move.
///
/// # Errors
///
/// Returns [`CouponWriteError::Validation`] (including
/// lock refusals), [`CouponWriteError::NotFound`], or
/// [`CouponWriteError::Db`].
pub async fn update_definition(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    input: &CouponDefinitionInput,
    expected: i64,
) -> Result<(), CouponWriteError> {
    use crate::validation::FieldError;
    let existing = coupon_repository::find_definition(pool, id)
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("load coupon", e)))?
        .ok_or(CouponWriteError::NotFound)?;
    let parsed = parse_definition(pool, input, Some(id))
        .await
        .map_err(CouponWriteError::Validation)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("begin coupon update", e)))?;
    // The form-ticket staleness guard (see
    // `form_ticket_service`); this service's error type
    // names the refusal `Stale` instead of the shared
    // conflict.
    let current = coupon_repository::updated_at_for_update(&mut tx, id)
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("load coupon stamp", e)))?;
    crate::services::form_ticket_service::ensure_fresh(current, expected)
        .map_err(|_| CouponWriteError::Stale)?;
    let redeemed = coupon_repository::has_redemptions(&mut *tx, &existing.code)
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("check redemptions", e)))?;
    if redeemed {
        let mut errors = Vec::new();
        if parsed.code != existing.code {
            errors.push(FieldError::new(
                "code",
                "利用済みのためコードは変更できません。",
            ));
        }
        if parsed.discount_yen != existing.discount_yen {
            errors.push(FieldError::new(
                "discount_yen",
                COUPON_AMOUNT_LOCKED_MESSAGE,
            ));
        }
        if parsed.min_subtotal_yen != existing.min_subtotal_yen {
            errors.push(FieldError::new(
                "min_subtotal_yen",
                COUPON_AMOUNT_LOCKED_MESSAGE,
            ));
        }
        // The Shopify-core fields are locked by the same rule:
        // changing what a redeemed coupon *is* would rewrite
        // placed orders' economics.
        let locked = "利用済みのため変更できません。";
        if parsed.discount_type != existing.discount_type {
            errors.push(FieldError::new("discount_type", locked));
        }
        if parsed.percent != existing.percent {
            errors.push(FieldError::new("percent", locked));
        }
        if parsed.scope != existing.scope {
            errors.push(FieldError::new("scope", locked));
        }
        if parsed.product_id != existing.product_id {
            errors.push(FieldError::new("product_id", locked));
        }
        if parsed.variant_id != existing.variant_id {
            errors.push(FieldError::new("variant_id", locked));
        }
        if parsed.max_uses_total != existing.max_uses_total {
            errors.push(FieldError::new("max_uses_total", locked));
        }
        if parsed.max_uses_per_customer != existing.max_uses_per_customer {
            errors.push(FieldError::new("max_uses_per_customer", locked));
        }
        if parsed.is_automatic != existing.is_automatic {
            errors.push(FieldError::new("discount_type", locked));
        }
        if !errors.is_empty() {
            return Err(CouponWriteError::Validation(errors));
        }
    }
    if input.band_featured && !existing.band_featured {
        coupon_repository::clear_featured(&mut tx, admin_id)
            .await
            .map_err(|e| CouponWriteError::Db(AppError::db("clear band flag", e)))?;
    }
    coupon_repository::update_definition(&mut tx, id, &parsed, admin_id)
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("update coupon", e)))?;
    crate::services::admin::audit_service::record(
        &mut tx,
        admin_id,
        "coupon.update",
        "coupon",
        Some(id),
    )
    .await
    .map_err(CouponWriteError::Db)?;
    tx.commit()
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("commit coupon update", e)))?;
    tracing::info!(
        action = "coupon.update",
        coupon_id = id,
        "coupon definition updated"
    );
    Ok(())
}

/// Activates or deactivates one definition, audited
/// under its own code. Deactivation is the only delete.
///
/// # Errors
///
/// Returns [`CouponWriteError::NotFound`] or
/// [`CouponWriteError::Db`].
pub async fn set_definition_active(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    active: bool,
) -> Result<(), CouponWriteError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("begin coupon state", e)))?;
    let exists = coupon_repository::find_definition(pool, id)
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("load coupon", e)))?
        .is_some();
    if !exists {
        return Err(CouponWriteError::NotFound);
    }
    coupon_repository::set_active(&mut tx, id, active, admin_id)
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("set coupon active", e)))?;
    let action = if active {
        "coupon.activate"
    } else {
        "coupon.deactivate"
    };
    crate::services::admin::audit_service::record(&mut tx, admin_id, action, "coupon", Some(id))
        .await
        .map_err(CouponWriteError::Db)?;
    tx.commit()
        .await
        .map_err(|e| CouponWriteError::Db(AppError::db("commit coupon state", e)))?;
    tracing::info!(action, coupon_id = id, "coupon definition state changed");
    Ok(())
}
