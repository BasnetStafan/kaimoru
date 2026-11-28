//! Return requests: creation from the inquiry intake
//! or directly against an order, and the audited status
//! walk. The refund itself is never executed here — the
//! `refunded` state only records that the shop completed
//! it through the order page's refund action.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::return_request;
use crate::repositories::admin::return_repository;
use crate::repositories::order_repository;
use crate::services::admin::audit_service;

/// Creates a request from a returns inquiry: the inquiry
/// supplies the order number and the reason (its body).
/// Refuses when the inquiry is not a returns inquiry, its
/// order cannot be found, or an open request already
/// exists for it.
pub async fn create_from_inquiry(
    pool: &PgPool,
    admin_id: i64,
    inquiry_id: i64,
) -> Result<i64, AppError> {
    let Some(inquiry) = crate::repositories::inquiry_repository::find(pool, inquiry_id)
        .await
        .map_err(AppError::from)?
    else {
        return Err(AppError::NotFound);
    };
    if inquiry.category != "returns" {
        return Err(AppError::Conflict(
            "返品・交換カテゴリのお問い合わせではありません".to_string(),
        ));
    }
    if return_repository::open_request_for_inquiry(pool, inquiry_id)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "このお問い合わせには進行中の返品申請があります".to_string(),
        ));
    }
    let order_number = inquiry.order_number.as_deref().unwrap_or("").trim();
    // The intake stored the order number as typed; resolve
    // it through the order number's own parse (AS-YYYYMMDD-N).
    let Some(order) = find_order_by_number(pool, order_number).await? else {
        return Err(AppError::Conflict(
            "お問い合わせの注文番号から注文が見つかりません".to_string(),
        ));
    };
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let id = return_repository::insert_return(&mut tx, order.id, Some(inquiry_id), &inquiry.body)
        .await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "return.create",
        "return_request",
        Some(id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(id)
}

/// Resolves a printed order number to the order: parses the
/// id suffix and verifies the formatted number matches, so
/// a mistyped date can never bind to the wrong order.
async fn find_order_by_number(
    pool: &PgPool,
    order_number: &str,
) -> Result<Option<crate::models::Order>, AppError> {
    let Some(id_part) = order_number.rsplit('-').next() else {
        return Ok(None);
    };
    let Ok(id) = id_part.parse::<i64>() else {
        return Ok(None);
    };
    let Some(order) = order_repository::find_by_id(pool, id)
        .await
        .map_err(AppError::from)?
    else {
        return Ok(None);
    };
    if order.order_number() != order_number {
        return Ok(None);
    }
    Ok(Some(order))
}

/// Moves one request along the walk, audited. Illegal jumps
/// are refused with the current state's name.
pub async fn change_status(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    next: &str,
) -> Result<(), AppError> {
    let Some(current) = return_repository::find_return(pool, id).await? else {
        return Err(AppError::NotFound);
    };
    if !return_request::legal_next_statuses(&current.status).contains(&next) {
        return Err(AppError::Conflict(format!(
            "現在の状態（{}）から{}へは進められません",
            return_request::status_label(&current.status),
            return_request::status_label(next),
        )));
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    // The status guard rides the UPDATE itself: if a
    // concurrent admin already moved the request, the row no
    // longer carries the status the legality check read, the
    // update lands on zero rows, and this mover is refused
    // instead of silently overwriting the first.
    if !return_repository::set_status(&mut tx, id, next, &current.status).await? {
        return Err(AppError::Conflict(
            "この返品リクエストは既に更新されています。最新の状態を確認してください".to_string(),
        ));
    }
    audit_service::record(
        &mut tx,
        admin_id,
        "return.status",
        "return_request",
        Some(id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}

/// The list rows.
pub async fn list_returns(
    pool: &PgPool,
) -> Result<Vec<return_repository::ReturnRequestRow>, AppError> {
    return_repository::list_returns(pool).await
}
