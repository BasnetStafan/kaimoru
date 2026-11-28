//! Admin content page rules (CMS spec C-2).
//!
//! The operator edits the shop's long-form words — guide,
//! terms, privacy, FAQ and pages of their own — as Markdown
//! or as HTML written directly, which the save path sanitises
//! before storing ([`crate::util::html_sanitize`]).
//! Every save of an existing page first snapshots the state it
//! replaces into `content_revisions` (capped at the newest 50),
//! so no edit is ever the only copy of a legal text. Slugs are
//! created once and never change; system pages cannot be
//! deleted, here or in the UI. Bodies and titles never reach a
//! log line — only action names and record IDs do.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{ContentPage, ContentPageSummary, ContentRevisionSummary, NewContentPage};
use crate::repositories::content_page_repository;
use crate::services::admin::audit_service;
use crate::util::html_sanitize;
use crate::util::markdown;
use crate::validation::FieldError;

/// Title limit, in characters.
const TITLE_MAX_CHARS: usize = 80;
/// Body limit, in characters.
const BODY_MAX_CHARS: usize = 50_000;
/// Meta description limit, in characters.
const META_MAX_CHARS: usize = 160;
/// Slug length bounds: layered here, over the shared
/// slug helper's character shape, because that helper's own
/// bounds (1–50) serve categories and tags — the page rule is
/// 3–60, and neither rule may silently rewrite the other.
const SLUG_MIN_CHARS: usize = 3;
/// Slug maximum length, in characters.
const SLUG_MAX_CHARS: usize = 60;

/// Slugs an operator may not claim: every existing top-level
/// route segment plus the system pages' slugs. A
/// custom page lives under `/pages/`, so none of these could
/// shadow a route today — the list keeps the names honest if a
/// page is ever promoted to a top-level route, and stops an
/// operator minting a second "guide".
const RESERVED_SLUGS: &[&str] = &[
    "admin",
    "api",
    "cart",
    "checkout",
    "contact",
    "faq",
    "guide",
    "healthz",
    "legal",
    "login",
    "logout",
    "mypage",
    "orders",
    "pages",
    "password-reset",
    "privacy",
    "products",
    "reviews",
    "search",
    "signup",
    "static",
    "terms",
    "webhooks",
    "withdrawn",
];

/// The raw content page form as submitted: every value a
/// string, exactly as typed. `slug` is read at creation only;
/// the update path never looks at it. `layout` is likewise
/// creation-only (CMS_SPEC §9): the update path never writes
/// it, so no save can move a page between namespaces.
#[derive(Clone, Default)]
pub struct ContentPageInput {
    /// Slug as typed (creation only).
    pub slug: String,
    /// Title as typed.
    pub title: String,
    /// Body source as typed (Markdown, or HTML when
    /// [`Self::format`] is `html`).
    pub body_md: String,
    /// Body format as chosen: `markdown` or `html`.
    pub format: String,
    /// Meta description as typed.
    pub meta_description: String,
    /// Status as typed: `draft` or `published`.
    pub status: String,
    /// Page type as preset by the create flow: `standard`
    /// or `landing` (creation only).
    pub layout: String,
    /// Whether the sitemap may list the page (the form's
    /// checkbox).
    pub in_sitemap: bool,
}

impl From<&ContentPage> for ContentPageInput {
    /// The editor's initial values: the stored page, verbatim.
    fn from(page: &ContentPage) -> Self {
        Self {
            slug: page.slug.clone(),
            title: page.title.clone(),
            body_md: page.body_md.clone(),
            format: page.format.clone(),
            meta_description: page.meta_description.clone(),
            status: page.status.clone(),
            layout: page.layout.clone(),
            in_sitemap: page.in_sitemap,
        }
    }
}

