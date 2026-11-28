//! Content pages (CMS spec C-2) and the pages hub
//! (CMS_SPEC §9).
//!
//! The operator's editor for the shop's long-form pages:
//! list, create, edit, preview, revision history and delete.
//! The hub at `/admin/content` is the one home for the
//! three page types — blog posts, landing pages, standard
//! pages — with type-first creation. The rules live in
//! `crate::services::admin::content_page_service` — validation,
//! the snapshot-on-save revision discipline, the delete
//! guards and the audited writes. Handlers here own HTTP
//! concerns only.
//!
//! Success answers with a redirect (Post/Redirect/Get). The
//! preview is the one POST that renders instead: it renders
//! the *unsaved* body through the storefront template inside
//! this admin session and answers `no-store` — there is no
//! preview URL or token a draft could leak through.

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, record_not_found, render, set_flash};
use crate::error::AppError;
use crate::services::admin::content_page_service::{self, ContentPageInput};
use crate::session::CurrentAdmin;
use crate::validation::{FieldError, field_error};
use crate::views::admin::content_pages::{
    ContentFormChrome, ContentPageFormView, ContentPageListView, ContentPageRow,
    ContentRevisionListView, ContentRevisionRow, PagesHubView,
};
use crate::views::storefront::content_page::{ContentView, LandingView};

/// Routes for the content pages surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/content", get(hub))
        .route("/admin/content/pages", get(list).post(create))
        .route("/admin/content/pages/new", get(new_page))
        .route("/admin/content/pages/preview", post(preview))
        .route("/admin/content/pages/{id}", get(edit).post(update))
        .route("/admin/content/pages/{id}/revisions", get(revisions))
        .route(
            "/admin/content/pages/{id}/revisions/{revision_id}/restore",
            post(restore),
        )
        .route("/admin/content/pages/{id}/delete", post(delete))
}

/// The format of a form that carries none: pages predate the
/// choice, and every pre-v2 submission means Markdown.
fn default_page_format() -> String {
    "markdown".to_string()
}

/// The page type of a form that carries none: standard —
/// the only type that existed before the hub (CMS_SPEC §9).
fn default_page_layout() -> String {
    "standard".to_string()
}

/// The page form body: every field a string, exactly as typed
/// — trimming and validation live in the service. The slug is
/// read at creation only; on updates it is ignored entirely
///, so a crafted slug field changes nothing (TC-04).
#[derive(Deserialize)]
pub struct ContentPageForm {
    /// Slug (creation only).
    #[serde(default)]
    slug: String,
    /// Title.
    title: String,
    /// Body source (Markdown, or HTML per `format`).
    body_md: String,
    /// Body format: `markdown` or `html`.
    #[serde(default = "default_page_format")]
    format: String,
    /// Meta description.
    meta_description: String,
    /// Status: `draft` or `published`.
    status: String,
    /// Page type: `standard` or `landing`. Read at creation
    /// only; on updates it is ignored entirely, so a crafted
    /// layout field changes nothing (the type is fixed at
    /// creation, CMS_SPEC §9).
    #[serde(default = "default_page_layout")]
    layout: String,
    /// Sitemap switch: present (with any value) when the
    /// form's checkbox is on, absent when it is off.
    #[serde(default)]
    in_sitemap: Option<String>,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the form, which the
    /// edit submit treats as stale. The create submit
    /// ignores it.
    #[serde(default)]
    form_ticket: String,
}

impl From<ContentPageForm> for ContentPageInput {
    /// Carries the submitted form into the service's input shape.
    fn from(form: ContentPageForm) -> Self {
        Self {
            slug: form.slug,
            title: form.title,
            body_md: form.body_md,
            format: form.format,
            meta_description: form.meta_description,
            status: form.status,
            layout: form.layout,
            in_sitemap: form.in_sitemap.is_some(),
        }
    }
}

/// The preview form body: the editor's unsaved title and body.
#[derive(Deserialize)]
pub struct PreviewForm {
    /// Unsaved title.
    title: String,
    /// Unsaved body source.
    body_md: String,
    /// Format the unsaved body is written in.
    #[serde(default = "default_page_format")]
    format: String,
    /// Page type being edited: a landing page previews in
    /// the landing layout with its embeds expanded, exactly
    /// as publishing will serve it.
    #[serde(default)]
    layout: String,
}

