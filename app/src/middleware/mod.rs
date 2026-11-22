//! Request-pipeline middleware and guards.
//!
//! The modules that sit between the router and the controllers:
//! CSRF protection, the admin IP allowlist, the admin permission
//! guard, the authentication throttle, and the maintenance-mode
//! switch. They are wired as layers in [`crate::routes`]; none
//! of them holds business logic.

pub mod admin_ip_guard;
pub mod admin_permission_guard;
pub mod auth_throttle;
pub mod csrf;
pub mod maintenance;
