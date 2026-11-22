//! Shop settings, storefront read side.
//!
//! Every page render needs the shop's words (logo, titles,
//! footer). The row changes only when the operator saves it,
//! so reads go through a short in-process cache — the coupon
//! band's shape: an admin save invalidates the cache, and
//! the TTL only bounds how long a change made outside the
//! admin (a restore, a hand edit) can linger.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use sqlx::PgPool;

use crate::models::ShopSettings;
use crate::repositories::shop_settings_repository;

/// How long the settings may be served from memory before
/// the database is asked again.
const SETTINGS_TTL: Duration = Duration::from_secs(60);

/// The cached settings and when they were read. Process-local,
/// like the band cache: on ECS each task refreshes its own
/// copy, and a save clears the copy in its own process at once.
static SETTINGS_CACHE: Mutex<Option<(Instant, ShopSettings)>> = Mutex::new(None);

/// Returns the current shop settings.
///
/// A missing row — or a failed read, already logged at the
/// repository — renders with [`ShopSettings::fallback`]: one
/// broken settings read must never take every page down with it.
/// A failed read is not cached, so a recovered database shows
/// on the next request, not a TTL later.
pub async fn load(pool: &PgPool) -> ShopSettings {
    if let Ok(cache) = SETTINGS_CACHE.lock()
        && let Some((read_at, settings)) = cache.as_ref()
        && read_at.elapsed() < SETTINGS_TTL
    {
        return settings.clone();
    }
    let settings = match shop_settings_repository::find(pool).await {
        Ok(found) => found.unwrap_or_else(ShopSettings::fallback),
        Err(_) => return ShopSettings::fallback(),
    };
    if let Ok(mut cache) = SETTINGS_CACHE.lock() {
        *cache = Some((Instant::now(), settings.clone()));
    }
    settings
}

/// Drops the cached settings, so the next [`load`] re-reads
/// the row. The admin settings service calls this after a
/// save commits: the operator's new words are live on the
/// next page, not up to a TTL later.
pub fn invalidate_cache() {
    if let Ok(mut cache) = SETTINGS_CACHE.lock() {
        *cache = None;
    }
}
