//! Background-job run records: when each supervised job
//! last finished a pass (the ops status panel's source).

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;

/// Records one finished pass of `job`, replacing the
/// previous record. Best-effort observability: callers log a
/// failure and move on.
pub async fn record_job_run(pool: &PgPool, job: &str, result: &str) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO ops_job_runs (job, last_run_at, last_result) \
         VALUES ($1, now(), $2) \
         ON CONFLICT (job) DO UPDATE \
         SET last_run_at = EXCLUDED.last_run_at, last_result = EXCLUDED.last_result",
    )
    .bind(job)
    .bind(result)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("record job run", e))?;
    Ok(())
}

/// Returns when `job` last finished a pass, or `None` when
/// it never recorded one.
pub async fn last_job_run(pool: &PgPool, job: &str) -> Result<Option<DateTime<Utc>>, AppError> {
    sqlx::query_scalar::<_, DateTime<Utc>>("SELECT last_run_at FROM ops_job_runs WHERE job = $1")
        .bind(job)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("last job run", e))
}
