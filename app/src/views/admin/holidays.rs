//! Shop holidays view.

use askama::Template;

use crate::controllers::admin::AdminLayout;

/// One holiday row of the admin list.
pub struct HolidayRowView {
    /// The closed day, ISO form (the delete form's value).
    pub date_value: String,
    /// The closed day, picker-label form.
    pub date_label: String,
    /// The operator's label; empty shows a dash.
    pub label: String,
    /// Whether the day is already past (listed for the
    /// record; it no longer affects any promise).
    pub past: bool,
}

/// The shop holidays page.
#[derive(Template)]
#[template(path = "admin/pages/holidays.html")]
pub struct HolidaysView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Registered holidays, in date order.
    pub holidays: Vec<HolidayRowView>,
    /// The add form's date value, echoed on error.
    pub date_value: String,
    /// The add form's label value, echoed on error.
    pub label_value: String,
    /// Date field error.
    pub date_error: Option<String>,
    /// Label field error.
    pub label_error: Option<String>,
}
