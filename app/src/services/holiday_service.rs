//! Shop holidays, storefront read side.
//!
//! The delivery promise reads the holiday set on hot paths
//! (product page, cart, checkout), so — like the shop
//! settings — reads go through a short in-process cache that
//! an admin save invalidates. Only dates from today onward
//! are held: the delivery math never looks back.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use sqlx::PgPool;

use crate::repositories::holiday_repository;

/// How long the holiday set may be served from memory.
const HOLIDAYS_TTL: Duration = Duration::from_secs(60);

/// The cached holiday dates and when they were read.
static HOLIDAYS_CACHE: Mutex<Option<(Instant, Vec<NaiveDate>)>> = Mutex::new(None);

/// Returns the registered holidays from today (JST) onward.
///
/// A failed read degrades to "no holidays" — the pre-feature
/// calendar — rather than failing the page: a holiday is a
/// refinement of the promise, never a dependency of it.
pub async fn upcoming(pool: &PgPool) -> Vec<NaiveDate> {
    if let Ok(cache) = HOLIDAYS_CACHE.lock()
        && let Some((read_at, dates)) = cache.as_ref()
        && read_at.elapsed() < HOLIDAYS_TTL
    {
        return dates.clone();
    }
    let today = crate::models::jst::jst_today();
    let dates = match holiday_repository::list_from(pool, today).await {
        Ok(dates) => dates,
        Err(e) => {
            tracing::error!("holiday read failed, treating as none: {e}");
            return Vec::new();
        }
    };
    if let Ok(mut cache) = HOLIDAYS_CACHE.lock() {
        *cache = Some((Instant::now(), dates.clone()));
    }
    dates
}

/// Drops the cached set; the admin holiday service calls
/// this after a change commits.
pub fn invalidate_cache() {
    if let Ok(mut cache) = HOLIDAYS_CACHE.lock() {
        *cache = None;
    }
}
