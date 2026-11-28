//! Review service.
//!
//! The rules of the feature live here: only a buyer may review
//! (proven by their own order lines, re-checked at posting), the
//! field validation, and the summary the product page renders.
//! Business logs carry record ids only — review text is personal
//! content and is never logged, as passwords and emails never are.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::models::review::{
    DistributionRow, RatingAggregate, Review, ReviewFieldErrors, ReviewInput, ValidReview,
};
use crate::repositories::review_repository::{self, ReviewSort};

/// The review list's sort orders, re-exported so controllers
/// never reach into the repository for them.
pub use crate::repositories::review_repository::ReviewSort as Sort;

/// Parses the request's sort value; anything unknown is the
/// default, newest first.
pub fn parse_sort(value: Option<&str>) -> Sort {
    review_repository::parse_sort(value)
}

/// Validates the submitted form, returning every field error at
/// once. The rating must land on a half step between 0.5 and 5.0 —
/// five stars, each divisible in half, so 0.5 fills half of the
/// first star. Half steps are exact in binary floating point, so
/// the step test is exact, never approximate. The comment is
/// optional (a rating alone is a review) and capped at 1000
/// characters; a cap is an error, never a silent truncation.
pub fn validate(input: &ReviewInput) -> Result<ValidReview, ReviewFieldErrors> {
    let mut errors = ReviewFieldErrors::default();

    let rating = match input.rating.as_deref().map(str::trim) {
        None | Some("") => {
            errors.rating = Some("評価を選択してください".to_string());
            None
        }
        Some(text) => match text.parse::<f64>() {
            Ok(value) if (0.5..=5.0).contains(&value) && (value * 2.0).fract() == 0.0 => {
                Some(value)
            }
            _ => {
                errors.rating = Some("評価は0.5から5の間で、0.5刻みで選択してください".to_string());
                None
            }
        },
    };

    let comment = input.comment.trim();
    if comment.chars().count() > 1000 {
        errors.comment = Some("コメントは1000文字以内で入力してください".to_string());
    }

    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(ValidReview {
        rating: rating.unwrap_or_default(),
        comment: comment.to_string(),
    })
}

/// Returns whether this member may review this product, proven by
/// their own delivered order lines. A database failure reads as "not
/// eligible" for the page, but it is logged: a silent failure
/// would hide outages behind missing review buttons.
pub async fn is_eligible(pool: &PgPool, user_id: i64, product_id: i64) -> bool {
    match review_repository::qualifying_order_id(pool, user_id, product_id).await {
        Ok(found) => found.is_some(),
        Err(e) => {
            tracing::error!(user_id, product_id, "review eligibility check failed: {e}");
            false
        }
    }
}

/// The member's own review of this product, when it exists.
/// A database failure is logged and reads as absence.
pub async fn own_review(pool: &PgPool, user_id: i64, product_id: i64) -> Option<Review> {
    match review_repository::find_own(pool, user_id, product_id).await {
        Ok(found) => found,
        Err(e) => {
            tracing::error!(user_id, product_id, "own review lookup failed: {e}");
            None
        }
    }
}

/// One review by id, only when it belongs to this member; a
/// foreign id and a missing id are the same None, so the
/// review's existence is never confirmed to anyone else.
/// A database failure is logged and reads as absence.
pub async fn own_review_by_id(pool: &PgPool, review_id: i64, user_id: i64) -> Option<Review> {
    match review_repository::find_by_id_for_user(pool, review_id, user_id).await {
        Ok(found) => found,
        Err(e) => {
            tracing::error!(review_id, user_id, "review lookup failed: {e}");
            None
        }
    }
}

/// The member's own review IDs among several products, as a
/// product_id → review_id map (the order detail page's review
/// buttons). A database failure is logged and yields an empty
/// map — the buttons are a page section, not the page.
pub async fn own_review_ids(pool: &PgPool, user_id: i64, product_ids: &[i64]) -> HashMap<i64, i64> {
    match review_repository::find_own_for_products(pool, user_id, product_ids).await {
        Ok(map) => map,
        Err(e) => {
            tracing::error!(user_id, "own review ids lookup failed: {e}");
            HashMap::new()
        }
    }
}

/// Returns the rating aggregates for a set of products, in one
/// grouped query (the product cards' ratings). A database failure
/// is logged and yields an empty map — cards then render without
/// stars rather than failing the page.
pub async fn aggregate_map(pool: &PgPool, product_ids: &[i64]) -> HashMap<i64, RatingAggregate> {
    match review_repository::aggregate_for_products(pool, product_ids).await {
        Ok(map) => map,
        Err(e) => {
            tracing::error!("review aggregates failed: {e}");
            HashMap::new()
        }
    }
}

/// The outcome of trying to store a new review.
pub enum CreateOutcome {
    /// Stored; carries the new review id.
    Created(i64),
    /// The member has no delivered order containing the product.
    NotEligible,
    /// A review for this member and product already exists.
    Duplicate,
}

