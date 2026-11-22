//! Customer list and detail.
//!
//! The member record itself stays read-only — there is no
//! path here that can suspend, edit or delete a member — and
//! the queries behind it never select the password hash (the
//! detail's model has no such field at all). C1 adds writes
//! for OPERATOR metadata only (memo + tags), below.

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, pagination, record_not_found, render, set_flash};
use crate::config::CONFIG;
use crate::pricing;
use crate::services::admin::customer_service;
use crate::session::CurrentAdmin;
use crate::views::admin::customers::{CustomerDetailView, CustomerListView};
use crate::views::admin::{Page, filter_query};

/// Routes for the customers surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/customers", get(list))
        .route("/admin/customers/{id}", get(detail))
        .route("/admin/customers/{id}/memo", post(save_memo))
        .route("/admin/customers/{id}/tags", post(add_tag))
        .route("/admin/customers/{id}/tags/remove", post(remove_tag))
}

/// List query parameters.
#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// Name / kana / email partial-match search text.
    q: Option<String>,
    /// Page number.
    page: Option<String>,
}

/// The customers list page.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<ListQuery>,
) -> Response {
    let q = query.q.clone().unwrap_or_default();
    // The raw search text travels down as-is: the repository
    // matches it over decrypted values in Rust (the columns
    // hold ciphertext now), so no LIKE pattern is built here.
    let pattern = (!q.trim().is_empty()).then(|| q.trim().to_string());
    let (page_no, limit, offset) = pagination(query.page.as_deref(), CONFIG.admin_per_page);
    let (rows, total) = match customer_service::list_customers(&pool, pattern, limit, offset).await
    {
        Ok(v) => v,
        Err(e) => return db_error("customers", e),
    };
    let base = Page::base_with(
        "/admin/customers",
        &filter_query(&[("q", (!q.trim().is_empty()).then_some(q.trim()))]),
    );
    render(CustomerListView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "customers",
        q,
        customers: rows,
        page: Page::new(page_no, total, limit, base),
    })
}

/// The customer detail page. A missing member is the styled
/// admin 404, indistinguishable from a wrong path.
pub async fn detail(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let customer = match customer_service::find_customer_detail(&pool, id).await {
        Ok(Some(customer)) => customer,
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("customer load", e),
    };
    // Every figure on this page comes from the database, so a
    // failed query is a 500, never an invented zero — the totals
    // query errored behind a 200 until 2026-11-02 and the page
    // rendered a fictional ¥0 purchase total.
    let (order_count, purchase_total) = match customer_service::customer_totals(&pool, id).await {
        Ok(v) => v,
        Err(e) => return db_error("customers", e),
    };
    let orders = match customer_service::orders_for_customer(&pool, id).await {
        Ok(v) => v,
        Err(e) => return db_error("customers", e),
    };
    let (memo, tags) = match customer_service::customer_meta(&pool, id).await {
        Ok(v) => v,
        Err(e) => return db_error("customers", e),
    };
    render(CustomerDetailView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "customers",
        customer,
        order_count,
        purchase_total: pricing::display_price(purchase_total),
        orders,
        withdrawn_at: crate::repositories::admin::customer_repository::withdrawn_at(&pool, id)
            .await
            .unwrap_or(None),
        memo,
        tags,
    })
}

/// Reads one named field out of raw form pairs.
fn field(pairs: &[(String, String)], name: &str) -> String {
    pairs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// Saves the operator memo, then returns to the detail
/// (same-screen completion, back-office rule).
pub async fn save_memo(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match customer_service::save_memo(&pool, admin.id, id, &field(&pairs, "memo")).await {
        Ok(None) => {
            set_flash(&session, "メモを保存しました").await;
            Redirect::to(&format!("/admin/customers/{id}")).into_response()
        }
        Ok(Some(message)) => {
            set_flash(&session, &message).await;
            Redirect::to(&format!("/admin/customers/{id}")).into_response()
        }
        Err(e) => db_error("customer memo", e),
    }
}

/// Adds a tag to the member.
pub async fn add_tag(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match customer_service::add_tag(&pool, admin.id, id, &field(&pairs, "tag")).await {
        Ok(None) => {
            set_flash(&session, "タグを追加しました").await;
            Redirect::to(&format!("/admin/customers/{id}")).into_response()
        }
        Ok(Some(message)) => {
            set_flash(&session, &message).await;
            Redirect::to(&format!("/admin/customers/{id}")).into_response()
        }
        Err(e) => db_error("customer tag", e),
    }
}

/// Removes one tag from the member.
pub async fn remove_tag(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match customer_service::remove_tag(&pool, admin.id, id, &field(&pairs, "tag")).await {
        Ok(()) => {
            set_flash(&session, "タグを外しました").await;
            Redirect::to(&format!("/admin/customers/{id}")).into_response()
        }
        Err(e) => db_error("customer tag", e),
    }
}
