//! Admin blog category rules (CMS blog spec,
//! docs/specs/CMS_SPEC.md).
//!
//! The blog's own small vocabulary: a name and a slug.
//! Slugs are canonical lowercase and unique (the service
//! checks for a friendly form error; the UNIQUE constraint
//! is the race-proof floor). Deleting a category never
//! deletes a post — the foreign key sets the posts'
//! category to NULL in the same statement, so an archive
//! disappearing never takes articles with it.

use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{
    SLUG_CHECK_FAILED_MESSAGE, SLUG_FORMAT_MESSAGE, SLUG_REQUIRED_MESSAGE, SLUG_TAKEN_MESSAGE,
};
use crate::models::PostCategory;
use crate::repositories::post_category_repository;
use crate::services::admin::audit_service;
use crate::validation::{FieldError, is_valid_slug, push_uniqueness_check};

/// Name limit, in characters (the tag label's rule).
const NAME_MAX_CHARS: usize = 50;

/// The raw category form as submitted. Text fields stay
/// untrimmed so a failed submit re-renders what was typed.
#[derive(Clone, Default)]
pub struct PostCategoryInput {
    /// Category name as typed.
    pub name: String,
    /// Slug as typed.
    pub slug: String,
}

impl From<&PostCategory> for PostCategoryInput {
    /// The editor's initial values: the stored category,
    /// verbatim.
    fn from(category: &PostCategory) -> Self {
        Self {
            name: category.name.clone(),
            slug: category.slug.clone(),
        }
    }
}

/// A category form that passed validation, ready to store.
#[derive(Debug)]
pub struct ValidatedPostCategory {
    /// Trimmed name.
    pub name: String,
    /// Canonical (lowercased) slug.
    pub slug: String,
}

/// Validates the category form, returning every error at
/// once. The slug uniqueness check reads the database, so
/// the signature is async (the tag form's pattern).
pub async fn validate_category(
    pool: &PgPool,
    input: &PostCategoryInput,
    editing: Option<i64>,
) -> Result<ValidatedPostCategory, Vec<FieldError>> {
    let mut errors = Vec::new();
    let name = input.name.trim().to_string();
    if name.is_empty() {
        errors.push(FieldError::new("name", "カテゴリ名を入力してください。"));
    } else if name.chars().count() > NAME_MAX_CHARS {
        errors.push(FieldError::new(
            "name",
            &format!("カテゴリ名は{NAME_MAX_CHARS}文字以内で入力してください。"),
        ));
    }
    let slug = input.slug.trim().to_lowercase();
    if slug.is_empty() {
        errors.push(FieldError::new("slug", SLUG_REQUIRED_MESSAGE));
    } else if !is_valid_slug(&slug) {
        errors.push(FieldError::new("slug", SLUG_FORMAT_MESSAGE));
    } else {
        push_uniqueness_check(
            &mut errors,
            "slug",
            post_category_repository::slug_taken(pool, &slug, editing).await,
            SLUG_TAKEN_MESSAGE,
            SLUG_CHECK_FAILED_MESSAGE,
        );
    }
    if errors.is_empty() {
        Ok(ValidatedPostCategory { name, slug })
    } else {
        Err(errors)
    }
}

/// Returns every category in registration order.
///
/// # Errors
/// Returns a store error when the read fails.
pub async fn list_categories(pool: &PgPool) -> Result<Vec<PostCategory>, AppError> {
    post_category_repository::list_all(pool).await
}

/// Returns one category for the editor, or `NotFound`.
///
/// # Errors
/// Returns `NotFound` when no category has the ID, and
/// a store error when the read fails.
pub async fn get_category(pool: &PgPool, id: i64) -> Result<PostCategory, AppError> {
    post_category_repository::find_by_id(pool, id)
        .await?
        .ok_or(AppError::NotFound)
}

/// Creates one category with its `post_category.create`
/// audit row.
///
/// # Errors
/// Returns a store error when the insert or its audit
/// write fails; the transaction rolls back.
pub async fn create_category(
    pool: &PgPool,
    admin_id: i64,
    valid: &ValidatedPostCategory,
) -> Result<i64, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let id = post_category_repository::insert(&mut tx, &valid.name, &valid.slug).await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "post_category.create",
        "post_category",
        Some(id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    tracing::info!(
        admin_id,
        category_id = id,
        "post_category.create: category created"
    );
    Ok(id)
}

/// Updates one category with its `post_category.update`
/// audit row, under the edit form's staleness guard.
///
/// # Errors
/// Returns a store error when a read or write fails;
/// the transaction rolls back.
pub async fn update_category(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    valid: &ValidatedPostCategory,
    expected: i64,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let stamp = post_category_repository::updated_at_for_update(&mut tx, id).await?;
    crate::services::form_ticket_service::ensure_fresh(stamp, expected)?;
    post_category_repository::update(&mut tx, id, &valid.name, &valid.slug).await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "post_category.update",
        "post_category",
        Some(id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    tracing::info!(
        admin_id,
        category_id = id,
        "post_category.update: category saved"
    );
    Ok(())
}

/// Deletes one category with its `post_category.delete`
/// audit row. Its posts are uncategorised by the foreign
/// key, never deleted. The audit row is written only when
/// a row was actually removed (the idempotent-delete
/// pattern, like notices).
///
/// # Errors
/// Returns a store error when the delete or its audit
/// write fails; the transaction rolls back.
pub async fn delete_category(pool: &PgPool, admin_id: i64, id: i64) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let removed = post_category_repository::delete(&mut tx, id).await?;
    if removed > 0 {
        audit_service::record(
            &mut tx,
            admin_id,
            "post_category.delete",
            "post_category",
            Some(id),
        )
        .await?;
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    tracing::info!(
        admin_id,
        category_id = id,
        "post_category.delete: category deleted"
    );
    Ok(())
}
