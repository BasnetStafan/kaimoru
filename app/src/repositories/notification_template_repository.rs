//! Notification template repository: the operator
//! overrides over the notification mail defaults.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::notification_template::NotificationTemplate;

/// Returns every stored override, keyed by nothing — the
/// caller indexes by `mail_key`. The table holds at most
/// one row per catalog mail, so a full read is the cheap
/// shape the render cache wants.
pub async fn list_overrides(pool: &PgPool) -> Result<Vec<NotificationTemplate>, AppError> {
    sqlx::query_as::<_, NotificationTemplate>(
        "SELECT mail_key, subject, body, updated_at FROM notification_templates",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list notification templates", e))
}

/// Returns the override's current `updated_at` with the
/// row locked: the edit form's staleness guard compares
/// its form ticket's stamp against this value inside the
/// save transaction. No override row reads as `None`,
/// which the service treats as the stamp 0 (the code
/// default the form was rendered from).
pub async fn updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM notification_templates WHERE mail_key = $1 FOR UPDATE",
    )
    .bind(key)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

/// Stores one override inside the caller's transaction
/// (insert, or replace on a re-save), stamping `updated_at`
/// in the statement.
pub async fn upsert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    mail_key: &str,
    subject: &str,
    body: &str,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO notification_templates (mail_key, subject, body, updated_at, updated_by) \
         VALUES ($1, $2, $3, now(), $4) \
         ON CONFLICT (mail_key) DO UPDATE SET subject = EXCLUDED.subject, \
             body = EXCLUDED.body, updated_at = now(), updated_by = EXCLUDED.updated_by",
    )
    .bind(mail_key)
    .bind(subject)
    .bind(body)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("upsert notification template", e))?;
    Ok(())
}

/// Deletes one override inside the caller's transaction:
/// the mail falls back to its code default (初期値に戻す).
pub async fn delete(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    mail_key: &str,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM notification_templates WHERE mail_key = $1")
        .bind(mail_key)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete notification template", e))?;
    Ok(())
}
