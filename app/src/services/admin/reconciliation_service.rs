//! Admin payment reconciliation: the read-only screen
//! version of `scripts/ops/reconcile_payments.sh`.
//!
//! The same three questions, the same queries' logic: what
//! is still open, what was actually captured per method net
//! of refunds, and what is flagged for a human. Nothing here
//! writes; money movement stays in the order/payment flows.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;

/// One open (unsettled) KOMOJU payment.
pub struct OpenPayment {
    /// Payment row id.
    pub id: i64,
    /// The order it belongs to.
    pub order_id: i64,
    /// Method code (`credit_card` etc.).
    pub method: String,
    /// Payment status code.
    pub status: String,
    /// Expected amount in yen.
    pub amount_yen: i64,
    /// When the payment row was created.
    pub created_at: DateTime<Utc>,
    /// The payment deadline, when the provider set one.
    pub payment_deadline: Option<DateTime<Utc>>,
    /// The order's creation time (its number derives from it).
    pub order_created_at: DateTime<Utc>,
}

/// One method's captured totals.
pub struct CapturedTotal {
    /// Method code.
    pub method: String,
    /// Number of captured payments.
    pub count: i64,
    /// Net captured yen (amount minus refunds).
    pub net_yen: i64,
}

/// One flagged payment.
pub struct FlaggedPayment {
    /// Payment row id.
    pub id: i64,
    /// The order it belongs to.
    pub order_id: i64,
    /// Payment status code.
    pub status: String,
    /// The operator-facing flag note.
    pub flag_note: String,
    /// When the row last changed.
    pub updated_at: DateTime<Utc>,
    /// The order's creation time (its number derives from it).
    pub order_created_at: DateTime<Utc>,
}

/// Loads the three reconciliation lists.
///
/// # Errors
///
/// Returns [`AppError`] when a query fails: the screen shows
/// real figures or an error, never a partial picture.
pub async fn load(
    pool: &PgPool,
) -> Result<(Vec<OpenPayment>, Vec<CapturedTotal>, Vec<FlaggedPayment>), AppError> {
    let open = sqlx::query_as::<
        _,
        (
            i64,
            i64,
            String,
            String,
            i64,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
            DateTime<Utc>,
        ),
    >(
        "SELECT p.id, p.order_id, p.method, p.status, p.amount_yen, p.created_at, \
                p.payment_deadline, o.created_at \
         FROM payments p JOIN orders o ON o.id = p.order_id \
         WHERE p.provider = 'komoju' AND p.status IN ('pending_payment', 'failed') \
         ORDER BY p.created_at",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("open payments", e))?
    .into_iter()
    .map(
        |(
            id,
            order_id,
            method,
            status,
            amount_yen,
            created_at,
            payment_deadline,
            order_created_at,
        )| OpenPayment {
            id,
            order_id,
            method,
            status,
            amount_yen,
            created_at,
            payment_deadline,
            order_created_at,
        },
    )
    .collect();
    let captured = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT method, count(*)::bigint, \
                sum(amount_yen - amount_refunded_yen)::bigint \
         FROM payments \
         WHERE provider = 'komoju' AND status IN ('paid', 'partially_refunded') \
         GROUP BY method ORDER BY method",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("captured totals", e))?
    .into_iter()
    .map(|(method, count, net_yen)| CapturedTotal {
        method,
        count,
        net_yen,
    })
    .collect();
    let flagged = sqlx::query_as::<_, (i64, i64, String, String, DateTime<Utc>, DateTime<Utc>)>(
        "SELECT p.id, p.order_id, p.status, p.flag_note, p.updated_at, o.created_at \
         FROM payments p JOIN orders o ON o.id = p.order_id \
         WHERE p.flag_note IS NOT NULL ORDER BY p.updated_at",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("flagged payments", e))?
    .into_iter()
    .map(
        |(id, order_id, status, flag_note, updated_at, order_created_at)| FlaggedPayment {
            id,
            order_id,
            status,
            flag_note,
            updated_at,
            order_created_at,
        },
    )
    .collect();
    Ok((open, captured, flagged))
}
