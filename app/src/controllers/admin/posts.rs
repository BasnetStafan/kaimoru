//! Blog posts (CMS blog spec, docs/specs/CMS_SPEC.md).
//!
//! The operator's editor for dated articles: list, create,
//! edit, preview, revision history and delete. The rules
//! live in `crate::services::admin::post_service` —
//! validation, the snapshot-on-save revision discipline and
//! the audited writes. Handlers here own HTTP concerns only.
//!
//! The form is multipart: the cover image rides in the same
//! submit as the words (the CSV imports' shape — admin.js
//! sends multipart forms with the CSRF token in the
//! `x-csrf-token` header, the middleware never reads one
//! out of a multipart body). Success answers with a redirect
//! (Post/Redirect/Get); the preview is the one POST that
//! renders instead, exactly like the content pages'.

use axum::{
    Form, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, record_not_found, render, set_flash};
use crate::error::AppError;
use crate::models::Post;
use crate::services::admin::post_category_service;
use crate::services::admin::post_service::{self, PostInput};
use crate::services::admin::product_service::UploadError;
use crate::session::CurrentAdmin;
use crate::validation::{FieldError, field_error};
use crate::views::admin::posts::{
    CategoryOption, PostFormChrome, PostFormView, PostListView, PostRevisionListView,
    PostRevisionRow, PostRow,
};
use crate::views::admin::{Page, filter_query};
use crate::views::storefront::blog::BlogPostView;

/// Body ceiling for the post form routes: the 20 MB image
/// cap (see `product_service`) plus room for the words and
/// the multipart framing.
const POST_FORM_BODY_LIMIT: usize = 24 * 1024 * 1024;

/// Routes for the posts surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route(
            "/admin/posts",
            get(list)
                .post(create)
                .layer(DefaultBodyLimit::max(POST_FORM_BODY_LIMIT)),
        )
        .route("/admin/posts/new", get(new_post))
        .route("/admin/posts/preview", post(preview))
        .route(
            "/admin/posts/{id}",
            get(edit)
                .post(update)
                .layer(DefaultBodyLimit::max(POST_FORM_BODY_LIMIT)),
        )
        .route("/admin/posts/{id}/revisions", get(revisions))
        .route(
            "/admin/posts/{id}/revisions/{revision_id}/restore",
            post(restore),
        )
        .route("/admin/posts/{id}/delete", post(delete))
}

/// List query: `state` is the derived-state filter
/// (`draft` / `scheduled` / `live`), `page` the page number.
#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// State filter text.
    state: Option<String>,
    /// Page number.
    page: Option<String>,
}

/// GET /admin/posts: the posts list, newest save first.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<ListQuery>,
) -> Response {
    let state = query
        .state
        .as_deref()
        .filter(|s| matches!(*s, "draft" | "scheduled" | "live"))
        .map(str::to_string);
    let (page_no, limit, offset) =
        super::pagination(query.page.as_deref(), crate::config::CONFIG.admin_per_page);
    let (summaries, total) =
        match post_service::list_posts(&pool, state.as_deref(), limit, offset).await {
            Ok(v) => v,
            Err(e) => return db_error("posts", e),
        };
    let now = chrono::Utc::now();
    let rows = summaries
        .iter()
        .map(|s| {
            let derived = s.post.state_at(now);
            PostRow {
                edit_url: format!("/admin/posts/{}", s.post.id),
                title: s.post.title.clone(),
                url_path: s.post.url_path(),
                category_name: s.category_name.clone(),
                state_label: derived.label(),
                live: derived == crate::models::PostState::Live,
                scheduled: derived == crate::models::PostState::Scheduled,
                published_label: s.post.published_label(),
                updated_label: s.post.updated_date_label(),
                author_name: s.author_name.clone(),
                editor_name: s.last_editor_name.clone(),
            }
        })
        .collect();
    let base = Page::base_with(
        "/admin/posts",
        &filter_query(&[("state", state.as_deref())]),
    );
    render(PostListView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "posts",
        state: state.unwrap_or_default(),
        rows,
        page: Page::new(page_no, total, limit, base),
    })
}

/// The cover instruction a parsed form carries, beside the
/// words: the file input, the clear checkbox, or neither
/// (keep the submitted path).
enum CoverAction {
    /// Keep the path the form submitted.
    Keep,
    /// The operator ticked "remove the cover".
    Clear,
    /// A new file was chosen: its bytes and original name.
    Upload(Vec<u8>, Option<String>),
}

/// One parsed multipart submit: the words as a service
/// input, the cover instruction, and the form ticket.
struct ParsedForm {
    /// The words, exactly as typed.
    input: PostInput,
    /// What to do about the cover.
    cover: CoverAction,
    /// The one-time form ticket (edit submits).
    form_ticket: String,
}