/// The pages list's query: the hub tab's type filter.
#[derive(Deserialize, Default)]
pub struct PageListQuery {
    /// `landing` or `standard`; anything else (or nothing)
    /// lists every page — the pre-hub list's behaviour.
    layout: Option<String>,
}

/// The create form's query: the hub's type preset.
#[derive(Deserialize, Default)]
pub struct NewPageQuery {
    /// `landing` presets the landing editor; anything else
    /// is the standard page editor.
    layout: Option<String>,
}

/// GET /admin/content: the pages hub (CMS_SPEC §9) — the
/// three page types with their counts and type-first
/// create entries.
pub async fn hub(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    let blog_count = match crate::services::admin::post_service::count_posts(&pool).await {
        Ok(n) => n,
        Err(e) => return db_error("pages hub", e),
    };
    let landing_count = match content_page_service::count_pages(&pool, "landing").await {
        Ok(n) => n,
        Err(e) => return db_error("pages hub", e),
    };
    let standard_count = match content_page_service::count_pages(&pool, "standard").await {
        Ok(n) => n,
        Err(e) => return db_error("pages hub", e),
    };
    render(PagesHubView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "pages",
        blog_count,
        landing_count,
        standard_count,
    })
}

/// GET /admin/content/pages: pages, system first; the
/// `layout` query narrows to one page type (the hub's
/// landing / standard tabs).
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<PageListQuery>,
) -> Response {
    let layout_filter = match query.layout.as_deref() {
        Some("landing") => Some("landing"),
        Some("standard") => Some("standard"),
        _ => None,
    };
    match content_page_service::list_pages(&pool, layout_filter).await {
        Ok(summaries) => {
            let rows = summaries
                .iter()
                .map(|s| ContentPageRow {
                    edit_url: format!("/admin/content/pages/{}", s.page.id),
                    title: s.page.title.clone(),
                    url_path: s.page.url_path(),
                    type_label: s.page.type_label(),
                    status_label: s.page.status_label(),
                    published: s.page.is_published(),
                    updated_label: s.page.updated_date_label(),
                    editor_name: s.last_editor_name.clone(),
                    is_system: s.page.is_system(),
                })
                .collect();
            render(ContentPageListView {
                layout: layout_for(&pool, &admin, &session).await,
                nav: "content",
                active_tab: layout_filter.unwrap_or(""),
                rows,
            })
        }
        Err(e) => db_error("content pages", e),
    }
}

/// GET /admin/content/pages/new: the blank editor. The
/// `layout` query presets the page type from the hub's
/// type-first create flow (CMS_SPEC §9): a landing page
/// starts with the sitemap switch off, a standard page
/// with it on.
pub async fn new_page(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<NewPageQuery>,
) -> Response {
    let landing = query.layout.as_deref() == Some("landing");
    let values = ContentPageInput {
        layout: if landing { "landing" } else { "standard" }.to_string(),
        in_sitemap: !landing,
        status: "draft".to_string(),
        format: "markdown".to_string(),
        ..ContentPageInput::default()
    };
    let view = ContentPageFormView::new(
        layout_for(&pool, &admin, &session).await,
        ContentFormChrome {
            heading: if landing {
                "ランディングページの登録".to_string()
            } else {
                "ページの登録".to_string()
            },
            action: "/admin/content/pages".to_string(),
            page_layout: values.layout.clone(),
            slug_editable: true,
            is_system: false,
            revisions_url: None,
            delete_url: None,
        },
        values,
    );
    render(view)
}

/// POST /admin/content/pages: create a custom page.
pub async fn create(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<ContentPageForm>,
) -> Response {
    let input: ContentPageInput = form.into();
    let page = match content_page_service::validate_page(&input, true) {
        Ok(page) => page,
        Err(errors) => {
            return form_with_errors(&pool, &admin, &session, input, None, &errors).await;
        }
    };
    match content_page_service::create_page(&pool, admin.id, &page).await {
        Ok(id) => {
            set_flash(&session, "ページを登録しました").await;
            Redirect::to(&format!("/admin/content/pages/{id}")).into_response()
        }
        Err(AppError::Validation(errors)) => {
            form_with_errors(&pool, &admin, &session, input, None, &errors).await
        }
        Err(e) => db_error("content page create", e),
    }
}

