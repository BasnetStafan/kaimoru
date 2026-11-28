//! Customer segments: the definition list, creation
//! with all-at-once validation, and deletion behind the
//! native confirmation. Membership is evaluated live by the
//! repository — this controller only shapes forms.

use axum::{
    Form, Router,
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, record_not_found, render, set_flash};
use crate::services::admin::segment_service::{self, SegmentInput};
use crate::session::CurrentAdmin;
use crate::views::admin::segments::{SegmentListView, SegmentRow};

/// Routes for the segments surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/segments", get(list).post(create))
        .route("/admin/segments/{id}/delete", post(delete))
}

/// Builds the page view from the current definitions plus a
/// form state (echoes + joined error text).
#[allow(clippy::too_many_arguments)]
async fn build_view(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    form: (&str, &str, &str, &str, &str, &str, &str, &str, &str),
    form_error: String,
) -> Result<SegmentListView, crate::error::AppError> {
    let rows = segment_service::list_with_counts(pool)
        .await?
        .into_iter()
        .map(|(s, c)| SegmentRow::new(s, c))
        .collect();
    Ok(SegmentListView {
        layout: layout_for(pool, admin, session).await,
        nav: "segments",
        rows,
        form_name: form.0.to_string(),
        form_tag: form.1.to_string(),
        form_min_spend: form.2.to_string(),
        form_min_orders: form.3.to_string(),
        form_before: form.4.to_string(),
        form_after: form.5.to_string(),
        form_r: form.6.to_string(),
        form_f: form.7.to_string(),
        form_m: form.8.to_string(),
        form_error,
    })
}

/// The list + create form.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    match build_view(
        &pool,
        &admin,
        &session,
        ("", "", "", "", "", "", "", "", ""),
        String::new(),
    )
    .await
    {
        Ok(view) => render(view),
        Err(e) => db_error("segments", e),
    }
}

/// Creates one segment; validation errors re-render the
/// page with the typed values kept (back-office rule).
pub async fn create(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let get = |name: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let input = SegmentInput {
        name: get("name"),
        tag: get("tag"),
        min_total_spend_yen: get("min_total_spend_yen"),
        min_order_count: get("min_order_count"),
        last_order_before: get("last_order_before"),
        last_order_after: get("last_order_after"),
        min_r_score: get("min_r_score"),
        min_f_score: get("min_f_score"),
        min_m_score: get("min_m_score"),
    };
    let echo = (
        input.name.clone(),
        input.tag.clone(),
        input.min_total_spend_yen.clone(),
        input.min_order_count.clone(),
        input.last_order_before.clone(),
        input.last_order_after.clone(),
        input.min_r_score.clone(),
        input.min_f_score.clone(),
        input.min_m_score.clone(),
    );
    match segment_service::validate(&input) {
        Ok(segment) => match segment_service::create_segment(&pool, admin.id, &segment).await {
            Ok(_) => {
                set_flash(&session, "セグメントを作成しました").await;
                Redirect::to("/admin/segments").into_response()
            }
            Err(e) => db_error("segment create", e),
        },
        Err(errors) => {
            let joined = errors
                .iter()
                .map(|(_, m)| m.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            match build_view(
                &pool,
                &admin,
                &session,
                (
                    &echo.0, &echo.1, &echo.2, &echo.3, &echo.4, &echo.5, &echo.6, &echo.7, &echo.8,
                ),
                joined,
            )
            .await
            {
                Ok(view) => render(view),
                Err(e) => db_error("segments", e),
            }
        }
    }
}

/// Deletes one segment definition, audited.
pub async fn delete(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match segment_service::delete_segment(&pool, admin.id, id).await {
        Ok(true) => {
            set_flash(&session, "セグメントを削除しました").await;
            Redirect::to("/admin/segments").into_response()
        }
        Ok(false) => record_not_found(&pool, &admin, &session).await,
        Err(crate::error::AppError::Conflict(message)) => {
            set_flash(&session, &message).await;
            Redirect::to("/admin/segments").into_response()
        }
        Err(e) => db_error("segment delete", e),
    }
}