/// A page form that passed validation, ready to store.
#[derive(Debug)]
pub struct ValidatedPage {
    /// Trimmed slug (creation only; empty on updates).
    pub slug: String,
    /// Trimmed title.
    pub title: String,
    /// Body as stored: Markdown exactly as typed; for HTML,
    /// the sanitised form of what was typed — sanitising is
    /// part of validation, so a stored HTML body is safe by
    /// construction.
    pub body_md: String,
    /// `markdown` or `html`.
    pub format: String,
    /// Trimmed meta description.
    pub meta_description: String,
    /// `draft` or `published`.
    pub status: String,
    /// Page type (creation only): `standard` or `landing`.
    pub layout: String,
    /// Whether the sitemap may list the page.
    pub in_sitemap: bool,
}

/// Whether a slug satisfies the page rule: 3–60
/// characters of lowercase ASCII letters, digits and hyphens,
/// never starting or ending with a hyphen. Layered in this
/// service rather than reusing [`crate::validation::is_valid_slug`],
/// whose 1–50 bounds belong to categories and tags.
fn is_valid_page_slug(slug: &str) -> bool {
    let len = slug.chars().count();
    (SLUG_MIN_CHARS..=SLUG_MAX_CHARS).contains(&len)
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Validates the page form, reporting every failing field at
/// once. `creating` adds the slug checks (shape, reserved list);
/// slug uniqueness needs the database and is checked by
/// [`create_page`]. The body is link-checked here in whichever
/// format it was written: a destination outside the scheme
/// allowlist is refused, naming the link — the same allowlist
/// the renderer enforces. An HTML body that passes
/// is sanitised before it is stored, so the stored text is
/// safe to render as-is.
pub fn validate_page(
    input: &ContentPageInput,
    creating: bool,
) -> Result<ValidatedPage, Vec<FieldError>> {
    let mut page = ValidatedPage {
        slug: input.slug.trim().to_string(),
        title: input.title.trim().to_string(),
        body_md: input.body_md.clone(),
        format: input.format.trim().to_string(),
        meta_description: input.meta_description.trim().to_string(),
        status: input.status.trim().to_string(),
        layout: input.layout.trim().to_string(),
        in_sitemap: input.in_sitemap,
    };
    let mut errors = Vec::new();
    if creating {
        if page.layout != "standard" && page.layout != "landing" {
            errors.push(FieldError::new("layout", "ページの種別が不正です。"));
        }
        if !is_valid_page_slug(&page.slug) {
            errors.push(FieldError::new(
                "slug",
                "スラッグは3〜60文字の半角英小文字・数字・ハイフンで入力してください。",
            ));
        } else if RESERVED_SLUGS.contains(&page.slug.as_str()) {
            errors.push(FieldError::new(
                "slug",
                "そのスラッグは予約されているため使えません。",
            ));
        }
    }
    if page.title.is_empty() {
        errors.push(FieldError::new("title", "タイトルを入力してください。"));
    } else if page.title.chars().count() > TITLE_MAX_CHARS {
        errors.push(FieldError::new(
            "title",
            &format!("タイトルは{TITLE_MAX_CHARS}文字以内で入力してください。"),
        ));
    }
    if page.body_md.chars().count() > BODY_MAX_CHARS {
        errors.push(FieldError::new(
            "body_md",
            &format!("本文は{BODY_MAX_CHARS}文字以内で入力してください。"),
        ));
    }
    if page.format != "markdown" && page.format != "html" {
        errors.push(FieldError::new("format", "本文の形式が不正です。"));
    }
    // The destination allowlist is shared by both formats: a
    // link the renderer would refuse is refused here first,
    // naming the link.
    let invalid_url = if page.format == "html" {
        html_sanitize::find_invalid_url_html(&page.body_md)
    } else {
        markdown::find_invalid_url(&page.body_md)
    };
    if let Some(bad) = invalid_url {
        errors.push(FieldError::new(
            "body_md",
            &format!(
                "リンク「{bad}」は使用できません。リンクには https:、http:、mailto:、または「/」で始まるパスを使ってください。"
            ),
        ));
    }
    if page.meta_description.chars().count() > META_MAX_CHARS {
        errors.push(FieldError::new(
            "meta_description",
            &format!("メタディスクリプションは{META_MAX_CHARS}文字以内で入力してください。"),
        ));
    }
    if page.status != "draft" && page.status != "published" {
        errors.push(FieldError::new("status", "ステータスが不正です。"));
    }
    if errors.is_empty() {
        // Sanitising is the last validation step for HTML: what
        // is stored is the cleaned body, never the raw typing.
        if page.format == "html" {
            page.body_md = html_sanitize::sanitize_html(&page.body_md);
        }
        Ok(page)
    } else {
        Err(errors)
    }
}

/// Returns pages for the admin list; `layout` narrows to
/// one page type (the hub's tabs, CMS_SPEC §9).
pub async fn list_pages(
    pool: &PgPool,
    layout: Option<&str>,
) -> Result<Vec<ContentPageSummary>, AppError> {
    content_page_repository::list_all(pool, layout).await
}

/// Counts pages of one layout (the hub's type cards).
pub async fn count_pages(pool: &PgPool, layout: &str) -> Result<i64, AppError> {
    content_page_repository::count_by_layout(pool, layout).await
}

/// Returns one page for the editor, or `NotFound`.
pub async fn get_page(pool: &PgPool, id: i64) -> Result<ContentPage, AppError> {
    content_page_repository::find_by_id(pool, id)
        .await?
        .ok_or(AppError::NotFound)
}

/// Returns a page's revisions, newest first.
pub async fn list_revisions(
    pool: &PgPool,
    page_id: i64,
) -> Result<Vec<ContentRevisionSummary>, AppError> {
    content_page_repository::list_revisions(pool, page_id).await
}

/// Creates a custom page and audits it in the same transaction.
/// Slug uniqueness is the one validation that needs the
/// database; it is reported as the slug field's error.
pub async fn create_page(
    pool: &PgPool,
    admin_id: i64,
    page: &ValidatedPage,
) -> Result<i64, AppError> {
    if content_page_repository::slug_taken(pool, &page.slug, &page.layout).await? {
        return Err(AppError::Validation(vec![FieldError::new(
            "slug",
            "そのスラッグは既に使われています。",
        )]));
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let id = content_page_repository::insert(
        &mut tx,
        &NewContentPage {
            slug: &page.slug,
            kind: "custom",
            title: &page.title,
            body_md: &page.body_md,
            format: &page.format,
            meta_description: &page.meta_description,
            status: &page.status,
            layout: &page.layout,
            in_sitemap: page.in_sitemap,
        },
    )
    .await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "content.page.create",
        "content_page",
        Some(id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    tracing::info!(admin_id, page_id = id, "content.page.create: page created");
    Ok(id)
}

/// Saves an existing page: snapshots the replaced state as a
/// revision, prunes to the cap, stores the new state and audits
/// — one transaction, so the trail and the undo history commit
/// or roll back together. The audit action is
/// `content.page.publish` when this save takes the page live,
/// else `content.page.update`. A submit that changes nothing
/// writes nothing (the settings rule).
pub async fn update_page(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    page: &ValidatedPage,
    expected: i64,
) -> Result<(), AppError> {
    let current = get_page(pool, id).await?;
    if current.title == page.title
        && current.body_md == page.body_md
        && current.format == page.format
        && current.meta_description == page.meta_description
        && current.status == page.status
        && current.in_sitemap == page.in_sitemap
    {
        return Ok(());
    }
    let action = if current.status != "published" && page.status == "published" {
        "content.page.publish"
    } else {
        "content.page.update"
    };
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    // The form-ticket staleness guard (see `form_ticket_service`).
    let stamp = content_page_repository::updated_at_for_update(&mut tx, id).await?;
    crate::services::form_ticket_service::ensure_fresh(stamp, expected)?;
    content_page_repository::insert_revision(&mut tx, &current, admin_id).await?;
    content_page_repository::prune_revisions(&mut tx, id).await?;
    // The layout is deliberately not a parameter of the
    // update: the stored page keeps its creation-time type
    // (CMS_SPEC §9), whatever the form carried.
    content_page_repository::update(
        &mut tx,
        id,
        &page.title,
        &page.body_md,
        &page.format,
        &page.meta_description,
        &page.status,
        page.in_sitemap,
        admin_id,
    )
    .await?;
    audit_service::record(&mut tx, admin_id, action, "content_page", Some(id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    tracing::info!(admin_id, page_id = id, "content.page save: {action}");
    Ok(())
}

/// Restores one revision: the current state is snapshotted
/// first (so the restore is itself undoable, spec D5), then the
/// revision's title, body, format and meta become the page's.
/// The status is
/// not part of a snapshot and never changes here.
pub async fn restore_revision(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    revision_id: i64,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    // Both reads ride the transaction under the page row's
    // lock: the snapshot below must be the state the restore
    // actually replaces, and a concurrent save must not slip
    // between the read and the overwrite and be lost.
    let current = content_page_repository::find_by_id_for_update(&mut tx, id)
        .await?
        .ok_or(AppError::NotFound)?;
    let revision = content_page_repository::find_revision_tx(&mut tx, id, revision_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if current.title == revision.title
        && current.body_md == revision.body_md
        && current.format == revision.format
        && current.meta_description == revision.meta_description
    {
        return Ok(());
    }
    let status = current.status.clone();
    content_page_repository::insert_revision(&mut tx, &current, admin_id).await?;
    content_page_repository::prune_revisions(&mut tx, id).await?;
    content_page_repository::update(
        &mut tx,
        id,
        &revision.title,
        &revision.body_md,
        &revision.format,
        &revision.meta_description,
        &status,
        current.in_sitemap,
        admin_id,
    )
    .await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "content.page.restore",
        "content_page",
        Some(id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    tracing::info!(
        admin_id,
        page_id = id,
        revision_id,
        "content.page.restore: revision restored"
    );
    Ok(())
}

/// Deletes a page and audits it in the same transaction. The
/// two refusals are business rules, enforced here so no caller
/// can bypass them: system pages are undeletable,
/// and a published page must be unpublished first, so a public
/// URL never disappears in one click.
pub async fn delete_page(pool: &PgPool, admin_id: i64, id: i64) -> Result<(), AppError> {
    let page = get_page(pool, id).await?;
    if page.is_system() {
        return Err(AppError::Conflict(
            "固定ページは削除できません。".to_string(),
        ));
    }
    if page.is_published() {
        return Err(AppError::Conflict(
            "公開中のページは削除できません。下書きに戻してから削除してください。".to_string(),
        ));
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    content_page_repository::delete(&mut tx, id).await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "content.page.delete",
        "content_page",
        Some(id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    tracing::info!(admin_id, page_id = id, "content.page.delete: page deleted");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimally valid create input of one layout.
    fn input(layout: &str) -> ContentPageInput {
        ContentPageInput {
            slug: "spring-sale".to_string(),
            title: "春のセール".to_string(),
            body_md: "本文".to_string(),
            format: "markdown".to_string(),
            meta_description: String::new(),
            status: "draft".to_string(),
            layout: layout.to_string(),
            in_sitemap: false,
        }
    }

    #[test]
    fn create_accepts_both_layouts_and_carries_the_sitemap_switch() {
        for layout in ["standard", "landing"] {
            let page = validate_page(&input(layout), true).expect("valid page");
            assert_eq!(page.layout, layout);
            assert!(!page.in_sitemap);
        }
    }

    #[test]
    fn create_refuses_an_unknown_layout() {
        let errors = validate_page(&input("weird"), true).expect_err("must refuse");
        assert!(errors.iter().any(|e| e.field == "layout"));
    }

    #[test]
    fn update_never_validates_layout_because_it_never_writes_it() {
        // The update path ignores the layout entirely (the
        // stored page keeps its creation-time type), so a
        // form value — even a crafted one — is not an error
        // and is never stored.
        let page = validate_page(&input("weird"), false).expect("update ignores layout");
        assert_eq!(page.layout, "weird");
    }
}
