//! Admin inquiry inbox handlers (Contact spec §3.5,
//! §5.4): the status-filtered list, the detail page with
//! status/memo saves, and the two-step reply (preview
//! confirm, then send — a sent mail cannot be unsent).
//! Every mutation writes its audit row; inquiry contents
//! themselves are never logged or audited, only ids.

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Form, Router, routing::get};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use crate::config::CONFIG;
use crate::controllers::admin::{layout_for, render, set_flash};
use crate::error::AppError;
use crate::messages::INQUIRY_NOT_FOUND_MESSAGE;
use crate::models;
use crate::models::AdminIdentity;
use crate::services::admin::audit_service;
use crate::services::inquiry_service;
use crate::session::CurrentAdmin;
use crate::views::admin::Page;
use crate::views::admin::inquiries::{
    InquiryChip, InquiryDetailView, InquiryListView, InquiryReplyConfirmView, ReplyErrors,
    ReplyValues,
};

/// Writes one audit row in its own transaction (the
/// house pattern for controller-side audits: the audit
/// service records inside a transaction, and here the
/// transaction exists only for the audit row).
async fn audit(pool: &PgPool, admin_id: i64, action: &'static str, inquiry_id: i64) {
    if let Ok(mut tx) = pool.begin().await {
        let _ = audit_service::record(&mut tx, admin_id, action, "inquiry", Some(inquiry_id)).await;
        let _ = tx.commit().await;
    }
}

/// The inquiry routes.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/inquiries", get(list))
        .route("/admin/inquiries/{id}", get(detail))
        .route("/admin/inquiries/{id}/update", axum::routing::post(update))
        .route(
            "/admin/inquiries/{id}/reply",
            axum::routing::post(reply_preview),
        )
        .route(
            "/admin/inquiries/{id}/reply/send",
            axum::routing::post(reply_send),
        )
        .route(
            "/admin/inquiries/{id}/reply/back",
            axum::routing::post(reply_back),
        )
}

/// The list's query string.
#[derive(Deserialize)]
pub struct ListQuery {
    /// Status filter chip.
    pub status: Option<String>,
    /// 1-based page.
    pub page: Option<String>,
}

/// `GET /admin/inquiries`: chips with counts, then the
/// filtered page, newest first.
async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<ListQuery>,
) -> Response {
    let status = query.status.as_deref().filter(|s| models::status_known(s));
    let (all, open, in_progress, closed) = inquiry_service::chip_counts(&pool).await;
    let chip = |code: Option<&str>, label: &str, count: i64| InquiryChip {
        label: label.to_string(),
        url: match code {
            Some(c) => format!("/admin/inquiries?status={c}"),
            None => "/admin/inquiries".to_string(),
        },
        count,
        active: status == code,
    };
    let chips = vec![
        chip(None, "すべて", all),
        chip(Some("open"), "未対応", open),
        chip(Some("in_progress"), "対応中", in_progress),
        chip(Some("closed"), "対応済み", closed),
    ];
    let total = inquiry_service::count_filtered(&pool, status).await;
    let total_pages = crate::util::pagination::total_pages(total, CONFIG.admin_per_page).max(1);
    let page = query
        .page
        .as_deref()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(1)
        .clamp(1, total_pages);
    let rows = inquiry_service::list_page(
        &pool,
        status,
        CONFIG.admin_per_page,
        page.saturating_sub(1).saturating_mul(CONFIG.admin_per_page),
    )
    .await;
    let base = match status {
        Some(c) => Page::base_with("/admin/inquiries", &format!("status={c}")),
        None => Page::base_with("/admin/inquiries", ""),
    };
    render(InquiryListView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "inquiries",
        chips,
        rows,
        page: Page::new(page, total, CONFIG.admin_per_page, base),
    })
}

/// Converts the service's field errors to the view's
/// per-field slots.
fn to_reply_errors(errors: &[crate::validation::FieldError]) -> ReplyErrors {
    let get = |field: &str| {
        errors
            .iter()
            .find(|e| e.field == field)
            .map(|e| e.message.clone())
    };
    ReplyErrors {
        subject: get("subject"),
        body: get("body"),
    }
}

