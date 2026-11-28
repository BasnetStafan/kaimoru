//! Cart repository.
//!
//! A cart row belongs to exactly one owner: a member (`user_id`)
//! or a guest session (`session_id`). Every function takes the
//! owner as separate optional values so a query can never mix
//! the two up.
//!
//! Lines key on the variant (combination), not the product: the
//! same product in two combinations is two lines, and the stock
//! a line caps against is its variant's.

use sqlx::PgPool;

use crate::models::CartLine;
use crate::repositories::variant_repository::LABEL_SQL;

const LINE_COLUMNS: &str = "c.variant_id, v.product_id, p.name, __LABEL__ AS variant_label, \
                            COALESCE(v.price_yen, __EFFECTIVE__) AS unit_price_yen, \
                            v.stock, p.image_path, p.status, c.quantity, \
                            v.is_enabled AS variant_enabled, \
                            ((p.publish_starts_at IS NULL OR p.publish_starts_at <= now()) \
                              AND (p.publish_ends_at IS NULL OR p.publish_ends_at > now())) AS publish_open, \
                            p.track_stock, p.allow_backorder, p.free_shipping, \
                            tc.code AS tax_class_code, tc.name AS tax_class_name, \
                            COALESCE((SELECT tr.rate_percent FROM tax_rates tr \
                              WHERE tr.tax_class_id = p.tax_class_id \
                                AND tr.effective_from <= (now() AT TIME ZONE 'Asia/Tokyo')::date \
                                AND (tr.effective_to IS NULL OR tr.effective_to >= (now() AT TIME ZONE 'Asia/Tokyo')::date) \
                              ORDER BY tr.effective_from DESC LIMIT 1), 0) AS tax_rate_percent, \
                            p.undeliverable_regions, c.added_by_agent";

/// The line column list with the shared label fragment and the
/// effective-price expression inlined: a variant's own price
/// wins, else the product's effective (sale-aware) price — the
/// SQL twin of the pricing module's one rule.
fn line_columns() -> String {
    LINE_COLUMNS
        .replace("__LABEL__", LABEL_SQL)
        .replace("__EFFECTIVE__", crate::repositories::EFFECTIVE_PRICE_SQL)
}

/// Adds `qty` units of a variant to the owner's cart, in one upsert.
///
/// The stock cap is enforced inside the statement
/// (`LEAST(..., $stock_cap)`): the service's earlier read is only
/// advisory, so two concurrent adds cannot race past the cap.
///
/// `added_by_agent` is written on insert only: the conflict
/// path grows the quantity but never moves an existing line's
/// provenance — a line changes hands only by being removed
/// and re-added, which is what an agent run does with its own.
pub async fn add(
    pool: &PgPool,
    user_id: Option<i64>,
    session_id: Option<&str>,
    variant_id: i64,
    qty: i32,
    stock_cap: i32,
    added_by_agent: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO cart_items (user_id, session_id, variant_id, quantity, added_by_agent) \
         VALUES ($1, $2, $3, LEAST($4, $5), $6) \
         ON CONFLICT (user_id, variant_id) WHERE user_id IS NOT NULL \
         DO UPDATE SET quantity = LEAST(cart_items.quantity + EXCLUDED.quantity, $5), \
             updated_at = now()",
    )
    .bind(user_id)
    .bind(session_id)
    .bind(variant_id)
    .bind(qty)
    .bind(stock_cap)
    .bind(added_by_agent)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Guest-session variant of the add upsert (the partial unique
/// index differs per owner kind, so each gets its own statement).
pub async fn add_for_session(
    pool: &PgPool,
    session_id: &str,
    variant_id: i64,
    qty: i32,
    stock_cap: i32,
    added_by_agent: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO cart_items (session_id, variant_id, quantity, added_by_agent) \
         VALUES ($1, $2, LEAST($3, $4), $5) \
         ON CONFLICT (session_id, variant_id) WHERE session_id IS NOT NULL \
         DO UPDATE SET quantity = LEAST(cart_items.quantity + EXCLUDED.quantity, $4), \
             updated_at = now()",
    )
    .bind(session_id)
    .bind(variant_id)
    .bind(qty)
    .bind(stock_cap)
    .bind(added_by_agent)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Lists the owner's cart lines joined with current product and
/// variant data, oldest additions first.
/// The cart-line SELECT shared by [`list_lines`] and
/// [`list_lines_in_tx`]: one definition, two executors.
fn lines_select() -> String {
    format!(
        "SELECT {} FROM cart_items c \
         JOIN product_variants v ON v.id = c.variant_id \
         JOIN products p ON p.id = v.product_id \
         JOIN tax_classes tc ON tc.id = p.tax_class_id \
         WHERE c.user_id IS NOT DISTINCT FROM $1 \
           AND c.session_id IS NOT DISTINCT FROM $2 \
         ORDER BY c.created_at, c.id",
        line_columns()
    )
}

pub async fn list_lines(
    pool: &PgPool,
    user_id: Option<i64>,
    session_id: Option<&str>,
) -> Result<Vec<CartLine>, sqlx::Error> {
    sqlx::query_as::<_, CartLine>(&lines_select())
        .bind(user_id)
        .bind(session_id)
        .fetch_all(pool)
        .await
}

/// Locks the owner's cart rows inside `tx` until it ends.
///
/// Order placement takes this lock before re-reading the
/// lines, so two placements racing the same cart — a double
/// submit, two tabs, or the storefront checkout and the
/// agent settling at the same moment — cannot both place an
/// order from the same lines: the second waits here, then
/// reads the cleared cart and stops.
pub async fn lock_owner_lines(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Option<i64>,
    session_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT id FROM cart_items \
         WHERE user_id IS NOT DISTINCT FROM $1 \
           AND session_id IS NOT DISTINCT FROM $2 \
         FOR UPDATE",
    )
    .bind(user_id)
    .bind(session_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The owner's cart lines, read inside `tx` — the
/// authoritative set when the caller holds the row locks
/// from [`lock_owner_lines`].
pub async fn list_lines_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Option<i64>,
    session_id: Option<&str>,
) -> Result<Vec<CartLine>, sqlx::Error> {
    sqlx::query_as::<_, CartLine>(&lines_select())
        .bind(user_id)
        .bind(session_id)
        .fetch_all(&mut **tx)
        .await
}

/// The quantity of one variant in the owner's cart (0 when absent).
pub async fn quantity_of(
    pool: &PgPool,
    user_id: Option<i64>,
    session_id: Option<&str>,
    variant_id: i64,
) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT quantity FROM cart_items \
         WHERE user_id IS NOT DISTINCT FROM $1 \
           AND session_id IS NOT DISTINCT FROM $2 AND variant_id = $3",
    )
    .bind(user_id)
    .bind(session_id)
    .bind(variant_id)
    .fetch_optional(pool)
    .await
    .map(|row| row.unwrap_or(0))
}