/// Creates a review. Eligibility is re-checked here, at posting,
/// never trusted from the form having been shown: a form can be
/// kept and submitted later, and a POST can be crafted without
/// ever opening the form.
pub async fn create(
    pool: &PgPool,
    user_id: i64,
    product_id: i64,
    valid: &ValidReview,
) -> Result<CreateOutcome, AppError> {
    let Some(order_id) = review_repository::qualifying_order_id(pool, user_id, product_id).await?
    else {
        return Ok(CreateOutcome::NotEligible);
    };
    match review_repository::insert(pool, user_id, product_id, order_id, valid).await {
        Ok(review_id) => {
            tracing::info!(review_id, product_id, user_id, "review_created");
            Ok(CreateOutcome::Created(review_id))
        }
        Err(err) if is_unique_violation(&err) => Ok(CreateOutcome::Duplicate),
        Err(err) => Err(err.into()),
    }
}

/// Updates the member's own review. Returns false when the
/// row is gone — a foreign or missing id, which the
/// controller answers with the styled 404.
///
/// `expected` is the form-ticket staleness guard: the
/// review's `updated_at` (epoch microseconds) recorded
/// server-side when the edit form was rendered. When the
/// row still exists but no longer carries that stamp, the
/// save is stale and refuses with [`AppError::Conflict`]
/// rather than silently reverting the newer edit.
pub async fn update(
    pool: &PgPool,
    user_id: i64,
    review: &Review,
    valid: &ValidReview,
    expected: i64,
) -> Result<bool, AppError> {
    let Some(expected_at) = chrono::DateTime::from_timestamp_micros(expected) else {
        return Err(crate::services::form_ticket_service::stale_conflict());
    };
    let changed = review_repository::update(pool, review.id, user_id, valid, expected_at).await?;
    if changed {
        tracing::info!(
            review_id = review.id,
            product_id = review.product_id,
            user_id,
            "review_updated"
        );
        return Ok(true);
    }
    match review_repository::find_by_id_for_user(pool, review.id, user_id).await? {
        Some(_) => Err(crate::services::form_ticket_service::stale_conflict()),
        None => Ok(false),
    }
}

/// Hard-deletes the member's own review. Returns false when
/// no row matched.
pub async fn delete(pool: &PgPool, user_id: i64, review: &Review) -> Result<bool, AppError> {
    let removed = review_repository::delete(pool, review.id, user_id).await?;
    if removed {
        tracing::info!(
            review_id = review.id,
            product_id = review.product_id,
            user_id,
            "review_deleted"
        );
    }
    Ok(removed)
}

/// One page of a product's reviews plus everything the
/// summary block needs, all computed from the same rows.
#[derive(Default)]
pub struct ReviewPage {
    /// The reviews on this page, in the requested order.
    pub reviews: Vec<Review>,
    /// Reviews across all pages.
    pub total: i64,
    /// The product's aggregate (absent average at 0 reviews).
    pub aggregate: RatingAggregate,
    /// The star distribution, 5.0 down to 0.5 in half steps.
    pub distribution: Vec<DistributionRow>,
}

/// Loads one page of the review section for the product page.
pub async fn review_page(
    pool: &PgPool,
    product_id: i64,
    sort: ReviewSort,
    page: i64,
) -> Result<ReviewPage, AppError> {
    let page = page.max(1);
    let (reviews, total) = review_repository::find_by_product(
        pool,
        product_id,
        sort,
        CONFIG.reviews_per_page,
        page.saturating_sub(1)
            .saturating_mul(CONFIG.reviews_per_page),
    )
    .await?;
    let aggregate = review_repository::aggregate_for_products(pool, &[product_id])
        .await?
        .get(&product_id)
        .copied()
        .unwrap_or_default();
    let counts = review_repository::distribution(pool, product_id).await?;
    // Ten rows, 5.0 down to 0.5: whole steps labelled "5",
    // half steps "4.5". The stored values are exact halves,
    // so matching a row to its count by equality is safe.
    let distribution = (1..=10)
        .rev()
        .map(|half| {
            let stars = half as f64 / 2.0;
            let count = counts
                .iter()
                .find(|(rating, _)| *rating == stars)
                .map(|(_, count)| *count)
                .unwrap_or(0);
            let percent = if total > 0 {
                (count as f64 / total as f64 * 100.0).round() as i64
            } else {
                0
            };
            let label = if stars.fract() == 0.0 {
                format!("{}", stars as i64)
            } else {
                format!("{stars:.1}")
            };
            DistributionRow {
                stars,
                label,
                count,
                percent,
            }
        })
        .collect();
    Ok(ReviewPage {
        reviews,
        total,
        aggregate,
        distribution,
    })
}

/// True when the error is a uniqueness constraint (PostgreSQL
/// 23505) answering a raced duplicate.
pub(crate) fn is_unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db| db.code())
        .is_some_and(|code| code == "23505")
}