/// Reads one multipart submit into a [`ParsedForm`]. A body
/// that does not parse is a 400 with the upload wording —
/// the words never half-arrive.
async fn parse_form(mut multipart: Multipart) -> Result<ParsedForm, Box<Response>> {
    let mut input = PostInput::default();
    let mut cover = CoverAction::Keep;
    let mut form_ticket = String::new();
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let name = field.name().unwrap_or_default().to_string();
                if name == "cover_image" {
                    let original_name = field.file_name().map(str::to_owned);
                    match field.bytes().await {
                        Ok(data) if !data.is_empty() => {
                            cover = CoverAction::Upload(data.to_vec(), original_name);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!("post form body read failed: {e}");
                            return Err(upload_refused(
                                "画像の読み込みに失敗しました。時間をおいてもう一度お試しください",
                            ));
                        }
                    }
                    continue;
                }
                let value = match field.text().await {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("post form field read failed: {e}");
                        return Err(upload_refused(
                            "フォームの読み込みに失敗しました。時間をおいてもう一度お試しください",
                        ));
                    }
                };
                match name.as_str() {
                    "slug" => input.slug = value,
                    "title" => input.title = value,
                    "body_md" => input.body_md = value,
                    "format" => input.format = value,
                    "excerpt" => input.excerpt = value,
                    "meta_description" => input.meta_description = value,
                    "cover_image_path" => input.cover_image_path = value,
                    "category_id" => input.category_id = value,
                    "status" => input.status = value,
                    "published_at" => input.published_at = value,
                    "clear_cover" => {
                        if value == "1" && matches!(cover, CoverAction::Keep) {
                            cover = CoverAction::Clear;
                        }
                    }
                    "form_ticket" => form_ticket = value,
                    _ => {}
                }
            }
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("post form parse failed: {e}");
                return Err(upload_refused(
                    "フォームの読み込みに失敗しました。時間をおいてもう一度お試しください",
                ));
            }
        }
    }
    Ok(ParsedForm {
        input,
        cover,
        form_ticket,
    })
}

/// A refused multipart body as a plain 400 (the product
/// upload's shape): the message is user-facing wording.
fn upload_refused(message: &str) -> Box<Response> {
    Box::new((StatusCode::BAD_REQUEST, message.to_string()).into_response())
}

/// Applies the cover instruction to the input: an upload
/// stores the file through the product machinery and the
/// input takes its path; a clear empties it. A refused
/// upload is the error — the save does not proceed with a
/// cover the operator did not choose.
/// The cover step's outcome: the input to continue with,
/// or the input plus the refusal to show on the form (the
/// typed words are never lost to a refused upload).
enum CoverOutcome {
    /// Continue saving with this input.
    Ready(PostInput),
    /// Re-render the form with this input and message.
    Refused(PostInput, String),
}

/// Applies the cover action of a parsed form: keep, clear,
/// or store the uploaded file through the image machinery.
async fn apply_cover(pool: &PgPool, admin_id: i64, parsed: ParsedForm) -> CoverOutcome {
    let mut input = parsed.input;
    match parsed.cover {
        CoverAction::Keep => CoverOutcome::Ready(input),
        CoverAction::Clear => {
            input.cover_image_path = String::new();
            CoverOutcome::Ready(input)
        }
        CoverAction::Upload(bytes, name) => {
            match post_service::store_cover_image(pool, admin_id, name.as_deref(), &bytes).await {
                Ok(path) => {
                    input.cover_image_path = path;
                    CoverOutcome::Ready(input)
                }
                Err(UploadError::Refused(message)) => CoverOutcome::Refused(input, message),
                Err(UploadError::Internal) => CoverOutcome::Refused(
                    input,
                    "アップロードに失敗しました。時間をおいてもう一度お試しください".to_string(),
                ),
            }
        }
    }
}

