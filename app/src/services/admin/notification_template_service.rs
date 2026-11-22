//! Notification template administration: the operator
//! override write path for transactional mail templates
//! (save and reset), split from the read/render engine
//! in `services::notification_template_service`.

use sqlx::PgPool;

use crate::error::AppError;

use crate::models::notification_template::NotificationMailDef;
use crate::repositories::notification_template_repository;

/// Saves one override with its audit row in one
/// transaction, then drops the render cache.
pub async fn save_override(
    pool: &PgPool,
    admin_id: i64,
    def: &NotificationMailDef,
    subject: &str,
    body: &str,
    expected: i64,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    // The form-ticket staleness guard: `expected` is the
    // override's `updated_at` (epoch microseconds) recorded
    // server-side when the edit form was rendered, or 0
    // when no override existed then (the form showed the
    // code default). The row is locked for the comparison.
    let current = notification_template_repository::updated_at_for_update(&mut tx, def.key).await?;
    if current.map(|t| t.timestamp_micros()).unwrap_or(0) != expected {
        return Err(crate::services::form_ticket_service::stale_conflict());
    }
    notification_template_repository::upsert(&mut tx, def.key, subject, body, admin_id).await?;
    crate::services::admin::audit_service::record(
        &mut tx,
        admin_id,
        "notification_template.update",
        "notification_template",
        None,
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    crate::services::notification_template_service::invalidate_cache();
    Ok(())
}

/// Deletes one override (初期値に戻す) with its audit row
/// in one transaction, then drops the render cache.
pub async fn reset_override(
    pool: &PgPool,
    admin_id: i64,
    def: &NotificationMailDef,
    expected: i64,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    // The same staleness guard as the save: a reset
    // submitted from a stale form must not delete an
    // override another operator saved after the form was
    // rendered.
    let current = notification_template_repository::updated_at_for_update(&mut tx, def.key).await?;
    if current.map(|t| t.timestamp_micros()).unwrap_or(0) != expected {
        return Err(crate::services::form_ticket_service::stale_conflict());
    }
    notification_template_repository::delete(&mut tx, def.key).await?;
    crate::services::admin::audit_service::record(
        &mut tx,
        admin_id,
        "notification_template.reset",
        "notification_template",
        None,
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    crate::services::notification_template_service::invalidate_cache();
    Ok(())
}
