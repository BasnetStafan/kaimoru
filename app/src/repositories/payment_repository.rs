//! Payment repository.
//!
//! Every query against `payments` and `webhook_events` lives
//! here, bound parameters throughout. Writes that belong to a
//! larger transaction (placement, status application) take the
//! caller's executor; reads take the pool.

use sqlx::PgPool;

use crate::models::{Payment, WebhookEvent};

/// Every column of the payments table, named explicitly.
const PAYMENT_COLUMNS: &str = "id, order_id, provider, method, status, amount_yen, \
    amount_refunded_yen, currency, komoju_session_id, komoju_payment_id, \
    stripe_session_id, stripe_payment_id, \
    external_order_num, payment_deadline, instructions_url, payment_details, \
    flag_note, captured_at, confirmation_mail_sent_at, compensation_pending, \
    created_at, updated_at";

/// The fields of a new payment row, computed by the services
/// from the placed order (never from the request directly).
pub struct NewPayment {
    /// The order this payment settles.
    pub order_id: i64,
    /// `komoju` or `cod`.
    pub provider: &'static str,
    /// Method slug.
    pub method: &'static str,
    /// Initial status.
    pub status: &'static str,
    /// The order total in yen.
    pub amount_yen: i64,
    /// Whether the confirmation mail is already claimed (COD:
    /// the checkout controller sends it at placement, as always).
    pub confirmation_mail_claimed: bool,
}

/// Inserts the payment row for an order and returns its ID.
///
/// The UNIQUE(order_id) constraint is the double-placement
/// backstop: a second insert for the same order fails rather
/// than creating a second payment.
pub async fn insert_payment<'e, E>(executor: E, payment: &NewPayment) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_scalar(
        "INSERT INTO payments (order_id, provider, method, status, amount_yen, \
         confirmation_mail_sent_at) \
         VALUES ($1, $2, $3, $4, $5, CASE WHEN $6 THEN now() END) \
         RETURNING id",
    )
    .bind(payment.order_id)
    .bind(payment.provider)
    .bind(payment.method)
    .bind(payment.status)
    .bind(payment.amount_yen)
    .bind(payment.confirmation_mail_claimed)
    .fetch_one(executor)
    .await
}

/// Loads one payment by ID.
pub async fn find_by_id(pool: &PgPool, payment_id: i64) -> Result<Option<Payment>, sqlx::Error> {
    sqlx::query_as::<_, Payment>(&format!(
        "SELECT {PAYMENT_COLUMNS} FROM payments WHERE id = $1"
    ))
    .bind(payment_id)
    .fetch_optional(pool)
    .await
}

/// Loads the payment of one order, when it has one. Orders
/// placed before payments existed have no row; that absence
/// reads as cash on delivery everywhere.
pub async fn find_by_order_id<'e, E>(
    executor: E,
    order_id: i64,
) -> Result<Option<Payment>, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_as::<_, Payment>(&format!(
        "SELECT {PAYMENT_COLUMNS} FROM payments WHERE order_id = $1"
    ))
    .bind(order_id)
    .fetch_optional(executor)
    .await
}

/// The payments of many orders in one query (one payment
/// per order, so the caller keys them by `order_id`).
pub async fn find_by_order_ids(
    pool: &PgPool,
    order_ids: &[i64],
) -> Result<Vec<Payment>, sqlx::Error> {
    sqlx::query_as::<_, Payment>(&format!(
        "SELECT {PAYMENT_COLUMNS} FROM payments WHERE order_id = ANY($1)"
    ))
    .bind(order_ids)
    .fetch_all(pool)
    .await
}

/// Loads a payment by its KOMOJU session ID — the return
/// handler's correlation key.
pub async fn find_by_session_id(
    pool: &PgPool,
    session_id: &str,
) -> Result<Option<Payment>, sqlx::Error> {
    sqlx::query_as::<_, Payment>(&format!(
        "SELECT {PAYMENT_COLUMNS} FROM payments WHERE komoju_session_id = $1 OR stripe_session_id = $1"
    ))
    .bind(session_id)
    .fetch_optional(pool)
    .await
}

/// Loads a payment by its gateway payment ID — the
/// webhook's primary correlation key. Every provider's ID
/// column is searched: the IDs are provider-shaped, so a
/// value can only ever match its own provider's column.
pub async fn find_by_provider_payment_id(
    pool: &PgPool,
    provider_payment_id: &str,
) -> Result<Option<Payment>, sqlx::Error> {
    sqlx::query_as::<_, Payment>(&format!(
        "SELECT {PAYMENT_COLUMNS} FROM payments WHERE komoju_payment_id = $1 OR stripe_payment_id = $1"
    ))
    .bind(provider_payment_id)
    .fetch_optional(pool)
    .await
}

