//! Home controller.
//! Serves the store top page, assembled from every top page section.

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use sqlx::PgPool;
use tower_sessions::Session;

use crate::services::{home_content_service, notice_service, product_service};
use crate::views::storefront::home::{HeroView, HomeBlock, HomeView};

/// Routes owned by the home controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/", get(index_handler))
        .route("/index.html", get(index_handler))
}

/// Handles GET / by loading every section's data and rendering
/// the top page. A database failure returns a plain 500: details
/// go to the log only, never to the customer.
async fn index_handler(session: Session, State(pool): State<PgPool>) -> Response {
    let data = tokio::try_join!(
        product_service::categories(&pool),
        product_service::new_arrivals(&pool),
        product_service::ranking(&pool),
        product_service::event(&pool),
        product_service::staff_picks(&pool),
        notice_service::latest(&pool),
    );

    let (categories, new_auto, ranking_auto, event_auto, staff_products, notices) = match data {
        Ok(values) => values,
        Err(e) => {
            tracing::error!("Failed to load top page data: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };

    // The top page is where sign-up and login land, so it takes
    // (and thereby clears) the one-time flash message.
    let header = super::header_for(&pool, &session, true).await;

    // The operator's top page content (CMS C-3): the hero
    // leads the page — it is the page's visual anchor — then
    // the sections in their stored order, hidden ones
    // skipped, with the one remaining fixed block, event
    // pickup, anchored after the ranking. The anchored block
    // renders at its anchor's position whether or not that
    // section is visible, so hiding a section never hides
    // its neighbour.
    let content = home_content_service::load(&pool).await;
    let hero = content
        .live_hero()
        .map(|slot| HeroView::from_slot(slot, &content.hero_images));

    // The operator's pins lead their sections' rows, the
    // automatic selections filling the remaining slots;
    // the featured section is its pins alone. A failed pin
    // read degrades to the automatic rows, never to a 500:
    // the pins are an overlay, not the page's foundation.
    let pins = &content.pins;
    // The four pin reads run concurrently;
    // each section's failure degrades that
    // section alone to its automatic row.
    let (new_pinned, ranking_pinned, event_pinned, featured_pinned) = tokio::join!(
        product_service::pinned_products(&pool, pins, "new_arrivals"),
        product_service::pinned_products(&pool, pins, "ranking"),
        product_service::pinned_products(&pool, pins, "event"),
        product_service::pinned_products(&pool, pins, "featured"),
    );
    let (new_products, ranking_products, event_products, featured_products) = (
        product_service::merge_section(
            new_pinned.unwrap_or_default(),
            new_auto,
            product_service::section_limit("new_arrivals").unwrap_or(0),
        ),
        product_service::merge_section(
            ranking_pinned.unwrap_or_default(),
            ranking_auto,
            product_service::section_limit("ranking").unwrap_or(0),
        ),
        product_service::merge_section(
            event_pinned.unwrap_or_default(),
            event_auto,
            product_service::section_limit("event").unwrap_or(0),
        ),
        product_service::merge_section(
            featured_pinned.unwrap_or_default(),
            Vec::new(),
            product_service::section_limit("featured").unwrap_or(0),
        ),
    );

    let mut blocks = Vec::new();
    if hero.is_some() {
        blocks.push(HomeBlock::fixed("hero"));
    }
    for section in content.ordered_sections() {
        // The staff picks section renders only when it has
        // products: an empty recommendation row would read
        // as a defect, not as content.
        if section.is_visible
            && !(section.section_key == "staff_picks" && staff_products.is_empty())
        {
            blocks.push(HomeBlock::section(&section));
        }
        if section.section_key == "ranking" {
            blocks.push(HomeBlock::fixed("event"));
        }
    }

    // One aggregate query for all three sections' cards, not one
    // per section: the sections are concatenated, carded in a
    // single call and split back by their lengths.
    let new_len = new_products.len();
    let ranking_len = ranking_products.len();
    let event_len = event_products.len();
    let featured_len = featured_products.len();
    let staff_len = staff_products.len();
    let mut all_products = new_products;
    all_products.extend(ranking_products);
    all_products.extend(event_products);
    all_products.extend(featured_products);
    all_products.extend(staff_products);
    let viewer = crate::session::current_user(&pool, &session)
        .await
        .map(|user| user.id);
    let mut cards = super::product_cards(&pool, viewer, all_products)
        .await
        .into_iter();
    let new_products: Vec<_> = cards.by_ref().take(new_len).collect();
    let ranking_products: Vec<_> = cards.by_ref().take(ranking_len).collect();
    let event_products: Vec<_> = cards.by_ref().take(event_len).collect();
    let featured_products: Vec<_> = cards.by_ref().take(featured_len).collect();
    let staff_products: Vec<_> = cards.by_ref().take(staff_len).collect();
    // The product rows gain their see-more links under
    // the see-more rule: only when the section holds more
    // than its row displays.
    // The see-more totals arrive as one grouped query
    //; a failed read leaves every row
    // without its link, as a failed per-section read did.
    let totals = product_service::section_totals(&pool)
        .await
        .unwrap_or_default();
    for block in blocks.iter_mut() {
        if matches!(
            block.key,
            "new_arrivals" | "ranking" | "event" | "staff_picks"
        ) && let Some(&total) = totals.get(block.key)
            && total > product_service::section_limit(block.key).unwrap_or(0)
        {
            block.more_url = format!("/sections/{}", block.key);
        }
    }

    // The category nav shows only the config number of
    // categories; the rest live behind its see-more link
    // to the category index (the see-more rule).
    let categories_more = categories.len() > product_service::category_nav_count().max(0) as usize;
    let categories: Vec<String> = categories
        .into_iter()
        .take(product_service::category_nav_count().max(0) as usize)
        .collect();

    let view = HomeView {
        header,
        categories,
        categories_more,
        new_products,
        ranking_products,
        event_products,
        featured_products,
        staff_products,
        notices,
        blocks,
        hero,
    };

    super::render(view)
}
