//! Coupon repository.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{AdminCouponRow, Coupon, MemberCoupon};
use crate::pii::decrypt_field;

/// The evaluation column list: everything the coupon
/// engine prices from (Coupon Shopify-Core spec).
const EVAL_COLS: &str = "code, discount_type, discount_yen, percent, min_subtotal_yen,
                         scope, product_id, variant_id,
                         max_uses_total, max_uses_per_customer, is_automatic, description";

/// Finds an active coupon by its exact code.
pub async fn find_active<'e, E>(executor: E, code: &str) -> Result<Option<Coupon>, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "SELECT {EVAL_COLS} FROM coupons \
         WHERE code = $1 AND active \
         AND (starts_at IS NULL OR starts_at <= now()) \
         AND (ends_at IS NULL OR ends_at > now())"
    );
    sqlx::query_as::<_, Coupon>(&sql)
        .bind(code)
        .fetch_optional(executor)
        .await
}

/// The bulk twin of [`find_active`]: every active
/// definition among `codes`, in one query. Result order is
/// unspecified; the caller re-imposes its own.
pub async fn find_active_many(pool: &PgPool, codes: &[String]) -> Result<Vec<Coupon>, sqlx::Error> {
    let sql = format!(
        "SELECT {EVAL_COLS} FROM coupons \
         WHERE code = ANY($1) AND active \
         AND (starts_at IS NULL OR starts_at <= now()) \
         AND (ends_at IS NULL OR ends_at > now())"
    );
    sqlx::query_as::<_, Coupon>(&sql)
        .bind(codes)
        .fetch_all(pool)
        .await
}

/// Redemption totals for many codes at once, as (code,
/// count) rows; a code with no redemptions has no row.
pub async fn redemption_counts(
    pool: &PgPool,
    codes: &[String],
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT code, count(*) FROM coupon_redemptions \
         WHERE code = ANY($1) GROUP BY code",
    )
    .bind(codes)
    .fetch_all(pool)
    .await
}

/// One member's redemption totals for many codes at once,
/// as (code, count) rows; a code the member never redeemed
/// has no row.
pub async fn redemption_counts_for_user(
    pool: &PgPool,
    user_id: i64,
    codes: &[String],
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT code, count(*) FROM coupon_redemptions \
         WHERE user_id = $1 AND code = ANY($2) GROUP BY code",
    )
    .bind(user_id)
    .bind(codes)
    .fetch_all(pool)
    .await
}

/// Finds an active coupon by code and locks its definition
/// row inside the caller's transaction: placement serialises
/// on the definition so two racing orders cannot both pass
/// a usage cap.
pub async fn find_active_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    code: &str,
) -> Result<Option<Coupon>, sqlx::Error> {
    let sql = format!(
        "SELECT {EVAL_COLS} FROM coupons \
         WHERE code = $1 AND active \
         AND (starts_at IS NULL OR starts_at <= now()) \
         AND (ends_at IS NULL OR ends_at > now()) \
         FOR UPDATE"
    );
    sqlx::query_as::<_, Coupon>(&sql)
        .bind(code)
        .fetch_optional(&mut **tx)
        .await
}

/// Returns every active automatic discount, oldest first:
/// the candidates the engine slots into empty classes
/// without a code.
pub async fn list_automatics(pool: &PgPool) -> Result<Vec<Coupon>, sqlx::Error> {
    let sql = format!(
        "SELECT {EVAL_COLS} FROM coupons \
         WHERE active AND is_automatic \
         AND (starts_at IS NULL OR starts_at <= now()) \
         AND (ends_at IS NULL OR ends_at > now()) \
         ORDER BY id"
    );
    sqlx::query_as::<_, Coupon>(&sql).fetch_all(pool).await
}

