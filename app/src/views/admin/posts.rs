//! Blog post views (CMS blog spec, docs/specs/CMS_SPEC.md).
//!
//! The list, the editor and the revision history for blog
//! posts — the content pages' view shapes, carried to
//! dated articles.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::services::admin::post_service::PostInput;
use crate::views::admin::Page;

/// One category choice of the editor's select.
pub struct CategoryOption {
    /// Category ID (the submitted value).
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Whether this is the post's current category.
    pub selected: bool,
}

/// One row of the posts list.
pub struct PostRow {
    /// Editor URL.
    pub edit_url: String,
    /// Post title.
    pub title: String,
    /// Storefront URL the post answers at.
    pub url_path: String,
    /// Category display name; `-` in the template when none.
    pub category_name: Option<String>,
    /// Derived state label （下書き / 予約中 / 公開中）.
    pub state_label: &'static str,
    /// Whether the post is live (styles the state label).
    pub live: bool,
    /// Whether the post is scheduled (styles the label).
    pub scheduled: bool,
    /// Publication stamp (`YYYY/MM/DD HH:MM` or `-`).
    pub published_label: String,
    /// Last-update date (`YYYY/MM/DD`).
    pub updated_label: String,
    /// The author's display name.
    pub author_name: String,
    /// Name of the admin who last saved the post, when known.
    pub editor_name: Option<String>,
}

/// The posts list.
#[derive(Template)]
#[template(path = "admin/pages/posts/list.html")]
pub struct PostListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The active state filter (`""` = all).
    pub state: String,
    /// The rows of this page.
    pub rows: Vec<PostRow>,
    /// Pagination state.
    pub page: Page,
}

/// The post editor, for both creation and editing.
#[derive(Template)]
#[template(path = "admin/pages/posts/form.html")]
pub struct PostFormView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Page heading （記事の登録 / 記事の編集）.
    pub heading: String,
    /// Where the form posts.
    pub action: String,
    /// The form values: the stored post, the submitted
    /// values after a refused save, or blanks for a new post.
    pub values: PostInput,
    /// Whether the slug may be typed (creation only — after
    /// that it is locked, as on content pages).
    pub slug_editable: bool,
    /// Category choices for the select.
    pub category_options: Vec<CategoryOption>,
    /// Revisions URL for an existing post.
    pub revisions_url: Option<String>,
    /// Delete form URL for an existing post.
    pub delete_url: Option<String>,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`); empty on
    /// the create form.
    pub form_ticket: String,
    /// Slug field error.
    pub slug_error: Option<String>,
    /// Title field error.
    pub title_error: Option<String>,
    /// Body field error.
    pub body_error: Option<String>,
    /// Format field error.
    pub format_error: Option<String>,
    /// Excerpt field error.
    pub excerpt_error: Option<String>,
    /// Meta description field error.
    pub meta_error: Option<String>,
    /// Category field error.
    pub category_error: Option<String>,
    /// Status field error.
    pub status_error: Option<String>,
    /// Published-at field error.
    pub published_error: Option<String>,
    /// Cover image error (a refused upload).
    pub cover_error: Option<String>,
}

/// The editor's fixed chrome: where the form posts, and
/// which affordances the form carries. Grouped so the
/// view's constructor stays small (the content pages'
/// pattern).
pub struct PostFormChrome {
    /// Page heading.
    pub heading: String,
    /// Where the form posts.
    pub action: String,
    /// Whether the slug may be typed (creation only).
    pub slug_editable: bool,
    /// Category choices for the select.
    pub category_options: Vec<CategoryOption>,
    /// Revisions URL for an existing post.
    pub revisions_url: Option<String>,
    /// Delete form URL for an existing post.
    pub delete_url: Option<String>,
}

impl PostFormView {
    /// Builds the editor around one set of values, with no
    /// field errors.
    pub fn new(layout: AdminLayout, chrome: PostFormChrome, values: PostInput) -> Self {
        Self {
            layout,
            nav: "posts",
            heading: chrome.heading,
            action: chrome.action,
            values,
            slug_editable: chrome.slug_editable,
            category_options: chrome.category_options,
            revisions_url: chrome.revisions_url,
            delete_url: chrome.delete_url,
            form_ticket: String::new(),
            slug_error: None,
            title_error: None,
            body_error: None,
            format_error: None,
            excerpt_error: None,
            meta_error: None,
            category_error: None,
            status_error: None,
            published_error: None,
            cover_error: None,
        }
    }
}

/// One row of the revision history.
pub struct PostRevisionRow {
    /// When this state was replaced (`YYYY/MM/DD HH:MM`).
    pub created_label: String,
    /// Title of the snapshotted state.
    pub title: String,
    /// Name of the admin whose save recorded the snapshot.
    pub editor_name: String,
    /// Restore form URL.
    pub restore_url: String,
}

/// A post's revision history.
#[derive(Template)]
#[template(path = "admin/pages/posts/revisions.html")]
pub struct PostRevisionListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The post whose history this is.
    pub post_title: String,
    /// Editor URL, for the back link.
    pub edit_url: String,
    /// Snapshots, newest first.
    pub rows: Vec<PostRevisionRow>,
}
