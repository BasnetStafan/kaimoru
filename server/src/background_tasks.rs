//! Background tasks for the server binary: supervision,
//! registration, the graceful drain at shutdown, and
//! log-retention pruning.
//!
//! ALL registration lives here (owner's design principle,
//! 2026-11-20): [`start_all`] is the one place that names
//! every worker, so `main` starts the whole set with a
//! single call and a new worker grows this file only —
//! never `main.rs`.

use std::fs;
use std::path::Path;

use app::config::CONFIG;

use crate::resolve_log_dir;

/// Supervises one background task: if the
/// task ends before the shutdown watch fires - a panic or
/// an unexpected exit - the end is logged at error level
/// and the task restarts after a short delay. Shutdown
/// semantics are unchanged: once the watch fires, the
/// pass in flight finishes, then the supervisor returns.
pub(crate) fn supervise<F>(
    name: &'static str,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    mut start: F,
) -> tokio::task::JoinHandle<()>
where
    F: FnMut() -> Option<tokio::task::JoinHandle<()>> + Send + 'static,
{
    tokio::spawn(async move {
        // Restart backoff: a worker
        // that dies instantly on every start (one poisoned
        // row is all it takes) must not spin at a flat 5 s
        // forever — the delay doubles to a 60 s cap, and a
        // run that stayed up a minute earns the base delay
        // back.
        let mut restart_delay = std::time::Duration::from_secs(5);
        loop {
            let Some(mut handle) = start() else {
                // The task is disabled (e.g. no payment
                // gateway configured): wait out the shutdown.
                let _ = shutdown.changed().await;
                return;
            };
            let started = std::time::Instant::now();
            tokio::select! {
                result = &mut handle => {
                    if *shutdown.borrow() {
                        return;
                    }
                    let earned_reset =
                        started.elapsed() >= std::time::Duration::from_secs(60);
                    if earned_reset {
                        restart_delay = std::time::Duration::from_secs(5);
                    }
                    let delay_secs = restart_delay.as_secs();
                    match result {
                        Ok(()) => tracing::error!(
                            "background task {name} ended unexpectedly; restarting in {delay_secs}s"
                        ),
                        Err(e) => tracing::error!(
                            "background task {name} panicked: {e}; restarting in {delay_secs}s"
                        ),
                    }
                    tokio::select! {
                        () = tokio::time::sleep(restart_delay) => {}
                        _ = shutdown.changed() => return,
                    }
                    // A run that earned the reset keeps the
                    // base delay it restarted with; doubling
                    // here would halve the reset's value
                    //.
                    if !earned_reset {
                        restart_delay =
                            (restart_delay * 2).min(std::time::Duration::from_secs(60));
                    }
                }
                _ = shutdown.changed() => {
                    // Let the worker finish the pass in
                    // flight, as the drain contract promises
                    // — up to the same grace the drain allows.
                    // Past it, abort the worker itself:
                    // aborting this supervisor instead would
                    // only DROP the worker's handle, which
                    // detaches the task rather than stopping
                    // it.
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        &mut handle,
                    )
                    .await
                    {
                        Ok(_) => {}
                        Err(_) => {
                            handle.abort();
                            let _ = (&mut handle).await;
                        }
                    }
                    return;
                }
            }
        }
    })
}

/// The server's background tasks, in registration order.
///
/// Every worker used to cost `main` a near-identical
/// supervise block plus a matching drain line, so the file
/// grew by a dozen lines per feature. The registry owns
/// both ends instead: [`start_all`] names each worker
/// once, and a new worker with the house starter shape is
/// a one-line registration there.
pub(crate) struct BackgroundTasks {
    /// (name, supervisor handle) pairs, in registration order.
    tasks: Vec<(&'static str, tokio::task::JoinHandle<()>)>,
}

impl BackgroundTasks {
    /// An empty registry.
    pub(crate) fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    /// Registers one worker with the house starter shape —
    /// `fn(pool, shutdown) -> JoinHandle` — supervised like
    /// every other task and started only when the process
    /// role runs tasks. Generic over the pool type so this
    /// crate never names the database crate.
    pub(crate) fn worker<P>(
        &mut self,
        name: &'static str,
        runs_tasks: bool,
        pool: &P,
        shutdown: &tokio::sync::watch::Receiver<bool>,
        start: fn(P, tokio::sync::watch::Receiver<bool>) -> tokio::task::JoinHandle<()>,
    ) where
        P: Clone + Send + 'static,
    {
        let pool = pool.clone();
        let rx = shutdown.clone();
        self.spawn(name, shutdown, move || {
            runs_tasks.then(|| start(pool.clone(), rx.clone()))
        });
    }

