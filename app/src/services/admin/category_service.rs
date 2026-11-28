//! Admin category rules.
//!
//! A category carries a name, a slug, a description, an
//! optional parent, a display order and an active flag. Names
//! and slugs are unique case-insensitively (the service checks
//! for a friendly form error; the `lower()` unique indexes are
//! the race-proof floor). Deletion is guarded: a category that
//! still has products or children is refused with the reason —
//! deactivation is the retirement path, hiding the category
//! from the storefront navigation and the product form while
//! its products keep theirs.

use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{
    DESCRIPTION_MAX_MESSAGE, PARENT_CATEGORY_CHECK_FAILED_MESSAGE, SLUG_CHECK_FAILED_MESSAGE,
    SLUG_FORMAT_MESSAGE, SLUG_REQUIRED_MESSAGE, SLUG_TAKEN_MESSAGE,
};
use crate::models::ProductCategory;
use crate::repositories::category_repository;
use crate::services::{admin::audit_service, product_service};
use crate::validation::{FieldError, is_valid_slug, push_uniqueness_check};

/// The raw category form as submitted. Text fields stay
/// untrimmed so a failed submit re-renders what was typed.
#[derive(Clone, Default)]
pub struct CategoryFormInput {
    /// Category name.
    pub name: String,
    /// Slug (ASCII identifier).
    pub slug: String,
    /// Description.
    pub description: String,
    /// Parent category ID as typed; empty for a top-level row.
    pub parent_id: String,
    /// Display order as typed; empty means 0.
    pub display_order: String,
    /// The 有効 (active) checkbox.
    pub is_active: bool,
}

/// A category form that passed validation, ready to store.
#[derive(Debug)]
pub struct ValidatedCategory {
    /// Trimmed name.
    pub name: String,
    /// Canonical (lowercased) slug.
    pub slug: String,
    /// Trimmed description.
    pub description: String,
    /// Parent category ID; `None` at the top level.
    pub parent_id: Option<i64>,
    /// Display order.
    pub display_order: i32,
    /// The active flag.
    pub is_active: bool,
}

/// The outcome of a category delete attempt.
pub enum CategoryDelete {
    /// The category was removed (or was already gone — deletion
    /// is idempotent, like the notice delete).
    Deleted,
    /// The category is still referenced; the string is the
    /// operator-facing reason, shown as the flash message.
    Blocked(String),
}

