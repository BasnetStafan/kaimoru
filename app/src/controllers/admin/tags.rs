//! Tag management.
//!
//! Tags are the controlled vocabulary the product form's
//! checkboxes and the needs matching read. Deleting a
//! tag removes its product links in the same transaction (the
//! service), so a product is never left pointing at a tag that
//! no longer exists.

use axum::{
    Form, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::header,
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
use crate::messages::{
    CSV_FILE_REQUIRED_MESSAGE, CSV_FILE_TOO_LARGE_MESSAGE, CSV_IMPORT_DISCARDED_NOTICE,
    csv_import_applied_message,
};
use crate::models::AdminIdentity;
use crate::repositories::csv_import_batch_repository;
use crate::services::admin::tag_csv_service;
use crate::services::admin::tag_service::{self, TagFormInput};
use crate::session::CurrentAdmin;
use crate::util::csv_transfer::BatchReport;
use crate::validation::FieldError;
use crate::views::admin::Page;
use crate::views::admin::filter_query;
use crate::views::admin::tags::{
    TagCsvImportView, TagCsvPreviewView, TagFormView, TagListView, TagSortOption, rows_for,
};

/// The sort options: registration (seed) order is the default —
/// it is the product form's checkbox order.
const SORTS: [(&str, &str); 4] = [
    ("id", "登録順"),
    ("label", "名前順"),
    ("slug", "スラッグ順"),
    ("products", "商品数順"),
];

/// Routes for the tags surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/tags", get(list))
        .route("/admin/tags/new", get(new_form).post(create))
        .route("/admin/tags/{id}/edit", get(edit_form).post(update))
        .route("/admin/tags/{id}/delete", post(delete))
        .route("/admin/tags/csv-export", get(csv_export))
        .route(
            "/admin/tags/csv-import",
            get(csv_import_form)
                .post(csv_upload)
                .layer(DefaultBodyLimit::max(CSV_BODY_LIMIT)),
        )
        .route("/admin/tags/csv-import/{id}", get(csv_preview))
        .route("/admin/tags/csv-import/{id}/apply", post(csv_apply))
        .route("/admin/tags/csv-import/{id}/discard", post(csv_discard))
}

/// List query parameters.
#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// Label/slug search text.
    q: Option<String>,
    /// Sort key.
    sort: Option<String>,
    /// Page number.
    page: Option<String>,
}

/// The tag list: label/slug search, sorting, pagination.
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
    let sort = query
        .sort
        .as_deref()
        .filter(|s| SORTS.iter().any(|(v, _)| v == s))
        .unwrap_or("id")
        .to_string();
    let (page_no, limit, offset) = pagination(query.page.as_deref(), CONFIG.admin_per_page);
    let (rows, total) =
        match tag_service::list_tags(&pool, q.as_deref(), &sort, limit, offset).await {
            Ok(v) => v,
            Err(e) => return db_error("tags", e),
        };
    let page_text = page_no.to_string();
    let return_url = format!(
        "/admin/tags?{}",
        filter_query(&[
            ("q", q.as_deref()),
            ("sort", Some(sort.as_str())),
            ("page", (page_no > 1).then_some(page_text.as_str())),
        ])
    );
    let base = Page::base_with(
        "/admin/tags",
        &filter_query(&[("q", q.as_deref()), ("sort", Some(sort.as_str()))]),
    );
    render(TagListView {
        return_url,
        layout: layout_for(&pool, &admin, &session).await,
        nav: "tags",
        rows: rows_for(&rows),
        page: Page::new(page_no, total, limit, base),
        q: q.clone().unwrap_or_default(),
        sort_options: SORTS
            .iter()
            .map(|(v, l)| TagSortOption {
                value: v,
                label: l,
                selected: sort == *v,
            })
            .collect(),
        filtered: q.is_some(),
    })
}

/// The tag form body.
#[derive(Deserialize)]
pub struct TagForm {
    /// Label as submitted.
    label: String,
    /// Slug as submitted.
    slug: String,
    /// Description as submitted.
    #[serde(default)]
    description: String,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the form, which the
    /// edit submit treats as stale. The create submit
    /// ignores it.
    #[serde(default)]
    form_ticket: String,
}

