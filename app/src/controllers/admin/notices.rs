//! Notice management.
//!
//! The form carries a title, a body, an optional display
//! period and a visibility flag. A notice reaches the top page
//! only while its flag is on and today is inside its period —
//! the form says so, so the behaviour is never a surprise.
//! Deletes sit behind the browser's native confirmation.

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
use crate::models::AdminIdentity;
use crate::services::admin::notice_service::{self as admin_notice_service, NoticeFormInput};
use crate::services::notice_service;
use crate::session::CurrentAdmin;
use crate::validation::FieldError;
use crate::views::admin::Page;
use crate::views::admin::filter_query;
use crate::views::admin::notices::{NoticeFormView, NoticeListView, NoticeStatusOption, rows_for};

/// The status filter options: the derived display state, with
/// the two not-live states under one option — the operator's
/// question is "is it on the top page or not".
const STATUSES: [(&str, &str); 3] = [
    ("", "すべて"),
    ("visible", "表示中"),
    ("hidden", "非表示・期間外"),
];

/// Routes for the notices surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/notices", get(list))
        .route("/admin/notices/new", get(new_form).post(create))
        .route("/admin/notices/{id}/edit", get(edit_form).post(update))
        .route("/admin/notices/{id}/delete", post(delete))
}

/// List query parameters.
#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// Title search text.
    q: Option<String>,
    /// Status filter value.
    status: Option<String>,
    /// Page number.
    page: Option<String>,
}

/// The notices list: title search, status filter, pagination.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<ListQuery>,
) -> Response {
    let q = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let status = query
        .status
        .as_deref()
        .filter(|s| STATUSES.iter().any(|(v, _)| v == s))
        .unwrap_or("")
        .to_string();
    let (page_no, limit, offset) = pagination(query.page.as_deref(), CONFIG.admin_per_page);
    let status_param = if status.is_empty() {
        None
    } else {
        Some(status.as_str())
    };
    let (notices, total) =
        match admin_notice_service::list_notices(&pool, q.as_deref(), status_param, limit, offset)
            .await
        {
            Ok(v) => v,
            Err(e) => return db_error("notices", e),
        };
    let duplicates = match admin_notice_service::duplicate_counts(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("notices", e),
    };
    let preview = match notice_service::latest(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("notices", e),
    };
    let base = Page::base_with(
        "/admin/notices",
        &filter_query(&[("q", q.as_deref()), ("status", status_param)]),
    );
    render(NoticeListView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "notices",
        rows: rows_for(&notices, &duplicates),
        preview,
        page: Page::new(page_no, total, limit, base),
        q: q.clone().unwrap_or_default(),
        status_options: STATUSES
            .iter()
            .map(|(v, l)| NoticeStatusOption {
                value: v,
                label: l,
                selected: status == *v,
            })
            .collect(),
        filtered: q.is_some() || !status.is_empty(),
    })
}

/// The notice form body. The checkbox arrives as a value when
/// checked and not at all when unchecked — the standard HTML
/// form behaviour the `Option` captures.
#[derive(Deserialize)]
pub struct NoticeForm {
    /// Headline as submitted.
    title: String,
    /// Body as submitted.
    body: String,
    /// Period start as submitted (`YYYY-MM-DD` or empty).
    display_from: String,
    /// Period end as submitted (`YYYY-MM-DD` or empty).
    display_to: String,
    /// The 表示する checkbox value, when checked.
    #[serde(default)]
    is_visible: Option<String>,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the form, which the
    /// edit submit treats as stale. The create submit
    /// ignores it.
    #[serde(default)]
    form_ticket: String,
}

impl From<&NoticeForm> for NoticeFormInput {
    fn from(f: &NoticeForm) -> Self {
        NoticeFormInput {
            title: f.title.clone(),
            body: f.body.clone(),
            display_from: f.display_from.clone(),
            display_to: f.display_to.clone(),
            is_visible: f.is_visible.is_some(),
        }
    }
}

/// A blank create form: no period (shows immediately, never
/// expires) and the visibility flag on.
pub async fn new_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    form_response(
        &pool,
        &admin,
        &session,
        &NoticeFormInput {
            is_visible: true,
            ..NoticeFormInput::default()
        },
        None,
        None,
        String::new(),
    )
    .await
}

/// Handles the create submit.
pub async fn create(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<NoticeForm>,
) -> Response {
    let input = NoticeFormInput::from(&form);
    match admin_notice_service::validate_notice(&input) {
        Ok(valid) => match admin_notice_service::create_notice(&pool, admin.id, &valid).await {
            Ok((id, duplicated)) => {
                set_flash(
                    &session,
                    if duplicated {
                        "お知らせを登録しました（同じ内容のお知らせが他にも表示設定されています）"
                    } else {
                        "お知らせを登録しました"
                    },
                )
                .await;
                Redirect::to(&format!("/admin/notices/{id}/edit")).into_response()
            }
            Err(e) => db_error("notices", e),
        },
        Err(errors) => {
            form_response(
                &pool,
                &admin,
                &session,
                &input,
                None,
                Some(&errors),
                String::new(),
            )
            .await
        }
    }
}