/// Total item count for the header badge: SUM(quantity).
pub async fn total_quantity(
    pool: &PgPool,
    user_id: Option<i64>,
    session_id: Option<&str>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(quantity), 0) FROM cart_items \
         WHERE user_id IS NOT DISTINCT FROM $1 \
           AND session_id IS NOT DISTINCT FROM $2",
    )
    .bind(user_id)
    .bind(session_id)
    .fetch_one(pool)
    .await
}

/// Sets one line's quantity outright (the cart page stepper).
pub async fn set_quantity(
    pool: &PgPool,
    user_id: Option<i64>,
    session_id: Option<&str>,
    variant_id: i64,
    qty: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE cart_items SET quantity = $4, updated_at = now() \
         WHERE user_id IS NOT DISTINCT FROM $1 \
           AND session_id IS NOT DISTINCT FROM $2 AND variant_id = $3",
    )
    .bind(user_id)
    .bind(session_id)
    .bind(variant_id)
    .bind(qty)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Removes one line from the owner's cart.
pub async fn remove_line(
    pool: &PgPool,
    user_id: Option<i64>,
    session_id: Option<&str>,
    variant_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM cart_items \
         WHERE user_id IS NOT DISTINCT FROM $1 \
           AND session_id IS NOT DISTINCT FROM $2 AND variant_id = $3",
    )
    .bind(user_id)
    .bind(session_id)
    .bind(variant_id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Removes only the given variants from the owner's cart.
///
/// Placement clears exactly the lines it ordered, so an item added
/// in another tab during checkout is never deleted unordered.
/// Runs on the caller's executor, inside the order transaction.
pub async fn clear_lines<'e, E>(
    executor: E,
    user_id: Option<i64>,
    session_id: Option<&str>,
    variant_ids: &[i64],
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "DELETE FROM cart_items \
         WHERE user_id IS NOT DISTINCT FROM $1 \
           AND session_id IS NOT DISTINCT FROM $2 \
           AND variant_id = ANY($3)",
    )
    .bind(user_id)
    .bind(session_id)
    .bind(variant_ids)
    .execute(executor)
    .await
    .map(|_| ())
}

