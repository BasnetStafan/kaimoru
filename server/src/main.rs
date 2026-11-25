//! Server entry point: logging, port binding and graceful shutdown.

use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};
use tokio::signal;
use tracing::info;
use tracing_appender::rolling::daily;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt, prelude::*};

mod background_tasks;
mod log_routing;

use background_tasks::start_all;

use app::config::CONFIG;
use app::routes;

/// Starts the HTTP server and blocks until a shutdown signal arrives.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = init_tracing()?;

    // Fail fast on missing secrets: a variable read lazily inside a
    // request (DB password, code secret) would panic mid-traffic.
    validate_environment()?;

    // One shared pool for every request; init_pool also runs the migrations.
    let pool = app::db::init_pool().await?;
    // Bootstraps the first admin from the environment, exactly once.
    app::services::admin::auth_service::bootstrap_admin(&pool).await?;
    let session_layer = app::session_store::init_session_layer(&pool).await?;
    // Separate admin session layer: the two identities never share a session.
    let admin_session_layer = app::session_store::init_admin_session_layer(&pool).await?;
    // Payments resolve once at boot: a misconfigured KOMOJU is
    // reported here and degrades checkout to cash on delivery,
    // and the reconciliation sweep starts when a gateway exists.
    app::services::payment_service::log_startup_state();
    // Background loops are drained after the HTTP server (P-3):
    // once serve returns, the watch fires, each loop finishes
    // the pass in flight, and main awaits the handles in order.
    let (shutdown_watch, shutdown_rx) = tokio::sync::watch::channel(false);
    // Every background task runs supervised: an unexpected end is logged and the task is
    // restarted, so a dead worker cannot go unnoticed.
    // The role split: `web` processes start no
    // background tasks, `worker` processes bind no listener;
    // `all` (the default) is the single-server behaviour.
    let role = CONFIG.app_role;
    tracing::info!(
        "app_role={role:?}: http={}, background tasks={}",
        role.serves_http(),
        role.runs_tasks()
    );
    // Every background task starts through the registry
    // (background_tasks::start_all): registration lives
    // there alone, so this call is the only line `main`
    // spends on workers; the drain after the HTTP server
    // stops is the other end of the same contract.
    let tasks = start_all(role.runs_tasks(), &pool, &shutdown_rx);
    if role.serves_http() {
        let app = routes::create_router(pool, session_layer, admin_session_layer);
        let addr: SocketAddr = format!("{}:{}", CONFIG.bind_host, CONFIG.port).parse()?;

        info!("{:<12} - {}", "LISTENING", addr);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await?;

        info!("{:<12} - {}", "SHUTTING DOWN", "Axum Server down.");
    } else {
        // A worker process has no listener: it lives until
        // the same shutdown signal the HTTP path waits on.
        info!("app_role=worker: no HTTP listener in this process");
        shutdown_signal().await;
        info!("{:<12} - {}", "SHUTTING DOWN", "Worker down.");
    }
    let _ = shutdown_watch.send(true);
    tasks.drain().await;
    Ok(())
}

/// Checks that every required environment variable is set.
///
/// Credential pairs (mail, admin bootstrap) count as one: half a pair
/// would only fail at first use, so it is rejected here.
///
/// # Errors
///
/// Returns an error naming the missing variable.
fn validate_environment() -> Result<(), Box<dyn std::error::Error>> {
    for name in [
        "DB_PASSWORD",
        "VERIFICATION_CODE_SECRET",
        "PII_ENCRYPTION_KEY",
        "APP_DB_PASSWORD",
    ] {
        match std::env::var(name) {
            Ok(value) if !value.trim().is_empty() => {}
            _ => {
                return Err(format!(
                    "{name} must be set and non-empty (see README / run.sh); refusing to start"
                )
                .into());
            }
        }
    }
    // Secret strength: presence was
    // checked above; the verification-code secret is also
    // the HMAC key protecting the 6-digit signup/reset
    // codes at rest, so a short value would let anyone who
    // obtains the challenge table brute-force codes
    // offline. Refuse below 32 characters. The PII key
    // only warns: rotating it is a data migration, not a
    // startup decision.
    let code_secret = std::env::var("VERIFICATION_CODE_SECRET").unwrap_or_default();
    if code_secret.chars().count() < 32 {
        return Err(
            "VERIFICATION_CODE_SECRET must be at least 32 characters (it keys the              verification-code HMAC); refusing to start"
                .into(),
        );
    }
    let pii_key = std::env::var("PII_ENCRYPTION_KEY").unwrap_or_default();
    if pii_key.chars().count() < 32 {
        tracing::warn!(
            "PII_ENCRYPTION_KEY is shorter than 32 characters;              plan a key rotation (a data migration) rather than changing it in place"
        );
    }
    // Cookie posture: an https
    // public URL with cookie_secure off serves session
    // cookies without the Secure attribute. The default
    // stays false for the http dev setup, so this warns
    // loudly instead of refusing.
    if let Some(base) = CONFIG.public_base_url()
        && base.starts_with("https://")
        && !CONFIG.cookie_secure
    {
        tracing::warn!(
            "PUBLIC_BASE_URL is https but cookie_secure is false:              session cookies will be served without the Secure attribute"
        );
    }
    // Both or neither: "neither" means no bootstrap, the correct
    // state once an admin exists.
    let admin_email = std::env::var("ADMIN_EMAIL").is_ok_and(|v| !v.trim().is_empty());
    let admin_password = std::env::var("ADMIN_PASSWORD").is_ok_and(|v| !v.trim().is_empty());
    if admin_email != admin_password {
        let missing = if admin_email {
            "ADMIN_PASSWORD"
        } else {
            "ADMIN_EMAIL"
        };
        return Err(format!(
            "{missing} must be set when its pair is set (admin bootstrap credentials are a pair); refusing to start"
        )
        .into());
    }
    let mail_user = std::env::var("MAIL_USER").is_ok_and(|v| !v.trim().is_empty());
    let mail_password = std::env::var("MAIL_PASSWORD").is_ok_and(|v| !v.trim().is_empty());
    if mail_user != mail_password {
        let missing = if mail_user {
            "MAIL_PASSWORD"
        } else {
            "MAIL_USER"
        };
        return Err(format!(
            "{missing} must be set when its pair is set (mail credentials are a pair); refusing to start"
        )
        .into());
    }
    Ok(())
}