/// Returns every active automatic discount inside the
/// caller's transaction, locking each definition row (the
/// placement twin of [`list_automatics`]).
pub async fn list_automatics_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Vec<Coupon>, sqlx::Error> {
    let sql = format!(
        "SELECT {EVAL_COLS} FROM coupons \
         WHERE active AND is_automatic \
         AND (starts_at IS NULL OR starts_at <= now()) \
         AND (ends_at IS NULL OR ends_at > now()) \
         ORDER BY id FOR UPDATE"
    );
    sqlx::query_as::<_, Coupon>(&sql).fetch_all(&mut **tx).await
}

/// Counts a code's recorded redemptions (the usage ledger).
pub async fn count_redemptions<'e, E>(executor: E, code: &str) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_scalar("SELECT count(*) FROM coupon_redemptions WHERE code = $1")
        .bind(code)
        .fetch_one(executor)
        .await
}

/// Counts one member's recorded redemptions of a code.
pub async fn count_redemptions_for_user<'e, E>(
    executor: E,
    user_id: i64,
    code: &str,
) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_scalar("SELECT count(*) FROM coupon_redemptions WHERE user_id = $1 AND code = $2")
        .bind(user_id)
        .bind(code)
        .fetch_one(executor)
        .await
}

/// Records one applied coupon on an order, inside the
/// caller's transaction: the redemption ledger row the
/// usage caps and the admin counts read.
pub async fn insert_redemption<'e, E>(
    executor: E,
    order_id: i64,
    user_id: Option<i64>,
    applied: &crate::models::AppliedCoupon,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "INSERT INTO coupon_redemptions (order_id, user_id, code, class, discount_yen)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(order_id)
    .bind(user_id)
    .bind(&applied.code)
    .bind(&applied.class)
    .bind(applied.discount_yen)
    .execute(executor)
    .await
    .map(|_| ())
}

/// Returns an order's redemption rows (code per row), for
/// the cancellation release.
pub async fn redemption_codes_for_order<'e, E>(
    executor: E,
    order_id: i64,
) -> Result<Vec<String>, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_scalar("SELECT code FROM coupon_redemptions WHERE order_id = $1")
        .bind(order_id)
        .fetch_all(executor)
        .await
}

/// Deletes an order's redemption rows, inside the caller's
/// transaction: the usage half of the coupon release (the
/// order never settled, so its coupons were never spent).
pub async fn delete_redemptions_for_order<'e, E>(
    executor: E,
    order_id: i64,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query("DELETE FROM coupon_redemptions WHERE order_id = $1")
        .bind(order_id)
        .execute(executor)
        .await
        .map(|_| ())
}

/// Recomputes a held copy's used stamp from the redemption
/// ledger and the definition's per-customer cap, inside the
/// caller's transaction: the stamp reads "this copy is
/// exhausted" — set exactly when the member's count reaches
/// a finite cap, cleared when a release drops it back below.
/// Copies the member does not hold are untouched.
pub async fn sync_used_stamp<'e, E>(
    executor: E,
    user_id: i64,
    code: &str,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "UPDATE user_coupons uc SET used_at = CASE
             WHEN c.max_uses_per_customer IS NOT NULL
              AND (SELECT count(*) FROM coupon_redemptions r
                   WHERE r.user_id = uc.user_id AND r.code = uc.code)
                  >= c.max_uses_per_customer
             THEN COALESCE(uc.used_at, now())
             ELSE NULL
         END
         FROM coupons c
         WHERE uc.user_id = $1 AND uc.code = $2 AND c.code = uc.code",
    )
    .bind(user_id)
    .bind(code)
    .execute(executor)
    .await
    .map(|_| ())
}

/// Releases a member's use of a code, inside the caller's
/// transaction: the grant row's used stamp is cleared, so the
/// once-only coupon becomes usable again. The exact inverse of
/// [`mark_used`], called when the order that consumed the coupon
/// is cancelled before the money settles (payment expiry or
/// cancellation) — never for a settled order: a refunded order
/// keeps the coupon consumed.
pub async fn release_used<'e, E>(executor: E, user_id: i64, code: &str) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "UPDATE user_coupons SET used_at = NULL \
         WHERE user_id = $1 AND code = $2 AND used_at IS NOT NULL",
    )
    .bind(user_id)
    .bind(code)
    .execute(executor)
    .await
    .map(|_| ())
}

