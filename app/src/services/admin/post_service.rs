//! Admin blog post rules (CMS blog spec,
//! docs/specs/CMS_SPEC.md).
//!
//! The pages CMS's discipline, carried to dated articles:
//! HTML bodies are sanitised before storing
//! ([`crate::util::html_sanitize`]), every save of an
//! existing post first snapshots the words it replaces into
//! `post_revisions` (capped at the newest 50), and every
//! write is audited in the same transaction. A snapshot
//! holds words only — restoring one never moves the post's
//! status, category, cover or schedule.
//!
//! Scheduling is the `published_at` instant: publishing
//! with no instant stamps the save time, and a future
//! instant is a reservation the storefront's liveness rule
//! (see [`crate::models::Post::is_live`]) honours on its
//! own — no worker flips anything.

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::jst::jst;
use crate::models::{NewPost, Post, PostRevisionSummary, PostSummary};
use crate::repositories::{post_category_repository, post_repository};
use crate::services::admin::audit_service;
use crate::services::admin::product_service::{self, UploadError};
use crate::util::html_sanitize;
use crate::util::markdown;
use crate::validation::FieldError;

/// Title limit, in characters (the pages editor's rule).
const TITLE_MAX_CHARS: usize = 80;
/// Body limit, in characters (the pages editor's rule).
const BODY_MAX_CHARS: usize = 50_000;
/// Meta description limit, in characters (the pages
/// editor's rule).
const META_MAX_CHARS: usize = 160;
/// Excerpt limit, in characters: a card summary is longer
/// than a meta description but still a summary.
const EXCERPT_MAX_CHARS: usize = 300;
/// Slug length bounds: the pages rule (3–60), layered here
/// for the same reason the pages service layers its own —
/// the shared helper's 1–50 bounds serve categories and
/// tags, and neither rule may silently rewrite the other.
const SLUG_MIN_CHARS: usize = 3;
/// Slug maximum length, in characters.
const SLUG_MAX_CHARS: usize = 60;

/// Slugs an operator may not claim: the blog's own
/// subroutes. A post lives at `/blog/{slug}`, so `category`
/// would shadow the archive route's first segment.
const RESERVED_SLUGS: &[&str] = &["category"];

/// The raw post form as submitted: every value a string,
/// exactly as typed. `slug` is read at creation only; the
/// update path never looks at it. `published_at` is the
/// `datetime-local` value — a JST wall time, converted by
/// [`parse_published_at`].
#[derive(Clone, Default)]
pub struct PostInput {
    /// Slug as typed (creation only).
    pub slug: String,
    /// Title as typed.
    pub title: String,
    /// Body source as typed (Markdown, or HTML when
    /// [`Self::format`] is `html`).
    pub body_md: String,
    /// Body format as chosen: `markdown` or `html`.
    pub format: String,
    /// Excerpt as typed.
    pub excerpt: String,
    /// Meta description as typed.
    pub meta_description: String,
    /// Cover image path: the stored path of the current or
    /// newly uploaded cover (a file upload replaces it in
    /// the controller before validation; empty = no cover).
    pub cover_image_path: String,
    /// Category ID as typed; empty = uncategorised.
    pub category_id: String,
    /// Status as typed: `draft` or `published`.
    pub status: String,
    /// Publication instant as typed (`datetime-local`, JST).
    pub published_at: String,
}

impl From<&Post> for PostInput {
    /// The editor's initial values: the stored post,
    /// verbatim, with the instant rendered back into the
    /// JST wall time the form edits.
    fn from(post: &Post) -> Self {
        Self {
            slug: post.slug.clone(),
            title: post.title.clone(),
            body_md: post.body_md.clone(),
            format: post.format.clone(),
            excerpt: post.excerpt.clone(),
            meta_description: post.meta_description.clone(),
            cover_image_path: post.cover_image_path.clone().unwrap_or_default(),
            category_id: post
                .category_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
            status: post.status.clone(),
            published_at: published_at_input(post.published_at),
        }
    }
}

