//! Tag list and form views.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::repositories::tag_repository::TagListRow;
use crate::views::admin::Page;

/// One sort option of the tag list.
pub struct TagSortOption {
    /// Sort key value.
    pub value: &'static str,
    /// Japanese label.
    pub label: &'static str,
    /// Whether it is the active sort.
    pub selected: bool,
}

/// One tag row of the admin list, display-ready.
pub struct TagRow {
    /// Primary key.
    pub id: i64,
    /// Japanese label （名前）.
    pub label: String,
    /// Stable ASCII identifier.
    pub slug: String,
    /// Operator-facing description.
    pub description: String,
    /// Products carrying the tag.
    pub product_count: i64,
}

/// Builds the rows for one list page.
pub fn rows_for(rows: &[TagListRow]) -> Vec<TagRow> {
    rows.iter()
        .map(|r| TagRow {
            id: r.tag.id,
            label: r.tag.label.clone(),
            slug: r.tag.slug.clone(),
            description: r.tag.description.clone(),
            product_count: r.product_count,
        })
        .collect()
}

/// The tag list page: searched, sorted and paginated at 20
/// like every other admin list.
#[derive(Template)]
#[template(path = "admin/pages/tags/list.html")]
pub struct TagListView {
    /// List URL carrying the current search/sort/page:
    /// row delete forms post it back as `return_url`, so a
    /// delete returns to the exact list the operator was in.
    pub return_url: String,
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The rows of this page.
    pub rows: Vec<TagRow>,
    /// Pagination state.
    pub page: Page,
    /// The search text, echoed back into the form.
    pub q: String,
    /// Sort options.
    pub sort_options: Vec<TagSortOption>,
    /// Whether a search is active — selects the empty-state
    /// wording (no tags at all vs none matching).
    pub filtered: bool,
}

/// The shared create/edit tag form page.
#[derive(Template)]
#[template(path = "admin/pages/tags/form.html")]
pub struct TagFormView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Page heading （タグを登録 / タグを編集）.
    pub heading: String,
    /// Form POST target.
    pub action: String,
    /// Delete form target on the edit form; `None` on create.
    pub delete_action: Option<String>,
    /// Label value.
    pub label: String,
    /// Slug value.
    pub slug: String,
    /// Description value.
    pub description: String,
    /// Label field error.
    pub label_error: Option<String>,
    /// Slug field error.
    pub slug_error: Option<String>,
    /// Description field error.
    pub description_error: Option<String>,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`); empty on
    /// the create form, which has no record to guard.
    pub form_ticket: String,
}

/// The CSV import upload page for タグ.
#[derive(Template)]
#[template(path = "admin/pages/tags/csv_import.html")]
pub struct TagCsvImportView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
}

/// The CSV import preview page for タグ: one batch's
/// validation report with its apply/discard actions.
#[derive(Template)]
#[template(path = "admin/pages/tags/csv_preview.html")]
pub struct TagCsvPreviewView {
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