/// The form-ticket scope of one inquiry.
fn inquiry_scope(id: i64) -> String {
    format!("inquiry:{id}")
}

/// Builds the detail view for one inquiry.
async fn detail_view(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    inquiry: models::Inquiry,
    reply: ReplyValues,
    reply_errors: ReplyErrors,
    memo_error: Option<String>,
) -> Response {
    let member = match inquiry.user_id {
        Some(uid) => inquiry_service::member_of(pool, uid).await,
        None => None,
    };
    let matched_order = match &inquiry.order_number {
        Some(typed) => inquiry_service::match_order(pool, typed)
            .await
            .map(|o| (o.id, o.order_number())),
        None => None,
    };
    let replies = inquiry_service::replies(pool, inquiry.id).await;
    let form_ticket = crate::services::form_ticket_service::issue(
        session,
        &inquiry_scope(inquiry.id),
        inquiry.updated_at.timestamp_micros(),
    )
    .await;
    render(InquiryDetailView {
        layout: layout_for(pool, admin, session).await,
        nav: "inquiries",
        inquiry,
        member,
        matched_order,
        replies,
        reply,
        reply_errors,
        memo_error,
        form_ticket,
    })
}

/// `GET /admin/inquiries/{id}`: the detail page; the
/// reply subject starts prefilled per spec §3.3.
async fn detail(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        set_flash(&session, INQUIRY_NOT_FOUND_MESSAGE).await;
        return Redirect::to("/admin/inquiries").into_response();
    };
    let Some(inquiry) = inquiry_service::find_admin(&pool, id).await else {
        set_flash(&session, INQUIRY_NOT_FOUND_MESSAGE).await;
        return Redirect::to("/admin/inquiries").into_response();
    };
    let settings = crate::services::shop_settings_service::load(&pool).await;
    let reply = ReplyValues {
        subject: format!(
            "{} お問い合わせへのご回答（受付番号：{}）",
            settings.shop_name,
            inquiry.reference()
        ),
        body: String::new(),
    };
    detail_view(
        &pool,
        &admin,
        &session,
        inquiry,
        reply,
        ReplyErrors::default(),
        None,
    )
    .await
}

/// The status/memo form.
#[derive(Deserialize)]
pub struct UpdateForm {
    /// New status code.
    pub status: String,
    /// New memo text.
    #[serde(default)]
    pub memo: String,
    /// One-time form ticket from the detail render.
    #[serde(default)]
    pub form_ticket: String,
}

/// `POST /admin/inquiries/{id}/update`: saves status
/// and memo, auditing each change under its own code.
async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<UpdateForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return Redirect::to("/admin/inquiries").into_response();
    };
    let Some(inquiry) = inquiry_service::find_admin(&pool, id).await else {
        return Redirect::to("/admin/inquiries").into_response();
    };
    if form.memo.chars().count() > 1000 {
        // Re-render with the typed memo in place of the
        // stored one: the operator corrects the text they
        // wrote instead of starting over from the DB value.
        let mut inquiry = inquiry;
        inquiry.admin_memo = form.memo.clone();
        let reply = ReplyValues {
            subject: String::new(),
            body: String::new(),
        };
        return detail_view(
            &pool,
            &admin,
            &session,
            inquiry,
            reply,
            ReplyErrors::default(),
            Some("管理メモは1000文字以内で入力してください。".to_string()),
        )
        .await;
    }
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &inquiry_scope(id),
        &form.form_ticket,
    )
    .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/inquiries/{id}")).into_response();
    };
    let update = match inquiry_service::update_admin_fields(
        &pool,
        id,
        &form.status,
        &form.memo,
        expected,
    )
    .await
    {
        Ok(update) => update,
        Err(AppError::Conflict(_)) => {
            set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
            return Redirect::to(&format!("/admin/inquiries/{id}")).into_response();
        }
        Err(AppError::NotFound) => {
            return Redirect::to("/admin/inquiries").into_response();
        }
        Err(e) => {
            tracing::error!(action = "inquiry.update", inquiry_id = id, error = %e, "inquiry update failed");
            set_flash(&session, "保存に失敗しました。").await;
            return Redirect::to(&format!("/admin/inquiries/{id}")).into_response();
        }
    };
    if update.status_changed {
        audit(&pool, admin.id, "inquiry.status.update", id).await;
    }
    if update.memo_changed {
        audit(&pool, admin.id, "inquiry.memo.update", id).await;
    }
    set_flash(&session, "保存しました。").await;
    Redirect::to(&format!("/admin/inquiries/{id}")).into_response()
}