/// A post form that passed validation, ready to store.
#[derive(Debug)]
pub struct ValidatedPost {
    /// Trimmed slug (creation only; empty on updates).
    pub slug: String,
    /// Trimmed title.
    pub title: String,
    /// Body as stored: Markdown exactly as typed; for HTML,
    /// the sanitised form of what was typed.
    pub body_md: String,
    /// `markdown` or `html`.
    pub format: String,
    /// Trimmed excerpt; may be empty (cards derive one).
    pub excerpt: String,
    /// Trimmed meta description; may be empty.
    pub meta_description: String,
    /// Cover image path, when one is set.
    pub cover_image_path: Option<String>,
    /// The post's category, when categorised.
    pub category_id: Option<i64>,
    /// `draft` or `published`.
    pub status: String,
    /// The submitted publication instant, when one was
    /// typed; the save paths resolve the empty case
    /// (`resolve_published_at`).
    pub published_at: Option<DateTime<Utc>>,
}

/// Whether a slug satisfies the post rule: 3–60 characters
/// of lowercase ASCII letters, digits and hyphens, never
/// starting or ending with a hyphen (the pages rule).
fn is_valid_post_slug(slug: &str) -> bool {
    let len = slug.chars().count();
    (SLUG_MIN_CHARS..=SLUG_MAX_CHARS).contains(&len)
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The three shapes a submitted `published_at` value
/// takes: absent, unparseable, or a real instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishedAtParse {
    /// The field was empty.
    Empty,
    /// The field held something that is not a date/time.
    Invalid,
    /// The parsed instant.
    At(DateTime<Utc>),
}

/// Parses the form's `datetime-local` value as a JST wall
/// time: the operator thinks in shop time, the store keeps
/// UTC.
pub fn parse_published_at(raw: &str) -> PublishedAtParse {
    let raw = raw.trim();
    if raw.is_empty() {
        return PublishedAtParse::Empty;
    }
    let parsed = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S"))
        .ok()
        .and_then(|naive| jst().from_local_datetime(&naive).single());
    match parsed {
        Some(local) => PublishedAtParse::At(local.with_timezone(&Utc)),
        None => PublishedAtParse::Invalid,
    }
}

/// Renders a stored instant back into the `datetime-local`
/// value (JST wall time) the edit form shows; empty when no
/// instant is set.
pub fn published_at_input(at: Option<DateTime<Utc>>) -> String {
    at.map(|at| {
        at.with_timezone(&jst())
            .format("%Y-%m-%dT%H:%M")
            .to_string()
    })
    .unwrap_or_default()
}

/// The publication instant a save stores: a typed instant
/// always wins; publishing without one keeps the existing
/// instant, or stamps the save time when there is none —
/// a published post always carries the instant it went (or
/// goes) live. A draft keeps whatever was typed (a planned
/// date) and nothing else.
fn resolve_published_at(
    status: &str,
    submitted: Option<DateTime<Utc>>,
    existing: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if submitted.is_some() {
        return submitted;
    }
    if status == "published" {
        return existing.or(Some(now));
    }
    None
}

