//! Category management.
//!
//! Categories are the store's managed vocabulary: the product
//! form's select, the top page navigation and the search
//! filter all read the same table. Deleting a category that
//! still has products or children is refused by the service
//! with the reason, which lands here as the flash — the
//! operator's retirement path for a used category is the
//! active flag, not deletion.

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
use crate::services::admin::category_csv_service;
use crate::services::admin::category_service::{self, CategoryDelete, CategoryFormInput};
use crate::session::CurrentAdmin;
use crate::util::csv_transfer::BatchReport;
use crate::validation::FieldError;
use crate::views::admin::Page;
use crate::views::admin::categories::{
    CategoryCsvImportView, CategoryCsvPreviewView, CategoryFormView, CategoryListView,
    CategorySortOption, CategoryStatusOption, ParentOption, rows_for,
};
use crate::views::admin::filter_query;

/// The status filter options.
const STATUSES: [(&str, &str); 3] = [("", "すべて"), ("active", "有効"), ("inactive", "無効")];

/// The sort options: display order is the default (it is the
/// storefront navigation's order).
const SORTS: [(&str, &str); 3] = [
    ("order", "表示順"),
    ("name", "名前順"),
    ("products", "商品数順"),
];

/// Routes for the categories surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/categories", get(list))
        .route("/admin/categories/new", get(new_form).post(create))
        .route("/admin/categories/{id}/edit", get(edit_form).post(update))
        .route("/admin/categories/{id}/delete", post(delete))
        .route("/admin/categories/csv-export", get(csv_export))
        .route(
            "/admin/categories/csv-import",
            get(csv_import_form)
                .post(csv_upload)
                .layer(DefaultBodyLimit::max(CSV_BODY_LIMIT)),
        )
        .route("/admin/categories/csv-import/{id}", get(csv_preview))
        .route("/admin/categories/csv-import/{id}/apply", post(csv_apply))
        .route(
            "/admin/categories/csv-import/{id}/discard",
            post(csv_discard),
        )
}

/// List query parameters.
#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// Name/slug search text.
    q: Option<String>,
    /// Status filter value.
    status: Option<String>,
    /// Sort key.
    sort: Option<String>,
    /// Page number.
    page: Option<String>,
}

/// The category list: name/slug search, status filter, sorting,
/// pagination — the same furniture as the other admin lists.
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
    let sort = query
        .sort
        .as_deref()
        .filter(|s| SORTS.iter().any(|(v, _)| v == s))
        .unwrap_or("order")
        .to_string();
    let (page_no, limit, offset) = pagination(query.page.as_deref(), CONFIG.admin_per_page);
    let status_param = if status.is_empty() {
        None
    } else {
        Some(status.as_str())
    };
    let (rows, total) = match category_service::list_categories(
        &pool,
        q.as_deref(),
        status_param,
        &sort,
        limit,
        offset,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return db_error("categories", e),
    };
    let page_text = page_no.to_string();
    let return_url = format!(
        "/admin/categories?{}",
        filter_query(&[
            ("q", q.as_deref()),
            ("status", status_param),
            ("sort", Some(sort.as_str())),
            ("page", (page_no > 1).then_some(page_text.as_str())),
        ])
    );
    let base = Page::base_with(
        "/admin/categories",
        &filter_query(&[
            ("q", q.as_deref()),
            ("status", status_param),
            ("sort", Some(sort.as_str())),
        ]),
    );
    render(CategoryListView {
        return_url,
        layout: layout_for(&pool, &admin, &session).await,
        nav: "categories",
        rows: rows_for(&rows),
        page: Page::new(page_no, total, limit, base),
        q: q.clone().unwrap_or_default(),
        status_options: STATUSES
            .iter()
            .map(|(v, l)| CategoryStatusOption {
                value: v,
                label: l,
                selected: status == *v,
            })
            .collect(),
        sort_options: SORTS
            .iter()
            .map(|(v, l)| CategorySortOption {
                value: v,
                label: l,
                selected: sort == *v,
            })
            .collect(),
        filtered: q.is_some() || !status.is_empty(),
    })
}

