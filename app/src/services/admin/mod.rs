//! Admin-surface services: the business rules only the
//! back office invokes. Services shared with the storefront
//! stay in the parent module.

pub mod admin_account_service;
pub mod analytics_service;
pub mod audit_query_service;
pub mod audit_service;
pub mod auth_service;
pub mod category_csv_service;
pub mod category_service;
pub mod content_page_service;
pub mod coupon_service;
pub mod customer_service;
pub mod dashboard_service;
pub mod holiday_service;
pub mod home_content_service;
pub mod mail_service;
pub mod menu_service;
pub mod notice_service;
pub mod notification_template_service;
pub mod ops_status_service;
pub mod order_service;
pub mod product_csv_service;
pub mod product_service;
pub mod reconciliation_service;
pub mod return_service;
pub mod review_service;
pub mod segment_service;
pub mod shipment_service;
pub mod shipping_csv_service;
pub mod shop_settings_service;
pub mod tag_csv_service;
pub mod tag_service;
pub mod tax_service;
pub mod totp_service;
