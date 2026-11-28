//! Session store setup: the PostgreSQL-backed
//! tower-sessions layers for the member and admin
//! surfaces, plus the expired-session cleanup task.

use crate::config::CONFIG;
use sqlx::PgPool;
use time::Duration;
use tower_sessions::{Expiry, SessionManagerLayer, session_store::ExpiredDeletion};
use tower_sessions_sqlx_store::PostgresStore;

/// Creates the member session layer on the shared pool.
///
/// The session record keys the member by user ID and carries
/// flow state (see [`crate::session`]); the cart lives in its own
/// table, and in-memory state is used only for disposable
/// counters.
pub async fn init_session_layer(
    pool: &PgPool,
) -> Result<SessionManagerLayer<PostgresStore>, sqlx::Error> {
    let store = PostgresStore::new(pool.clone());
    // No store.migrate(): the session schema and table are
    // owned by the migration history (20261101000055), because
    // the runtime role this pool connects as holds no DDL
    // rights (see init_pool's role split).

    Ok(SessionManagerLayer::new(store)
        // From config: false for local HTTP, true behind TLS at the ALB.
        .with_secure(CONFIG.cookie_secure)
        // Explicit, not the library default: a session id is a
        // credential, so script access to the cookie stays off
        // even if a future tower-sessions upgrade changes its
        // own default.
        .with_http_only(true)
        .with_same_site(tower_sessions::cookie::SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(Duration::seconds(
            CONFIG.member_session_ttl_duration().as_secs() as i64,
        ))))
}

/// Creates the admin session layer: same store, separate cookie
/// (`admin_session`) and an inactivity lifetime set by
/// `admin_session_ttl` in the typed config.
///
/// The separation is the security point: member and admin session
/// records are distinct and keyed by different IDs, so a member
/// session can never be presented as an admin one.
pub async fn init_admin_session_layer(
    pool: &PgPool,
) -> Result<SessionManagerLayer<PostgresStore>, sqlx::Error> {
    let store = PostgresStore::new(pool.clone());
    // No store.migrate(): the session schema and table are
    // owned by the migration history (20261101000055), because
    // the runtime role this pool connects as holds no DDL
    // rights (see init_pool's role split).

    Ok(SessionManagerLayer::new(store)
        .with_name("admin_session")
        // Scoped to /admin: the browser sends this cookie only under
        // the back office, shrinking where a leaked admin cookie
        // could be replayed.
        .with_path("/admin")
        .with_secure(CONFIG.cookie_secure)
        .with_http_only(true)
        .with_same_site(tower_sessions::cookie::SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(Duration::seconds(
            CONFIG.admin_session_ttl_duration().as_secs() as i64,
        ))))
}

/// Starts the expired-session cleanup: a periodic loop deleting
/// session records past their expiry.
///
/// The store never deletes expired rows on its own (it only
/// offers [`ExpiredDeletion::delete_expired`]), so without this
/// loop the session table grows without bound — it is already
/// the largest table in the database. One store serves both
/// cookie scopes: member and admin sessions live in the same
/// table, so one loop covers both. The first tick fires at
/// once, so a long-neglected table is cleaned at boot.
///
/// The caller keeps the returned handle and drains the task at
/// shutdown (P-3): the watch is consulted only between passes,
/// so a deletion pass in flight always runs to completion.
pub fn start_session_cleanup(
    pool: PgPool,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let handle = tokio::spawn(async move {
        let store = PostgresStore::new(pool);
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(
            CONFIG.session_cleanup_interval_duration().as_secs(),
        ));
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = shutdown.changed() => break,
            }
            if let Err(e) = store.delete_expired().await {
                // Housekeeping: a failed pass is logged and
                // retried on the next tick, never fatal.
                tracing::warn!("expired session cleanup failed: {e}");
            }
        }
        tracing::info!("expired session cleanup stopped");
    });
    tracing::info!(
        "expired session cleanup started (interval {})",
        CONFIG.session_cleanup_interval
    );
    handle
}
