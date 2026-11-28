//! The store's discovery profile: a machine-readable statement
//! of what this store is and which agent-facing capabilities it
//! offers, served at `/.well-known/ucp`.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use sqlx::PgPool;

use crate::services::shop_settings_service;

/// The UCP draft whose shape this profile follows. The profile
/// is inspired by UCP, not compliant with it, and lists only
/// capabilities the store actually implements.
const PROFILE_VERSION: &str = "2026-08-25";

/// One HTTP endpoint that implements part of a capability.
#[derive(Serialize)]
pub struct Endpoint {
    /// Short endpoint name an agent can refer to.
    pub name: &'static str,
    /// HTTP method, e.g. `GET`.
    pub method: &'static str,
    /// Path template, e.g. `/api/products/{id}`.
    pub path: &'static str,
}

/// One capability the store offers to agents.
#[derive(Serialize)]
pub struct Capability {
    /// Capability name, e.g. `catalog`.
    pub name: &'static str,
    /// Version of the capability's shape.
    pub version: &'static str,
    /// What the capability means, when its shape alone could
    /// mislead: the checkout capability carries one, because
    /// an empty endpoint list must never read as a checkout
    /// an agent can call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<&'static str>,
    /// The endpoints that implement it.
    pub endpoints: Vec<Endpoint>,
}

/// Public facts about the shop itself.
#[derive(Serialize)]
pub struct Shop {
    /// Shop name from the shop settings.
    pub name: String,
    /// Plain-text shop introduction from the shop settings.
    pub description: String,
    /// ISO 4217 currency code; prices are integer yen.
    pub currency: &'static str,
}

/// The `ucp` block: protocol version and offered capabilities.
#[derive(Serialize)]
pub struct Ucp {
    /// The UCP draft version this profile follows.
    pub version: &'static str,
    /// Capabilities the store actually implements.
    pub capabilities: Vec<Capability>,
}

/// The discovery profile document as a whole.
#[derive(Serialize)]
pub struct Profile {
    /// Protocol block: version and capabilities.
    pub ucp: Ucp,
    /// Shop block: who this store is.
    pub shop: Shop,
}

/// Serves the discovery profile. Public by design: it contains
/// only facts the storefront already shows to every visitor.
pub async fn profile(State(pool): State<PgPool>) -> Json<Profile> {
    let settings = shop_settings_service::load(&pool).await;
    Json(Profile {
        ucp: Ucp {
            version: PROFILE_VERSION,
            capabilities: vec![
                Capability {
                    name: "catalog",
                    version: PROFILE_VERSION,
                    description: None,
                    endpoints: vec![
                        Endpoint {
                            name: "list_products",
                            method: "GET",
                            path: "/api/products",
                        },
                        Endpoint {
                            name: "get_product",
                            method: "GET",
                            path: "/api/products/{id}",
                        },
                    ],
                },
                Capability {
                    name: "cart",
                    version: PROFILE_VERSION,
                    description: None,
                    endpoints: vec![
                        Endpoint {
                            name: "create_cart",
                            method: "POST",
                            path: "/api/carts",
                        },
                        Endpoint {
                            name: "get_cart",
                            method: "GET",
                            path: "/api/carts/current",
                        },
                        Endpoint {
                            name: "add_cart_item",
                            method: "POST",
                            path: "/api/carts/current/items",
                        },
                    ],
                },
                Capability {
                    name: "checkout",
                    version: PROFILE_VERSION,
                    description: Some(
                        "Checkout is completed by the cart owner in the browser after explicit approval of the final contents, paid through the store's payment provider. There is no agent-callable checkout endpoint.",
                    ),
                    endpoints: vec![],
                },
            ],
        },
        shop: Shop {
            name: settings.shop_name,
            description: settings.shop_description,
            currency: "JPY",
        },
    })
}

/// Routes for the discovery profile.
pub fn router() -> Router<PgPool> {
    Router::new().route("/.well-known/ucp", get(profile))
}
