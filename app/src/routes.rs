//! Route assembly for the whole app.
//!
//! Combines the storefront and `/admin` routers, each with its own
//! session stack, and mounts the static files under `/static`. The
//! pool is shared with every handler as Axum state.

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;
use sqlx::PgPool;
use std::path::PathBuf;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_sessions::SessionManagerLayer;
use tower_sessions_sqlx_store::PostgresStore;
use tracing::info;

use crate::controllers;
use crate::util::logging;

/// Builds a layer that sets one response header on every response.
fn header_layer(name: &'static str, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(
        HeaderName::from_static(name),
        HeaderValue::from_static(value),
    )
}

/// Returns the directory the static files are served from.
///
/// `STATIC_DIR` wins when set: the deployed binary runs from a
/// copied directory with no source tree, where the compile-time
/// default would point nowhere. The default is
/// `resources/static` beside the build workspace — the Spring
/// Boot resources layout. The asset fingerprint hashes this
/// same tree, so the two can never disagree.
pub(crate) fn static_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("STATIC_DIR")
        && !dir.trim().is_empty()
    {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap() // -> workspace root
        .join("resources")
        .join("static")
}

/// Builds the complete application router around the shared pool.
///
/// [`crate::util::logging::request_log`] sits inside the session layer,
/// so each log line can carry the surface, the member number and a
/// session fingerprint (never the raw session ID). Security headers
/// are set on every response — the store serves only its own
/// content, so a strict same-origin policy costs nothing. Static
/// assets get a cache lifetime; HTML is never cached.
pub fn create_router(
    pool: PgPool,
    session_layer: SessionManagerLayer<PostgresStore>,
    admin_session_layer: SessionManagerLayer<PostgresStore>,
) -> Router {
    let static_files_path = static_dir();
    // Warm the asset fingerprint at boot: computed lazily, its
    // tree read would otherwise land on the first page request.
    crate::util::asset_fingerprint::asset_fingerprint();
    let static_files = Router::new()
        .nest_service("/static", ServeDir::new(&static_files_path))
        .layer(header_layer("cache-control", "public, max-age=86400"))
        .layer(CompressionLayer::new())
        .layer(axum::middleware::from_fn(fingerprinted_static_cache));

    info!(
        "{:<12} - Serving static files from: {:?}",
        "STATIC FILES", static_files_path
    );
    // The load balancer's health check is a sibling of the whole
    // page stack: it must not create a session, pass CSRF, write a
    // log line or touch the database — the ALB polls it constantly.
    let health = Router::new().route("/healthz", axum::routing::get(|| async { "ok" }));

    // The webhook receiver is a sibling of the health check:
    // no session stack (KOMOJU has no cookie), no CSRF layer
    // (it authenticates by HMAC signature, not by token), but
    // it does get request logging under its own surface.
    let webhooks =
        crate::api::router()
            .with_state(pool.clone())
            .layer(axum::middleware::from_fn_with_state(
                logging::Surface::Webhook,
                logging::request_log,
            ));

    // The agent-readable API is a sibling of the webhook
    // receiver: no session stack, no CSRF layer, but request
    // logging under its own surface.
    let agent_api = crate::api::agent_router()
        .with_state(pool.clone())
        .layer(axum::middleware::from_fn_with_state(
            pool.clone(),
            crate::middleware::maintenance::gate,
        ))
        .layer(axum::middleware::from_fn_with_state(
            logging::Surface::Api,
            logging::request_log,
        ));

    // The agent cart API shares the member session — the
    // guest cart an agent builds is the cart its owner later
    // reviews and approves in the browser — but not the CSRF
    // layer: it accepts JSON only, and the session cookie is
    // SameSite=Lax, so a cross-site form post cannot ride it.
    // Logged under the API surface like the catalog reads.
    let agent_cart = crate::api::cart_router()
        .with_state(pool.clone())
        .layer(axum::middleware::from_fn_with_state(
            pool.clone(),
            crate::middleware::maintenance::gate,
        ))
        .layer(axum::middleware::from_fn_with_state(
            logging::Surface::Api,
            logging::request_log,
        ))
        .layer(session_layer.clone());

    // The postal code lookup is public reference data with
    // no per-visitor state: like the agent reads it mounts
    // with no session stack and no CSRF layer, logged under
    // the API surface.
    let postal_api = crate::api::postal_router()
        .with_state(pool.clone())
        .layer(axum::middleware::from_fn_with_state(
            pool.clone(),
            crate::middleware::maintenance::gate,
        ))
        .layer(axum::middleware::from_fn_with_state(
            logging::Surface::Api,
            logging::request_log,
        ));

    // The crawler documents (sitemap.xml, robots.txt) are
    // public catalog data with no per-visitor state: like the
    // postal lookup they mount with no session stack and no
    // CSRF layer — a crawler fetch must not create a session —
    // under the maintenance gate, logged on the storefront
    // surface they describe. The OAuth protected-resource
    // metadata document rides the same mount: it is the
    // public pointer MCP clients follow to the authorization
    // server, and it answers 404 while OAuth is inactive.
    let crawler_docs = controllers::storefront::sitemap::router()
        .merge(controllers::storefront::oauth_metadata::router())
        .with_state(pool.clone())
        .layer(axum::middleware::from_fn_with_state(
            pool.clone(),
            crate::middleware::maintenance::gate,
        ))
        .layer(axum::middleware::from_fn_with_state(
            logging::Surface::Storefront,
            logging::request_log,
        ));

    // The MCP server (Phase 1: the read-only public catalog
    // for external AI clients) mounts only when the deployment
    // enables it — with the flag off the path does not exist.
    // Like the agent API it carries no session stack and no
    // CSRF layer: its callers are MCP clients, not browsers,
    // and Phase 1 serves public data only. It sits under the
    // maintenance gate, logged on the API surface.
    let mcp_api = if crate::config::CONFIG.mcp_enabled {
        Router::new()
            .route_service("/mcp", crate::api::mcp::service(pool.clone()))
            .layer(axum::middleware::from_fn(
                crate::middleware::mcp_oauth::gate,
            ))
            .layer(axum::middleware::from_fn_with_state(
                pool.clone(),
                crate::middleware::maintenance::gate,
            ))
            .layer(axum::middleware::from_fn_with_state(
                logging::Surface::Api,
                logging::request_log,
            ))
    } else {
        Router::new()
    };

    // Each surface gets its own stack: CSRF inside, its own session
    // layer outside, so the two cookie jars never mix.
    let pages = surface_stack(
        controllers::storefront::router(),
        &pool,
        logging::Surface::Storefront,
    )
    .layer(axum::middleware::from_fn_with_state(
        pool.clone(),
        crate::middleware::maintenance::gate,
    ))
    .layer(session_layer);
    let admin_pages = surface_stack(controllers::admin::router(), &pool, logging::Surface::Admin)
        // The permission guard runs inside the session layer
        // (it needs the resolved session) and outside the
        // handlers: a staff admin asking for an area they do
        // not hold is refused before any handler work happens.
        .layer(axum::middleware::from_fn_with_state(
            pool.clone(),
            crate::middleware::admin_permission_guard::require_permission,
        ))
        .layer(admin_session_layer)
        // The IP allowlist guard wraps the whole admin stack:
        // a disallowed source is refused before any session
        // work happens.
        .layer(axum::middleware::from_fn_with_state(
            crate::middleware::admin_ip_guard::from_config(),
            crate::middleware::admin_ip_guard::require_allowed_ip,
        ));

    // Campaign tracking endpoints are mail infrastructure,
    // not a shopping surface: already-delivered mail must
    // keep recording opens and landing clicks even while
    // the storefront is in maintenance, so — like the
    // webhook receiver — this stack skips the maintenance
    // gate, and like it carries no session stack.
    let mail_tracking =
        crate::api::tracking_router()
            .with_state(pool)
            .layer(axum::middleware::from_fn_with_state(
                logging::Surface::Api,
                logging::request_log,
            ));

    let router = pages
        .merge(admin_pages)
        .merge(mail_tracking)
        // Static assets sit outside both session stacks: the file
        // server never reads a session, so asset requests create
        // no sessions and no log lines.
        .merge(static_files)
        .merge(health)
        .merge(webhooks)
        .merge(agent_api)
        .merge(agent_cart)
        .merge(postal_api)
        .merge(crawler_docs)
        .merge(mcp_api)
        // The header layers come after every merge: in axum a
        // layer applied before a `.merge` does not reach the
        // merged-in routes, so the health, webhook and API
        // responses missed the headers while the doc above
        // promised them on every response.
        // form-action names both hosted payment pages: the
        // checkout confirm POST redirects the browser to the
        // active rail's hosted page, and a redirect target
        // outside form-action is blocked by the browser after
        // the order is already placed (the buyer is left on
        // the confirm screen). KOMOJU and Stripe Checkout are
        // the two hosted rails.
        .layer(header_layer(
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; form-action 'self' https://komoju.com https://checkout.stripe.com; base-uri 'self'; frame-ancestors 'none'",
        ))
        .layer(header_layer("x-content-type-options", "nosniff"))
        .layer(header_layer("referrer-policy", "same-origin"))
        .layer(header_layer("x-frame-options", "DENY"));
    // The outermost layer: a panicking
    // handler degrades to the styled error page instead of
    // dropping the visitor's connection. Panics stay loud -
    // the handler logs the panic message at error level.
    router.layer(tower_http::catch_panic::CatchPanicLayer::custom(
        handle_panic,
    ))
}

