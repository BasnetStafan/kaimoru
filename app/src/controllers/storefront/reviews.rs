//! Reviews controller.
//!
//! Members review products they bought. Guests are turned away
//! by the [`CurrentUser`] extractor (login redirect); a member
//! who never bought the product gets the styled 404, and a
//! review id that is foreign or missing gets the app's standard
//! styled 404 — indistinguishable, so a foreign id never
//! confirms the review exists. Every POST is CSRF-checked by
//! the csrf middleware, as every form in the app is.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Form, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::error::AppError;
use crate::messages::{REVIEW_DELETED_NOTICE, REVIEW_POSTING_BUSY_MESSAGE};
use crate::models::Product;
use crate::models::review::{Review, ReviewFieldErrors, ReviewInput};
use crate::services::product_service;
use crate::services::review_service::{self, CreateOutcome};
use crate::session::{CurrentUser, SESSION_FLASH};
use crate::util::rate_limit_knobs::SWEEP_THRESHOLD;
use crate::views::storefront::errors::PageNotFoundView;
use crate::views::storefront::header::Header;
use crate::views::storefront::product::NotFoundView;
use crate::views::storefront::reviews::{HelpfulView, ReviewFormView, ReviewNotFoundView};

/// Posts (creates and edits together) one member may make in an
/// hour before further posts are refused.
const POST_LIMIT: usize = 10;
const POST_WINDOW: Duration = Duration::from_secs(3600);

/// In-memory post log, one entry per member — the app's
/// disposable-throttle pattern, as with the login throttle:
/// counters reset on restart, and that is accepted.
static POST_LOG: LazyLock<Mutex<HashMap<i64, Vec<Instant>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Whether this member is still under the hourly post limit.
fn post_allowed(user_id: i64) -> bool {
    let now = Instant::now();
    let mut log = POST_LOG.lock().unwrap_or_else(|p| p.into_inner());
    if log.len() >= SWEEP_THRESHOLD && !log.contains_key(&user_id) {
        log.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < POST_WINDOW);
            !times.is_empty()
        });
    }
    let entry = log.entry(user_id).or_default();
    entry.retain(|t| now.duration_since(*t) < POST_WINDOW);
    entry.len() < POST_LIMIT
}

/// Records one successful post against the member's limit.
fn record_post(user_id: i64) {
    let mut log = POST_LOG.lock().unwrap_or_else(|p| p.into_inner());
    log.entry(user_id).or_default().push(Instant::now());
}

/// Routes owned by the reviews controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/products/{id}/reviews/new", get(new_form))
        .route("/products/{id}/reviews", post(create))
        .route("/reviews/{id}/edit", get(edit_form))
        .route("/reviews/{id}", post(update))
        .route("/reviews/{id}/delete", post(delete))
        .route("/products/{pid}/reviews/{rid}/helpful", post(mark_helpful))
}

/// The review form as posted. The rating stays text until the
/// service validates it, so a crafted value reaches the spec's
/// messages instead of failing form parsing.
#[derive(Deserialize)]
struct ReviewForm {
    rating: Option<String>,
    #[serde(default)]
    comment: String,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the form, which the
    /// edit submit treats as stale. The create submit
    /// ignores it: a new review has no record to guard.
    #[serde(default)]
    form_ticket: String,
}

impl ReviewForm {
    fn into_input(self) -> ReviewInput {
        ReviewInput {
            rating: self.rating,
            comment: self.comment,
        }
    }
}

/// Loads the product a raw path id names; a non-numeric id and a
/// missing product are the same None to the visitor. A database
/// failure is not folded into that None: it comes back as an
/// error, and each caller decides what it means for its page.
async fn load_product(pool: &PgPool, raw_id: &str) -> Result<Option<Product>, AppError> {
    let Some(id) = raw_id.parse::<i64>().ok() else {
        return Ok(None);
    };
    product_service::detail(pool, id).await
}

/// The outcome of a create path's product load.
enum ProductLoad {
    /// The product exists.
    Found(Box<Product>),
    /// No such product (or an unparseable id).
    Missing,
    /// The database failed; already logged.
    Failed,
}

