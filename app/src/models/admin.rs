//! Admin-side data types.
//!
//! [`Admin`] is the back-office identity row, deliberately a
//! separate type from the member [`User`](super::User): the two
//! share nothing — tables, sessions, password hashes — so neither
//! side's code can accept the other side's credential.

use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// One row of the `admins` table, WITH the password hash.
///
/// Authentication use only: the login flow loads this type;
/// everything downstream carries [`AdminIdentity`], so the hash
/// cannot wander into a page, a layout or a log line. The
/// struct deliberately has no `Debug` derive (the same rule
/// as `User`): the hash must not be printable.
#[derive(Clone, FromRow)]
pub struct Admin {
    /// Primary key.
    pub id: i64,
    /// Login email (stored trimmed and lowercased).
    pub email: String,
    /// Argon2id password hash. Authentication code only: never
    /// logged, never rendered.
    pub password_hash: String,
    /// Name shown in the admin sidebar.
    pub display_name: String,
    /// Role: `owner` (every area, manages admin accounts) or
    /// `staff` (only the areas in `permissions`).
    pub role: String,
    /// Granted permission-area keys (the `AREA_*` constants);
    /// ignored for owners, who hold every area.
    pub permissions: Vec<String>,
    /// Whether the account may sign in. A disabled account is
    /// refused at login and resolved as signed out on every
    /// request, so disabling takes effect immediately.
    pub is_active: bool,
    /// When the account was created.
    pub created_at: DateTime<Utc>,
    /// When the admin last signed in; `None` until the first login.
    pub last_login_at: Option<DateTime<Utc>>,
}

/// One row of the `admins` table WITHOUT the password hash: the
/// identity the session layer resolves on every admin request.
#[derive(Debug, Clone, FromRow)]
pub struct AdminIdentity {
    /// Primary key.
    pub id: i64,
    /// Login email (stored trimmed and lowercased).
    pub email: String,
    /// Name shown in the admin sidebar.
    pub display_name: String,
    /// Role: `owner` or `staff` (see [`Admin::role`]).
    pub role: String,
    /// Granted permission-area keys; ignored for owners.
    pub permissions: Vec<String>,
    /// Whether the account may sign in (see [`Admin::is_active`]).
    pub is_active: bool,
}

impl AdminIdentity {
    /// Whether this admin is an owner: owners hold every area
    /// and are the only admins who may manage admin accounts.
    pub fn is_owner(&self) -> bool {
        self.role == "owner"
    }

    /// Whether this admin may use `area` (one of the `AREA_*`
    /// keys): owners always may; staff only when the area is
    /// among their granted permissions.
    pub fn allows(&self, area: &str) -> bool {
        self.is_owner() || self.permissions.iter().any(|p| p == area)
    }
}

/// Permission area: the sales group of the admin surface
/// (orders, inquiries, reviews).
pub const AREA_SALES: &str = "sales";
/// Permission area: the products group (products, categories,
/// tags, notices).
pub const AREA_PRODUCTS: &str = "products";
/// Permission area: the content group (content pages, home
/// content, menus, coupons).
pub const AREA_CONTENT: &str = "content";
/// Permission area: the mail group (delivery, templates,
/// campaigns).
pub const AREA_MAIL: &str = "mail";
/// Permission area: the store group (customers, audit log).
pub const AREA_STORE: &str = "store";
/// Permission area: the shop and tax settings pages.
pub const AREA_SETTINGS: &str = "settings";

/// Every permission area with its Japanese label, in sidebar
/// order. The account form's checkbox list, the list page's
/// permission summary and the guard's path table all read
/// these keys from this one definition.
pub const ADMIN_AREAS: [(&str, &str); 6] = [
    (AREA_SALES, "販売"),
    (AREA_PRODUCTS, "商品"),
    (AREA_CONTENT, "コンテンツ"),
    (AREA_MAIL, "メール配信"),
    (AREA_STORE, "店舗"),
    (AREA_SETTINGS, "設定"),
];

/// One admin account as the management screens show it: the
/// identity columns plus role, permissions, status and the
/// timestamps the list prints. Never the password hash.
#[derive(Debug, Clone, FromRow)]
pub struct AdminSummary {
    /// Primary key.
    pub id: i64,
    /// Login email (stored trimmed and lowercased).
    pub email: String,
    /// Name shown in the admin sidebar.
    pub display_name: String,
    /// Role: `owner` or `staff`.
    pub role: String,
    /// Granted permission-area keys; ignored for owners.
    pub permissions: Vec<String>,
    /// Whether the account may sign in.
    pub is_active: bool,
    /// When the account was created.
    pub created_at: DateTime<Utc>,
    /// When the admin last signed in; `None` until the first login.
    pub last_login_at: Option<DateTime<Utc>>,
}

impl AdminSummary {
    /// Whether this account is an owner.
    pub fn is_owner(&self) -> bool {
        self.role == "owner"
    }

    /// The Japanese label for the role.
    pub fn role_label(&self) -> &'static str {
        if self.is_owner() {
            "オーナー"
        } else {
            "スタッフ"
        }
    }

    /// The permission summary for the list: owners hold every
    /// area; staff show their granted area labels, or 権限なし
    /// when they hold none.
    pub fn permissions_label(&self) -> String {
        if self.is_owner() {
            return "すべての権限".to_string();
        }
        let labels: Vec<&str> = ADMIN_AREAS
            .iter()
            .filter(|(key, _)| self.permissions.iter().any(|p| p == key))
            .map(|(_, label)| *label)
            .collect();
        if labels.is_empty() {
            "権限なし".to_string()
        } else {
            labels.join("・")
        }
    }

    /// The Japanese label for the active flag.
    pub fn status_label(&self) -> &'static str {
        if self.is_active { "有効" } else { "無効" }
    }

    /// The creation timestamp as a JST date-and-time label.
    pub fn created_display(&self) -> String {
        super::jst::jst_datetime_label(self.created_at)
    }

    /// The last-login timestamp as a JST date-and-time label,
    /// or `—` when the admin has never signed in.
    pub fn last_login_display(&self) -> String {
        match self.last_login_at {
            Some(at) => super::jst::jst_datetime_label(at),
            None => "—".to_string(),
        }
    }
}