/// Loads a payment by the external order number actually sent
/// (retries carry suffixed values, and the row stores the value
/// used) — the webhook's fallback correlation.
pub async fn find_by_external_order_num(
    pool: &PgPool,
    external_order_num: &str,
) -> Result<Option<Payment>, sqlx::Error> {
    sqlx::query_as::<_, Payment>(&format!(
        "SELECT {PAYMENT_COLUMNS} FROM payments WHERE external_order_num = $1"
    ))
    .bind(external_order_num)
    .fetch_optional(pool)
    .await
}

/// Locks one payment row inside the caller's transaction.
///
/// Status application always locks the order first and the
/// payment second — one lock order everywhere, so the webhook,
/// the return handler, the sweep and the back office cannot
/// deadlock each other over the same money.
pub async fn lock_by_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    payment_id: i64,
) -> Result<Option<Payment>, sqlx::Error> {
    sqlx::query_as::<_, Payment>(&format!(
        "SELECT {PAYMENT_COLUMNS} FROM payments WHERE id = $1 FOR UPDATE"
    ))
    .bind(payment_id)
    .fetch_optional(&mut **tx)
    .await
}

/// Records the session a creation attempt produced: its ID and
/// the external order number actually sent (suffixed on
/// retries), inside the caller's transaction.
pub async fn set_session<'e, E>(
    executor: E,
    payment_id: i64,
    session_id: &str,
    external_order_num: &str,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "UPDATE payments SET \
         komoju_session_id = CASE WHEN provider = 'komoju' THEN $2 ELSE komoju_session_id END, \
         stripe_session_id = CASE WHEN provider = 'stripe' THEN $2 ELSE stripe_session_id END, \
         external_order_num = $3, updated_at = now() WHERE id = $1",
    )
    .bind(payment_id)
    .bind(session_id)
    .bind(external_order_num)
    .execute(executor)
    .await
    .map(|_| ())
}

/// The provider-truth fields one status application writes.
/// `None` leaves the stored value standing (COALESCE): a later
/// sighting that knows less never erases what an earlier one
/// recorded.
pub struct ProviderStateWrite {
    /// The new status.
    pub status: &'static str,
    /// The owning provider's payment ID, once known (written
    /// to that provider's own column by [`apply_state`]).
    pub provider_payment_id: Option<String>,
    /// Konbini deadline, once issued.
    pub payment_deadline: Option<chrono::DateTime<chrono::Utc>>,
    /// The provider's hosted instructions page, once issued.
    pub instructions_url: Option<String>,
    /// Provider payment details snapshot.
    pub payment_details: Option<serde_json::Value>,
    /// Capture instant, once captured.
    pub captured_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Cumulative refunded amount in yen (always written: it is
    /// a total, not a sighting).
    pub amount_refunded_yen: i64,
}

/// Applies one provider sighting to the payment row, inside the
/// caller's transaction (the caller holds the row lock).
pub async fn apply_state<'e, E>(
    executor: E,
    payment_id: i64,
    write: &ProviderStateWrite,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "UPDATE payments SET status = $2, \
         komoju_payment_id = CASE WHEN provider = 'komoju' THEN COALESCE($3, komoju_payment_id) ELSE komoju_payment_id END, \
         stripe_payment_id = CASE WHEN provider = 'stripe' THEN COALESCE($3, stripe_payment_id) ELSE stripe_payment_id END, \
         payment_deadline = COALESCE($4, payment_deadline), \
         instructions_url = COALESCE($5, instructions_url), \
         payment_details = COALESCE($6, payment_details), \
         captured_at = COALESCE($7, captured_at), \
         amount_refunded_yen = $8, updated_at = now() \
         WHERE id = $1",
    )
    .bind(payment_id)
    .bind(write.status)
    .bind(&write.provider_payment_id)
    .bind(write.payment_deadline)
    .bind(&write.instructions_url)
    .bind(&write.payment_details)
    .bind(write.captured_at)
    .bind(write.amount_refunded_yen)
    .execute(executor)
    .await
    .map(|_| ())
}

/// Sets only the status (the COD settlement flips and the
/// cancellation bookkeeping), inside the caller's transaction.
pub async fn set_status<'e, E>(
    executor: E,
    payment_id: i64,
    status: &str,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query("UPDATE payments SET status = $2, updated_at = now() WHERE id = $1")
        .bind(payment_id)
        .bind(status)
        .execute(executor)
        .await
        .map(|_| ())
}

/// Flags a payment for a human's attention, inside the caller's
/// transaction. The note is operator-facing only; it is never
/// rendered to customers.
pub async fn set_flag<'e, E>(executor: E, payment_id: i64, note: &str) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query("UPDATE payments SET flag_note = $2, updated_at = now() WHERE id = $1")
        .bind(payment_id)
        .bind(note)
        .execute(executor)
        .await
        .map(|_| ())
}

