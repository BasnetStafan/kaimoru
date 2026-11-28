//! RFM raw measures and band reads: the one aggregate every
//! RFM surface shares.
//!
//! A member's raw measures come from their counted orders —
//! every non-cancelled order placed on their account (guest
//! orders carry no member and are never scored), monetary
//! net of refunds (each order contributes its total minus
//! what its payments refunded, never below zero). Withdrawn
//! members are tombstones and stay out of the population, so
//! the scored population is exactly the members a segment
//! can match. Dates are JST calendar dates, the convention
//! the segment matcher already uses.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::error::AppError;

/// The counted-order aggregate, as a SELECT over one row per
/// scored member. Shared verbatim by the measures read and
/// the band CTE below, so the page and the matcher can never
/// aggregate differently.
const AGGREGATE: &str = "SELECT o.user_id AS user_id, \
        ((now() AT TIME ZONE 'Asia/Tokyo')::date \
            - (MAX(o.created_at) AT TIME ZONE 'Asia/Tokyo')::date) AS r_days, \
        COUNT(*)::bigint AS f_count, \
        COALESCE(SUM(GREATEST(o.total_yen - COALESCE(p.refunded, 0), 0)), 0)::bigint AS m_yen, \
        (MAX(o.created_at) AT TIME ZONE 'Asia/Tokyo')::date AS last_order_date \
    FROM orders o \
    JOIN users u ON u.id = o.user_id AND u.withdrawn_at IS NULL \
    LEFT JOIN (SELECT order_id, SUM(amount_refunded_yen) AS refunded \
               FROM payments GROUP BY order_id) p ON p.order_id = o.id \
    WHERE o.user_id IS NOT NULL AND o.status <> 'cancelled' \
    GROUP BY o.user_id";

/// Builds the band CTE: `rfm_bands(user_id, r_band,
/// f_band, m_band)` computed in SQL with exactly the
/// banding rule the service applies in Rust — band =
/// ceil(5 * k / n), where k counts the members at-or-below
/// the member's value (at-or-above for recency days, where
/// fewer is better), so equal values always share a band.
/// The segment matcher joins this CTE; the suite pins the
/// two implementations to each other.
pub fn bands_cte() -> String {
    format!(
        "WITH rfm_agg AS ({AGGREGATE}), \
         rfm_bands AS ( \
            SELECT a.user_id, \
                ((5 * (SELECT COUNT(*) FROM rfm_agg b WHERE b.r_days >= a.r_days) \
                    + (SELECT COUNT(*) FROM rfm_agg) - 1) \
                    / (SELECT COUNT(*) FROM rfm_agg))::int AS r_band, \
                ((5 * (SELECT COUNT(*) FROM rfm_agg b WHERE b.f_count <= a.f_count) \
                    + (SELECT COUNT(*) FROM rfm_agg) - 1) \
                    / (SELECT COUNT(*) FROM rfm_agg))::int AS f_band, \
                ((5 * (SELECT COUNT(*) FROM rfm_agg b WHERE b.m_yen <= a.m_yen) \
                    + (SELECT COUNT(*) FROM rfm_agg) - 1) \
                    / (SELECT COUNT(*) FROM rfm_agg))::int AS m_band \
            FROM rfm_agg a \
        )"
    )
}

/// One member's raw RFM measures plus the profile fields the
/// RFM page shows (still PII-encrypted at this layer; the
/// service decrypts, the customers-repository convention).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemberMeasures {
    /// The member.
    pub user_id: i64,
    /// Family name (ciphertext).
    pub family_name: String,
    /// Given name (ciphertext).
    pub given_name: String,
    /// Account email (ciphertext).
    pub email: String,
    /// Days since the newest counted order (JST dates).
    pub r_days: i32,
    /// Counted order count.
    pub f_count: i64,
    /// Net monetary total, integer yen.
    pub m_yen: i64,
    /// The newest counted order's JST date.
    pub last_order_date: chrono::NaiveDate,
}

/// Every scored member's raw measures, in no promised order
/// (the service bands and sorts).
pub async fn member_measures(pool: &PgPool) -> Result<Vec<MemberMeasures>, AppError> {
    let sql = format!(
        "SELECT a.user_id, u.family_name, u.given_name, u.email, \
                a.r_days, a.f_count, a.m_yen, a.last_order_date \
         FROM ({AGGREGATE}) a JOIN users u ON u.id = a.user_id"
    );
    sqlx::query_as::<_, MemberMeasures>(&sql)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("rfm member measures", e))
}

/// Every scored member's bands by user id — the customers
/// list's read (its rows are paged over all members, scored
/// or not, so the bands arrive as a map to look up).
pub async fn band_map(pool: &PgPool) -> Result<HashMap<i64, (i32, i32, i32)>, AppError> {
    let sql = format!(
        "{} SELECT user_id, r_band, f_band, m_band FROM rfm_bands",
        bands_cte()
    );
    let rows = sqlx::query_as::<_, (i64, i32, i32, i32)>(&sql)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("rfm band map", e))?;
    Ok(rows
        .into_iter()
        .map(|(id, r, f, m)| (id, (r, f, m)))
        .collect())
}
