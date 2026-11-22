//! Customer list and detail views.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::models::{AdminCustomer, AdminCustomerDetail, Order};
use crate::views::admin::Page;

/// The customers list page.
#[derive(Template)]
#[template(path = "admin/pages/customers/list.html")]
pub struct CustomerListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The search text, as typed (empty when unfiltered).
    pub q: String,
    /// The rows of this page.
    pub customers: Vec<AdminCustomer>,
    /// Pagination state.
    pub page: Page,
}

/// The customer detail page: profile fields (never the
/// password hash — the model has no such field), purchase
/// totals, and the member's order history.
#[derive(Template)]
#[template(path = "admin/pages/customers/detail.html")]
pub struct CustomerDetailView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The member's profile.
    pub customer: AdminCustomerDetail,
    /// Order count excluding cancelled orders (identical to the
    /// list's figure).
    pub order_count: i64,
    /// Purchase total excluding cancelled orders, formatted.
    pub purchase_total: String,
    /// The member's orders, newest first.
    pub orders: Vec<Order>,
    /// When the member withdrew, when they have (the
    /// detail's 退会済み marker, Account Deletion D7).
    pub withdrawn_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Operator memo; internal only, never shown to
    /// the member.
    pub memo: String,
    /// Operator tags on this member.
    pub tags: Vec<String>,
}

impl CustomerDetailView {
    /// Full name in kanji.
    pub fn full_name(&self) -> String {
        format!("{} {}", self.customer.family_name, self.customer.given_name)
    }

    /// Full name in katakana.
    pub fn full_name_kana(&self) -> String {
        format!(
            "{} {}",
            self.customer.family_name_kana, self.customer.given_name_kana
        )
    }

    /// The postal/address block joined for display.
    pub fn address(&self) -> String {
        let mut text = format!(
            "〒{} {} {} {}",
            self.customer.postal_code,
            self.customer.prefecture,
            self.customer.city,
            self.customer.address_line
        );
        if let Some(building) = &self.customer.building {
            text.push(' ');
            text.push_str(building);
        }
        text
    }

    /// Birth date formatted, or `-` when not given.
    pub fn birth_date_text(&self) -> String {
        self.customer
            .birth_date
            .map(|d| d.format("%Y/%m/%d").to_string())
            .unwrap_or_else(|| "-".to_string())
    }

    /// Gender text, or `-` when not given.
    pub fn gender_text(&self) -> String {
        self.customer
            .gender
            .clone()
            .unwrap_or_else(|| "-".to_string())
    }

    /// The withdrawal marker line, when withdrawn:
    /// 退会済み（YYYY年M月D日） in JST.
    pub fn withdrawn_text(&self) -> Option<String> {
        self.withdrawn_at.map(|at| {
            let jst = at.with_timezone(&crate::models::jst::jst());
            format!("退会済み（{}）", jst.format("%Y年%m月%d日"))
        })
    }

    /// Registration date formatted JST.
    pub fn registered_text(&self) -> String {
        crate::models::jst::jst_datetime_label(self.customer.created_at)
    }
}
