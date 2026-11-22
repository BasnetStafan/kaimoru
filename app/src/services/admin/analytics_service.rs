//! Admin analytics: period sales figures for the 売上分析
//! page.
//!
//! A period is a JST date span; the previous period is the
//! same-length span immediately before it (the comparison
//! every Japanese EC dashboard shows). Figures come straight
//! from the repository aggregates and a failed query
//! propagates: the page renders real numbers or a 500, never
//! invented zeros (the dashboard's doctrine).

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::repositories::admin::order_repository;

/// Ranking length: the top 10 of each ranking.
pub const RANKING_LIMIT: i64 = 10;

/// Which period the operator picked.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PeriodKind {
    /// The current JST month, month-start through today.
    ThisMonth,
    /// The full previous JST month.
    LastMonth,
    /// An operator-chosen inclusive date span.
    Custom,
}

/// A resolved period: the current span and its comparison
/// span, as JST dates (the span end is exclusive).
pub struct Period {
    /// Which kind produced this period.
    pub kind: PeriodKind,
    /// First day of the current span.
    pub from: NaiveDate,
    /// Day after the last day of the current span.
    pub to: NaiveDate,
    /// First day of the previous span.
    pub prev_from: NaiveDate,
    /// Day after the last day of the previous span (= `from`).
    pub prev_to: NaiveDate,
}

/// Resolves the requested period against JST today. A custom
/// span parses both dates and requires from <= to; anything
/// else falls back to this month (the caller echoes the raw
/// input, so a typo shows as the fallback, not a blank page).
pub fn resolve(kind: PeriodKind, from_raw: &str, to_raw: &str, today: NaiveDate) -> Period {
    let (from, to) = match kind {
        PeriodKind::ThisMonth => {
            let first = today.with_day(1).unwrap_or(today);
            (first, today + chrono::Duration::days(1))
        }
        PeriodKind::LastMonth => {
            let first_this = today.with_day(1).unwrap_or(today);
            let first_last = first_this - chrono::Duration::days(1);
            let first_last = first_last.with_day(1).unwrap_or(first_last);
            (first_last, first_this)
        }
        PeriodKind::Custom => {
            let parsed = (
                NaiveDate::parse_from_str(from_raw.trim(), "%Y-%m-%d"),
                NaiveDate::parse_from_str(to_raw.trim(), "%Y-%m-%d"),
            );
            match parsed {
                (Ok(f), Ok(t)) if f <= t => (f, t + chrono::Duration::days(1)),
                _ => return resolve(PeriodKind::ThisMonth, "", "", today),
            }
        }
    };
    let len = to - from;
    Period {
        kind,
        from,
        to,
        prev_from: from - len,
        prev_to: from,
    }
}

/// Converts a JST date's start-of-day to a UTC instant for
/// the repository bounds.
fn jst_start(date: NaiveDate) -> DateTime<Utc> {
    let jst = crate::models::jst::jst();
    date.and_hms_opt(0, 0, 0)
        .and_then(|naive| naive.and_local_timezone(jst).single())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default())
}

/// Everything the analytics page renders.
pub struct Analytics {
    /// Orders in the period (cancelled excluded).
    pub orders: i64,
    /// Sales in the period in yen (cancelled excluded).
    pub sales_yen: i64,
    /// Orders in the previous period.
    pub prev_orders: i64,
    /// Sales in the previous period in yen.
    pub prev_sales_yen: i64,
    /// Product ranking rows.
    pub products: Vec<order_repository::ProductSalesRow>,
    /// Category ranking rows.
    pub categories: Vec<order_repository::CategorySalesRow>,
    /// Members who ordered at least once in the period.
    pub member_buyers: i64,
    /// Of those, members who ordered at least twice.
    pub member_repeaters: i64,
}

/// Loads the whole page's figures for one period.
///
/// # Errors
///
/// Returns [`AppError`] when any aggregate fails.
pub async fn load(pool: &PgPool, period: &Period) -> Result<Analytics, AppError> {
    let (from, to) = (jst_start(period.from), jst_start(period.to));
    let (prev_from, prev_to) = (jst_start(period.prev_from), jst_start(period.prev_to));
    let (orders, sales_yen) = order_repository::period_totals(pool, from, to).await?;
    let (prev_orders, prev_sales_yen) =
        order_repository::period_totals(pool, prev_from, prev_to).await?;
    let products = order_repository::product_ranking(pool, from, to, RANKING_LIMIT).await?;
    let categories = order_repository::category_ranking(pool, from, to, RANKING_LIMIT).await?;
    let (member_buyers, member_repeaters) =
        order_repository::member_buyer_counts(pool, from, to).await?;
    Ok(Analytics {
        orders,
        sales_yen,
        prev_orders,
        prev_sales_yen,
        products,
        categories,
        member_buyers,
        member_repeaters,
    })
}
