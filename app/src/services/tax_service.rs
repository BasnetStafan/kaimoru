//! Tax service: resolves dated rates for the money paths.
//!
//! Rates are resolved by date — today for display and cart
//! totals, the order's own JST date at placement — so a rate
//! registered for a future date changes nothing until its day.

use chrono::NaiveDate;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::tax::ClassRate;
use crate::models::tax::TaxRounding;
use crate::repositories::tax_repository;

/// Everything `cart_service::totals` needs to decompose a total:
/// the standard class's rate today and the shop rounding mode.
/// Shipping and the COD fee are standard-class amounts, so the
/// standard rate is needed even for a cart of reduced-rate goods.
#[derive(Clone, Copy, Debug)]
pub struct TaxParams {
    /// The standard class's rate in whole percent.
    pub standard_rate_percent: i32,
    /// The shop's rounding mode.
    pub rounding: TaxRounding,
}

impl TaxParams {
    /// The parameters when no database is at hand (unit tests):
    /// today's statutory standard rate, floor rounding.
    pub fn fallback() -> Self {
        Self {
            standard_rate_percent: 10,
            rounding: TaxRounding::Floor,
        }
    }
}

/// Today's JST date: the date rates are resolved against for
/// display-side totals.
pub fn jst_today() -> NaiveDate {
    crate::models::jst::jst_today()
}

/// Cache of today's parameters: the totals
/// paths ask for them on nearly every cart and checkout
/// render, while the underlying rates change only when an
/// admin registers a new one. Keyed by the JST date so a
/// rate taking effect tomorrow is never served early;
/// entries live a minute, and the admin rate save
/// invalidates the cache so a new rate applies at once.
static PARAMS_CACHE: std::sync::Mutex<Option<(NaiveDate, std::time::Instant, TaxParams)>> =
    std::sync::Mutex::new(None);

/// Drops the cached parameters; the admin rate save calls
/// this so a registered rate applies immediately.
pub fn invalidate_params_cache() {
    if let Ok(mut cache) = PARAMS_CACHE.lock() {
        *cache = None;
    }
}

/// Loads the tax parameters for display-side totals.
pub async fn params_today(pool: &PgPool) -> Result<TaxParams, AppError> {
    let today = jst_today();
    if let Ok(cache) = PARAMS_CACHE.lock()
        && let Some((date, fetched_at, params)) = *cache
        && date == today
        && fetched_at.elapsed() < std::time::Duration::from_secs(60)
    {
        return Ok(params);
    }
    let rates = class_rates_on(pool, today).await?;
    let standard = rates
        .iter()
        .find(|r| r.class.code == "standard")
        .map(|r| r.rate_percent)
        .unwrap_or(10);
    let params = TaxParams {
        standard_rate_percent: standard,
        rounding: TaxRounding::configured(),
    };
    if let Ok(mut cache) = PARAMS_CACHE.lock() {
        *cache = Some((today, std::time::Instant::now(), params));
    }
    Ok(params)
}

/// [`params_today`], degrading to the fallback parameters on
/// failure — with the failure logged here, once, so no caller
/// ever prices silently on fallback parameters. Callers that
/// may degrade (cart, checkout drafts, agent proposals) use
/// this; the error itself is never swallowed.
pub async fn params_today_or_fallback(pool: &PgPool) -> TaxParams {
    params_today(pool).await.unwrap_or_else(|e| {
        tracing::error!("tax params load failed; using fallback parameters: {e}");
        TaxParams::fallback()
    })
}

/// Every class with its rate in force on `date`.
pub async fn class_rates_on(pool: &PgPool, date: NaiveDate) -> Result<Vec<ClassRate>, AppError> {
    Ok(tax_repository::class_rates_on(pool, date).await?)
}

/// The rate of one class in force on `date`, in whole percent.
pub async fn rate_for_class_on(
    pool: &PgPool,
    tax_class_id: i64,
    date: NaiveDate,
) -> Result<i32, AppError> {
    Ok(tax_repository::rate_for_class_on(pool, tax_class_id, date).await?)
}