impl From<&TagForm> for TagFormInput {
    fn from(f: &TagForm) -> Self {
        TagFormInput {
            label: f.label.clone(),
            slug: f.slug.clone(),
            description: f.description.clone(),
        }
    }
}

/// A blank create form.
pub async fn new_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    form_response(
        &pool,
        &admin,
        &session,
        &TagFormInput::default(),
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
    Form(form): Form<TagForm>,
) -> Response {
    let input = TagFormInput::from(&form);
    match tag_service::validate_tag(&pool, &input, None).await {
        Ok(valid) => match tag_service::create_tag(&pool, admin.id, &valid).await {
            Ok(id) => {
                set_flash(&session, "タグを登録しました").await;
                Redirect::to(&format!("/admin/tags/{id}/edit")).into_response()
            }
            Err(e) => db_error("tags", e),
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

/// The edit form for one tag.
pub async fn edit_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match tag_service::find_tag(&pool, id).await {
        Ok(Some(tag)) => {
            let input = TagFormInput {
                label: tag.label.clone(),
                slug: tag.slug.clone(),
                description: tag.description.clone(),
            };
            let form_ticket = tag_ticket(&pool, &session, id).await;
            form_response(&pool, &admin, &session, &input, Some(id), None, form_ticket).await
        }
        Ok(None) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("tags", e),
    }
}

/// Handles the edit submit.
pub async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<TagForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match tag_service::find_tag(&pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("tags", e),
    }
    // Spend the form ticket before validating: a submit
    // without a valid ticket was not rendered from the
    // current record, so it is refused as stale. A ticket
    // whose stamp has gone stale is refused by the
    // service's check instead.
    let Some(expected) =
        crate::services::form_ticket_service::consume(&session, &tag_scope(id), &form.form_ticket)
            .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/tags/{id}/edit")).into_response();
    };
    let input = TagFormInput::from(&form);
    match tag_service::validate_tag(&pool, &input, Some(id)).await {
        Ok(valid) => {
            match tag_service::update_tag(&pool, admin.id, id, &valid, Some(expected)).await {
                Ok(()) => {
                    set_flash(&session, "タグを更新しました").await;
                    Redirect::to(&format!("/admin/tags/{id}/edit")).into_response()
                }
                Err(crate::error::AppError::Conflict(message)) => {
                    set_flash(&session, &message).await;
                    Redirect::to(&format!("/admin/tags/{id}/edit")).into_response()
                }
                Err(e) => db_error("tags", e),
            }
        }
        Err(errors) => {
            let form_ticket = tag_ticket(&pool, &session, id).await;
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

/// Deletes one tag. The service removes the tag's product
/// links in the same transaction, so products are never left
/// with a dangling reference.
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
    match tag_service::delete_tag(&pool, admin.id, id).await {
        Ok(()) => {
            set_flash(&session, "タグを削除しました").await;
            Redirect::to(&safe_return_url(&pairs, "/admin/tags")).into_response()
        }
        Err(e) => db_error("tags", e),
    }
}

/// Renders the shared tag form.
/// The form-ticket scope of one tag.
fn tag_scope(id: i64) -> String {
    format!("tag:{id}")
}

/// Mints an edit ticket for the tag at its current stamp,
/// read from the repository (the `Tag` row struct does not
/// carry `updated_at`).
async fn tag_ticket(pool: &PgPool, session: &Session, id: i64) -> String {
    // A failed stamp read is logged and reads as 0, a value
    // no real stamp equals, so the ticket fails closed (the
    // rule `profile_stamp` documents on the member side).
    let stamp = match crate::repositories::tag_repository::updated_at(pool, id).await {
        Ok(stamp) => stamp.map(|t| t.timestamp_micros()).unwrap_or(0),
        Err(e) => {
            tracing::error!("tag stamp read failed: {e}");
            0
        }
    };
    crate::services::form_ticket_service::issue(session, &tag_scope(id), stamp).await
}

async fn form_response(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    input: &TagFormInput,
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
    render(TagFormView {
        layout: layout_for(pool, admin, session).await,
        nav: "tags",
        heading: if id.is_some() {
            "タグを編集".to_string()
        } else {
            "タグを登録".to_string()
        },
        action: match id {
            Some(id) => format!("/admin/tags/{id}/edit"),
            None => "/admin/tags/new".to_string(),
        },
        delete_action: id.map(|id| format!("/admin/tags/{id}/delete")),
        label: input.label.clone(),
        slug: input.slug.clone(),
        description: input.description.clone(),
        label_error: err("label"),
        slug_error: err("slug"),
        description_error: err("description"),
        form_ticket,
    })
}

// ------------------------------------------------------------------
// CSV import/export (tag CSV service): download, upload,
// preview, apply, discard. All POSTs ride the admin surface
// stack like every other admin form.
// ------------------------------------------------------------------

/// Body ceiling for the CSV upload routes: the configured
/// file ceiling (10 MiB by default) plus room for the
/// multipart framing. The handler's check against
/// `csv_max_file_bytes` reports the real limit.
const CSV_BODY_LIMIT: usize = 12 * 1024 * 1024;

/// CSV export download: the whole vocabulary, in ID order.
pub async fn csv_export(
    State(pool): State<PgPool>,
    CurrentAdmin(_admin): CurrentAdmin,
) -> Response {
    match tag_csv_service::export_csv(&pool).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"tags.csv\"",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => db_error("tag csv export", e),
    }
}

/// The CSV upload form page.
pub async fn csv_import_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    render(TagCsvImportView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "tags",
    })
}