/// The create paths' product load: a database failure is kept
/// distinct from absence — the form's whole subject failed to
/// load, which is a 500, not a 404.
async fn load_product_or_500(pool: &PgPool, raw_id: &str) -> ProductLoad {
    match load_product(pool, raw_id).await {
        Ok(Some(product)) => ProductLoad::Found(Box::new(product)),
        Ok(None) => ProductLoad::Missing,
        Err(e) => {
            tracing::error!("review form product load failed: {e}");
            ProductLoad::Failed
        }
    }
}

/// The edit paths' product load: a failure is logged and reads
/// as missing, keeping the foreign/missing indistinguishable
/// 404 those paths promise.
async fn load_product_lenient(pool: &PgPool, product_id: i64) -> Option<Product> {
    match product_service::detail(pool, product_id).await {
        Ok(found) => found,
        Err(e) => {
            tracing::error!(product_id, "edit form product load failed: {e}");
            None
        }
    }
}

/// Loads a review only when it belongs to this member;
/// ownership is enforced inside the query.
async fn load_own_review(pool: &PgPool, raw_id: &str, user_id: i64) -> Option<Review> {
    let id: i64 = raw_id.parse().ok()?;
    review_service::own_review_by_id(pool, id, user_id).await
}

/// The product-missing 404 (the product page's own view).
async fn product_not_found(pool: &PgPool, session: &Session) -> Response {
    (
        StatusCode::NOT_FOUND,
        render(NotFoundView {
            header: header_for(pool, session, false).await,
        }),
    )
        .into_response()
}

/// The review-denied 404 for a member who never bought the
/// product: the app does not render a form it would refuse.
async fn review_denied(pool: &PgPool, session: &Session) -> Response {
    (
        StatusCode::NOT_FOUND,
        render(ReviewNotFoundView {
            header: header_for(pool, session, false).await,
        }),
    )
        .into_response()
}

/// The app's standard styled 404, for foreign or missing
/// review ids — indistinguishable either way.
fn page_not_found(header: Header) -> Response {
    (StatusCode::NOT_FOUND, render(PageNotFoundView { header })).into_response()
}

/// Sets the one-time confirmation the product page shows.
async fn flash(session: &Session, message: &str) {
    let _ = session.insert(SESSION_FLASH, message).await;
}

/// The form-ticket scope of one review.
fn review_scope(id: i64) -> String {
    format!("review:{id}")
}

/// Mints the edit form's ticket for `review` at its current
/// stamp.
async fn review_ticket(session: &Session, review: &Review) -> String {
    crate::services::form_ticket_service::issue(
        session,
        &review_scope(review.id),
        review.updated_at.timestamp_micros(),
    )
    .await
}

/// Renders the review edit form as a stale-save refusal:
/// the current values (never the stale form's), the shared
/// refusal message, and a fresh ticket, so the re-rendered
/// form can be saved from the current record.
async fn stale_review_response(
    pool: &PgPool,
    session: &Session,
    review: &Review,
    product: Product,
) -> Response {
    let form_ticket = review_ticket(session, review).await;
    render_form(
        pool,
        session,
        product,
        Some(review.id),
        &ReviewInput {
            rating: Some(review.rating.to_string()),
            comment: review.comment.clone(),
        },
        ReviewFieldErrors::default(),
        Some(crate::messages::STALE_FORM_MESSAGE.to_string()),
        form_ticket,
    )
    .await
}

/// Renders the review form. The member's input travels back in
/// on a failed post, never cleared.
#[allow(clippy::too_many_arguments)]
async fn render_form(
    pool: &PgPool,
    session: &Session,
    product: Product,
    editing_id: Option<i64>,
    input: &ReviewInput,
    errors: ReviewFieldErrors,
    form_error: Option<String>,
    form_ticket: String,
) -> Response {
    render(ReviewFormView {
        header: header_for(pool, session, false).await,
        product,
        editing_id,
        rating: input.rating.as_deref().and_then(|t| t.trim().parse().ok()),
        comment: input.comment.clone(),
        errors,
        form_error,
        form_ticket,
    })
    .into_response()
}