/// Whether this member has already used the code (a grant row
/// with `used_at` set). Members without any grant row may still
/// use a public store code once; placement records the use.
pub async fn member_has_used<'e, E>(
    executor: E,
    user_id: i64,
    code: &str,
) -> Result<bool, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM user_coupons \
         WHERE user_id = $1 AND code = $2 AND used_at IS NOT NULL)",
    )
    .bind(user_id)
    .bind(code)
    .fetch_one(executor)
    .await
}

/// Records that a member used a code, inside the caller's
/// transaction: updates their grant row when one exists, and
/// inserts a used row for a public code they had no grant for,
/// so a public code is also once-per-member.
pub async fn mark_used<'e, E>(executor: E, user_id: i64, code: &str) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    // One statement: a grant row is stamped used; a member with no
    // row gets a used row inserted, so once-per-member holds either
    // way. The CTE shape (rather than an upsert) keeps the stamp
    // and the insert in one pass over the caller's transaction.
    // The unique (user_id, code) index is the backstop when two
    // placements race: the loser fails with a unique violation,
    // which placement maps to a coupon refusal.
    sqlx::query(
        "WITH updated AS ( \
             UPDATE user_coupons SET used_at = now() \
             WHERE user_id = $1 AND code = $2 AND used_at IS NULL RETURNING id \
         ) \
         INSERT INTO user_coupons (user_id, code, used_at) \
         SELECT $1, $2, now() \
         WHERE NOT EXISTS (SELECT 1 FROM updated) \
           AND NOT EXISTS (SELECT 1 FROM user_coupons WHERE user_id = $1 AND code = $2)",
    )
    .bind(user_id)
    .bind(code)
    .execute(executor)
    .await
    .map(|_| ())
}

/// Returns one page of issued coupon copies with definitions and
/// holders, newest first — the admin read-only coupon screen —
/// plus the total count.
///
/// `user_coupons` carries no order FK, so the using order is
/// resolved by (member, code) — the member's latest order that
/// recorded this code — folded into this one query as a lateral
/// join, so the screen costs one query however long the history.
pub async fn list_admin(
    pool: &PgPool,
    limit: i64,
    offset: i64,
    code: Option<&str>,
) -> Result<(Vec<AdminCouponRow>, i64), AppError> {
    let rows = sqlx::query_as::<_, AdminCouponRowWithTotal>(
        "SELECT c.code, c.description, \
                u.family_name AS holder_family_name, \
                u.given_name AS holder_given_name, \
                uc.issued_at, uc.used_at, \
                o.id AS used_order_id, o.created_at AS used_order_created_at, \
                COUNT(*) OVER () AS total \
         FROM user_coupons uc \
         JOIN coupons c ON c.code = uc.code \
         JOIN users u ON u.id = uc.user_id \
         LEFT JOIN LATERAL ( \
             SELECT o2.id, o2.created_at FROM orders o2 \
             WHERE o2.user_id = uc.user_id AND o2.coupon_code = uc.code \
             ORDER BY o2.created_at DESC LIMIT 1 \
         ) o ON true \
         WHERE ($3::text IS NULL OR uc.code = $3) \
         ORDER BY uc.issued_at DESC, uc.id DESC \
         LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .bind(code)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list coupons (admin)", e))?;
    // A page past the end returns no rows to carry the total;
    // only then is a separate count worth its round trip.
    if rows.is_empty() && offset > 0 {
        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM user_coupons uc WHERE ($1::text IS NULL OR uc.code = $1)",
        )
        .bind(code)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("count coupons (admin)", e))?;
        return Ok((Vec::new(), total));
    }
    let total = rows.first().map(|r| r.total).unwrap_or(0);
    Ok((
        rows.into_iter()
            .map(|r| AdminCouponRow {
                code: r.code,
                description: r.description,
                holder_name: match (r.holder_family_name, r.holder_given_name) {
                    (Some(family), Some(given)) => Some(format!(
                        "{} {}",
                        decrypt_field(&family),
                        decrypt_field(&given)
                    )),
                    _ => None,
                },
                issued_at: r.issued_at,
                used_at: r.used_at,
                used_order_id: r.used_order_id,
                used_order_created_at: r.used_order_created_at,
            })
            .collect(),
        total,
    ))
}

