//! Search controller.
//! Product search for guests and members alike: identical routes,
//! identical results, no login gate and no search state in the
//! session — everything lives in the URL.

use std::net::SocketAddr;

use axum::{
    Router,
    extract::{ConnectInfo, Query, State},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::config::CONFIG;
use crate::services::{product_service, search_service};
use crate::views::storefront::search::{SearchView, SuggestView};

/// Routes owned by the search controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/search", get(search_page))
        .route("/search/suggest", get(suggest))
}

/// Query parameters of the results page. All optional; the URL is
/// the whole state, so result pages are bookmarkable and shareable.
#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    category: Option<String>,
    tag: Option<String>,
    sort: Option<String>,
    page: Option<String>,
    in_stock: Option<String>,
    min_price: Option<String>,
    max_price: Option<String>,
}

/// GET /search: the results page.
async fn search_page(
    session: Session,
    State(pool): State<PgPool>,
    Query(params): Query<SearchQuery>,
) -> Response {
    let query_text = params.q.clone().unwrap_or_default();
    let patterns = search_service::normalize_patterns(&query_text);
    let category = params
        .category
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let (sort, sort_key) = search_service::parse_sort(params.sort.as_deref());
    let in_stock = params.in_stock.as_deref() == Some("1");
    // The price range validates in the service: an invalid
    // bound is dropped with a per-field error shown beside
    // its input, never silently reinterpreted.
    let price_range =
        search_service::parse_price_range(params.min_price.as_deref(), params.max_price.as_deref());
    let min_price_text = params
        .min_price
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let max_price_text = params
        .max_price
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let tag_slug = params
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let page = params
        .page
        .as_deref()
        .and_then(|raw| raw.parse::<i64>().ok())
        .unwrap_or(1)
        .max(1);
    let offset = page
        .saturating_sub(1)
        .saturating_mul(CONFIG.search_per_page);

    // The category list is page furniture: a failure is logged
    // and the page renders without it.
    let categories = match product_service::categories(&pool).await {
        Ok(categories) => categories,
        Err(e) => {
            tracing::error!("search categories failed: {e}");
            Vec::new()
        }
    };

    // A bookmarked URL for a category outside the active
    // list (retired, or with no public products right now)
    // still shows its own filter as the selected one.
    let mut categories = categories;
    if let Some(name) = category
        && !categories.iter().any(|c| c == name)
        && product_service::category_exists(&pool, name)
            .await
            .unwrap_or(false)
    {
        categories.push(name.to_string());
    }
    // The tag filter's display label; an unknown slug simply
    // matches nothing and shows no chip.
    let tag_label = match tag_slug {
        Some(slug) => search_service::tag_label(&pool, slug).await.unwrap_or(None),
        None => None,
    };

    let (products, total, error_message) = match search_service::search(
        &pool,
        search_service::SearchArgs {
            patterns: &patterns,
            category,
            tag_slug,
            in_stock_only: in_stock,
            min_price: price_range.min,
            max_price: price_range.max,
            sort,
            limit: CONFIG.search_per_page,
            offset,
        },
    )
    .await
    {
        Ok(outcome) => (outcome.products, outcome.total, None),
        Err(e) => {
            // Includes the statement timeout: the spec's answer to a
            // slow search is a busy message, never a broader retry.
            tracing::error!("Product search failed: {e}");
            (
                Vec::new(),
                0,
                Some("検索が混み合っています。しばらくしてから再度お試しください。".to_string()),
            )
        }
    };

    let total_pages = crate::util::pagination::total_pages(total, CONFIG.search_per_page);
    // A page past the end of a non-empty result set clamps to
    // the last page (the section listings' rule) instead of
    // rendering the zero-result block; a genuinely empty search
    // (total 0) keeps the block.
    let (products, page) = if error_message.is_none() && total > 0 && page > total_pages {
        let last_offset = (total_pages - 1).saturating_mul(CONFIG.search_per_page);
        match search_service::search(
            &pool,
            search_service::SearchArgs {
                patterns: &patterns,
                category,
                tag_slug,
                in_stock_only: in_stock,
                min_price: price_range.min,
                max_price: price_range.max,
                sort,
                limit: CONFIG.search_per_page,
                offset: last_offset,
            },
        )
        .await
        {
            Ok(outcome) => (outcome.products, total_pages),
            Err(e) => {
                tracing::error!("Product search clamp failed: {e}");
                (products, page)
            }
        }
    } else {
        (products, page)
    };
    let viewer = crate::session::current_user(&pool, &session)
        .await
        .map(|user| user.id);
    let products = super::product_cards(&pool, viewer, products).await;
    render(SearchView {
        header: header_for(&pool, &session, false).await,
        query: query_text,
        category: category.unwrap_or_default().to_string(),
        tag_slug: tag_slug.unwrap_or_default().to_string(),
        tag_label,
        sort: sort_key.to_string(),
        in_stock,
        min_price: min_price_text,
        max_price: max_price_text,
        min_price_error: price_range.min_error,
        max_price_error: price_range.max_error,
        products,
        total,
        page,
        total_pages: total_pages.max(1),
        categories,
        error_message,
    })
}

/// Query parameters of the suggestion endpoint.
#[derive(Deserialize)]
pub struct SuggestQuery {
    q: Option<String>,
}

/// GET /search/suggest: name suggestions as an HTML fragment for
/// htmx. Rate limited per IP; a refused or too-short query simply
/// returns an empty fragment, which clears the suggestion box.
async fn suggest(
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Query(params): Query<SuggestQuery>,
) -> Response {
    let empty = || Html(String::new()).into_response();
    if !search_service::suggest_allowed(&crate::session::client_ip(&addr, &headers)) {
        return empty();
    }
    let Some(pattern) = search_service::suggest_pattern(params.q.as_deref().unwrap_or_default())
    else {
        return empty();
    };
    match search_service::suggest_names(&pool, &pattern, search_service::SUGGEST_LIMIT).await {
        Ok(names) => render(SuggestView { names }),
        Err(e) => {
            tracing::error!("Suggest failed: {e}");
            empty()
        }
    }
}