/// Claims the confirmation mail for this payment: returns true
/// for exactly one caller, ever. Whichever path (webhook,
/// return, sweep) wins the claim inside its apply transaction
/// sends the mail after commit; every other path sees the claim
/// already taken and sends nothing.
pub async fn claim_confirmation_mail<'e, E>(
    executor: E,
    payment_id: i64,
) -> Result<bool, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let result = sqlx::query(
        "UPDATE payments SET confirmation_mail_sent_at = now() \
         WHERE id = $1 AND confirmation_mail_sent_at IS NULL",
    )
    .bind(payment_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Sets or clears the pending-compensation marker of a
/// full refund (see `Payment::compensation_pending`),
/// inside the caller's transaction.
pub async fn set_compensation_pending<'e, E>(
    executor: E,
    payment_id: i64,
    pending: bool,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query("UPDATE payments SET compensation_pending = $2, updated_at = now() WHERE id = $1")
        .bind(payment_id)
        .bind(pending)
        .execute(executor)
        .await
        .map(|_| ())
}

/// Payments whose full-refund compensation is still
/// outstanding, oldest first — the sweep's retry list.
pub async fn list_compensation_pending(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<Payment>, sqlx::Error> {
    sqlx::query_as::<_, Payment>(&format!(
        "SELECT {PAYMENT_COLUMNS} FROM payments \
         WHERE compensation_pending ORDER BY id LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Records one webhook delivery, keyed by the event id in the
/// body. Returns false when the event was already recorded —
/// but recorded is not processed: a first delivery whose apply
/// step failed leaves the row unapplied, and its redelivery
/// must apply the event rather than count as a duplicate (see
/// `event_applied`). KOMOJU mints a new delivery id per
/// redelivery; the event id is the stable key.
pub async fn record_event<'e, E>(
    executor: E,
    event_id: &str,
    delivery_id: Option<&str>,
    event_type: &str,
    raw_body: &serde_json::Value,
) -> Result<bool, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let result = sqlx::query(
        "INSERT INTO webhook_events (event_id, delivery_id, event_type, raw_body) \
         VALUES ($1, $2, $3, $4) ON CONFLICT (event_id) DO NOTHING",
    )
    .bind(event_id)
    .bind(delivery_id)
    .bind(event_type)
    .bind(raw_body)
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Stamps a recorded event with its outcome: the payment it
/// resolved to and whether it changed a status. This stamp is
/// the ledger's only update; rows are never deleted.
pub async fn mark_event_outcome<'e, E>(
    executor: E,
    event_id: &str,
    payment_id: Option<i64>,
    applied: bool,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query("UPDATE webhook_events SET payment_id = $2, applied = $3 WHERE event_id = $1")
        .bind(event_id)
        .bind(payment_id)
        .bind(applied)
        .execute(executor)
        .await
        .map(|_| ())
}

/// Whether a recorded event has been applied to a payment.
///
/// A redelivery is a duplicate only once this is true: a first
/// delivery whose apply step failed leaves the row recorded
/// with `applied = false`, and its redelivery must apply the
/// event rather than be acknowledged as done.
pub async fn event_applied(pool: &PgPool, event_id: &str) -> Result<bool, sqlx::Error> {
    let applied =
        sqlx::query_scalar::<_, bool>("SELECT applied FROM webhook_events WHERE event_id = $1")
            .bind(event_id)
            .fetch_optional(pool)
            .await?;
    Ok(applied.unwrap_or(false))
}

/// The recorded events of one payment, oldest first — the
/// payment's own history on the admin order detail.
pub async fn events_for_payment(
    pool: &PgPool,
    payment_id: i64,
) -> Result<Vec<WebhookEvent>, sqlx::Error> {
    sqlx::query_as::<_, WebhookEvent>(
        "SELECT event_id, delivery_id, event_type, payment_id, received_at, applied \
         FROM webhook_events WHERE payment_id = $1 ORDER BY received_at, event_id",
    )
    .bind(payment_id)
    .fetch_all(pool)
    .await
}

/// Open payments of one provider (awaiting settlement or
/// resting on a failed attempt), oldest first — the sweep's
/// work list. Bounded per pass: the
/// sweep revisits on its interval, so a backlog drains
/// over successive passes instead of loading unboundedly.
pub async fn list_open_hosted(pool: &PgPool, provider: &str) -> Result<Vec<Payment>, sqlx::Error> {
    sqlx::query_as::<_, Payment>(&format!(
        "SELECT {PAYMENT_COLUMNS} FROM payments \
         WHERE provider = $1 AND status IN ('pending_payment', 'failed') \
         ORDER BY created_at, id LIMIT 200"
    ))
    .bind(provider)
    .fetch_all(pool)
    .await
}

/// How many hosted-gateway payments are awaiting payment —
/// the admin order list's 入金待ち chip count.
pub async fn count_pending_hosted(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM payments WHERE provider IN ('komoju', 'stripe') AND status = 'pending_payment'",
    )
    .fetch_one(pool)
    .await
}
