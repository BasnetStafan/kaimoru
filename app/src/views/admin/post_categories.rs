//! Blog category views (CMS blog spec,
//! docs/specs/CMS_SPEC.md).
//!
//! The list and the editor for the blog's small vocabulary.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::services::admin::post_category_service::PostCategoryInput;

/// One row of the categories list.
pub struct PostCategoryRow {
    /// Editor URL.
    pub edit_url: String,
    /// Category name.
    pub name: String,
    /// Category slug (the archive URL's last segment).
    pub slug: String,
    /// Delete form URL.
    pub delete_url: String,
}

/// The categories list.
#[derive(Template)]
#[template(path = "admin/pages/post_categories/list.html")]
pub struct PostCategoryListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Every category, registration order.
    pub rows: Vec<PostCategoryRow>,
}

/// The category editor, for both creation and editing.
#[derive(Template)]
#[template(path = "admin/pages/post_categories/form.html")]
pub struct PostCategoryFormView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Page heading （カテゴリの登録 / カテゴリの編集）.
    pub heading: String,
    /// Where the form posts.
    pub action: String,
    /// The form values.
    pub values: PostCategoryInput,
    /// Delete form URL for an existing category.
    pub delete_url: Option<String>,
    /// The one-time form ticket minted for this render of
    /// the edit form; empty on the create form.
    pub form_ticket: String,
    /// Name field error.
    pub name_error: Option<String>,
    /// Slug field error.
    pub slug_error: Option<String>,
}

impl PostCategoryFormView {
    /// Builds the editor around one set of values, with no
    /// field errors.
    pub fn new(
        layout: AdminLayout,
        heading: String,
        action: String,
        delete_url: Option<String>,
        values: PostCategoryInput,
    ) -> Self {
        Self {
            layout,
            nav: "post_categories",
            heading,
            action,
            values,
            delete_url,
            form_ticket: String::new(),
            name_error: None,
            slug_error: None,
        }
    }
}