/// Validates the category form, returning every error at once.
///
/// The uniqueness and hierarchy checks read the database (the
/// coupon form's pattern), so the signature is async: name and
/// slug must be free case-insensitively, and the parent must
/// exist and must not be the category itself or one of its
/// descendants — a cycle would make the hierarchy unwalkable.
pub async fn validate_category(
    pool: &PgPool,
    input: &CategoryFormInput,
    editing: Option<i64>,
) -> Result<ValidatedCategory, Vec<FieldError>> {
    let mut errors = Vec::new();

    let name = input.name.trim().to_string();
    if name.is_empty() {
        errors.push(FieldError::new("name", "カテゴリー名を入力してください"));
    } else if name.chars().count() > 50 {
        errors.push(FieldError::new(
            "name",
            "カテゴリー名は50文字以内で入力してください",
        ));
    } else {
        // A failed check is a field error, not a 500 (the coupon
        // form's pattern): the operator can retry, and the form
        // never claims a name is free when it could not look.
        push_uniqueness_check(
            &mut errors,
            "name",
            category_repository::name_taken(pool, &name, editing).await,
            "このカテゴリー名は既に登録されています",
            "カテゴリー名を確認できませんでした",
        );
    }

    // Slugs are canonical lowercase: the typed value is folded
    // before the shape check, so `Apparel` and `apparel` are the
    // same slug and collide as duplicates, never as two rows.
    let slug = input.slug.trim().to_lowercase();
    if slug.is_empty() {
        errors.push(FieldError::new("slug", SLUG_REQUIRED_MESSAGE));
    } else if !is_valid_slug(&slug) {
        errors.push(FieldError::new("slug", SLUG_FORMAT_MESSAGE));
    } else {
        push_uniqueness_check(
            &mut errors,
            "slug",
            category_repository::slug_taken(pool, &slug, editing).await,
            SLUG_TAKEN_MESSAGE,
            SLUG_CHECK_FAILED_MESSAGE,
        );
    }

    let description = input.description.trim().to_string();
    if description.chars().count() > 500 {
        errors.push(FieldError::new("description", DESCRIPTION_MAX_MESSAGE));
    }

    let display_order = match input.display_order.trim() {
        "" => 0,
        raw => match raw.parse::<i32>() {
            Ok(v) if (0..=9999).contains(&v) => v,
            _ => {
                errors.push(FieldError::new(
                    "display_order",
                    "表示順は0〜9999の整数で入力してください",
                ));
                0
            }
        },
    };

    let parent_raw = input.parent_id.trim();
    let parent_id = if parent_raw.is_empty() {
        None
    } else {
        match parent_raw.parse::<i64>() {
            Ok(pid) => {
                if editing == Some(pid) {
                    errors.push(FieldError::new(
                        "parent_id",
                        "自分自身を親カテゴリーにはできません",
                    ));
                    None
                } else {
                    match category_repository::find(pool, pid).await {
                        Ok(None) => {
                            errors.push(FieldError::new("parent_id", "親カテゴリーが存在しません"));
                            None
                        }
                        Ok(Some(_)) => match editing {
                            // Cycle guard: a descendant as parent
                            // would close a loop in the hierarchy.
                            Some(id) => match category_repository::descendant_ids(pool, id).await {
                                Ok(descendants) if descendants.contains(&pid) => {
                                    errors.push(FieldError::new(
                                        "parent_id",
                                        "子カテゴリーを親にはできません",
                                    ));
                                    None
                                }
                                Ok(_) => Some(pid),
                                Err(_) => {
                                    errors.push(FieldError::new(
                                        "parent_id",
                                        PARENT_CATEGORY_CHECK_FAILED_MESSAGE,
                                    ));
                                    None
                                }
                            },
                            None => Some(pid),
                        },
                        Err(_) => {
                            errors.push(FieldError::new(
                                "parent_id",
                                PARENT_CATEGORY_CHECK_FAILED_MESSAGE,
                            ));
                            None
                        }
                    }
                }
            }
            Err(_) => {
                errors.push(FieldError::new(
                    "parent_id",
                    "親カテゴリーの指定が正しくありません",
                ));
                None
            }
        }
    };

    if errors.is_empty() {
        Ok(ValidatedCategory {
            name,
            slug,
            description,
            parent_id,
            display_order,
            is_active: input.is_active,
        })
    } else {
        Err(errors)
    }
}

/// Returns one page of the admin category list with its
/// filtered total.
pub async fn list_categories(
    pool: &PgPool,
    query: Option<&str>,
    status: Option<&str>,
    sort: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<category_repository::CategoryListRow>, i64), AppError> {
    // The repository binds a finished LIKE pattern, built by
    // the one shared escaper so `%`/`_` in the typed text match
    // literally, as on every other admin list.
    let pattern = query.and_then(crate::util::like_pattern::like_pattern);
    category_repository::list_admin(pool, pattern.as_deref(), status, sort, limit, offset).await
}

/// Returns one category by ID, for the edit form and the edit
/// submit's existence check.
pub async fn find_category(pool: &PgPool, id: i64) -> Result<Option<ProductCategory>, AppError> {
    category_repository::find(pool, id).await
}

/// Returns the parent choices for the category form: every
/// category except the one being edited and its descendants
/// (the cycle guard as a picker, mirroring the validation).
pub async fn parent_options(
    pool: &PgPool,
    editing: Option<i64>,
) -> Result<Vec<ProductCategory>, AppError> {
    let all = category_repository::list_all(pool).await?;
    let Some(id) = editing else {
        return Ok(all);
    };
    let descendants = category_repository::descendant_ids(pool, id).await?;
    Ok(all
        .into_iter()
        .filter(|c| c.id != id && !descendants.contains(&c.id))
        .collect())
}