/// Renders the styled 500 page for a panicked handler
///. The header is the guest header with no
/// database work behind it: the panic may well be a broken
/// database path, and the error page must not depend on it.
fn handle_panic(err: Box<dyn std::any::Any + Send>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let detail = err
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| err.downcast_ref::<&str>().copied())
        .unwrap_or("unknown panic");
    tracing::error!("handler panicked: {detail}");
    let view = crate::views::storefront::errors::ServerErrorView {
        header: crate::views::storefront::header::Header::guest(),
    };
    use askama::Template;
    let body = view
        .render()
        .unwrap_or_else(|_| "システムエラーが発生しました".to_string());
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        axum::response::Html(body),
    )
        .into_response()
}

/// Applies the pages' default cache rule — `private,
/// no-cache`, HTML is never cached — only when the handler
/// did not set a rule of its own. A handler's stricter word
/// wins: the content-page preview answers `no-store`, because
/// a draft of a legal page must not persist in any cache
/// (CMS spec D6), and the old overriding header layer made
/// that impossible to say.
async fn default_cache_control(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    if !response.headers().contains_key(header::CACHE_CONTROL) {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-cache"),
        );
    }
    response
}

/// Gives fingerprinted CSS and JS a year of immutable caching.
///
/// Every stylesheet and script URL the layouts emit carries
/// `?v=<asset fingerprint>`, and the fingerprint changes when
/// any asset file does — so the bytes behind one fingerprinted
/// URL can never change, which is exactly what `immutable`
/// promises. Images stay on the static router's blanket day
/// rule: an uploaded image's name does not change when its
/// file is replaced, so immutable caching would freeze the
/// old picture.
async fn fingerprinted_static_cache(request: Request, next: Next) -> Response {
    let fingerprinted = {
        let uri = request.uri();
        let path = uri.path();
        (path.ends_with(".css") || path.ends_with(".js"))
            && uri.query().is_some_and(|q| {
                // Only the real fingerprint earns `immutable`:
                // a made-up v= falls back to the day rule, so a
                // crafted link cannot pin a visitor to stale CSS.
                q.split('&').any(|part| {
                    part.strip_prefix("v=")
                        == Some(crate::util::asset_fingerprint::asset_fingerprint())
                })
            })
    };
    let mut response = next.run(request).await;
    if fingerprinted {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    response
}

/// Builds the stack every surface subtree shares, minus its
/// session layer: CSRF enforcement, the no-cache rule for HTML,
/// and request logging.
///
/// The caller applies its own session layer outside this stack,
/// which is what lets CSRF read the session token. The `surface`
/// argument is what the request log stamps on every line from
/// this subtree ([`crate::util::logging::Surface`]): the assembly
/// declares it, so the logged surface cannot drift from the
/// routing. HTML is never cached: a page restored from the
/// browser's back/forward cache must not show a stale cart badge.
fn surface_stack(router: Router<PgPool>, pool: &PgPool, surface: logging::Surface) -> Router {
    router
        .layer(axum::middleware::from_fn_with_state(
            pool.clone(),
            crate::middleware::csrf::protect,
        ))
        .layer(axum::middleware::from_fn(default_cache_control))
        .with_state(pool.clone())
        .layer(axum::middleware::from_fn_with_state(
            surface,
            logging::request_log,
        ))
        // Compression wraps the whole stack, so it encodes the
        // final body — after CSRF has rewritten the forms into
        // it. The webhook and health routers are siblings of
        // this stack, not part of it, and stay uncompressed.
        .layer(CompressionLayer::new())
}