    /// Registers a worker whose starter does not fit the
    /// house shape — the payment sweep's gateway-dependent
    /// optional handle, log retention's pool-less loop —
    /// under the same supervision.
    pub(crate) fn spawn<F>(
        &mut self,
        name: &'static str,
        shutdown: &tokio::sync::watch::Receiver<bool>,
        start: F,
    ) where
        F: FnMut() -> Option<tokio::task::JoinHandle<()>> + Send + 'static,
    {
        self.tasks
            .push((name, supervise(name, shutdown.clone(), start)));
    }

    /// Drains every registered task CONCURRENTLY, each
    /// under the same per-task grace as before (whole-app
    /// review C1): the sequential drain's worst case grew
    /// by the 10 s grace per registered worker — past the
    /// 30 s platform stop window the grace was sized for —
    /// while the concurrent drain's worst case stays one
    /// grace however many workers register.
    pub(crate) async fn drain(self) {
        let mut drains = Vec::new();
        for (name, handle) in self.tasks {
            drains.push(tokio::spawn(drain_background(name, Some(handle))));
        }
        for drain in drains {
            let _ = drain.await;
        }
    }
}

/// Starts every background task the server runs, in
/// registration order, and returns the registry for the
/// shutdown drain. The one registration site for the whole
/// server: `main` calls this once and later drains the
/// result; adding a worker means adding its block here.
///
/// Every task is supervised: an unexpected
/// end is logged and the task restarted, so a dead worker
/// cannot go unnoticed. `runs_tasks` carries the role split: `web` processes start no
/// background tasks, `worker` processes bind no listener, `all` (the
/// default) does both. The pool arrives as the app crate's
/// [`app::db::DbPool`] alias: the worker starters are
/// concrete over the real pool, so a generic registration
/// could not call them (inference unified the types only
/// while the calls lived in `main`); the alias keeps this
/// crate free of any sqlx dependency all the same.
pub(crate) fn start_all(
    runs_tasks: bool,
    pool: &app::db::DbPool,
    shutdown_rx: &tokio::sync::watch::Receiver<bool>,
) -> BackgroundTasks {
    let mut tasks = BackgroundTasks::new();
    // The payment reconciliation sweep: always starts
    // (its compensation retries need no gateway); the
    // starter returns an optional handle, so it registers
    // through `spawn`.
    tasks.spawn("payment reconciliation sweep", shutdown_rx, {
        let pool = pool.clone();
        let rx = shutdown_rx.clone();
        move || {
            runs_tasks
                .then(|| app::services::payment_service::start_background(pool.clone(), rx.clone()))
                .flatten()
        }
    });
    // The mail campaign worker: claims due
    // campaign jobs and sends to opted-in members only.
    tasks.worker(
        "mail campaign worker",
        runs_tasks,
        pool,
        shutdown_rx,
        app::services::marketing_mail_service::start_worker,
    );
    // The restock sweep: finds pending subscriptions whose
    // combination is back in stock and mails each once.
    tasks.worker(
        "restock notification sweep",
        runs_tasks,
        pool,
        shutdown_rx,
        app::services::restock_service::start_worker,
    );
    // The shipping-mail worker: bulk ship paths enqueue one
    // outbox row per order; this worker claims and sends
    // them, so a bulk shipment never blocks on SMTP.
    tasks.worker(
        "shipping mail worker",
        runs_tasks,
        pool,
        shutdown_rx,
        app::services::shipping_mail_service::start_worker,
    );
    // The low-stock alert worker: mails the operator once a
    // day about products under their stock threshold.
    tasks.worker(
        "low stock alert worker",
        runs_tasks,
        pool,
        shutdown_rx,
        app::services::low_stock_alert_service::start_worker,
    );
    // The cart reminder worker: one reminder per member
    // cart state after the configured idle delay.
    tasks.worker(
        "cart reminder worker",
        runs_tasks,
        pool,
        shutdown_rx,
        app::services::cart_reminder_service::start_worker,
    );
    // The points expiry sweep (EC V2 core 1): turns expired
    // grant remainders into expire movements on the ledger.
    tasks.worker(
        "points expiry sweep",
        runs_tasks,
        pool,
        shutdown_rx,
        app::services::point_service::start_worker,
    );
    // The step-mail engine (EC V2 #2): enrolls delivered
    // orders and sends due scenario steps.
    tasks.worker(
        "step-mail worker",
        runs_tasks,
        pool,
        shutdown_rx,
        app::services::scenario_service::start_worker,
    );
    // The guest-cart cleanup sweep: deletes guest
    // carts untouched beyond the configured retention.
    tasks.worker(
        "guest cart sweep",
        runs_tasks,
        pool,
        shutdown_rx,
        app::services::cart_service::start_worker,
    );
    // Expired session records are deleted hourly; the
    // session store never removes them on its own.
    tasks.worker(
        "expired session cleanup",
        runs_tasks,
        pool,
        shutdown_rx,
        app::session_store::start_session_cleanup,
    );
    // Rotated log files are pruned to the configured
    // retention; the loop takes no pool, so
    // it registers through `spawn`.
    tasks.spawn("log retention", shutdown_rx, {
        let rx = shutdown_rx.clone();
        move || runs_tasks.then(|| start_log_retention(rx.clone()))
    });
    tasks
}

/// Starts the log retention pass: once at
/// boot and then daily, rotated log files older than the
/// configured retention are deleted. A retention of zero
/// disables pruning.
pub(crate) fn start_log_retention(
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let days = CONFIG.log_retention_days;
        if days == 0 {
            let _ = shutdown.changed().await;
            return;
        }
        let log_dir = resolve_log_dir(&CONFIG.log_dir);
        loop {
            prune_old_logs(&log_dir, days);
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)) => {}
                _ = shutdown.changed() => return,
            }
        }
    })
}

