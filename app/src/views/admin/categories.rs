//! Category list and form views.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::repositories::category_repository::CategoryListRow;
use crate::views::admin::Page;

/// One status filter option of the category list.
pub struct CategoryStatusOption {
    /// Filter value (empty = all).
    pub value: &'static str,
    /// Japanese label.
    pub label: &'static str,
    /// Whether it is the active filter.
    pub selected: bool,
}

/// One sort option of the category list.
pub struct CategorySortOption {
    /// Sort key value.
    pub value: &'static str,
    /// Japanese label.
    pub label: &'static str,
    /// Whether it is the active sort.
    pub selected: bool,
}

/// One parent choice of the category form.
pub struct ParentOption {
    /// Category ID (the submitted value).
    pub id: i64,
    /// Category name.
    pub name: String,
    /// Whether it is the current parent.
    pub selected: bool,
}

/// One category row of the admin list, display-ready: the
/// state pill and the timestamps are derived here, once, so
/// the template only prints.
pub struct CategoryRow {
    /// Primary key.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Stable ASCII identifier.
    pub slug: String,
    /// The parent category's name; `None` at the top level.
    pub parent_name: Option<String>,
    /// Ordering weight.
    pub display_order: i32,
    /// Whether the category is live.
    pub is_active: bool,
    /// The 状態 pill's label.
    pub state_label: &'static str,
    /// The pill's CSS modifier class.
    pub state_class: &'static str,
    /// Products linked to the category.
    pub product_count: i64,
    /// Child categories.
    pub child_count: i64,
    /// Last edit, `YYYY/MM/DD HH:mm` JST.
    pub updated_text: String,
}

/// Builds the rows for one list page.
pub fn rows_for(rows: &[CategoryListRow]) -> Vec<CategoryRow> {
    rows.iter()
        .map(|r| CategoryRow {
            id: r.category.id,
            name: r.category.name.clone(),
            slug: r.category.slug.clone(),
            parent_name: r.parent_name.clone(),
            display_order: r.category.display_order,
            is_active: r.category.is_active,
            state_label: if r.category.is_active {
                "有効"
            } else {
                "無効"
            },
            state_class: if r.category.is_active {
                "status-public"
            } else {
                "status-archived"
            },
            product_count: r.product_count,
            child_count: r.child_count,
            updated_text: crate::models::jst::jst_datetime_label(r.category.updated_at),
        })
        .collect()
}

/// The category list page: searched, status-filtered, sorted
/// and paginated at 20 like every other admin list.
#[derive(Template)]
#[template(path = "admin/pages/categories/list.html")]
pub struct CategoryListView {
    /// List URL carrying the current search/filter/page:
    /// row delete forms post it back as `return_url`, so a
    /// delete returns to the exact list the operator was in.
    pub return_url: String,
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The rows of this page.
    pub rows: Vec<CategoryRow>,
    /// Pagination state.
    pub page: Page,
    /// The search text, echoed back into the form.
    pub q: String,
    /// Status filter options.
    pub status_options: Vec<CategoryStatusOption>,
    /// Sort options.
    pub sort_options: Vec<CategorySortOption>,
    /// Whether any filter is active — selects the empty-state
    /// wording (no categories at all vs none matching).
    pub filtered: bool,
}

/// The shared create/edit category form page.
#[derive(Template)]
#[template(path = "admin/pages/categories/form.html")]
pub struct CategoryFormView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Page heading （カテゴリーを登録 / カテゴリーを編集）.
    pub heading: String,
    /// Form POST target.
    pub action: String,
    /// Delete form target on the edit form; `None` on create.
    pub delete_action: Option<String>,
    /// Name value.
    pub name: String,
    /// Slug value.
    pub slug: String,
    /// Description value.
    pub description: String,
    /// Parent choices (never the category itself or a descendant).
    pub parent_options: Vec<ParentOption>,
    /// Display order value, as typed.
    pub display_order: String,
    /// The 有効 checkbox state.
    pub is_active: bool,
    /// Name field error.
    pub name_error: Option<String>,
    /// Slug field error.
    pub slug_error: Option<String>,
    /// Description field error.
    pub description_error: Option<String>,
    /// Parent field error.
    pub parent_error: Option<String>,
    /// Display order field error.
    pub display_order_error: Option<String>,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`); empty on
    /// the create form, which has no record to guard.
    pub form_ticket: String,
}

/// The CSV import upload page for カテゴリー.
#[derive(Template)]
#[template(path = "admin/pages/categories/csv_import.html")]
pub struct CategoryCsvImportView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
}

/// The CSV import preview page for カテゴリー: one batch's
/// validation report with its apply/discard actions.
#[derive(Template)]
#[template(path = "admin/pages/categories/csv_preview.html")]
pub struct CategoryCsvPreviewView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The batch's ID (the apply/discard URLs carry it).
    pub batch_id: i64,
    /// The uploaded file's name.
    pub file_name: String,
    /// Batch lifecycle: `preview`, `applied` or `discarded`.
    pub status: String,
    /// Records the apply step would create.
    pub new_count: usize,
    /// Records the apply step would update.
    pub update_count: usize,
    /// How many problems the validation found.
    pub error_count: usize,
    /// The problems, in file order.
    pub errors: Vec<crate::util::csv_transfer::LineError>,
}