/// The helpful-vote identity of the current reader, when
/// they already have one: members are `u:<user id>`, guests
/// are `s:<cart owner token>` when a token exists. Reading
/// never mints a guest token — browsing must not create
/// identity — so a tokenless guest reads as "not voted".
pub async fn read_voter_key(
    session: &tower_sessions::Session,
    user: Option<&crate::models::User>,
) -> Option<String> {
    if let Some(user) = user {
        return Some(format!("u:{}", user.id));
    }
    crate::services::cart_service::existing_guest_token(session)
        .await
        .map(|token| format!("s:{token}"))
}

/// The helpful-vote identity for a vote being cast. Casting
/// is a deliberate action, so a guest without a cart token
/// gets one minted — the same rule as the first cart use
/// (see `cart_service::owner_for`).
pub async fn write_voter_key(pool: &PgPool, session: &tower_sessions::Session) -> String {
    match crate::services::cart_service::owner_for(pool, session).await {
        crate::services::cart_service::CartOwner::User(id) => format!("u:{id}"),
        crate::services::cart_service::CartOwner::Session(token) => format!("s:{token}"),
    }
}

/// (count, voted) per review id for one page of reviews:
/// two set queries total, never one per review.
pub async fn helpful_map(
    pool: &PgPool,
    review_ids: &[i64],
    voter: Option<&str>,
) -> Result<HashMap<i64, (i64, bool)>, AppError> {
    let counts = review_repository::helpful_counts(pool, review_ids).await?;
    let voted = match voter {
        Some(key) => review_repository::helpful_voted(pool, review_ids, key).await?,
        None => std::collections::HashSet::new(),
    };
    Ok(review_ids
        .iter()
        .map(|id| {
            (
                *id,
                (counts.get(id).copied().unwrap_or(0), voted.contains(id)),
            )
        })
        .collect())
}

/// Casts one reader's helpful vote on a review of
/// `product_id` and returns the review's new helpful
/// count, or `None` when the review does not exist or
/// belongs to another product (the caller answers 404).
/// The (review, voter) unique pair makes a repeat vote a
/// no-op, so the count never moves twice for one reader.
/// A failed count read after the vote lands degrades to 0
/// with a warning: the vote itself has succeeded, and the
/// number in the response is not worth failing over.
pub async fn mark_helpful(
    pool: &PgPool,
    product_id: i64,
    review_id: i64,
    voter: &str,
) -> Result<Option<i64>, AppError> {
    let review = review_repository::find_by_id(pool, review_id)
        .await
        .map_err(|e| AppError::db("helpful vote lookup", e))?;
    let Some(review) = review.filter(|r| r.product_id == product_id) else {
        return Ok(None);
    };
    review_repository::record_helpful_vote(pool, review.id, voter)
        .await
        .map_err(|e| AppError::db("helpful vote", e))?;
    let count = match review_repository::helpful_count(pool, review.id).await {
        Ok(count) => count,
        Err(e) => {
            tracing::warn!("helpful count read failed: {e}");
            0
        }
    };
    Ok(Some(count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(rating: Option<&str>, comment: &str) -> ReviewInput {
        ReviewInput {
            rating: rating.map(str::to_string),
            comment: comment.to_string(),
        }
    }

    #[test]
    fn half_step_ratings_pass_and_the_comment_is_optional() {
        let valid = validate(&input(Some("4.5"), "軽くて丈夫です")).unwrap();
        assert_eq!(valid.rating, 4.5);
        assert_eq!(valid.comment, "軽くて丈夫です");
        // A rating alone is a complete review.
        let valid = validate(&input(Some("0.5"), "")).unwrap();
        assert_eq!(valid.rating, 0.5);
        assert_eq!(valid.comment, "");
        // The comment is stored trimmed.
        let valid = validate(&input(Some("5"), "  満足です  ")).unwrap();
        assert_eq!(valid.comment, "満足です");
    }

    #[test]
    fn missing_rating_reports_and_a_long_comment_still_can() {
        let errors = validate(&input(None, &"あ".repeat(1001))).unwrap_err();
        assert_eq!(errors.rating.as_deref(), Some("評価を選択してください"));
        assert_eq!(
            errors.comment.as_deref(),
            Some("コメントは1000文字以内で入力してください")
        );
        // Exactly at the cap is valid: the cap counts
        // characters, not bytes (each あ is three bytes).
        assert!(validate(&input(Some("4"), &"あ".repeat(1000))).is_ok());
    }

    #[test]
    fn off_step_and_out_of_range_ratings_get_the_step_message() {
        for bad in ["0", "5.5", "6", "abc", "4.3", "1.25"] {
            let errors = validate(&input(Some(bad), "")).unwrap_err();
            assert_eq!(
                errors.rating.as_deref(),
                Some("評価は0.5から5の間で、0.5刻みで選択してください"),
                "rating {bad}"
            );
        }
        for good in ["0.5", "1", "1.5", "2.5", "3.0", "4.5", "5"] {
            assert!(validate(&input(Some(good), "")).is_ok(), "rating {good}");
        }
    }
}