/// Validates the post form, reporting every failing field
/// at once (the pages service's shape). Slug uniqueness and
/// category existence need the database and are checked by
/// [`create_post`] / [`update_post`]. An HTML body that
/// passes is sanitised before it is stored.
pub fn validate_post(input: &PostInput, creating: bool) -> Result<ValidatedPost, Vec<FieldError>> {
    let mut post = ValidatedPost {
        slug: input.slug.trim().to_string(),
        title: input.title.trim().to_string(),
        body_md: input.body_md.clone(),
        format: input.format.trim().to_string(),
        excerpt: input.excerpt.trim().to_string(),
        meta_description: input.meta_description.trim().to_string(),
        cover_image_path: {
            let path = input.cover_image_path.trim();
            if path.is_empty() {
                None
            } else {
                Some(path.to_string())
            }
        },
        category_id: None,
        status: input.status.trim().to_string(),
        published_at: None,
    };
    let mut errors = Vec::new();
    if creating {
        if !is_valid_post_slug(&post.slug) {
            errors.push(FieldError::new(
                "slug",
                "スラッグは3〜60文字の半角英小文字・数字・ハイフンで入力してください。",
            ));
        } else if RESERVED_SLUGS.contains(&post.slug.as_str()) {
            errors.push(FieldError::new(
                "slug",
                "そのスラッグは予約されているため使えません。",
            ));
        }
    }
    if post.title.is_empty() {
        errors.push(FieldError::new("title", "タイトルを入力してください。"));
    } else if post.title.chars().count() > TITLE_MAX_CHARS {
        errors.push(FieldError::new(
            "title",
            &format!("タイトルは{TITLE_MAX_CHARS}文字以内で入力してください。"),
        ));
    }
    if post.body_md.chars().count() > BODY_MAX_CHARS {
        errors.push(FieldError::new(
            "body_md",
            &format!("本文は{BODY_MAX_CHARS}文字以内で入力してください。"),
        ));
    }
    if post.format != "markdown" && post.format != "html" {
        errors.push(FieldError::new("format", "本文の形式が不正です。"));
    }
    let invalid_url = if post.format == "html" {
        html_sanitize::find_invalid_url_html(&post.body_md)
    } else {
        markdown::find_invalid_url(&post.body_md)
    };
    if let Some(bad) = invalid_url {
        errors.push(FieldError::new(
            "body_md",
            &format!(
                "リンク「{bad}」は使用できません。リンクには https:、http:、mailto:、または「/」で始まるパスを使ってください。"
            ),
        ));
    }
    if post.excerpt.chars().count() > EXCERPT_MAX_CHARS {
        errors.push(FieldError::new(
            "excerpt",
            &format!("要約は{EXCERPT_MAX_CHARS}文字以内で入力してください。"),
        ));
    }
    if post.meta_description.chars().count() > META_MAX_CHARS {
        errors.push(FieldError::new(
            "meta_description",
            &format!("メタディスクリプションは{META_MAX_CHARS}文字以内で入力してください。"),
        ));
    }
    if post.status != "draft" && post.status != "published" {
        errors.push(FieldError::new("status", "ステータスが不正です。"));
    }
    let category_raw = input.category_id.trim();
    if !category_raw.is_empty() {
        match category_raw.parse::<i64>() {
            Ok(id) => post.category_id = Some(id),
            Err(_) => errors.push(FieldError::new("category_id", "カテゴリの指定が不正です。")),
        }
    }
    match parse_published_at(&input.published_at) {
        PublishedAtParse::At(at) => post.published_at = Some(at),
        PublishedAtParse::Empty => post.published_at = None,
        PublishedAtParse::Invalid => errors.push(FieldError::new(
            "published_at",
            "公開日時の形式が不正です。",
        )),
    }
    if errors.is_empty() {
        if post.format == "html" {
            post.body_md = html_sanitize::sanitize_html(&post.body_md);
        }
        Ok(post)
    } else {
        Err(errors)
    }
}

/// Stores one uploaded cover image through the product
/// image machinery (magic-byte type check, generated name,
/// derivatives, its own audit row) and returns the served
/// path the post stores.
pub async fn store_cover_image(
    pool: &PgPool,
    admin_id: i64,
    original_name: Option<&str>,
    bytes: &[u8],
) -> Result<String, UploadError> {
    let uploaded =
        product_service::store_uploaded_image(pool, admin_id, original_name, bytes).await?;
    Ok(uploaded.path)
}

/// Returns one page of the admin list in one derived state
/// with its total. The state text is allowlisted here: an
/// unknown filter is the unfiltered list.
///
/// # Errors
/// Returns a store error when either read fails.
pub async fn list_posts(
    pool: &PgPool,
    state: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<PostSummary>, i64), AppError> {
    let state = state.filter(|s| matches!(*s, "draft" | "scheduled" | "live"));
    let rows = post_repository::list_admin(pool, state, limit, offset).await?;
    let total = post_repository::count_admin(pool, state).await?;
    Ok((rows, total))
}

/// Counts every post, whatever its state (the admin
/// pages hub's type card, CMS_SPEC §9).
pub async fn count_posts(pool: &PgPool) -> Result<i64, AppError> {
    post_repository::count_admin(pool, None).await
}

/// Returns one post for the editor, or `NotFound`.
///
/// # Errors
/// Returns `NotFound` when no post has the ID, and a
/// store error when the read fails.
pub async fn get_post(pool: &PgPool, id: i64) -> Result<Post, AppError> {
    post_repository::find_by_id(pool, id)
        .await?
        .ok_or(AppError::NotFound)
}

/// Returns a post's revisions, newest first.
///
/// # Errors
/// Returns a store error when the read fails.
pub async fn list_revisions(
    pool: &PgPool,
    post_id: i64,
) -> Result<Vec<PostRevisionSummary>, AppError> {
    post_repository::list_revisions(pool, post_id).await
}