/// The reply form.
#[derive(Deserialize)]
pub struct ReplyForm {
    /// Subject.
    pub subject: String,
    /// Body.
    pub body: String,
}

/// `POST /admin/inquiries/{id}/reply`: validates and
/// renders the confirm step — nothing is sent here.
async fn reply_preview(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<ReplyForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return Redirect::to("/admin/inquiries").into_response();
    };
    let Some(inquiry) = inquiry_service::find_admin(&pool, id).await else {
        return Redirect::to("/admin/inquiries").into_response();
    };
    let errors = inquiry_service::validate_reply(&form.subject, &form.body);
    if !errors.is_empty() {
        let reply = ReplyValues {
            subject: form.subject,
            body: form.body,
        };
        return detail_view(
            &pool,
            &admin,
            &session,
            inquiry,
            reply,
            to_reply_errors(&errors),
            None,
        )
        .await;
    }
    render(InquiryReplyConfirmView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "inquiries",
        inquiry_id: inquiry.id,
        reference: inquiry.reference(),
        to_email: inquiry.email.clone(),
        subject: form.subject,
        body: form.body,
    })
}

/// `POST /admin/inquiries/{id}/reply/back`: the confirm
/// step's "Back to edit" — re-renders the detail page with
/// the drafted reply restored, so backing out to fix a
/// typo never discards the draft.
async fn reply_back(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<ReplyForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return Redirect::to("/admin/inquiries").into_response();
    };
    let Some(inquiry) = inquiry_service::find_admin(&pool, id).await else {
        return Redirect::to("/admin/inquiries").into_response();
    };
    let reply = ReplyValues {
        subject: form.subject,
        body: form.body,
    };
    detail_view(
        &pool,
        &admin,
        &session,
        inquiry,
        reply,
        ReplyErrors::default(),
        None,
    )
    .await
}

/// `POST /admin/inquiries/{id}/reply/send`: sends the
/// confirmed reply; on mailer failure nothing is stored
/// and the admin stays on the detail page with an error.
async fn reply_send(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<ReplyForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return Redirect::to("/admin/inquiries").into_response();
    };
    let Some(inquiry) = inquiry_service::find_admin(&pool, id).await else {
        return Redirect::to("/admin/inquiries").into_response();
    };
    // Re-validate: the confirm step's hidden fields are
    // still client input.
    let errors = inquiry_service::validate_reply(&form.subject, &form.body);
    if !errors.is_empty() {
        let reply = ReplyValues {
            subject: form.subject,
            body: form.body,
        };
        return detail_view(
            &pool,
            &admin,
            &session,
            inquiry,
            reply,
            to_reply_errors(&errors),
            None,
        )
        .await;
    }
    match inquiry_service::send_reply(&pool, &inquiry, admin.id, &form.subject, &form.body).await {
        Ok(true) => {
            audit(&pool, admin.id, "inquiry.reply", id).await;
            set_flash(&session, "返信を送信しました。").await;
        }
        Ok(false) => {
            set_flash(
                &session,
                "メール送信に失敗しました。返信は記録されていません。",
            )
            .await;
        }
        Err(e) => {
            tracing::error!(action = "inquiry.reply", inquiry_id = id, error = %e, "reply store failed");
            set_flash(&session, "返信の保存に失敗しました。").await;
        }
    }
    Redirect::to(&format!("/admin/inquiries/{id}")).into_response()
}