/// The category form body. The checkbox arrives as a value
/// when checked and not at all when unchecked.
#[derive(Deserialize)]
pub struct CategoryForm {
    /// Name as submitted.
    name: String,
    /// Slug as submitted.
    slug: String,
    /// Description as submitted.
    #[serde(default)]
    description: String,
    /// Parent category ID as submitted; empty at the top level.
    #[serde(default)]
    parent_id: String,
    /// Display order as submitted; empty means 0.
    #[serde(default)]
    display_order: String,
    /// The 有効 checkbox value, when checked.
    #[serde(default)]
    is_active: Option<String>,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the form, which the
    /// edit submit treats as stale. The create submit
    /// ignores it.
    #[serde(default)]
    form_ticket: String,
}

impl From<&CategoryForm> for CategoryFormInput {
    fn from(f: &CategoryForm) -> Self {
        CategoryFormInput {
            name: f.name.clone(),
            slug: f.slug.clone(),
            description: f.description.clone(),
            parent_id: f.parent_id.clone(),
            display_order: f.display_order.clone(),
            is_active: f.is_active.is_some(),
        }
    }
}

/// A blank create form: active by default.
pub async fn new_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    form_response(
        &pool,
        &admin,
        &session,
        &CategoryFormInput {
            is_active: true,
            ..CategoryFormInput::default()
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
    Form(form): Form<CategoryForm>,
) -> Response {
    let input = CategoryFormInput::from(&form);
    match category_service::validate_category(&pool, &input, None).await {
        Ok(valid) => match category_service::create_category(&pool, admin.id, &valid).await {
            Ok(id) => {
                set_flash(&session, "カテゴリーを登録しました").await;
                Redirect::to(&format!("/admin/categories/{id}/edit")).into_response()
            }
            Err(e) => db_error("categories", e),
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

/// The edit form for one category.
pub async fn edit_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match category_service::find_category(&pool, id).await {
        Ok(Some(category)) => {
            let input = CategoryFormInput {
                name: category.name.clone(),
                slug: category.slug.clone(),
                description: category.description.clone(),
                parent_id: category
                    .parent_id
                    .map(|p| p.to_string())
                    .unwrap_or_default(),
                display_order: category.display_order.to_string(),
                is_active: category.is_active,
            };
            let form_ticket =
                category_ticket(&session, id, category.updated_at.timestamp_micros()).await;
            form_response(&pool, &admin, &session, &input, Some(id), None, form_ticket).await
        }
        Ok(None) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("categories", e),
    }
}

/// Handles the edit submit.
pub async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<CategoryForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let previous = match category_service::find_category(&pool, id).await {
        Ok(Some(category)) => category,
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("categories", e),
    };
    // Spend the form ticket before validating: a submit
    // without a valid ticket was not rendered from the
    // current record, so it is refused as stale (the flash
    // explains, the edit page shows the current values). A
    // ticket whose stamp has gone stale is refused by the
    // service's check instead.
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &category_scope(id),
        &form.form_ticket,
    )
    .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/categories/{id}/edit")).into_response();
    };
    let input = CategoryFormInput::from(&form);
    match category_service::validate_category(&pool, &input, Some(id)).await {
        Ok(valid) => {
            match category_service::update_category(
                &pool,
                admin.id,
                id,
                &valid,
                &previous.name,
                Some(expected),
            )
            .await
            {
                Ok(()) => {
                    set_flash(&session, "カテゴリーを更新しました").await;
                    Redirect::to(&format!("/admin/categories/{id}/edit")).into_response()
                }
                Err(crate::error::AppError::Conflict(message)) => {
                    set_flash(&session, &message).await;
                    Redirect::to(&format!("/admin/categories/{id}/edit")).into_response()
                }
                Err(e) => db_error("categories", e),
            }
        }
        Err(errors) => {
            let form_ticket =
                category_ticket(&session, id, previous.updated_at.timestamp_micros()).await;
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

/// Deletes one category. A referenced category is refused by
/// the service; its reason becomes the flash, so the operator
/// learns why and what to do instead (deactivate it).
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
    match category_service::delete_category(&pool, admin.id, id).await {
        Ok(CategoryDelete::Deleted) => {
            set_flash(&session, "カテゴリーを削除しました").await;
            Redirect::to(&safe_return_url(&pairs, "/admin/categories")).into_response()
        }
        Ok(CategoryDelete::Blocked(reason)) => {
            set_flash(&session, &reason).await;
            Redirect::to(&safe_return_url(&pairs, "/admin/categories")).into_response()
        }
        Err(e) => db_error("categories", e),
    }
}

/// Renders the shared category form, including the parent
/// choices (every category except the one being edited and its
/// descendants — the cycle guard as a picker).
/// The form-ticket scope of one category.
fn category_scope(id: i64) -> String {
    format!("category:{id}")
}

/// Mints an edit ticket for the category at `stamp` (epoch
/// microseconds of its `updated_at`).
async fn category_ticket(session: &Session, id: i64, stamp: i64) -> String {
    crate::services::form_ticket_service::issue(session, &category_scope(id), stamp).await
}

async fn form_response(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    input: &CategoryFormInput,
    id: Option<i64>,
    errors: Option<&Vec<FieldError>>,
    form_ticket: String,
) -> Response {
    let parents = match category_service::parent_options(pool, id).await {
        Ok(v) => v,
        Err(e) => return db_error("categories", e),
    };
    let selected_parent = input.parent_id.trim().parse::<i64>().ok();
    let err = |field: &str| {
        errors.and_then(|list| {
            list.iter()
                .find(|e| e.field == field)
                .map(|e| e.message.clone())
        })
    };
    render(CategoryFormView {
        layout: layout_for(pool, admin, session).await,
        nav: "categories",
        heading: if id.is_some() {
            "カテゴリーを編集".to_string()
        } else {
            "カテゴリーを登録".to_string()
        },
        action: match id {
            Some(id) => format!("/admin/categories/{id}/edit"),
            None => "/admin/categories/new".to_string(),
        },
        delete_action: id.map(|id| format!("/admin/categories/{id}/delete")),
        name: input.name.clone(),
        slug: input.slug.clone(),
        description: input.description.clone(),
        parent_options: parents
            .iter()
            .map(|c| ParentOption {
                id: c.id,
                name: c.name.clone(),
                selected: selected_parent == Some(c.id),
            })
            .collect(),
        display_order: input.display_order.clone(),
        is_active: input.is_active,
        name_error: err("name"),
        slug_error: err("slug"),
        description_error: err("description"),
        parent_error: err("parent_id"),
        display_order_error: err("display_order"),
        form_ticket,
    })
}

// ------------------------------------------------------------------
// CSV import/export (category CSV service): download, upload,
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
    match category_csv_service::export_csv(&pool).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"categories.csv\"",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => db_error("category csv export", e),
    }
}

