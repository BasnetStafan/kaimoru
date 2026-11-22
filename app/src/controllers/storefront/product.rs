//! Product detail controller.
//! One canonical page per product; unknown or malformed IDs get
//! a real 404 page, never a 500 or a redirect home.

use std::net::SocketAddr;

use axum::{
    Form, Router,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, header_for_user, render};
use crate::config::CONFIG;
use crate::services::{product_service, review_service};
use crate::views::storefront::product::{
    GalleryImage, NotFoundView, ProductDetailView, VariantPicker,
};
use crate::views::storefront::product_card::{BuyboxView, QuickAddView};
use crate::views::storefront::restock::{
    RestockCard, RestockErrors, RestockFormFragment, RestockOption,
};

/// Routes owned by the product controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/products/{id}", get(detail))
        .route("/products/{id}/buybox", get(buybox))
        .route("/products/{id}/quick-add", get(quick_add))
        .route("/products/{id}/restock", post(restock_subscribe))
}

/// Renders the 404 page with its status.
async fn not_found(pool: &PgPool, session: &Session) -> Response {
    let view = NotFoundView {
        header: header_for(pool, session, false).await,
    };
    let mut response = render(view);
    *response.status_mut() = StatusCode::NOT_FOUND;
    response
}

/// Query parameters of the detail page: the review section's
/// sort and page. Both are optional and validated in the
/// service/repository, never trusted raw.
#[derive(Deserialize)]
pub struct ReviewQuery {
    sort: Option<String>,
    page: Option<String>,
}

