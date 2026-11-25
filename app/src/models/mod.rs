//! Models for Kaimoru: plain data shared across layers,
//! with no HTTP details.

pub mod admin;
pub mod admin_rows;
pub mod agent;
pub mod agent_approval;
pub mod audit_log;
pub mod carrier;
pub mod cart;
pub mod code_challenge;
pub mod content_page;
pub mod coupon;
pub mod delivery;
pub mod favorite;
pub mod gift;
pub mod home_content;
pub mod inquiry;
pub mod jst;
pub mod mail;
pub mod mail_scenario;
pub mod member_address;
pub mod menu;
pub mod notice;
pub mod notification_template;
pub mod order;
pub mod payment;
pub mod pending_registration;
pub mod points;
pub mod postal_code;
pub mod product;
pub mod restock;
pub mod return_request;
pub mod review;
pub mod shop_settings;
pub mod tax;
pub mod user;
pub mod user_identity;
pub mod variant;

pub use admin::{
    ADMIN_AREAS, AREA_CONTENT, AREA_MAIL, AREA_PRODUCTS, AREA_SALES, AREA_SETTINGS, AREA_STORE,
    Admin, AdminIdentity, AdminSummary,
};
pub use admin_rows::{
    AdminCouponRow, AdminCustomer, AdminCustomerDetail, AdminNotice, AdminOrderListRow,
    AdminOrderSummary, AdminProduct, AdminProductListRow, AdminReviewRow, ProductCategory,
    SalesDay, Tag,
};
pub use agent::{Brief, Preset, Weights};
pub use audit_log::AuditLogRow;
pub use cart::CartLine;
pub use code_challenge::CodeChallenge;
pub use content_page::{
    ContentPage, ContentPageSummary, ContentRevision, ContentRevisionSummary, NewContentPage,
};
pub use coupon::{
    AppliedCoupon, BandCoupon, Coupon, CouponDefinition, CouponDefinitionRow,
    CouponDefinitionWrite, MemberCoupon,
};
pub use favorite::Favorite;
pub use home_content::{
    ANNOUNCEMENT_KEY, HERO_KEY, HeroImage, HomePin, HomeSection, HomeSlot, PIN_SECTION_KEYS,
    PinRow, ProductOption, SECTION_KEYS, canonical_key, default_description, default_heading,
    default_sort_order, section_label,
};
pub use inquiry::{
    CATEGORIES, Category, Inquiry, InquiryReply, RETURN_REASONS, STATUSES, Status, category_for,
    category_label, return_reason_for, status_known, status_label,
};
pub use member_address::{AddressInput, LABEL_MAX_CHARS, MemberAddress};
pub use menu::{FOOTER_KEY, HEADER_KEY, MENU_KEYS, MenuItem, menu_label};
pub use notice::{Notice, NoticeCandidate};
pub use notification_template::NotificationTemplate;
pub use order::{Order, OrderLine, RecentOrder};
pub use payment::{Payment, PaymentMethod, WebhookEvent};
pub use pending_registration::PendingRegistration;
pub use postal_code::PostalAddress;
pub use product::Product;
pub use shop_settings::{FeeSettings, ShopSettings};
pub use user::User;
pub use user_identity::UserIdentity;