/// Deletes rotated log files whose last write is older
/// than the retention window. The file being written
/// today never qualifies.
fn prune_old_logs(log_dir: &Path, retention_days: u32) {
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(u64::from(retention_days) * 24 * 60 * 60);
    let Ok(entries) = fs::read_dir(log_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        // Only this app's own rotated logs are prunable: the
        // directory is operator-configured, and an unrelated
        // file that happens to share it must never be deleted
        // for merely being old.
        let is_own_log = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains("_app.log"));
        if !is_own_log {
            continue;
        }
        let stale = metadata
            .modified()
            .map(|mtime| mtime < cutoff)
            .unwrap_or(false);
        if stale {
            match fs::remove_file(&path) {
                Ok(()) => tracing::info!("pruned old log file {path:?}"),
                Err(e) => tracing::warn!("could not prune log file {path:?}: {e}"),
            }
        }
    }
}

/// Awaits one background task's drain after the shutdown watch
/// fires (P-3). The tasks stop on their own between passes; a
/// task that overruns the grace period is aborted, so a stuck
/// pass cannot hold the process open past the platform's stop
/// window (ECS Fargate allows 30 seconds by default).
pub(crate) async fn drain_background(name: &str, task: Option<tokio::task::JoinHandle<()>>) {
    let Some(mut handle) = task else {
        return;
    };
    match tokio::time::timeout(std::time::Duration::from_secs(10), &mut handle).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!("background task {name} ended abnormally: {e}"),
        Err(_) => {
            handle.abort();
            tracing::warn!("background task {name} did not stop within 10s; aborted");
        }
    }
}