/// Moves a guest session's cart to a member, inside the caller's
/// transaction: overlapping variants merge into the member's line
/// (capped later by stock rules at checkout), guest-only variants
/// change owner, and the guest rows disappear.
///
/// Provenance merges conservatively: a merged line stays the
/// agent's only when both halves were the agent's, so a quantity
/// the owner added by hand is never inside a line a run may
/// replace. Guest-only rows keep their flag as they change owner.
pub async fn rekey_session_to_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    session_id: &str,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    // Overlapping variants: add the guest quantity to the member line.
    sqlx::query(
        "UPDATE cart_items m SET quantity = m.quantity + g.quantity, updated_at = now(), \
             added_by_agent = m.added_by_agent AND g.added_by_agent \
         FROM cart_items g \
         WHERE m.user_id = $2 AND g.session_id = $1 AND m.variant_id = g.variant_id",
    )
    .bind(session_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    // Guest-only variants: hand the rows to the member.
    sqlx::query(
        "UPDATE cart_items SET user_id = $2, session_id = NULL, updated_at = now() \
         WHERE session_id = $1 AND variant_id NOT IN \
           (SELECT variant_id FROM cart_items WHERE user_id = $2)",
    )
    .bind(session_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    // Anything left under the session was merged above; delete it.
    sqlx::query("DELETE FROM cart_items WHERE session_id = $1")
        .bind(session_id)
        .execute(&mut **tx)
        .await
        .map(|_| ())
}

/// Moves one session cart onto another session token — the
/// session-shaped sibling of [`rekey_session_to_user`], used
/// when an MCP cart is claimed by a guest visitor. Same
/// three-step merge: overlapping variants sum their
/// quantities (provenance AND-ed), source-only variants
/// change owner, and the source token is left with nothing.
pub async fn rekey_session_to_session(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    from_session_id: &str,
    to_session_id: &str,
) -> Result<(), sqlx::Error> {
    // Overlapping variants: add the source quantity to the target line.
    sqlx::query(
        "UPDATE cart_items t SET quantity = t.quantity + s.quantity, updated_at = now(), \
             added_by_agent = t.added_by_agent AND s.added_by_agent \
         FROM cart_items s \
         WHERE t.session_id = $2 AND s.session_id = $1 AND t.variant_id = s.variant_id",
    )
    .bind(from_session_id)
    .bind(to_session_id)
    .execute(&mut **tx)
    .await?;
    // Source-only variants: hand the rows to the target session.
    sqlx::query(
        "UPDATE cart_items SET session_id = $2, updated_at = now() \
         WHERE session_id = $1 AND variant_id NOT IN \
           (SELECT variant_id FROM cart_items WHERE session_id = $2)",
    )
    .bind(from_session_id)
    .bind(to_session_id)
    .execute(&mut **tx)
    .await?;
    // Anything left under the source was merged above; delete it.
    sqlx::query("DELETE FROM cart_items WHERE session_id = $1")
        .bind(from_session_id)
        .execute(&mut **tx)
        .await
        .map(|_| ())
}

/// Deletes every cart row of one member (account
/// withdrawal): the cart has no business meaning once
/// the identity is gone (Account Deletion spec 3.4).
pub async fn delete_all_for_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM cart_items WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map(|_| ())
}

/// Removes every agent-added line of the owner's cart
/// a run replaces exactly its own lines and can
/// never touch a line the owner added by hand.
pub async fn delete_agent_lines(
    pool: &PgPool,
    user_id: Option<i64>,
    session_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM cart_items \
         WHERE user_id IS NOT DISTINCT FROM $1 \
           AND session_id IS NOT DISTINCT FROM $2 AND added_by_agent",
    )
    .bind(user_id)
    .bind(session_id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Builds the cart line a variant would produce at `qty`,
/// without the variant being in any cart: the same joined
/// columns as [`list_lines`], resolved live, so a projection
/// over preview lines prices exactly like the real cart. The
/// preview's provenance is the owner's own (false): nothing
/// has been added yet.
pub async fn line_for_variant(
    pool: &PgPool,
    variant_id: i64,
    qty: i32,
) -> Result<Option<CartLine>, sqlx::Error> {
    let columns = line_columns()
        .replace("c.variant_id", "v.id AS variant_id")
        .replace("c.quantity", "$2 AS quantity")
        .replace("c.added_by_agent", "false AS added_by_agent");
    sqlx::query_as::<_, CartLine>(&format!(
        "SELECT {} FROM product_variants v \
         JOIN products p ON p.id = v.product_id \
         JOIN tax_classes tc ON tc.id = p.tax_class_id \
         WHERE v.id = $1",
        columns
    ))
    .bind(variant_id)
    .bind(qty)
    .fetch_optional(pool)
    .await
}

/// Deletes every guest cart whose newest line is older than
/// the retention window, and returns how many lines were
/// removed. The unit is the cart (one session), never the
/// line: a cart with any recently touched line survives
/// whole, so a returning guest never finds a half-emptied
/// cart. Member carts are out of scope by construction —
/// their rows carry a user id, not a session id.
pub async fn delete_abandoned_guest_carts(
    pool: &PgPool,
    retention_days: i64,
) -> Result<u64, sqlx::Error> {
    // Batched: pick up to 500 doomed
    // carts first, then delete exactly those. The old
    // single unbounded DELETE could take one giant
    // transaction after a long downtime; the sweep runs on
    // an interval, so a backlog drains over passes.
    let doomed: Vec<String> = sqlx::query_scalar(
        "SELECT session_id FROM cart_items \
         WHERE session_id IS NOT NULL \
         GROUP BY session_id \
         HAVING MAX(updated_at) < now() - make_interval(days => $1) \
         LIMIT 500",
    )
    .bind(retention_days as i32)
    .fetch_all(pool)
    .await?;
    if doomed.is_empty() {
        return Ok(0);
    }
    let result = sqlx::query("DELETE FROM cart_items WHERE session_id = ANY($1)")
        .bind(&doomed)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