/// The category-existence check shared by create and
/// update: a submitted category must name a real one.
async fn ensure_category_exists(pool: &PgPool, category_id: Option<i64>) -> Result<(), AppError> {
    if let Some(id) = category_id
        && post_category_repository::find_by_id(pool, id)
            .await?
            .is_none()
    {
        return Err(AppError::Validation(vec![FieldError::new(
            "category_id",
            "選択したカテゴリが見つかりません。",
        )]));
    }
    Ok(())
}

/// Creates a post and audits it in the same transaction.
/// The creating admin is the author — the byline never
/// changes afterwards.
///
/// # Errors
/// Returns a store error when the insert or its audit
/// write fails; the transaction rolls back, so a failed
/// create leaves no post behind.
pub async fn create_post(
    pool: &PgPool,
    admin_id: i64,
    post: &ValidatedPost,
) -> Result<i64, AppError> {
    if post_repository::slug_taken(pool, &post.slug).await? {
        return Err(AppError::Validation(vec![FieldError::new(
            "slug",
            "そのスラッグは既に使われています。",
        )]));
    }
    ensure_category_exists(pool, post.category_id).await?;
    let published_at = resolve_published_at(&post.status, post.published_at, None, Utc::now());
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let id = post_repository::insert(
        &mut tx,
        &NewPost {
            slug: &post.slug,
            title: &post.title,
            body_md: &post.body_md,
            format: &post.format,
            excerpt: &post.excerpt,
            meta_description: &post.meta_description,
            cover_image_path: post.cover_image_path.as_deref(),
            category_id: post.category_id,
            author_admin_id: admin_id,
            status: &post.status,
            published_at,
        },
    )
    .await?;
    audit_service::record(&mut tx, admin_id, "post.create", "post", Some(id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    tracing::info!(admin_id, post_id = id, "post.create: post created");
    Ok(id)
}

/// Saves an existing post: snapshots the replaced words as
/// a revision, prunes to the cap, stores the new state and
/// audits — one transaction. A submit that changes nothing
/// writes nothing (the settings rule, as on content pages).
///
/// # Errors
/// Returns a store error when the read, the revision
/// snapshot, the update or the audit write fails; the
/// transaction rolls back.
pub async fn update_post(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    post: &ValidatedPost,
    expected: i64,
) -> Result<(), AppError> {
    let current = get_post(pool, id).await?;
    ensure_category_exists(pool, post.category_id).await?;
    let published_at = resolve_published_at(
        &post.status,
        post.published_at,
        current.published_at,
        Utc::now(),
    );
    if current.title == post.title
        && current.body_md == post.body_md
        && current.format == post.format
        && current.excerpt == post.excerpt
        && current.meta_description == post.meta_description
        && current.cover_image_path == post.cover_image_path
        && current.category_id == post.category_id
        && current.status == post.status
        && current.published_at == published_at
    {
        return Ok(());
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let stamp = post_repository::updated_at_for_update(&mut tx, id).await?;
    crate::services::form_ticket_service::ensure_fresh(stamp, expected)?;
    post_repository::insert_revision(&mut tx, &current, admin_id).await?;
    post_repository::prune_revisions(&mut tx, id).await?;
    post_repository::update(
        &mut tx,
        id,
        &post.title,
        &post.body_md,
        &post.format,
        &post.excerpt,
        &post.meta_description,
        post.cover_image_path.as_deref(),
        post.category_id,
        &post.status,
        published_at,
    )
    .await?;
    audit_service::record(&mut tx, admin_id, "post.update", "post", Some(id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    tracing::info!(admin_id, post_id = id, "post.update: post saved");
    Ok(())
}

/// Restores one revision's words: the current words are
/// snapshotted first (so the restore is itself undoable),
/// then the revision's title, body, format, meta and
/// excerpt become the post's. Status, category, cover and
/// schedule are not part of a snapshot and never move here.
///
/// # Errors
/// Returns `NotFound` when the post or the revision does
/// not exist, and a store error when a read or write
/// fails; the transaction rolls back.
pub async fn restore_revision(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    revision_id: i64,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let current = post_repository::find_by_id_for_update(&mut tx, id)
        .await?
        .ok_or(AppError::NotFound)?;
    let revision = post_repository::find_revision_tx(&mut tx, id, revision_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if current.title == revision.title
        && current.body_md == revision.body_md
        && current.format == revision.format
        && current.meta_description == revision.meta_description
        && current.excerpt == revision.excerpt
    {
        return Ok(());
    }
    post_repository::insert_revision(&mut tx, &current, admin_id).await?;
    post_repository::prune_revisions(&mut tx, id).await?;
    post_repository::update_words(
        &mut tx,
        id,
        &revision.title,
        &revision.body_md,
        &revision.format,
        &revision.excerpt,
        &revision.meta_description,
    )
    .await?;
    audit_service::record(&mut tx, admin_id, "post.restore", "post", Some(id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    tracing::info!(
        admin_id,
        post_id = id,
        revision_id,
        "post.restore: revision restored"
    );
    Ok(())
}

/// Deletes a post and audits it in the same transaction.
/// Posts are all operator-made, so — unlike system pages —
/// there is no state guard: the confirm dialog on the form
/// is the protection, and the revisions cascade away with
/// the post.
///
/// # Errors
/// Returns a store error when the delete or its audit
/// write fails; the transaction rolls back.
pub async fn delete_post(pool: &PgPool, admin_id: i64, id: i64) -> Result<(), AppError> {
    let _ = get_post(pool, id).await?;
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    post_repository::delete(&mut tx, id).await?;
    audit_service::record(&mut tx, admin_id, "post.delete", "post", Some(id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    tracing::info!(admin_id, post_id = id, "post.delete: post deleted");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_at_parses_jst_wall_time() {
        // 2026-11-28 21:00 JST is 12:00 UTC.
        let PublishedAtParse::At(at) = parse_published_at("2026-11-28T21:00") else {
            panic!("21:00 parses");
        };
        assert_eq!(at, DateTime::from_timestamp(1_795_867_200, 0).unwrap());
        let PublishedAtParse::At(with_secs) = parse_published_at("2026-11-28T21:00:30") else {
            panic!("21:00:30 parses");
        };
        assert_eq!(
            with_secs,
            DateTime::from_timestamp(1_795_867_230, 0).unwrap()
        );
        assert_eq!(parse_published_at(""), PublishedAtParse::Empty);
        assert_eq!(parse_published_at("  "), PublishedAtParse::Empty);
        assert_eq!(
            parse_published_at("2026/11/28 21:00"),
            PublishedAtParse::Invalid
        );
        assert_eq!(parse_published_at("tomorrow"), PublishedAtParse::Invalid);
    }

    #[test]
    fn published_at_input_round_trips_through_jst() {
        let at = DateTime::from_timestamp(1_795_867_200, 0).unwrap();
        assert_eq!(published_at_input(Some(at)), "2026-11-28T21:00");
        assert_eq!(published_at_input(None), "");
        let PublishedAtParse::At(back) = parse_published_at(&published_at_input(Some(at))) else {
            panic!("the rendered value parses back");
        };
        assert_eq!(back, at);
    }

    #[test]
    fn resolve_published_at_rules() {
        let now = DateTime::from_timestamp(1_795_867_200, 0).unwrap();
        let typed = now + chrono::Duration::days(7);
        let existing = now - chrono::Duration::days(1);
        // A typed instant always wins.
        assert_eq!(
            resolve_published_at("published", Some(typed), Some(existing), now),
            Some(typed)
        );
        assert_eq!(
            resolve_published_at("draft", Some(typed), None, now),
            Some(typed)
        );
        // Publishing with none typed keeps the existing
        // instant, else stamps now.
        assert_eq!(
            resolve_published_at("published", None, Some(existing), now),
            Some(existing)
        );
        assert_eq!(
            resolve_published_at("published", None, None, now),
            Some(now)
        );
        // A draft with none typed carries none.
        assert_eq!(
            resolve_published_at("draft", None, Some(existing), now),
            None
        );
    }

    #[test]
    fn slug_rule_matches_the_pages_bounds() {
        assert!(is_valid_post_slug("hello-world"));
        assert!(is_valid_post_slug("a1-2b"));
        assert!(!is_valid_post_slug("ab"));
        assert!(!is_valid_post_slug("Hello"));
        assert!(!is_valid_post_slug("-hello"));
        assert!(!is_valid_post_slug("hello-"));
        assert!(!is_valid_post_slug(&"a".repeat(61)));
    }
}
