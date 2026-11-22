//! App crate for Kaimoru.
//!
//! The web layer, organised in a one way flow:
//! routes -> controllers -> services -> repositories -> models.
//!
//! Start at [`routes::create_router`]. The shared database pool comes
//! from [`db::init_pool`], and every page, admin and webhook
//! request passes through [`util::logging::request_log`] — static
//! assets and the health check deliberately do not.

pub mod api;
pub mod controllers;
pub mod db;
pub mod error;
pub mod mail;
pub mod messages;
pub mod middleware;
pub mod models;
pub mod pii;
pub mod pii_backfill;
pub mod pricing;
pub mod repositories;
pub mod routes;
pub mod services;
pub mod session;
pub mod session_store;
pub mod util;
pub mod validation;
pub mod views;

pub use configuration::config;

pub use util::asset_fingerprint::asset_fingerprint;