/// GET /admin/content/pages/{id}: the editor for one page.
pub async fn edit(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match content_page_service::get_page(&pool, id).await {
        Ok(page) => {
            let view = form_view_for(&pool, &admin, &session, &page, None).await;
            render(view)
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("content page", e),
    }
}

/// The form-ticket scope of one content page.
fn content_page_scope(id: i64) -> String {
    format!("content_page:{id}")
}

/// POST /admin/content/pages/{id}: save one page. The
/// pre-save state is snapshotted as a revision by the service.
pub async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<ContentPageForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    // Spend the form ticket before validating: a submit
    // without a valid ticket was not rendered from the
    // current record, so it is refused as stale. A ticket
    // whose stamp has gone stale is refused by the
    // service's check instead.
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &content_page_scope(id),
        &form.form_ticket,
    )
    .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/content/pages/{id}")).into_response();
    };
    let input: ContentPageInput = form.into();
    let page = match content_page_service::validate_page(&input, false) {
        Ok(page) => page,
        Err(errors) => {
            return form_with_errors(&pool, &admin, &session, input, Some(id), &errors).await;
        }
    };
    match content_page_service::update_page(&pool, admin.id, id, &page, expected).await {
        Ok(()) => {
            set_flash(&session, "ページを保存しました").await;
            Redirect::to(&format!("/admin/content/pages/{id}")).into_response()
        }
        Err(AppError::Conflict(message)) => {
            set_flash(&session, &message).await;
            Redirect::to(&format!("/admin/content/pages/{id}")).into_response()
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("content page update", e),
    }
}

