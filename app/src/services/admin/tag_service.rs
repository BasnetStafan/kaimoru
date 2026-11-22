//! Admin tag rules.
//!
//! A tag carries a label （名前）, a slug and a description.
//! Labels and slugs are unique case-insensitively (the service
//! checks for a friendly form error; the `lower()` unique
//! indexes are the race-proof floor). Tags have no inactive
//! state: deletion is the retirement path, and it removes the
//! tag's product links in the same transaction, so a deleted
//! tag never leaves a dangling reference on a product. The
//! vocabulary itself is the matching data — slugs are
//! the machine key, so they stay stable ASCII identifiers.

use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{
    DESCRIPTION_MAX_MESSAGE, SLUG_CHECK_FAILED_MESSAGE, SLUG_FORMAT_MESSAGE, SLUG_REQUIRED_MESSAGE,
    SLUG_TAKEN_MESSAGE,
};
use crate::models::Tag;
use crate::repositories::tag_repository;
use crate::services::admin::audit_service;
use crate::validation::{FieldError, is_valid_slug, push_uniqueness_check};

/// The raw tag form as submitted. Text fields stay untrimmed
/// so a failed submit re-renders what was typed.
#[derive(Clone, Default)]
pub struct TagFormInput {
    /// Tag label （名前）.
    pub label: String,
    /// Slug (ASCII identifier).
    pub slug: String,
    /// Description.
    pub description: String,
}

/// A tag form that passed validation, ready to store.
#[derive(Debug)]
pub struct ValidatedTag {
    /// Trimmed label.
    pub label: String,
    /// Canonical (lowercased) slug.
    pub slug: String,
    /// Trimmed description.
    pub description: String,
}

/// Validates the tag form, returning every error at once. The
/// uniqueness checks read the database, so the signature is
/// async; a failed check becomes a field error on that field
/// (the coupon form's pattern), never a silent pass.
pub async fn validate_tag(
    pool: &PgPool,
    input: &TagFormInput,
    editing: Option<i64>,
) -> Result<ValidatedTag, Vec<FieldError>> {
    let mut errors = Vec::new();

    let label = input.label.trim().to_string();
    if label.is_empty() {
        errors.push(FieldError::new("label", "タグ名を入力してください"));
    } else if label.chars().count() > 50 {
        errors.push(FieldError::new(
            "label",
            "タグ名は50文字以内で入力してください",
        ));
    } else {
        push_uniqueness_check(
            &mut errors,
            "label",
            tag_repository::label_taken(pool, &label, editing).await,
            "このタグ名は既に登録されています",
            "タグ名を確認できませんでした",
        );
    }

    // Slugs are canonical lowercase, like category slugs.
    let slug = input.slug.trim().to_lowercase();
    if slug.is_empty() {
        errors.push(FieldError::new("slug", SLUG_REQUIRED_MESSAGE));
    } else if !is_valid_slug(&slug) {
        errors.push(FieldError::new("slug", SLUG_FORMAT_MESSAGE));
    } else {
        push_uniqueness_check(
            &mut errors,
            "slug",
            tag_repository::slug_taken(pool, &slug, editing).await,
            SLUG_TAKEN_MESSAGE,
            SLUG_CHECK_FAILED_MESSAGE,
        );
    }

    let description = input.description.trim().to_string();
    if description.chars().count() > 500 {
        errors.push(FieldError::new("description", DESCRIPTION_MAX_MESSAGE));
    }

    if errors.is_empty() {
        Ok(ValidatedTag {
            label,
            slug,
            description,
        })
    } else {
        Err(errors)
    }
}

/// Returns one page of the admin tag list with its filtered
/// total.
pub async fn list_tags(
    pool: &PgPool,
    query: Option<&str>,
    sort: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<tag_repository::TagListRow>, i64), AppError> {
    // The repository binds a finished LIKE pattern, built by
    // the one shared escaper so `%`/`_` in the typed text match
    // literally, as on every other admin list.
    let pattern = query.and_then(crate::util::like_pattern::like_pattern);
    tag_repository::list_admin(pool, pattern.as_deref(), sort, limit, offset).await
}

/// Returns one tag by ID, for the edit form and the edit
/// submit's existence check.
pub async fn find_tag(pool: &PgPool, id: i64) -> Result<Option<Tag>, AppError> {
    tag_repository::find(pool, id).await
}

/// Creates one tag with its `tag.create` audit row.
pub async fn create_tag(
    pool: &PgPool,
    admin_id: i64,
    valid: &ValidatedTag,
) -> Result<i64, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let id = create_tag_tx(&mut tx, admin_id, valid).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(id)
}

/// The create above inside a caller-owned transaction: the
/// CSV batch apply runs every row through here so the whole
/// batch commits or rolls back as one unit.
pub async fn create_tag_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    valid: &ValidatedTag,
) -> Result<i64, AppError> {
    let id = tag_repository::create(tx, &valid.slug, &valid.label, &valid.description).await?;
    audit_service::record(tx, admin_id, "tag.create", "tag", Some(id)).await?;
    Ok(id)
}

/// Updates one tag with its `tag.update` audit row.
pub async fn update_tag(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    valid: &ValidatedTag,
    expected: Option<i64>,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    update_tag_tx(&mut tx, admin_id, id, valid, expected).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}

/// The update above inside a caller-owned transaction: the
/// CSV batch apply runs every row through here so the whole
/// batch commits or rolls back as one unit.
pub async fn update_tag_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    id: i64,
    valid: &ValidatedTag,
    expected: Option<i64>,
) -> Result<(), AppError> {
    // The form-ticket staleness guard, when the edit form
    // supplied one (the CSV import passes `None`: its
    // preview/revalidation flow is its own guard).
    if let Some(expected) = expected {
        let current = tag_repository::updated_at_for_update(tx, id).await?;
        crate::services::form_ticket_service::ensure_fresh(current, expected)?;
    }
    tag_repository::update(
        tx,
        id,
        &valid.slug,
        &valid.label,
        &valid.description,
        admin_id,
    )
    .await?;
    audit_service::record(tx, admin_id, "tag.update", "tag", Some(id)).await?;
    Ok(())
}

/// Deletes one tag and every product link pointing at it, in
/// one transaction with the `tag.delete` audit row — a product
/// can never be left referencing a tag that no longer exists.
/// The audit row is written only when a row was actually
/// removed (idempotent delete, like notices).
pub async fn delete_tag(pool: &PgPool, admin_id: i64, id: i64) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    tag_repository::unlink_products(&mut tx, id).await?;
    let removed = tag_repository::delete(&mut tx, id).await?;
    if removed > 0 {
        audit_service::record(&mut tx, admin_id, "tag.delete", "tag", Some(id)).await?;
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}