/// Maps a validated form onto the repository's field struct.
fn fields_of(valid: &ValidatedCategory) -> category_repository::CategoryFields<'_> {
    category_repository::CategoryFields {
        name: &valid.name,
        slug: &valid.slug,
        description: &valid.description,
        parent_id: valid.parent_id,
        display_order: valid.display_order,
        is_active: valid.is_active,
    }
}

/// Creates one category with its `category.create` audit row.
pub async fn create_category(
    pool: &PgPool,
    admin_id: i64,
    valid: &ValidatedCategory,
) -> Result<i64, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let id = create_category_tx(&mut tx, admin_id, valid).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    // The storefront navigation caches these names briefly;
    // drop the cache so the change shows at once.
    product_service::invalidate_categories_cache();
    Ok(id)
}

/// The create above inside a caller-owned transaction: the
/// CSV batch apply runs every row through here so the whole
/// batch commits or rolls back as one unit.
pub async fn create_category_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    valid: &ValidatedCategory,
) -> Result<i64, AppError> {
    let id = category_repository::create(tx, &fields_of(valid)).await?;
    audit_service::record(tx, admin_id, "category.create", "category", Some(id)).await?;
    Ok(id)
}

/// Updates one category with its `category.update` audit row.
/// A rename rewrites the category name snapshot on every linked
/// product in the same transaction — the storefront's search
/// reads the snapshot, so the rename is visible there at once
/// and the old name never lingers on a product.
pub async fn update_category(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    valid: &ValidatedCategory,
    previous_name: &str,
    expected: Option<i64>,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    update_category_tx(&mut tx, admin_id, id, valid, previous_name, expected).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    // The storefront navigation caches these names briefly;
    // drop the cache so the change shows at once.
    product_service::invalidate_categories_cache();
    Ok(())
}

/// The update above inside a caller-owned transaction: the
/// CSV batch apply runs every row through here so the whole
/// batch commits or rolls back as one unit.
pub async fn update_category_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    id: i64,
    valid: &ValidatedCategory,
    previous_name: &str,
    expected: Option<i64>,
) -> Result<(), AppError> {
    // The form-ticket staleness guard, when the edit form
    // supplied one. The CSV import passes `None`: its
    // preview/revalidation flow is its own guard (the
    // product save's shape).
    if let Some(expected) = expected {
        let current = category_repository::updated_at_for_update(tx, id).await?;
        crate::services::form_ticket_service::ensure_fresh(current, expected)?;
    }
    category_repository::update(tx, id, &fields_of(valid), admin_id).await?;
    if previous_name != valid.name {
        category_repository::sync_product_snapshots(tx, id, &valid.name).await?;
    }
    audit_service::record(tx, admin_id, "category.update", "category", Some(id)).await?;
    Ok(())
}

/// Deletes one category when nothing references it. A category
/// with products or children is [`CategoryDelete::Blocked`] with
/// the reason — the operator retires those by deactivation
/// instead. The foreign keys are `ON DELETE RESTRICT`, so even a
/// raced delete cannot orphan a product; this check exists to
/// answer with the reason instead of a database error. The
/// `category.delete` audit row is written only when a row was
/// actually removed (idempotent delete, like notices).
pub async fn delete_category(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
) -> Result<CategoryDelete, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let products = category_repository::product_count(&mut tx, id).await?;
    if products > 0 {
        return Ok(CategoryDelete::Blocked(format!(
            "このカテゴリーは{products}件の商品で使用されているため削除できません。削除の代わりに「無効」にすると、店舗のナビゲーションや商品フォームから選べなくなります"
        )));
    }
    let children = category_repository::child_count(&mut tx, id).await?;
    if children > 0 {
        return Ok(CategoryDelete::Blocked(
            "このカテゴリーには子カテゴリーがあるため削除できません。先に子カテゴリーの親を変更してください"
                .to_string(),
        ));
    }
    let removed = category_repository::delete(&mut tx, id).await?;
    if removed > 0 {
        audit_service::record(&mut tx, admin_id, "category.delete", "category", Some(id)).await?;
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    // The storefront navigation caches these names briefly;
    // drop the cache so the change shows at once.
    product_service::invalidate_categories_cache();
    Ok(CategoryDelete::Deleted)
}
