//! Notice service.
//!
//! Thin like the product service: it owns the news limit and
//! the brief candidate cache — the visible rows are cached,
//! the display-window decision never is.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{Notice, NoticeCandidate};
use crate::repositories::notice_repository;

/// How many announcements the news section shows.
const NEWS_LIMIT: i64 = 3;

/// Returns announcements for the news section, newest first: only
/// the ones live today (flag on, display period covering today).
/// The window is evaluated against today here, on every call —
/// only the candidate rows behind it are cached.
///
/// The same announcement registered twice is one announcement:
/// rows with an identical title and body collapse to the newest
/// before the limit applies, so a duplicated row can never crowd
/// distinct announcements out of the section. The stored rows are
/// untouched — this is a display rule only.
pub async fn latest(pool: &PgPool) -> Result<Vec<Notice>, AppError> {
    let today = crate::models::jst::jst_today();
    let candidates = visible_candidates(pool).await?;
    let mut shown: Vec<(&str, &str)> = Vec::new();
    Ok(candidates
        .iter()
        .filter(|candidate| candidate.is_live_on(today))
        .filter(|candidate| {
            let key = (candidate.title.as_str(), candidate.body.as_str());
            if shown.contains(&key) {
                false
            } else {
                shown.push(key);
                true
            }
        })
        .take(NEWS_LIMIT as usize)
        .map(NoticeCandidate::to_notice)
        .collect())
}

/// How long the visible notice rows may be served from memory
/// before the database is asked again.
const CANDIDATES_TTL: Duration = Duration::from_secs(60);

/// The cached visible notice rows and when they were read.
static CANDIDATES_CACHE: Mutex<Option<(Instant, Vec<NoticeCandidate>)>> = Mutex::new(None);

/// Returns the visible notice rows, from the brief cache when
/// fresh. A failed read is not cached and propagates, exactly
/// as the uncached read did.
async fn visible_candidates(pool: &PgPool) -> Result<Vec<NoticeCandidate>, AppError> {
    if let Ok(cache) = CANDIDATES_CACHE.lock()
        && let Some((read_at, rows)) = cache.as_ref()
        && read_at.elapsed() < CANDIDATES_TTL
    {
        return Ok(rows.clone());
    }
    let rows = notice_repository::find_visible_candidates(pool).await?;
    if let Ok(mut cache) = CANDIDATES_CACHE.lock() {
        *cache = Some((Instant::now(), rows.clone()));
    }
    Ok(rows)
}

/// Drops the cached candidate rows, so the next [`latest`]
/// call re-reads them. The admin notice service calls this
/// after a create, update or delete commits.
pub fn invalidate_candidates_cache() {
    if let Ok(mut cache) = CANDIDATES_CACHE.lock() {
        *cache = None;
    }
}
