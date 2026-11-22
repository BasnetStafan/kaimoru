//! Database setup for Kaimoru.
//!
//! Builds the shared pool (config plus the `DB_PASSWORD` environment
//! variable) and applies pending migrations at startup.

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};

use crate::config::CONFIG;

/// The application's connection pool type, named for the
/// crates that must pass the pool around without naming
/// the database crate themselves: the server binary's
/// background-task registration (`start_all`) takes the
/// pool by this alias, which keeps the server crate free
/// of any sqlx dependency while the registration stays
/// fully type-checked against the concrete starters.
pub type DbPool = PgPool;

/// Longest wait for a pooled connection before the caller fails.
///
/// sqlx bounds the whole acquisition — free-connection wait, idle
/// ping, new connection — by this one deadline. One request stacks
/// several acquisitions (session load, queries, session save), so
/// this per-acquire bound caps page latency when the database is
/// stopped or unreachable: at 2 s the deepest chain fails in about
/// 10 s instead of hanging past 45 s on sqlx's 30 s default.
const ACQUIRE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Builds the pool options every pool in this app uses.
///
/// One helper, so a future second pool cannot drift to different
/// bounds; today [`init_pool`] creates the only pool and both
/// session layers clone it. 15 connections serve page queries, the
/// session store and search transactions together.
fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(15)
        .acquire_timeout(ACQUIRE_TIMEOUT)
}

/// Creates the connection pool and runs migrations.
///
/// # Errors
///
/// Returns [`sqlx::Error`] when the database cannot be reached or a
/// migration fails; startup stops rather than serve against a
/// missing or outdated schema.
pub async fn init_pool() -> Result<PgPool, sqlx::Error> {
    let db = &CONFIG.database;
    // Fail fast, no fallback: a missing DB_PASSWORD stops startup
    // here rather than panicking later. (The server binary's startup
    // validation is the first line of defence.)
    let password = std::env::var("DB_PASSWORD").map_err(|_| {
        sqlx::Error::Configuration(
            std::io::Error::other("DB_PASSWORD must be set (see README / run.sh)").into(),
        )
    })?;

    let app_password = std::env::var("APP_DB_PASSWORD").map_err(|_| {
        sqlx::Error::Configuration(
            std::io::Error::other("APP_DB_PASSWORD must be set (see README / run.sh)").into(),
        )
    })?;

    // Migrations live at the workspace root so every crate
    // shares one history. They run as the owner (`db.user`):
    // schema changes are the one moment the powerful
    // connection is needed.
    let owner_pool = pool_options()
        .connect_with(connect_options(&db.user, &password))
        .await?;
    // The session store's schema and table are created here
    // as well as in the initial schema migration (the same
    // DDL, IF NOT EXISTS on both sides), so a completely fresh
    // database is bootable before migrations run; on an
    // initialised database both statements are no-ops.
    exec(
        &owner_pool,
        "CREATE SCHEMA IF NOT EXISTS tower_sessions".to_string(),
    )
    .await?;
    exec(
        &owner_pool,
        "CREATE TABLE IF NOT EXISTS tower_sessions.session \
         (id text PRIMARY KEY NOT NULL, data bytea NOT NULL, \
         expiry_date timestamptz NOT NULL)"
            .to_string(),
    )
    .await?;
    sqlx::migrate!("../migrations").run(&owner_pool).await?;
    ensure_runtime_role(&owner_pool, &db.app_user, &app_password).await?;
    owner_pool.close().await;

    // Serving runs as the runtime role: least privilege, so a
    // compromised app cannot rewrite the append-only ledgers
    // or the schema (the DB role split).
    let pool = pool_options()
        .connect_with(connect_options(&db.app_user, &app_password))
        .await?;
    // Personal data at rest: any pre-encryption rows are
    // encrypted in place before the first request can read
    // them (see [`crate::pii`]).
    crate::pii_backfill::backfill(&pool).await?;
    Ok(pool)
}

/// Builds the connect options for one database user.
fn connect_options(user: &str, password: &str) -> PgConnectOptions {
    let db = &CONFIG.database;
    PgConnectOptions::new()
        .host(&db.host)
        .port(db.port)
        .username(user)
        .password(password)
        .database(&db.name)
        // "require" on RDS, "disable" for the local container.
        .ssl_mode(if CONFIG.db_ssl_mode == "require" {
            PgSslMode::Require
        } else {
            PgSslMode::Disable
        })
}