/// Receives one CSV upload (multipart, field `file`):
/// parses and validates it into a stored preview batch,
/// then redirects to the preview page — including for a
/// file whose problems are all content problems, which the
/// preview page explains line by line.
pub async fn csv_upload(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    mut multipart: Multipart,
) -> Response {
    let mut file: Option<(String, Vec<u8>)> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let name = field
            .file_name()
            .map(str::to_string)
            .unwrap_or_else(|| "import.csv".to_string());
        match field.bytes().await {
            Ok(bytes) => file = Some((name, bytes.to_vec())),
            Err(_) => break,
        }
    }
    let Some((name, bytes)) = file.filter(|(_, bytes)| !bytes.is_empty()) else {
        set_flash(&session, CSV_FILE_REQUIRED_MESSAGE).await;
        return Redirect::to("/admin/tags/csv-import").into_response();
    };
    if bytes.len() as i64 > CONFIG.csv_max_file_bytes {
        set_flash(&session, CSV_FILE_TOO_LARGE_MESSAGE).await;
        return Redirect::to("/admin/tags/csv-import").into_response();
    }
    match tag_csv_service::store_upload(&pool, admin.id, &name, &bytes).await {
        Ok(id) => Redirect::to(&format!("/admin/tags/csv-import/{id}")).into_response(),
        Err(e) => db_error("tag csv upload", e),
    }
}

/// The preview page for one stored batch: the validation
/// report's counts and per-line errors, with the apply and
/// discard actions while the batch is still a preview.
pub async fn csv_preview(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(id): Path<String>,
) -> Response {
    let Ok(id) = id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let batch = match csv_import_batch_repository::find_batch(&pool, id).await {
        Ok(Some(batch)) if batch.entity == "tag" => batch,
        Ok(_) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("tag csv preview", e),
    };
    let report = BatchReport::from_json(&batch.report);
    render(TagCsvPreviewView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "tags",
        batch_id: batch.id,
        file_name: batch.file_name,
        status: batch.status,
        new_count: report.new_count,
        update_count: report.update_count,
        error_count: report.error_count(),
        errors: report.errors,
    })
}

/// Applies one preview batch through the tag save path.
pub async fn csv_apply(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(id): Path<String>,
) -> Response {
    let Ok(id) = id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match tag_csv_service::apply_batch(&pool, admin.id, id).await {
        Ok((created, updated)) => {
            set_flash(&session, &csv_import_applied_message(created, updated)).await;
            Redirect::to("/admin/tags").into_response()
        }
        Err(crate::error::AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("tag csv apply", e),
    }
}

/// Discards one preview batch; no tag is touched.
pub async fn csv_discard(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(id): Path<String>,
) -> Response {
    let Ok(id) = id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match tag_csv_service::discard_batch(&pool, id).await {
        Ok(()) => {
            set_flash(&session, CSV_IMPORT_DISCARDED_NOTICE).await;
            Redirect::to("/admin/tags").into_response()
        }
        Err(crate::error::AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("tag csv discard", e),
    }
}
