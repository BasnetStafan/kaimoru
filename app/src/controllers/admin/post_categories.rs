//! Blog categories (CMS blog spec, docs/specs/CMS_SPEC.md).
//!
//! Plain CRUD for the blog's small vocabulary: list,
//! create, edit, delete. The rules live in
//! `crate::services::admin::post_category_service` —
//! validation and the audited writes. Deleting a category
//! never deletes a post: the service relies on the foreign
//! key's SET NULL, so the articles are left uncategorised.

use axum::{
    Form, Router,
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, record_not_found, render, set_flash};
use crate::error::AppError;
use crate::services::admin::post_category_service::{self, PostCategoryInput};
use crate::session::CurrentAdmin;
use crate::validation::{FieldError, field_error};
use crate::views::admin::post_categories::{
    PostCategoryFormView, PostCategoryListView, PostCategoryRow,
};

/// Routes for the post categories surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/post-categories", get(list))
        .route("/admin/post-categories/new", get(new_form).post(create))
        .route(
            "/admin/post-categories/{id}/edit",
            get(edit_form).post(update),
        )
        .route("/admin/post-categories/{id}/delete", post(delete))
}

/// The category form body.
#[derive(Deserialize)]
pub struct PostCategoryForm {
    /// Name as submitted.
    name: String,
    /// Slug as submitted.
    slug: String,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the form, which the
    /// edit submit treats as stale. The create submit
    /// ignores it.
    #[serde(default)]
    form_ticket: String,
}

impl From<&PostCategoryForm> for PostCategoryInput {
    /// Carries the submitted form into the service's input
    /// shape.
    fn from(form: &PostCategoryForm) -> Self {
        Self {
            name: form.name.clone(),
            slug: form.slug.clone(),
        }
    }
}

/// GET /admin/post-categories: every category.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    match post_category_service::list_categories(&pool).await {
        Ok(categories) => {
            let rows = categories
                .iter()
                .map(|c| PostCategoryRow {
                    edit_url: format!("/admin/post-categories/{}/edit", c.id),
                    name: c.name.clone(),
                    slug: c.slug.clone(),
                    delete_url: format!("/admin/post-categories/{}/delete", c.id),
                })
                .collect();
            render(PostCategoryListView {
                layout: layout_for(&pool, &admin, &session).await,
                nav: "post_categories",
                rows,
            })
        }
        Err(e) => db_error("post categories", e),
    }
}

/// GET /admin/post-categories/new: the blank form.
pub async fn new_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    render(PostCategoryFormView::new(
        layout_for(&pool, &admin, &session).await,
        "カテゴリの登録".to_string(),
        "/admin/post-categories/new".to_string(),
        None,
        PostCategoryInput::default(),
    ))
}

/// POST /admin/post-categories/new: create a category.
pub async fn create(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<PostCategoryForm>,
) -> Response {
    let input = PostCategoryInput::from(&form);
    match post_category_service::validate_category(&pool, &input, None).await {
        Ok(valid) => match post_category_service::create_category(&pool, admin.id, &valid).await {
            Ok(id) => {
                set_flash(&session, "カテゴリを登録しました").await;
                Redirect::to(&format!("/admin/post-categories/{id}/edit")).into_response()
            }
            Err(e) => db_error("post category create", e),
        },
        Err(errors) => form_with_errors(&pool, &admin, &session, input, None, &errors).await,
    }
}

/// GET /admin/post-categories/{id}/edit: the editor.
pub async fn edit_form(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match post_category_service::get_category(&pool, id).await {
        Ok(category) => {
            let view = form_view_for(&pool, &admin, &session, &category, None).await;
            render(view)
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("post category", e),
    }
}

/// The form-ticket scope of one category.
fn category_scope(id: i64) -> String {
    format!("post_category:{id}")
}

/// POST /admin/post-categories/{id}/edit: save a category.
pub async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<PostCategoryForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &category_scope(id),
        &form.form_ticket,
    )
    .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/post-categories/{id}/edit")).into_response();
    };
    let input = PostCategoryInput::from(&form);
    match post_category_service::validate_category(&pool, &input, Some(id)).await {
        Ok(valid) => {
            match post_category_service::update_category(&pool, admin.id, id, &valid, expected)
                .await
            {
                Ok(()) => {
                    set_flash(&session, "カテゴリを保存しました").await;
                    Redirect::to(&format!("/admin/post-categories/{id}/edit")).into_response()
                }
                Err(AppError::Conflict(message)) => {
                    set_flash(&session, &message).await;
                    Redirect::to(&format!("/admin/post-categories/{id}/edit")).into_response()
                }
                Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
                Err(e) => db_error("post category update", e),
            }
        }
        Err(errors) => form_with_errors(&pool, &admin, &session, input, Some(id), &errors).await,
    }
}

/// POST /admin/post-categories/{id}/delete: remove a
/// category; its posts stay, uncategorised.
pub async fn delete(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match post_category_service::delete_category(&pool, admin.id, id).await {
        Ok(()) => {
            set_flash(&session, "カテゴリを削除しました").await;
            Redirect::to("/admin/post-categories").into_response()
        }
        Err(e) => db_error("post category delete", e),
    }
}

/// Builds the editor view for a stored category, optionally
/// carrying submitted values from a refused save.
async fn form_view_for(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    category: &crate::models::PostCategory,
    values: Option<PostCategoryInput>,
) -> PostCategoryFormView {
    let mut view = PostCategoryFormView::new(
        layout_for(pool, admin, session).await,
        "カテゴリの編集".to_string(),
        format!("/admin/post-categories/{}/edit", category.id),
        Some(format!("/admin/post-categories/{}/delete", category.id)),
        values.unwrap_or_else(|| PostCategoryInput::from(category)),
    );
    view.form_ticket = crate::services::form_ticket_service::issue(
        session,
        &category_scope(category.id),
        category.updated_at.timestamp_micros(),
    )
    .await;
    view
}

/// Renders the editor again with per-field errors.
async fn form_with_errors(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    input: PostCategoryInput,
    id: Option<i64>,
    errors: &[FieldError],
) -> Response {
    let mut view = match id {
        Some(id) => match post_category_service::get_category(pool, id).await {
            Ok(category) => form_view_for(pool, admin, session, &category, Some(input)).await,
            Err(_) => return record_not_found(pool, admin, session).await,
        },
        None => PostCategoryFormView::new(
            layout_for(pool, admin, session).await,
            "カテゴリの登録".to_string(),
            "/admin/post-categories/new".to_string(),
            None,
            input,
        ),
    };
    view.name_error = field_error(errors, "name");
    view.slug_error = field_error(errors, "slug");
    render(view)
}
