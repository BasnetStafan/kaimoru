//! Controllers for Kaimoru, grouped by surface:
//! [`admin`] (the back office at `/admin`) and [`storefront`]
//! (the member-facing shop). Controllers own HTTP concerns
//! only: request input, status codes and template rendering.
//! Business rules live in the shared services below both.

pub mod admin;
pub mod storefront;