/// Sets up tracing with one daily rolling file per surface.
///
/// The storefront, admin, webhook and agent-API surfaces each
/// write to their own file (`front_app.log`, `admin_app.log`,
/// `webhook_app.log`, `api_app.log`), routed by [`log_routing`];
/// the console carries every surface.
/// Returns the writer guards; dropping them stops file logging.
fn init_tracing()
-> Result<Vec<tracing_appender::non_blocking::WorkerGuard>, Box<dyn std::error::Error>> {
    let log_dir = resolve_log_dir(&CONFIG.log_dir);
    fs::create_dir_all(&log_dir)?;

    let (front_writer, front_guard) =
        tracing_appender::non_blocking(daily(&log_dir, "front_app.log"));
    let (admin_writer, admin_guard) =
        tracing_appender::non_blocking(daily(&log_dir, "admin_app.log"));
    let (webhook_writer, webhook_guard) =
        tracing_appender::non_blocking(daily(&log_dir, "webhook_app.log"));
    let (api_writer, api_guard) = tracing_appender::non_blocking(daily(&log_dir, "api_app.log"));

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // The files are the record; the console copy is for watching
    // live. Layers are boxed into one Vec so they fit a single
    // registry.
    tracing_subscriber::registry()
        .with(vec![
            build_base_layer(front_writer, false)
                .with_filter(log_routing::SurfaceFilter::new(
                    app::util::logging::Surface::Storefront,
                ))
                .boxed(),
            build_base_layer(admin_writer, false)
                .with_filter(log_routing::SurfaceFilter::new(
                    app::util::logging::Surface::Admin,
                ))
                .boxed(),
            build_base_layer(webhook_writer, false)
                .with_filter(log_routing::SurfaceFilter::new(
                    app::util::logging::Surface::Webhook,
                ))
                .boxed(),
            build_base_layer(api_writer, false)
                .with_filter(log_routing::SurfaceFilter::new(
                    app::util::logging::Surface::Api,
                ))
                .boxed(),
            build_base_layer(std::io::stdout, true),
        ])
        .with(log_routing::SurfaceTagLayer)
        .with(env_filter)
        .init();

    Ok(vec![front_guard, admin_guard, webhook_guard, api_guard])
}

/// Resolves the configured log directory against the workspace root when relative.
pub(crate) fn resolve_log_dir(log_dir: &str) -> PathBuf {
    let configured_path = Path::new(log_dir);
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap_or(manifest_dir);

    if configured_path.is_absolute() {
        configured_path.to_path_buf()
    } else {
        workspace_root.join(configured_path)
    }
}

/// Waits for Ctrl+C or SIGTERM.
///
/// ECS Fargate stops tasks with SIGTERM; handling only Ctrl+C would
/// drop in-flight requests on every deploy.
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

/// Builds a boxed formatting layer, so layers with different writer
/// types can stack in one registry.
fn build_base_layer<W>(writer: W, ansi: bool) -> Box<dyn Layer<Registry> + Send + Sync>
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    fmt::layer()
        .with_target(true)
        .with_thread_names(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_level(true)
        .with_timer(JstReadableTime)
        .with_writer(writer)
        .with_ansi(ansi)
        .boxed()
}

/// Log timestamp in JST, e.g. `2026-11-01 17:12:43.175`.
///
/// The offset is explicit because the process timezone is UTC on
/// ECS; `chrono::Local` would stamp lines 9 hours behind.
#[derive(Debug, Clone, Copy)]
struct JstReadableTime;

impl FormatTime for JstReadableTime {
    fn format_time(&self, w: &mut fmt::format::Writer<'_>) -> std::fmt::Result {
        let jst = chrono::FixedOffset::east_opt(9 * 3600).expect("9 hours is a valid offset");
        write!(
            w,
            "{}",
            chrono::Utc::now()
                .with_timezone(&jst)
                .format("%Y-%m-%d %H:%M:%S%.3f")
        )
    }
}