/// GET /products/{id}/reviews/new: the create form.
///
/// A member who already reviewed the product is taken to the
/// edit form instead — one review per product, reached by
/// editing, never by a duplicate.
async fn new_form(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let product = match load_product_or_500(&pool, &raw_id).await {
        ProductLoad::Found(product) => *product,
        ProductLoad::Missing => return product_not_found(&pool, &session).await,
        ProductLoad::Failed => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    if let Some(own) = review_service::own_review(&pool, user.id, product.id).await {
        return Redirect::to(&format!("/reviews/{}/edit", own.id)).into_response();
    }
    if !review_service::is_eligible(&pool, user.id, product.id).await {
        return review_denied(&pool, &session).await;
    }
    render_form(
        &pool,
        &session,
        product,
        None,
        &ReviewInput {
            rating: None,
            comment: String::new(),
        },
        ReviewFieldErrors::default(),
        None,
        String::new(),
    )
    .await
}

/// POST /products/{id}/reviews: create, with eligibility
/// re-checked at posting — the service re-checks again at the
/// insert, so a form kept open past a state change still cannot
/// write.
async fn create(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
    Form(form): Form<ReviewForm>,
) -> Response {
    let product = match load_product_or_500(&pool, &raw_id).await {
        ProductLoad::Found(product) => *product,
        ProductLoad::Missing => return product_not_found(&pool, &session).await,
        ProductLoad::Failed => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    // The eligibility gate comes before any form re-render:
    // an ineligible member never sees this form, posted to or
    // otherwise.
    if !review_service::is_eligible(&pool, user.id, product.id).await {
        return review_denied(&pool, &session).await;
    }
    let input = form.into_input();
    let valid = match review_service::validate(&input) {
        Ok(valid) => valid,
        Err(errors) => {
            return render_form(
                &pool,
                &session,
                product,
                None,
                &input,
                errors,
                None,
                String::new(),
            )
            .await;
        }
    };
    if !post_allowed(user.id) {
        return render_form(
            &pool,
            &session,
            product,
            None,
            &input,
            ReviewFieldErrors::default(),
            Some(REVIEW_POSTING_BUSY_MESSAGE.to_string()),
            String::new(),
        )
        .await;
    }
    match review_service::create(&pool, user.id, product.id, &valid).await {
        Ok(CreateOutcome::Created(_)) => {
            record_post(user.id);
            flash(&session, "レビューを投稿しました").await;
            Redirect::to(&format!("/products/{}#reviews", product.id)).into_response()
        }
        Ok(CreateOutcome::NotEligible) => review_denied(&pool, &session).await,
        Ok(CreateOutcome::Duplicate) => {
            render_form(
                &pool,
                &session,
                product,
                None,
                &input,
                ReviewFieldErrors::default(),
                Some("この商品のレビューはすでに投稿されています".to_string()),
                String::new(),
            )
            .await
        }
        Err(_) => {
            render_form(
                &pool,
                &session,
                product,
                None,
                &input,
                ReviewFieldErrors::default(),
                Some("投稿できませんでした。時間をおいて再度お試しください".to_string()),
                String::new(),
            )
            .await
        }
    }
}

/// GET /reviews/{id}/edit: the edit form for the member's own
/// review, prefilled from the stored row.
async fn edit_form(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let header = header_for(&pool, &session, false).await;
    let Some(review) = load_own_review(&pool, &raw_id, user.id).await else {
        return page_not_found(header);
    };
    let Some(product) = load_product_lenient(&pool, review.product_id).await else {
        return page_not_found(header);
    };
    let form_ticket = review_ticket(&session, &review).await;
    render_form(
        &pool,
        &session,
        product,
        Some(review.id),
        &ReviewInput {
            rating: Some(review.rating.to_string()),
            comment: review.comment.clone(),
        },
        ReviewFieldErrors::default(),
        None,
        form_ticket,
    )
    .await
}

/// POST /reviews/{id}: update the member's own review.
async fn update(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
    Form(form): Form<ReviewForm>,
) -> Response {
    let header = header_for(&pool, &session, false).await;
    let Some(review) = load_own_review(&pool, &raw_id, user.id).await else {
        return page_not_found(header);
    };
    let Some(product) = load_product_lenient(&pool, review.product_id).await else {
        return page_not_found(header);
    };
    // Spend the form ticket before validating: a submit
    // without a valid ticket was not rendered from the
    // current record (a replayed form, an expired session),
    // so it is refused as stale without touching the row. A
    // ticket whose stamp has gone stale is refused by the
    // service's check instead — both land on the same
    // refusal render with the current values.
    let submitted_ticket = form.form_ticket.clone();
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &review_scope(review.id),
        &submitted_ticket,
    )
    .await
    else {
        return stale_review_response(&pool, &session, &review, product).await;
    };
    // One fresh ticket serves every re-render below: the
    // record does not move during this request unless the
    // save itself lands, and that path redirects away.
    let fresh_ticket = review_ticket(&session, &review).await;
    let input = form.into_input();
    let valid = match review_service::validate(&input) {
        Ok(valid) => valid,
        Err(errors) => {
            return render_form(
                &pool,
                &session,
                product,
                Some(review.id),
                &input,
                errors,
                None,
                fresh_ticket,
            )
            .await;
        }
    };
    if !post_allowed(user.id) {
        return render_form(
            &pool,
            &session,
            product,
            Some(review.id),
            &input,
            ReviewFieldErrors::default(),
            Some(REVIEW_POSTING_BUSY_MESSAGE.to_string()),
            fresh_ticket,
        )
        .await;
    }
    match review_service::update(&pool, user.id, &review, &valid, expected).await {
        Ok(true) => {
            record_post(user.id);
            flash(&session, "レビューを更新しました").await;
            Redirect::to(&format!("/products/{}#reviews", review.product_id)).into_response()
        }
        Ok(false) => page_not_found(header),
        Err(AppError::Conflict(_)) => {
            let current = load_own_review(&pool, &raw_id, user.id)
                .await
                .unwrap_or(review);
            stale_review_response(&pool, &session, &current, product).await
        }
        Err(_) => {
            render_form(
                &pool,
                &session,
                product,
                Some(review.id),
                &input,
                ReviewFieldErrors::default(),
                Some("更新できませんでした。時間をおいて再度お試しください".to_string()),
                fresh_ticket,
            )
            .await
        }
    }
}

/// POST /reviews/{id}/delete: hard-delete the member's own
/// review. POST only, never a link and never a GET; the form
/// asks for confirmation first, like logout does.
async fn delete(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let header = header_for(&pool, &session, false).await;
    let Some(review) = load_own_review(&pool, &raw_id, user.id).await else {
        return page_not_found(header);
    };
    match review_service::delete(&pool, user.id, &review).await {
        Ok(true) => {
            flash(&session, REVIEW_DELETED_NOTICE).await;
            Redirect::to(&format!("/products/{}#reviews", review.product_id)).into_response()
        }
        _ => page_not_found(header),
    }
}

/// `POST /products/{pid}/reviews/{rid}/helpful`: one
/// 参考になった vote from this reader. The voter identity
/// is the cart's own discipline (member id, or the guest
/// session's cart token — minted here, as at first cart
/// use, because voting is a deliberate act). The
/// (review, voter) unique pair makes a repeat vote a
/// no-op: the count never moves twice for one reader.
/// Answers the htmx fragment, or a redirect back to the
/// product's review section for a plain form post.
async fn mark_helpful(
    session: Session,
    State(pool): State<PgPool>,
    Path((raw_pid, raw_rid)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Response {
    let (Ok(product_id), Ok(review_id)) = (raw_pid.parse::<i64>(), raw_rid.parse::<i64>()) else {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    };
    let voter = review_service::write_voter_key(&pool, &session).await;
    let count = match review_service::mark_helpful(&pool, product_id, review_id, &voter).await {
        Ok(Some(count)) => count,
        Ok(None) => return (StatusCode::NOT_FOUND, "Not Found").into_response(),
        // The service logged the failure at its source.
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    if headers.contains_key("hx-request") {
        return render(HelpfulView {
            helpful_url: format!("/products/{product_id}/reviews/{review_id}/helpful"),
            helpful_count: count,
            voted: true,
        });
    }
    Redirect::to(&format!("/products/{product_id}#reviews")).into_response()
}
