//! Admin-surface repositories: the queries only the back
//! office runs (customer/order/product/review administration
//! lists and actions). Domain repositories shared with the
//! storefront stay in the parent module. The admins-table
//! repository (`admin_repository`) and the admin two-factor
//! repository (`totp_repository`) live here too: both serve
//! only the back office - identity, the audit trail, and
//! admin TOTP state.

pub mod admin_repository;
pub mod customer_repository;
pub mod order_repository;
pub mod product_repository;
pub mod return_repository;
pub mod review_repository;
pub mod rfm_repository;
pub mod segment_repository;
pub mod shipment_repository;
pub mod totp_repository;