/// The CSV upload form page.
pub async fn csv_import_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    render(CategoryCsvImportView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "categories",
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
        return Redirect::to("/admin/categories/csv-import").into_response();
    };
    if bytes.len() as i64 > CONFIG.csv_max_file_bytes {
        set_flash(&session, CSV_FILE_TOO_LARGE_MESSAGE).await;
        return Redirect::to("/admin/categories/csv-import").into_response();
    }
    match category_csv_service::store_upload(&pool, admin.id, &name, &bytes).await {
        Ok(id) => Redirect::to(&format!("/admin/categories/csv-import/{id}")).into_response(),
        Err(e) => db_error("category csv upload", e),
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
        Ok(Some(batch)) if batch.entity == "category" => batch,
        Ok(_) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("category csv preview", e),
    };
    let report = BatchReport::from_json(&batch.report);
    render(CategoryCsvPreviewView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "categories",
        batch_id: batch.id,
        file_name: batch.file_name,
        status: batch.status,
        new_count: report.new_count,
        update_count: report.update_count,
        error_count: report.error_count(),
        errors: report.errors,
    })
}

/// Applies one preview batch through the category save path.
pub async fn csv_apply(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(id): Path<String>,
) -> Response {
    let Ok(id) = id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match category_csv_service::apply_batch(&pool, admin.id, id).await {
        Ok((created, updated)) => {
            set_flash(&session, &csv_import_applied_message(created, updated)).await;
            Redirect::to("/admin/categories").into_response()
        }
        Err(crate::error::AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("category csv apply", e),
    }
}

/// Discards one preview batch; no category is touched.
pub async fn csv_discard(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(id): Path<String>,
) -> Response {
    let Ok(id) = id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match category_csv_service::discard_batch(&pool, id).await {
        Ok(()) => {
            set_flash(&session, CSV_IMPORT_DISCARDED_NOTICE).await;
            Redirect::to("/admin/categories").into_response()
        }
        Err(crate::error::AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("category csv discard", e),
    }
}