/// [`AdminCouponRow`] plus the pagination total, before splitting.
#[derive(sqlx::FromRow)]
struct AdminCouponRowWithTotal {
    code: String,
    description: String,
    holder_family_name: Option<String>,
    holder_given_name: Option<String>,
    issued_at: chrono::DateTime<chrono::Utc>,
    used_at: Option<chrono::DateTime<chrono::Utc>>,
    used_order_id: Option<i64>,
    used_order_created_at: Option<chrono::DateTime<chrono::Utc>>,
    total: i64,
}

/// Lists every coupon copy one member holds, with definitions
/// and use state — the mypage coupon section.
///
/// Held copies first, then used ones, newest issue first within
/// each group. The using order is resolved with the same lateral
/// join as the admin list. One query for the whole section.
/// The SQL expression for a definition's scope-target
/// display name: the product's name, or the product name
/// with the variant's option label; NULL for order scope.
/// `alias` is the coupons table's alias in the query.
fn target_label_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN {alias}.scope = 'product' THEN \
             (SELECT p.name FROM products p WHERE p.id = {alias}.product_id) \
           WHEN {alias}.scope = 'variant' THEN \
             (SELECT p.name || CASE WHEN {label} = '' THEN '' ELSE '（' || {label} || '）' END \
              FROM product_variants v JOIN products p ON p.id = v.product_id \
              WHERE v.id = {alias}.variant_id) \
         END",
        label = crate::repositories::variant_repository::LABEL_SQL
    )
}

