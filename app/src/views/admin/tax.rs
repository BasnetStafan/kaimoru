//! View structs for the tax administration pages.

use crate::controllers::admin::AdminLayout;
use crate::services::admin::tax_service::ClassRates;

/// The tax settings page: every class with its rate history,
/// and the add-rate form.
#[derive(askama::Template)]
#[template(path = "admin/pages/tax/index.html")]
pub struct TaxSettingsView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key: highlights 消費税設定 in the
    /// sidebar while this page is shown.
    pub nav: &'static str,
    /// Classes with their rate histories.
    pub classes: Vec<ClassRates>,
    /// The add-rate form's class select value, as typed.
    pub form_class_id: String,
    /// The add-rate form's rate value, as typed.
    pub form_rate: String,
    /// The add-rate form's date value, as typed.
    pub form_from: String,
    /// Class field error.
    pub class_error: Option<String>,
    /// Rate field error.
    pub rate_error: Option<String>,
    /// Date field error.
    pub from_error: Option<String>,
}

impl TaxSettingsView {
    /// Whether a class option is the form's current selection.
    pub fn class_selected(&self, id: &i64) -> bool {
        self.form_class_id == id.to_string()
    }
}