/// POST /admin/content/pages/preview: render the unsaved body
/// in the storefront layout. Admin session only, `no-store`:
/// a draft of a legal page must never sit in a cache or be
/// reachable by a shared link. The header is the
/// guest header — the preview runs under the admin session,
/// so no member is signed in here.
pub async fn preview(
    State(pool): State<PgPool>,
    CurrentAdmin(_admin): CurrentAdmin,
    session: Session,
    Form(form): Form<PreviewForm>,
) -> Response {
    let header = crate::controllers::storefront::header_for(&pool, &session, false).await;
    // The preview renders exactly what publishing would serve:
    // an HTML body goes through the same sanitiser the save
    // path applies, so the two can never diverge.
    let rendered = if form.format == "html" {
        crate::util::html_sanitize::sanitize_html(&form.body_md)
    } else {
        crate::util::markdown::render_markdown(&form.body_md)
    };
    // A landing page previews in its own layout with its
    // embeds expanded — the real render, like the blog's
    // preview (CMS_SPEC §9). Its meta description is the
    // shop default here: the preview form carries no meta
    // field, and the published page falls back the same way
    // when its own is empty.
    let mut response = if form.layout == "landing" {
        let marked = crate::util::shortcode::mark_embeds(&rendered);
        let body_html = crate::services::blog_service::expand_embeds(&pool, &marked).await;
        crate::controllers::storefront::render(LandingView {
            meta_description: header.shop.meta_description().to_string(),
            header,
            title: form.title,
            body_html,
        })
    } else {
        crate::controllers::storefront::render(ContentView {
            header,
            title: form.title,
            body_html: rendered,
            updated_label: None,
        })
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// GET /admin/content/pages/{id}/revisions: the snapshot list.
pub async fn revisions(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let page = match content_page_service::get_page(&pool, id).await {
        Ok(page) => page,
        Err(AppError::NotFound) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("content page", e),
    };
    match content_page_service::list_revisions(&pool, id).await {
        Ok(summaries) => {
            let rows = summaries
                .iter()
                .map(|s| ContentRevisionRow {
                    created_label: s.revision.created_label(),
                    title: s.revision.title.clone(),
                    editor_name: s.editor_name.clone(),
                    restore_url: format!(
                        "/admin/content/pages/{id}/revisions/{}/restore",
                        s.revision.id
                    ),
                })
                .collect();
            render(ContentRevisionListView {
                layout: layout_for(&pool, &admin, &session).await,
                nav: "content",
                page_title: page.title.clone(),
                edit_url: format!("/admin/content/pages/{id}"),
                rows,
            })
        }
        Err(e) => db_error("content revisions", e),
    }
}

/// POST …/revisions/{revision_id}/restore: bring one snapshot
/// back; the current state is snapshotted first, so the
/// restore is itself undoable.
pub async fn restore(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path((raw_id, raw_revision_id)): Path<(String, String)>,
) -> Response {
    let (Ok(id), Ok(revision_id)) = (raw_id.parse::<i64>(), raw_revision_id.parse::<i64>()) else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match content_page_service::restore_revision(&pool, admin.id, id, revision_id).await {
        Ok(()) => {
            set_flash(&session, "選択した版の内容に戻しました").await;
            Redirect::to(&format!("/admin/content/pages/{id}")).into_response()
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("content revision restore", e),
    }
}

/// POST /admin/content/pages/{id}/delete: remove a draft
/// custom page. The service refuses system pages and
/// published pages; the refusal comes back as a flash.
pub async fn delete(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match content_page_service::delete_page(&pool, admin.id, id).await {
        Ok(()) => {
            set_flash(&session, "ページを削除しました").await;
            Redirect::to("/admin/content/pages").into_response()
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(AppError::Conflict(message)) => {
            set_flash(&session, &message).await;
            Redirect::to("/admin/content/pages").into_response()
        }
        Err(e) => db_error("content page delete", e),
    }
}

/// Builds the editor view for a stored page, optionally
/// carrying validation errors from a refused save (in which
/// case the submitted values, not the stored ones, are shown).
async fn form_view_for(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    page: &crate::models::ContentPage,
    values: Option<ContentPageInput>,
) -> ContentPageFormView {
    let delete_url = if !page.is_system() && !page.is_published() {
        Some(format!("/admin/content/pages/{}/delete", page.id))
    } else {
        None
    };
    let mut view = ContentPageFormView::new(
        layout_for(pool, admin, session).await,
        ContentFormChrome {
            heading: if page.is_landing() {
                "ランディングページの編集".to_string()
            } else if page.is_system() {
                "固定ページの編集".to_string()
            } else {
                "ページの編集".to_string()
            },
            action: format!("/admin/content/pages/{}", page.id),
            page_layout: page.layout.clone(),
            slug_editable: false,
            is_system: page.is_system(),
            revisions_url: Some(format!("/admin/content/pages/{}/revisions", page.id)),
            delete_url,
        },
        values.unwrap_or_else(|| ContentPageInput::from(page)),
    );
    view.form_ticket = crate::services::form_ticket_service::issue(
        session,
        &content_page_scope(page.id),
        page.updated_at.timestamp_micros(),
    )
    .await;
    view
}

/// Renders the editor again with per-field errors: for a
/// create (`id` is `None`) around the submitted values, and
/// for an update around the stored page's chrome (locked
/// slug, delete rules) with the submitted values inside.
async fn form_with_errors(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    input: ContentPageInput,
    id: Option<i64>,
    errors: &[FieldError],
) -> Response {
    let mut view = match id {
        Some(id) => match content_page_service::get_page(pool, id).await {
            Ok(page) => form_view_for(pool, admin, session, &page, Some(input)).await,
            Err(_) => return record_not_found(pool, admin, session).await,
        },
        None => {
            let landing = input.layout == "landing";
            ContentPageFormView::new(
                layout_for(pool, admin, session).await,
                ContentFormChrome {
                    heading: if landing {
                        "ランディングページの登録".to_string()
                    } else {
                        "ページの登録".to_string()
                    },
                    action: "/admin/content/pages".to_string(),
                    page_layout: input.layout.clone(),
                    slug_editable: true,
                    is_system: false,
                    revisions_url: None,
                    delete_url: None,
                },
                input,
            )
        }
    };
    view.layout_error = field_error(errors, "layout");
    view.slug_error = field_error(errors, "slug");
    view.title_error = field_error(errors, "title");
    view.body_error = field_error(errors, "body_md");
    view.format_error = field_error(errors, "format");
    view.meta_error = field_error(errors, "meta_description");
    view.status_error = field_error(errors, "status");
    render(view)
}