pub async fn list_for_member(pool: &PgPool, user_id: i64) -> Result<Vec<MemberCoupon>, AppError> {
    let target_label = target_label_sql("c");
    let sql = format!(
        "SELECT c.code, c.description, c.discount_yen, c.discount_type, c.percent, \
                c.scope, {target_label} AS target_label, c.min_subtotal_yen, \
                c.active, c.starts_at, c.ends_at, \
                uc.issued_at, uc.used_at, \
                o.id AS used_order_id, o.created_at AS used_order_created_at \
         FROM user_coupons uc \
         JOIN coupons c ON c.code = uc.code \
         LEFT JOIN LATERAL ( \
             SELECT o2.id, o2.created_at FROM orders o2 \
             WHERE o2.user_id = uc.user_id AND o2.coupon_code = uc.code \
             ORDER BY o2.created_at DESC LIMIT 1 \
         ) o ON true \
         WHERE uc.user_id = $1 AND uc.voided_at IS NULL \
         ORDER BY uc.used_at IS NOT NULL, uc.issued_at DESC, uc.id DESC"
    );
    sqlx::query_as::<_, MemberCoupon>(&sql)
        .bind(user_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list member coupons", e))
}

/// The coupon the header band shows: the one
/// definition carrying the band flag (Coupon Admin
/// spec D4), active and inside its window, when it
/// exists.
pub async fn find_band_coupon(
    pool: &PgPool,
) -> Result<Option<crate::models::BandCoupon>, sqlx::Error> {
    sqlx::query_as::<_, crate::models::BandCoupon>(
        "SELECT code, discount_type, discount_yen, percent, description FROM coupons \
         WHERE active AND band_featured \
         AND (starts_at IS NULL OR starts_at <= now()) \
         AND (ends_at IS NULL OR ends_at > now()) \
         ORDER BY id LIMIT 1",
    )
    .fetch_optional(pool)
    .await
}

// ---- Coupon definition management (Coupon Admin spec) ----

const DEFINITION_COLS: &str = "id, code, discount_type, discount_yen, percent,
                               scope, product_id, variant_id,
                               max_uses_total, max_uses_per_customer, is_automatic,
                               min_subtotal_yen, description,
                               active, starts_at, ends_at, band_featured";

/// Every definition for the admin list, oldest first,
/// each with its redemption count (used grant rows).
pub async fn list_definitions(
    pool: &PgPool,
) -> Result<Vec<crate::models::CouponDefinitionRow>, sqlx::Error> {
    let target_label = target_label_sql("c");
    let sql = format!(
        "SELECT {DEFINITION_COLS}, {target_label} AS target_label,
                (SELECT count(*) FROM coupon_redemptions r
                 WHERE r.code = c.code) AS redemptions
         FROM coupons c ORDER BY c.id"
    );
    sqlx::query_as::<_, crate::models::CouponDefinitionRow>(&sql)
        .fetch_all(pool)
        .await
}

/// The definition's current `updated_at`: the stamp an
/// edit form ticket is minted with at render time. (The
/// `CouponDefinition` row struct does not carry the stamp,
/// so it is read on its own, like the tag's.)
pub async fn updated_at(
    pool: &PgPool,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, sqlx::Error> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM coupons WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(value)
}

/// Returns the definition's current `updated_at` with the
/// row locked: the edit form's staleness guard compares
/// its form ticket's stamp against this value inside the
/// update transaction (the product guard's shape).
pub async fn updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, sqlx::Error> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM coupons WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

/// Loads one definition by id.
pub async fn find_definition(
    pool: &PgPool,
    id: i64,
) -> Result<Option<crate::models::CouponDefinition>, sqlx::Error> {
    let sql = format!("SELECT {DEFINITION_COLS} FROM coupons WHERE id = $1");
    sqlx::query_as::<_, crate::models::CouponDefinition>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Whether a canonical code is already taken by another
/// definition (case-insensitive by construction: stored
/// codes are canonical and the comparison uppercases).
pub async fn code_taken(
    pool: &PgPool,
    code: &str,
    exclude_id: Option<i64>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM coupons
         WHERE upper(code) = upper($1) AND ($2::bigint IS NULL OR id <> $2))",
    )
    .bind(code)
    .bind(exclude_id)
    .fetch_one(pool)
    .await
}

/// The §5.1 predicate: a coupon has redemptions when any
/// grant row for it is marked used, or any order
/// references it. Grants alone do not count.
pub async fn has_redemptions<'e, E>(executor: E, code: &str) -> Result<bool, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM user_coupons WHERE code = $1 AND used_at IS NOT NULL)
              OR EXISTS(SELECT 1 FROM orders WHERE coupon_code = $1)
              OR EXISTS(SELECT 1 FROM coupon_redemptions WHERE code = $1)",
    )
    .bind(code)
    .fetch_one(executor)
    .await
}

/// How many members hold this coupon unused — the
/// deactivation confirm's warning count (§5.2).
pub async fn unredeemed_holders(pool: &PgPool, code: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM user_coupons WHERE code = $1 AND used_at IS NULL AND voided_at IS NULL")
        .bind(code)
        .fetch_one(pool)
        .await
}