/// The category choices for one form render, with the
/// submitted (or stored) category selected.
async fn category_options(pool: &PgPool, selected_raw: &str) -> Vec<CategoryOption> {
    let selected: Option<i64> = selected_raw.trim().parse().ok();
    match post_category_service::list_categories(pool).await {
        Ok(categories) => categories
            .iter()
            .map(|c| CategoryOption {
                id: c.id,
                name: c.name.clone(),
                selected: selected == Some(c.id),
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// GET /admin/posts/new: the blank editor.
pub async fn new_post(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    let options = category_options(&pool, "").await;
    let view = PostFormView::new(
        layout_for(&pool, &admin, &session).await,
        PostFormChrome {
            heading: "記事の登録".to_string(),
            action: "/admin/posts".to_string(),
            slug_editable: true,
            category_options: options,
            revisions_url: None,
            delete_url: None,
        },
        PostInput::default(),
    );
    render(view)
}

/// POST /admin/posts: create a post.
pub async fn create(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    multipart: Multipart,
) -> Response {
    let parsed = match parse_form(multipart).await {
        Ok(p) => p,
        Err(response) => return *response,
    };
    let input = match apply_cover(&pool, admin.id, parsed).await {
        CoverOutcome::Ready(input) => input,
        CoverOutcome::Refused(input, message) => {
            return form_with_errors(&pool, &admin, &session, input, None, &[], Some(message))
                .await;
        }
    };
    let post = match post_service::validate_post(&input, true) {
        Ok(post) => post,
        Err(errors) => {
            return form_with_errors(&pool, &admin, &session, input, None, &errors, None).await;
        }
    };
    match post_service::create_post(&pool, admin.id, &post).await {
        Ok(id) => {
            set_flash(&session, "記事を登録しました").await;
            Redirect::to(&format!("/admin/posts/{id}")).into_response()
        }
        Err(AppError::Validation(errors)) => {
            form_with_errors(&pool, &admin, &session, input, None, &errors, None).await
        }
        Err(e) => db_error("post create", e),
    }
}

/// GET /admin/posts/{id}: the editor for one post.
pub async fn edit(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match post_service::get_post(&pool, id).await {
        Ok(post) => {
            let view = form_view_for(&pool, &admin, &session, &post, None).await;
            render(view)
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("post", e),
    }
}

/// The form-ticket scope of one post.
fn post_scope(id: i64) -> String {
    format!("post:{id}")
}

/// POST /admin/posts/{id}: save one post. The pre-save
/// words are snapshotted as a revision by the service.
pub async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    multipart: Multipart,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let parsed = match parse_form(multipart).await {
        Ok(p) => p,
        Err(response) => return *response,
    };
    // Spend the form ticket before validating (the content
    // pages' order): a submit without a valid ticket was
    // not rendered from the current record.
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &post_scope(id),
        &parsed.form_ticket,
    )
    .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/posts/{id}")).into_response();
    };
    let input = match apply_cover(&pool, admin.id, parsed).await {
        CoverOutcome::Ready(input) => input,
        CoverOutcome::Refused(input, message) => {
            return form_with_errors(&pool, &admin, &session, input, Some(id), &[], Some(message))
                .await;
        }
    };
    let post = match post_service::validate_post(&input, false) {
        Ok(post) => post,
        Err(errors) => {
            return form_with_errors(&pool, &admin, &session, input, Some(id), &errors, None).await;
        }
    };
    match post_service::update_post(&pool, admin.id, id, &post, expected).await {
        Ok(()) => {
            set_flash(&session, "記事を保存しました").await;
            Redirect::to(&format!("/admin/posts/{id}")).into_response()
        }
        Err(AppError::Validation(errors)) => {
            form_with_errors(&pool, &admin, &session, input, Some(id), &errors, None).await
        }
        Err(AppError::Conflict(message)) => {
            set_flash(&session, &message).await;
            Redirect::to(&format!("/admin/posts/{id}")).into_response()
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("post update", e),
    }
}

/// The preview form body: the editor's unsaved words. The
/// preview script sends title / body_md / format plus the
/// form's hidden fields (the cover path among them).
#[derive(Deserialize)]
pub struct PreviewForm {
    /// Unsaved title.
    title: String,
    /// Unsaved body source.
    body_md: String,
    /// Format the unsaved body is written in.
    #[serde(default = "default_post_format")]
    format: String,
    /// The cover path the form carries, when one is set.
    #[serde(default)]
    cover_image_path: String,
}

/// The format of a form that carries none: Markdown.
fn default_post_format() -> String {
    "markdown".to_string()
}

/// POST /admin/posts/preview: render the unsaved body in
/// the storefront post layout. Admin session only,
/// `no-store` — the content pages' preview discipline.
pub async fn preview(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<PreviewForm>,
) -> Response {
    let header = crate::controllers::storefront::header_for(&pool, &session, false).await;
    let body_html = if form.format == "html" {
        crate::util::html_sanitize::sanitize_html(&form.body_md)
    } else {
        crate::util::markdown::render_markdown(&form.body_md)
    };
    let cover = {
        let path = form.cover_image_path.trim();
        if path.is_empty() {
            None
        } else {
            Some(path.to_string())
        }
    };
    let meta_description = header.shop.meta_description().to_string();
    let mut response = crate::controllers::storefront::render(BlogPostView {
        header,
        title: form.title,
        date_label: chrono::Utc::now()
            .with_timezone(&crate::models::jst::jst())
            .format("%Y年%-m月%-d日")
            .to_string(),
        author_name: admin.display_name.clone(),
        category_name: None,
        category_url: None,
        cover_image_path: cover,
        body_html,
        meta_description,
    });
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// GET /admin/posts/{id}/revisions: the snapshot list.
pub async fn revisions(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let post = match post_service::get_post(&pool, id).await {
        Ok(post) => post,
        Err(AppError::NotFound) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("post", e),
    };
    match post_service::list_revisions(&pool, id).await {
        Ok(summaries) => {
            let rows = summaries
                .iter()
                .map(|s| PostRevisionRow {
                    created_label: s.revision.created_label(),
                    title: s.revision.title.clone(),
                    editor_name: s.editor_name.clone(),
                    restore_url: format!("/admin/posts/{id}/revisions/{}/restore", s.revision.id),
                })
                .collect();
            render(PostRevisionListView {
                layout: layout_for(&pool, &admin, &session).await,
                nav: "posts",
                post_title: post.title.clone(),
                edit_url: format!("/admin/posts/{id}"),
                rows,
            })
        }
        Err(e) => db_error("post revisions", e),
    }
}

/// POST …/revisions/{revision_id}/restore: bring one
/// snapshot's words back; the current words are
/// snapshotted first, so the restore is itself undoable.
pub async fn restore(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path((raw_id, raw_revision_id)): Path<(String, String)>,
) -> Response {
    let (Ok(id), Ok(revision_id)) = (raw_id.parse::<i64>(), raw_revision_id.parse::<i64>()) else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match post_service::restore_revision(&pool, admin.id, id, revision_id).await {
        Ok(()) => {
            set_flash(&session, "選択した版の内容に戻しました").await;
            Redirect::to(&format!("/admin/posts/{id}")).into_response()
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("post revision restore", e),
    }
}

/// POST /admin/posts/{id}/delete: remove a post and its
/// revisions. The form's native confirm is the guard.
pub async fn delete(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match post_service::delete_post(&pool, admin.id, id).await {
        Ok(()) => {
            set_flash(&session, "記事を削除しました").await;
            Redirect::to("/admin/posts").into_response()
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("post delete", e),
    }
}

/// Builds the editor view for a stored post, optionally
/// carrying submitted values from a refused save.
async fn form_view_for(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    post: &Post,
    values: Option<PostInput>,
) -> PostFormView {
    let values = values.unwrap_or_else(|| PostInput::from(post));
    let options = category_options(pool, &values.category_id).await;
    let mut view = PostFormView::new(
        layout_for(pool, admin, session).await,
        PostFormChrome {
            heading: "記事の編集".to_string(),
            action: format!("/admin/posts/{}", post.id),
            slug_editable: false,
            category_options: options,
            revisions_url: Some(format!("/admin/posts/{}/revisions", post.id)),
            delete_url: Some(format!("/admin/posts/{}/delete", post.id)),
        },
        values,
    );
    view.form_ticket = crate::services::form_ticket_service::issue(
        session,
        &post_scope(post.id),
        post.updated_at.timestamp_micros(),
    )
    .await;
    view
}

/// Renders the editor again with per-field errors: for a
/// create (`id` is `None`) around the submitted values, and
/// for an update around the stored post's chrome with the
/// submitted values inside. A cover refusal arrives as
/// `cover_error` with the rest of the form intact — the
/// submitted values are re-rendered, never the stored ones,
/// so no typed word is lost.
async fn form_with_errors(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    input: PostInput,
    id: Option<i64>,
    errors: &[FieldError],
    cover_error: Option<String>,
) -> Response {
    let mut view = match id {
        Some(id) => match post_service::get_post(pool, id).await {
            Ok(post) => form_view_for(pool, admin, session, &post, Some(input)).await,
            Err(_) => return record_not_found(pool, admin, session).await,
        },
        None => {
            let options = category_options(pool, &input.category_id).await;
            PostFormView::new(
                layout_for(pool, admin, session).await,
                PostFormChrome {
                    heading: "記事の登録".to_string(),
                    action: "/admin/posts".to_string(),
                    slug_editable: true,
                    category_options: options,
                    revisions_url: None,
                    delete_url: None,
                },
                input,
            )
        }
    };
    view.slug_error = field_error(errors, "slug");
    view.title_error = field_error(errors, "title");
    view.body_error = field_error(errors, "body_md");
    view.format_error = field_error(errors, "format");
    view.excerpt_error = field_error(errors, "excerpt");
    view.meta_error = field_error(errors, "meta_description");
    view.category_error = field_error(errors, "category_id");
    view.status_error = field_error(errors, "status");
    view.published_error = field_error(errors, "published_at");
    view.cover_error = cover_error;
    render(view)
}
