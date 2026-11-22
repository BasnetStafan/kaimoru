//! Review moderation.
//!
//! The list is anonymous — it carries no reviewer field at
//! all, mirroring the storefront's anonymity decision.
//! Actions: hide/unhide (reversible), the shop reply, and
//! deletion behind the browser's native confirmation.
//! Hiding and deleting also move the aggregates, since
//! ratings are computed from live rows.

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{
    db_error, layout_for, pagination, record_not_found, render, safe_return_url, set_flash,
};
use crate::config::CONFIG;
use crate::messages::REVIEW_DELETED_NOTICE;
use crate::services::admin::product_service;
use crate::services::admin::review_service;
use crate::session::CurrentAdmin;
use crate::views::admin::Page;
use crate::views::admin::reviews::{ReviewListView, ReviewRow};

/// Routes for the moderation surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/reviews", get(list))
        .route("/admin/reviews/{id}/delete", post(delete))
        .route("/admin/reviews/{id}/hide", post(hide))
        .route("/admin/reviews/{id}/unhide", post(unhide))
        .route("/admin/reviews/{id}/reply", post(reply))
}

/// List query parameters.
#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// Optional product ID filter.
    product_id: Option<String>,
    /// Page number.
    page: Option<String>,
}

/// The moderation list page.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<ListQuery>,
) -> Response {
    // The filter arrives as typed: an unparseable value
    // simply applies no filter, like an absent one.
    let product_id = query.product_id.as_deref().and_then(|raw| raw.parse().ok());
    let (page_no, limit, offset) = pagination(query.page.as_deref(), CONFIG.admin_per_page);
    let (rows, total) = match review_service::list_reviews(&pool, product_id, limit, offset).await {
        Ok(v) => v,
        Err(e) => return db_error("reviews", e),
    };
    let base = match product_id {
        Some(p) => format!("/admin/reviews?product_id={p}&"),
        None => "/admin/reviews?".to_string(),
    };
    let product_options = match product_service::list_all_id_name(&pool).await {
        Ok(all) => all
            .into_iter()
            .map(
                |(id, name)| crate::views::admin::reviews::ProductFilterOption {
                    selected: Some(id) == product_id,
                    value: id.to_string(),
                    name,
                },
            )
            .collect(),
        Err(e) => return db_error("reviews", e),
    };
    let return_url = if page_no > 1 {
        format!("{base}page={page_no}")
    } else {
        base.trim_end_matches(['?', '&']).to_string()
    };
    render(ReviewListView {
        return_url,
        layout: layout_for(&pool, &admin, &session).await,
        nav: "reviews",
        reviews: rows
            .iter()
            .map(|r| ReviewRow {
                id: r.id,
                product_id: r.product_id,
                product_name: r.product_name.clone(),
                star_pct: crate::models::review::stars_fill_percent(r.rating),
                rating_text: format!("{:.1}", r.rating),
                comment: r.comment.clone().unwrap_or_default(),
                has_comment: r.comment.is_some(),
                posted: crate::models::jst::jst_datetime_label(r.created_at),
                hidden: r.hidden,
                shop_reply: r.shop_reply.clone(),
            })
            .collect(),
        product_options,
        selected_product: product_id,
        page: Page::new(page_no, total, limit, base),
    })
}

/// Deletes one review (audited), then returns to the list.
/// Deletion is idempotent from the operator's chair: a review
/// that is already gone — the member deleted it first, or this is
/// a double-submit — gets the same redirect and flash, and no
/// second audit row (the service audits only an actual deletion).
pub async fn delete(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match review_service::delete_review(&pool, admin.id, id).await {
        Ok(_) => {
            set_flash(&session, REVIEW_DELETED_NOTICE).await;
            Redirect::to(&safe_return_url(&pairs, "/admin/reviews")).into_response()
        }
        Err(e) => db_error("review delete", e),
    }
}

/// Shared hide/unhide handler: flips the visibility, flashes,
/// and returns to the list with its state in `return_url`.
async fn set_hidden(
    pool: PgPool,
    admin: crate::models::AdminIdentity,
    session: Session,
    raw_id: String,
    pairs: Vec<(String, String)>,
    hidden: bool,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match review_service::set_hidden(&pool, admin.id, id, hidden).await {
        Ok(true) => {
            set_flash(
                &session,
                if hidden {
                    "レビューを非表示にしました"
                } else {
                    "レビューの非表示を解除しました"
                },
            )
            .await;
            Redirect::to(&safe_return_url(&pairs, "/admin/reviews")).into_response()
        }
        Ok(false) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("review hide", e),
    }
}

/// Hides one review from the storefront.
pub async fn hide(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    set_hidden(pool, admin, session, raw_id, pairs, true).await
}

/// Unhides one review.
pub async fn unhide(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    set_hidden(pool, admin, session, raw_id, pairs, false).await
}

/// Saves the shop reply under one review (empty clears it).
pub async fn reply(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let text = pairs
        .iter()
        .find(|(k, _)| k == "shop_reply")
        .map(|(_, v)| v.as_str())
        .unwrap_or("");
    match review_service::set_reply(&pool, admin.id, id, text).await {
        Ok(None) => {
            set_flash(&session, "店舗からの返信を保存しました").await;
            Redirect::to(&safe_return_url(&pairs, "/admin/reviews")).into_response()
        }
        Ok(Some(message)) => {
            set_flash(&session, &message).await;
            Redirect::to(&safe_return_url(&pairs, "/admin/reviews")).into_response()
        }
        Err(e) => db_error("review reply", e),
    }
}