/// The edit form for one notice.
pub async fn edit_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match admin_notice_service::find_notice(&pool, id).await {
        Ok(Some(notice)) => {
            let date = |d: Option<chrono::NaiveDate>| {
                d.map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_default()
            };
            let input = NoticeFormInput {
                title: notice.title.clone(),
                body: notice.body.clone(),
                display_from: date(notice.display_from),
                display_to: date(notice.display_to),
                is_visible: notice.is_visible,
            };
            let form_ticket = notice_ticket(&pool, &session, id).await;
            form_response(&pool, &admin, &session, &input, Some(id), None, form_ticket).await
        }
        Ok(None) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("notices", e),
    }
}

/// Handles the edit submit.
pub async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<NoticeForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match admin_notice_service::find_notice(&pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("notices", e),
    }
    // Spend the form ticket before validating: a submit
    // without a valid ticket was not rendered from the
    // current record, so it is refused as stale. A ticket
    // whose stamp has gone stale is refused by the
    // service's check instead.
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &notice_scope(id),
        &form.form_ticket,
    )
    .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/notices/{id}/edit")).into_response();
    };
    let input = NoticeFormInput::from(&form);
    match admin_notice_service::validate_notice(&input) {
        Ok(valid) => {
            match admin_notice_service::update_notice(&pool, admin.id, id, &valid, expected).await {
                Ok(duplicated) => {
                    set_flash(
                    &session,
                    if duplicated {
                        "お知らせを更新しました（同じ内容のお知らせが他にも表示設定されています）"
                    } else {
                        "お知らせを更新しました"
                    },
                )
                .await;
                    Redirect::to(&format!("/admin/notices/{id}/edit")).into_response()
                }
                Err(crate::error::AppError::Conflict(message)) => {
                    set_flash(&session, &message).await;
                    Redirect::to(&format!("/admin/notices/{id}/edit")).into_response()
                }
                Err(e) => db_error("notices", e),
            }
        }
        Err(errors) => {
            let form_ticket = notice_ticket(&pool, &session, id).await;
            form_response(
                &pool,
                &admin,
                &session,
                &input,
                Some(id),
                Some(&errors),
                form_ticket,
            )
            .await
        }
    }
}

/// Deletes one notice. Idempotent like the review delete: an
/// already-gone ID still redirects with the flash, and the
/// service writes no second audit row.
pub async fn delete(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match admin_notice_service::delete_notice(&pool, admin.id, id).await {
        Ok(()) => {
            set_flash(&session, "お知らせを削除しました").await;
            Redirect::to("/admin/notices").into_response()
        }
        Err(e) => db_error("notices", e),
    }
}

/// Renders the shared notice form.
/// The form-ticket scope of one notice.
fn notice_scope(id: i64) -> String {
    format!("notice:{id}")
}

/// Mints an edit ticket for the notice at its current
/// stamp, read from the repository (the notice row struct
/// does not carry `updated_at`).
async fn notice_ticket(pool: &PgPool, session: &Session, id: i64) -> String {
    // A failed stamp read is logged and reads as 0, a value
    // no real stamp equals, so the ticket fails closed (the
    // rule `profile_stamp` documents on the member side).
    let stamp = match crate::repositories::notice_repository::updated_at(pool, id).await {
        Ok(stamp) => stamp.map(|t| t.timestamp_micros()).unwrap_or(0),
        Err(e) => {
            tracing::error!("notice stamp read failed: {e}");
            0
        }
    };
    crate::services::form_ticket_service::issue(session, &notice_scope(id), stamp).await
}

async fn form_response(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    input: &NoticeFormInput,
    id: Option<i64>,
    errors: Option<&Vec<FieldError>>,
    form_ticket: String,
) -> Response {
    let err = |field: &str| {
        errors.and_then(|list| {
            list.iter()
                .find(|e| e.field == field)
                .map(|e| e.message.clone())
        })
    };
    render(NoticeFormView {
        layout: layout_for(pool, admin, session).await,
        nav: "notices",
        heading: if id.is_some() {
            "お知らせを編集".to_string()
        } else {
            "お知らせを登録".to_string()
        },
        action: match id {
            Some(id) => format!("/admin/notices/{id}/edit"),
            None => "/admin/notices/new".to_string(),
        },
        delete_action: id.map(|id| format!("/admin/notices/{id}/delete")),
        title: input.title.clone(),
        body: input.body.clone(),
        display_from: input.display_from.clone(),
        display_to: input.display_to.clone(),
        is_visible: input.is_visible,
        title_error: err("title"),
        body_error: err("body"),
        display_from_error: err("display_from"),
        display_to_error: err("display_to"),
        form_ticket,
    })
}