/// GET /products/{id}: the detail page.
async fn detail(
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
    Query(query): Query<ReviewQuery>,
) -> Response {
    // The segment is an ID when numeric and a slug otherwise:
    // /products/{id} stays canonical, and a product's slug
    // resolves to the same page.
    // A database failure is a 500, never a 404: "we could not
    // look" and "it does not exist" are different answers, and
    // masking an outage as 404 hid failures from both visitors
    // and monitoring.
    let product = match raw_id.parse::<i64>() {
        Ok(id) => match product_service::detail(&pool, id).await {
            Ok(Some(product)) => product,
            Ok(None) => return not_found(&pool, &session).await,
            Err(e) => {
                tracing::error!(product_id = id, "product lookup failed: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
                    .into_response();
            }
        },
        Err(_) => match product_service::detail_by_slug(&pool, &raw_id).await {
            Ok(Some(product)) => product,
            Ok(None) => return not_found(&pool, &session).await,
            Err(e) => {
                tracing::error!(slug = %raw_id, "product slug lookup failed: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
                    .into_response();
            }
        },
    };
    let id = product.id;
    // The review section: one page of the list plus the summary
    // computed from the same rows.
    let sort = review_service::parse_sort(query.sort.as_deref());
    let mut page = query
        .page
        .as_deref()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(1)
        .max(1);
    // The page's independent reads run concurrently
    //: related products, the variation
    // picker, the review section, the signed-in member, the
    // tags, the category state, the gallery and today's tax
    // rates share no inputs, so none of them waits for
    // another. Each keeps its own degrade-on-error handling
    // below, exactly as when they ran in sequence.
    let (
        related_products,
        picker,
        mut section,
        user,
        tags_result,
        category_active,
        images_result,
        tax_rates_result,
    ) = tokio::join!(
        product_service::related(&pool, &product),
        load_picker(&pool, product.id),
        load_review_section(&pool, product.id, sort, page),
        crate::session::current_user(&pool, &session),
        product_service::product_tags(&pool, product.id),
        product_service::category_is_active(&pool, product.category_id),
        product_service::product_images_detailed(&pool, product.id),
        crate::repositories::tax_repository::class_rates_on(&pool, crate::models::jst::jst_today()),
    );
    let total_pages =
        crate::util::pagination::total_pages(section.total, CONFIG.reviews_per_page).max(1);
    if page > total_pages {
        // A page past the end shows the last page instead of an
        // empty list under a full summary.
        page = total_pages;
        section = load_review_section(&pool, product.id, sort, page).await;
    }

    // Write-button state: only a buyer who has not reviewed
    // yet is offered the form; an existing review turns the
    // button into edit. Guests and non-buyers see neither.
    // Favourite state for the buybox toggle and the related
    // cards: one bulk resolution for the cards, one point read
    // for this product; guests are simply unmarked.
    // The three member-dependent reads also run concurrently
    //; the review-write chain keeps its own
    // internal order (an existing review makes the
    // eligibility read unnecessary).
    let viewer_id = user.as_ref().map(|u| u.id);
    let (related, is_favorite, write_state) = tokio::join!(
        super::product_cards(&pool, viewer_id, related_products),
        async {
            match &user {
                Some(viewer) => {
                    crate::services::favorite_service::is_favorite(&pool, viewer.id, product.id)
                        .await
                }
                None => false,
            }
        },
        async {
            match &user {
                Some(user) => match review_service::own_review(&pool, user.id, product.id).await {
                    Some(own) => (false, Some(own.id)),
                    None => (
                        review_service::is_eligible(&pool, user.id, product.id).await,
                        None,
                    ),
                },
                None => (false, None),
            }
        },
    );
    let is_member = user.is_some();
    let (can_write, own_review_id) = write_state;

    // Helpful-vote state for this page of reviews: counts
    // plus this reader's own votes, two set queries total.
    let voter = review_service::read_voter_key(&session, user.as_ref()).await;
    let review_cards = build_review_cards(&pool, section.reviews, voter.as_deref()).await;

    // The product page takes the flash: review posts land here
    // with their one-time confirmation.
    // The tag row and the category link state. A failed tag
    // read degrades to no chips — the product is the page.
    let tags = match tags_result {
        Ok(tags) => tags,
        Err(e) => {
            tracing::error!(product_id = id, "product tags failed: {e}");
            Vec::new()
        }
    };
    // The gallery: ordered image paths, main first. A failed
    // or empty read degrades to the main image alone — the
    // synced accessor on the product is the truth for that.
    let images = match images_result {
        Ok(images) if !images.is_empty() => images,
        Ok(_) => vec![(product.image_path.clone(), String::new())],
        Err(e) => {
            tracing::error!(product_id = id, "product images failed: {e}");
            vec![(product.image_path.clone(), String::new())]
        }
    };
    let images: Vec<GalleryImage> = images
        .into_iter()
        .map(|(original, alt)| GalleryImage { original, alt })
        .collect();

    // The tax class name and today's rate, for the price
    // decomposition and the spec table's 消費税区分 row. A
    // failed read degrades to the joined name at a 0 rate:
    // the product is the page.
    let (tax_class_name, tax_rate_percent) = match tax_rates_result {
        Ok(rates) => rates
            .iter()
            .find(|cr| cr.class.id == product.tax_class_id)
            .map(|cr| (cr.class.name.clone(), cr.rate_percent))
            .unwrap_or_else(|| (product.tax_class_name.clone(), 0)),
        Err(e) => {
            tracing::error!(product_id = id, "product tax class failed: {e}");
            (product.tax_class_name.clone(), 0)
        }
    };

    // The back-in-stock card: offered only while a
    // combination is registrably sold out (same conditions
    // the service re-checks at registration).
    let restock = build_restock_card(&product, &picker, user.as_ref());

    let settings = crate::services::shop_settings_service::load(&pool).await;
    // Points display (override layer): what one item earns
    // at the product's effective rate, while the programme
    // runs; excluded (0%) and sub-point prices show nothing.
    let points_cfg = settings.points_settings();
    let points_for_one = if points_cfg.enabled {
        let pts = crate::services::point_service::earned_points(
            product.effective_price_yen(),
            product.points_earn_percent_or(points_cfg.earn_percent),
        );
        (pts > 0).then_some(pts)
    } else {
        None
    };
    let fees = settings.fee_settings();
    let (earliest_off, latest_off) = settings.delivery_offsets();
    let today = crate::models::jst::jst_today();
    render(ProductDetailView {
        header: header_for_user(&pool, &session, true, user).await,
        payment_method_labels: crate::services::payment_service::offered_methods()
            .iter()
            .map(|method| method.label().to_string())
            .collect(),
        shipping_fee_label: crate::pricing::display_price(fees.shipping_flat_yen),
        free_shipping_threshold_label: crate::pricing::display_price(
            fees.free_shipping_threshold_yen,
        ),
        cod_fee_label: crate::pricing::display_price(fees.cod_fee_yen),
        delivery_earliest_label: crate::models::delivery::earliest_delivery_label_with_holidays(
            today,
            earliest_off,
            latest_off,
            &crate::services::holiday_service::upcoming(&pool).await,
        ),
        delivery_range_label: crate::models::delivery::delivery_window_range_label_with_holidays(
            today,
            earliest_off,
            latest_off,
            &crate::services::holiday_service::upcoming(&pool).await,
        ),
        product,
        tax_class_name,
        tax_rate_percent,
        images,
        tags,
        category_active,
        picker,
        rating: section.aggregate,
        related,
        reviews: review_cards,
        review_total: section.total,
        points_for_one,
        distribution: section.distribution,
        review_sort: sort.as_str().to_string(),
        review_page: page,
        review_total_pages: total_pages,
        can_write,
        own_review_id,
        is_favorite,
        is_member,
        restock,
    })
}

/// Builds the back-in-stock card for the detail page, or
/// `None` when nothing on the product is registrable:
/// registration is offered exactly for enabled combinations
/// at zero stock on a listed product that tracks stock
/// without backorders or preorders.
fn build_restock_card(
    product: &crate::models::Product,
    picker: &VariantPicker,
    user: Option<&crate::models::User>,
) -> Option<RestockCard> {
    if !product.track_stock || product.allow_backorder || product.is_preorder {
        return None;
    }
    let options: Vec<RestockOption> = picker
        .variants
        .iter()
        .filter(|v| v.is_enabled && v.stock == 0)
        .map(|v| RestockOption {
            variant_id: v.id,
            label: picker_variant_label(picker, v),
        })
        .collect();
    if options.is_empty() {
        return None;
    }
    let single_variant_id = if options.len() == 1 {
        Some(options[0].variant_id)
    } else {
        None
    };
    Some(RestockCard {
        product_id: product.id,
        options,
        single_variant_id,
        email: user.map(|u| u.email.clone()).unwrap_or_default(),
        errors: RestockErrors::default(),
        done: false,
        already: false,
    })
}

/// Returns one combination's display label from the picker
/// state: its axis/value pairs in axis order, the same
/// wording the cart and order lines carry.
fn picker_variant_label(
    picker: &VariantPicker,
    variant: &crate::models::variant::ProductVariant,
) -> String {
    if variant.option_signature.is_empty() {
        return "標準".to_string();
    }
    let pairs: Vec<(String, String)> = variant
        .option_signature
        .split('-')
        .filter_map(|raw| {
            let option_id: i64 = raw.parse().ok()?;
            picker.axes.iter().find_map(|axis| {
                axis.options
                    .iter()
                    .find(|o| o.id == option_id)
                    .map(|o| (axis.axis.name.clone(), o.value.clone()))
            })
        })
        .collect();
    let label = crate::models::variant::label_for(&pairs);
    if label.is_empty() {
        "標準".to_string()
    } else {
        label
    }
}

/// Form posted by the back-in-stock card.
#[derive(Deserialize)]
pub struct RestockForm {
    variant_id: String,
    email: String,
}

/// POST /products/{id}/restock: registers one back-in-stock
/// subscription and answers with the card fragment — the
/// registered (or already-registered) state, or the form
/// again with its errors. The service re-checks every
/// registrability condition; the card's own offer is only
/// the page's rendering of them.
async fn restock_subscribe(
    session: Session,
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(raw_id): Path<String>,
    Form(form): Form<RestockForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    };
    let product = match product_service::detail(&pool, id).await {
        Ok(Some(product)) => product,
        Ok(None) => return (StatusCode::NOT_FOUND, "Not Found").into_response(),
        Err(e) => {
            tracing::error!(product_id = id, "restock product lookup failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let picker = load_picker(&pool, product.id).await;
    let user = crate::session::current_user(&pool, &session).await;
    let Some(mut card) = build_restock_card(&product, &picker, user.as_ref()) else {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    };
    card.email = form.email.clone();
    // The chosen combination must be one the card offered:
    // a tampered ID is a form error, never a registration
    // against some other product's combination.
    let variant_id: i64 = form.variant_id.trim().parse().unwrap_or(0);
    if !card.options.iter().any(|o| o.variant_id == variant_id) {
        card.errors.form = Some("選択した種類は現在、再入荷通知に登録できません。".to_string());
        return render(RestockFormFragment { restock: card });
    }
    let email_errors = crate::services::restock_service::validate_email(&form.email);
    if let Some(error) = email_errors.into_iter().next() {
        card.errors.email = Some(error.message);
        return render(RestockFormFragment { restock: card });
    }
    let ip = crate::session::client_ip(&addr, &headers);
    let outcome = crate::services::restock_service::subscribe(
        &pool,
        &ip,
        variant_id,
        &form.email,
        user.as_ref().map(|u| u.id),
    )
    .await;
    match outcome {
        Ok(crate::services::restock_service::SubscribeOutcome::Subscribed) => {
            card.done = true;
        }
        Ok(crate::services::restock_service::SubscribeOutcome::AlreadyRegistered) => {
            card.already = true;
        }
        Ok(crate::services::restock_service::SubscribeOutcome::RateLimited) => {
            card.errors.form =
                Some(crate::services::restock_service::RATE_LIMIT_MESSAGE.to_string());
        }
        Ok(crate::services::restock_service::SubscribeOutcome::NotAvailable) => {
            card.errors.form = Some("この商品は現在、再入荷通知に登録できません。".to_string());
        }
        Err(e) => {
            tracing::error!(product_id = id, "restock subscribe failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    }
    render(RestockFormFragment { restock: card })
}

/// Loads one page of the review section. A failure renders empty
/// with a log line: the product is the page, the reviews only a
/// section of it.
async fn load_review_section(
    pool: &PgPool,
    product_id: i64,
    sort: review_service::Sort,
    page: i64,
) -> review_service::ReviewPage {
    match review_service::review_page(pool, product_id, sort, page).await {
        Ok(section) => section,
        Err(e) => {
            tracing::error!(product_id, "review section failed: {e}");
            review_service::ReviewPage::default()
        }
    }
}

/// Loads the variation picker state for a product. A failure
/// degrades to a picker-less buy area with a log line: the
/// product itself is the page, the variations only its choice.
async fn load_picker(pool: &PgPool, product_id: i64) -> VariantPicker {
    // The combinations are ordered from the same axes load the
    // picker renders: ordering from a second load would repeat
    // the axes and options queries on every page.
    let axes = match product_service::variant_axes(pool, product_id).await {
        Ok(axes) => axes,
        Err(e) => {
            tracing::error!(product_id, "variant axes failed: {e}");
            return VariantPicker {
                axes: Vec::new(),
                variants: Vec::new(),
            };
        }
    };
    let variants = product_service::variants_for_axes(pool, product_id, &axes)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(product_id, "variants failed: {e}");
            Vec::new()
        });
    VariantPicker { axes, variants }
}

/// GET /products/{id}/quick-add: the quick-add modal fragment
/// — product summary, the shared variation
/// picker and the add form, loaded into the layout's modal
/// shell by the listing cards' 選択する button. Same split as
/// the buybox: failure is a 500, absence is a 404.
async fn quick_add(State(pool): State<PgPool>, Path(raw_id): Path<String>) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    };
    match product_service::detail(&pool, id).await {
        Ok(Some(product)) => {
            let picker = load_picker(&pool, product.id).await;
            render(QuickAddView { product, picker })
        }
        Ok(None) => (StatusCode::NOT_FOUND, "Not Found").into_response(),
        Err(e) => {
            tracing::error!(product_id = id, "quick-add lookup failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// GET /products/{id}/buybox: the buy area fragment.
async fn buybox(
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    };
    // Same split as the detail page: failure is a 500, absence
    // is a 404.
    match product_service::detail(&pool, id).await {
        Ok(Some(product)) => {
            let picker = load_picker(&pool, product.id).await;
            let user = crate::session::current_user(&pool, &session).await;
            let is_favorite = match &user {
                Some(viewer) => {
                    crate::services::favorite_service::is_favorite(&pool, viewer.id, product.id)
                        .await
                }
                None => false,
            };
            render(BuyboxView {
                product,
                picker,
                is_favorite,
                is_member: user.is_some(),
            })
        }
        Ok(None) => (StatusCode::NOT_FOUND, "Not Found").into_response(),
        Err(e) => {
            tracing::error!(product_id = id, "buybox lookup failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// Wraps one page of reviews with their helpful-vote state
/// for the template. A failed vote read degrades to zeroes:
/// the reviews are the content, the counts are garnish.
async fn build_review_cards(
    pool: &PgPool,
    reviews: Vec<crate::models::review::Review>,
    voter: Option<&str>,
) -> Vec<crate::views::storefront::product::ReviewCardView> {
    let ids: Vec<i64> = reviews.iter().map(|review| review.id).collect();
    let map = match review_service::helpful_map(pool, &ids, voter).await {
        Ok(map) => map,
        Err(e) => {
            tracing::warn!("helpful vote state failed: {e}");
            Default::default()
        }
    };
    reviews
        .into_iter()
        .map(|review| {
            let (count, voted) = map.get(&review.id).copied().unwrap_or((0, false));
            crate::views::storefront::product::ReviewCardView {
                review,
                helpful_count: count,
                voted,
            }
        })
        .collect()
}