/// The append-only ledgers: the runtime role may read and
/// append, never rewrite or delete. Everything else is a
/// business table with full DML. One exception, granted
/// separately in [`ensure_runtime_role`]: the webhook
/// handler stamps `payment_id` and `applied` onto the
/// `webhook_events` row it recorded — those two columns
/// only; the event payload itself stays immutable.
const APPEND_ONLY_TABLES: [&str; 4] = [
    "admin_audit_logs",
    "stock_adjustments",
    "email_consent_logs",
    "webhook_events",
];

/// Executes one DDL statement on the owner pool.
async fn exec(pool: &PgPool, sql: String) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(&sql).execute(pool).await
}

/// Provisions the runtime role and (re)applies its grants.
///
/// Runs on the owner pool at every startup, after migrations,
/// so grants can never drift from this list: a table created
/// by a future migration receives its class of grants at the
/// same boot that creates it. The password is (re)set from
/// `APP_DB_PASSWORD` so rotating the env value is enough.
///
/// # Errors
///
/// Returns [`sqlx::Error`] when the role name is not a plain
/// identifier, equals the owner, or a statement fails.
async fn ensure_runtime_role(
    pool: &PgPool,
    app_user: &str,
    app_password: &str,
) -> Result<(), sqlx::Error> {
    let invalid = |msg: &str| sqlx::Error::Configuration(std::io::Error::other(msg).into());
    if app_user.is_empty()
        || !app_user
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(invalid(
            "database app_user must be a plain [a-z0-9_] identifier",
        ));
    }
    if app_user == CONFIG.database.user {
        return Err(invalid("database app_user must differ from the owner user"));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)")
            .bind(app_user)
            .fetch_one(pool)
            .await?;
    if !exists {
        exec(pool, format!("CREATE ROLE \"{app_user}\" LOGIN")).await?;
    }
    // The password arrives from the operator's environment;
    // single quotes are doubled, the SQL literal escaping.
    let escaped = app_password.replace('\'', "''");
    exec(
        pool,
        format!("ALTER ROLE \"{app_user}\" WITH LOGIN PASSWORD '{escaped}'"),
    )
    .await?;
    exec(
        pool,
        format!(
            "GRANT CONNECT ON DATABASE \"{}\" TO \"{app_user}\"",
            CONFIG.database.name
        ),
    )
    .await?;
    for schema in ["public", "tower_sessions"] {
        exec(
            pool,
            format!("GRANT USAGE ON SCHEMA {schema} TO \"{app_user}\""),
        )
        .await?;
        exec(
            pool,
            format!("REVOKE ALL PRIVILEGES ON ALL TABLES IN SCHEMA {schema} FROM \"{app_user}\""),
        )
        .await?;
    }
    exec(
        pool,
        format!("REVOKE ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA public FROM \"{app_user}\""),
    )
    .await?;
    let tables: Vec<String> =
        sqlx::query_scalar("SELECT tablename FROM pg_tables WHERE schemaname = 'public'")
            .fetch_all(pool)
            .await?;
    for table in &tables {
        let privileges = if APPEND_ONLY_TABLES.contains(&table.as_str()) {
            "SELECT, INSERT"
        } else {
            "SELECT, INSERT, UPDATE, DELETE"
        };
        exec(
            pool,
            format!("GRANT {privileges} ON TABLE public.\"{table}\" TO \"{app_user}\""),
        )
        .await?;
    }
    // The webhook ledger's two state columns (see
    // APPEND_ONLY_TABLES): the only UPDATE the runtime role
    // holds on any ledger.
    exec(
        pool,
        format!(
            "GRANT UPDATE (payment_id, applied) ON TABLE public.\"webhook_events\" TO \"{app_user}\""
        ),
    )
    .await?;
    // The session store's table is a business table: sessions
    // are created, read, updated and expiry-deleted.
    exec(
        pool,
        format!(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE tower_sessions.session TO \"{app_user}\""
        ),
    )
    .await?;
    exec(
        pool,
        format!("GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO \"{app_user}\""),
    )
    .await?;
    // Tables a future migration creates between boots receive
    // the business-table grants by default; this function
    // re-classifies the ledgers exactly on the next boot.
    exec(pool, format!(
        "ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO \"{app_user}\""
    ))
    .await?;
    exec(pool, format!(
        "ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO \"{app_user}\""
    ))
    .await?;
    Ok(())
}
