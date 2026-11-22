//! Return requests: the list + status walk, and the
//! create-from-inquiry action that lives on the inquiry
//! detail. No refund is executed here by design.

use axum::{
    Form, Router,
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, record_not_found, render, set_flash};
use crate::error::AppError;
use crate::services::admin::return_service;
use crate::session::CurrentAdmin;
use crate::views::admin::returns::{ReturnListView, ReturnRow};

/// Routes for the returns surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/returns", get(list))
        .route("/admin/returns/{id}/status", post(change_status))
        .route(
            "/admin/returns/from-inquiry/{id}",
            post(create_from_inquiry),
        )
}

/// The returns list.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    let rows = match return_service::list_returns(&pool).await {
        Ok(rows) => rows.iter().map(ReturnRow::from).collect(),
        Err(e) => return db_error("returns", e),
    };
    render(ReturnListView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "returns",
        rows,
    })
}

/// Advances one request along the walk.
pub async fn change_status(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let next = pairs
        .iter()
        .find(|(k, _)| k == "status")
        .map(|(_, v)| v.as_str())
        .unwrap_or("");
    match return_service::change_status(&pool, admin.id, id, next).await {
        Ok(()) => {
            set_flash(&session, "返品申請の状態を更新しました").await;
            Redirect::to("/admin/returns").into_response()
        }
        Err(AppError::Conflict(message)) => {
            set_flash(&session, &message).await;
            Redirect::to("/admin/returns").into_response()
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("return status", e),
    }
}

/// Creates a return request from a returns inquiry, then
/// lands on the returns list.
pub async fn create_from_inquiry(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match return_service::create_from_inquiry(&pool, admin.id, id).await {
        Ok(_) => {
            set_flash(&session, "返品申請を作成しました").await;
            Redirect::to("/admin/returns").into_response()
        }
        Err(AppError::Conflict(message)) => {
            set_flash(&session, &message).await;
            Redirect::to(&format!("/admin/inquiries/{id}")).into_response()
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("return create", e),
    }
}