/// Inserts one definition inside the caller's
/// transaction and returns its id.
pub async fn insert_definition(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    data: &crate::models::CouponDefinitionWrite,
    active: bool,
) -> Result<i64, sqlx::Error> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO coupons (code, discount_type, discount_yen, percent,
                              scope, product_id, variant_id,
                              max_uses_total, max_uses_per_customer, is_automatic,
                              min_subtotal_yen, description,
                              active, starts_at, ends_at, band_featured)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) RETURNING id",
    )
    .bind(&data.code)
    .bind(&data.discount_type)
    .bind(data.discount_yen)
    .bind(data.percent)
    .bind(&data.scope)
    .bind(data.product_id)
    .bind(data.variant_id)
    .bind(data.max_uses_total)
    .bind(data.max_uses_per_customer)
    .bind(data.is_automatic)
    .bind(data.min_subtotal_yen)
    .bind(&data.description)
    .bind(active)
    .bind(data.starts_at)
    .bind(data.ends_at)
    .bind(data.band_featured)
    .fetch_one(&mut **tx)
    .await?;
    Ok(id)
}

/// Updates a definition's editable fields inside the
/// caller's transaction. The active flag is NOT written
/// here — activation is its own audited action.
pub async fn update_definition(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    data: &crate::models::CouponDefinitionWrite,
    admin_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE coupons SET code = $1, discount_type = $2, discount_yen = $3,
                percent = $4, scope = $5, product_id = $6, variant_id = $7,
                max_uses_total = $8, max_uses_per_customer = $9, is_automatic = $10,
                min_subtotal_yen = $11, description = $12, starts_at = $13,
                ends_at = $14, band_featured = $15, updated_by = $17
         WHERE id = $16",
    )
    .bind(&data.code)
    .bind(&data.discount_type)
    .bind(data.discount_yen)
    .bind(data.percent)
    .bind(&data.scope)
    .bind(data.product_id)
    .bind(data.variant_id)
    .bind(data.max_uses_total)
    .bind(data.max_uses_per_customer)
    .bind(data.is_automatic)
    .bind(data.min_subtotal_yen)
    .bind(&data.description)
    .bind(data.starts_at)
    .bind(data.ends_at)
    .bind(data.band_featured)
    .bind(id)
    .bind(admin_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Moves the band flag: clears it everywhere inside
/// the caller's transaction (the caller sets it on the
/// chosen definition in the same transaction, so at
/// most one row ever carries it).
pub async fn clear_featured(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE coupons SET band_featured = false, updated_by = $1 WHERE band_featured")
        .bind(admin_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Sets the active flag inside the caller's transaction.
pub async fn set_active(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    active: bool,
    admin_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE coupons SET active = $1, updated_by = $3 WHERE id = $2")
        .bind(active)
        .bind(id)
        .bind(admin_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Returns every product as a form option (id, name),
/// catalogue order: the coupon form's scope picker.
pub async fn product_options(pool: &PgPool) -> Result<Vec<(i64, String)>, sqlx::Error> {
    sqlx::query_as("SELECT id, name FROM products ORDER BY id")
        .fetch_all(pool)
        .await
}

/// Returns every variant as a form option (id, product name
/// with the option label, parent product id): the coupon
/// form's variant picker, filtered by product in the browser.
pub async fn variant_options(pool: &PgPool) -> Result<Vec<(i64, String, i64)>, sqlx::Error> {
    let sql = format!(
        "SELECT v.id, p.name || CASE WHEN {label} = '' THEN '' ELSE '（' || {label} || '）' END, \
         v.product_id FROM product_variants v \
         JOIN products p ON p.id = v.product_id ORDER BY p.id, v.id",
        label = crate::repositories::variant_repository::LABEL_SQL
    );
    sqlx::query_as(&sql).fetch_all(pool).await
}

/// Whether a product row exists (scope validation).
pub async fn product_exists(pool: &PgPool, id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM products WHERE id = $1)")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// Whether a variant row exists (scope validation).
pub async fn variant_exists(pool: &PgPool, id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM product_variants WHERE id = $1)")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// Voids a member's unused coupon grants at withdrawal
/// (Account Deletion spec D6): rows are marked, never
/// deleted, so the grant history stays auditable;
/// redemption rows are untouched.
pub async fn void_unused_for_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE user_coupons SET voided_at = now() \
         WHERE user_id = $1 AND used_at IS NULL AND voided_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map(|_| ())
}
