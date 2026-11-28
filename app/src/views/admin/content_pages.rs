//! Content page views (CMS spec C-2).
//!
//! The list, the editor and the revision history for the
//! shop's long-form pages. Bodies are Markdown or HTML source
//! and are shown in a plain textarea.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::services::admin::content_page_service::ContentPageInput;

/// One row of the pages list.
pub struct ContentPageRow {
    /// Editor URL.
    pub edit_url: String,
    /// Page title.
    pub title: String,
    /// Storefront URL the page answers at.
    pub url_path: String,
    /// 種別 badge （固定ページ / 自由ページ /
    /// ランディングページ）.
    pub type_label: &'static str,
    /// Status label （公開 / 下書き）.
    pub status_label: &'static str,
    /// Whether the page is live (styles the status label).
    pub published: bool,
    /// Last-update date (`YYYY/MM/DD`).
    pub updated_label: String,
    /// Name of the admin who last saved the page, when known.
    pub editor_name: Option<String>,
    /// System pages cannot be deleted; the list says so
    /// instead of offering the action.
    pub is_system: bool,
}

/// The content pages list.
#[derive(Template)]
#[template(path = "admin/pages/content/list.html")]
pub struct ContentPageListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The hub tab this list is showing (`landing`,
    /// `standard`, or empty for the unfiltered legacy
    /// list): the page-type tabs mark it active.
    pub active_tab: &'static str,
    /// The pages, system first.
    pub rows: Vec<ContentPageRow>,
}

/// The pages hub (CMS_SPEC §9): the one admin home for the
/// three page types — blog posts, landing pages and
/// standard pages — with each type's count and its
/// type-first create entry.
#[derive(Template)]
#[template(path = "admin/pages/content/hub.html")]
pub struct PagesHubView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Number of blog posts, every state.
    pub blog_count: i64,
    /// Number of landing pages.
    pub landing_count: i64,
    /// Number of standard pages (system + custom).
    pub standard_count: i64,
}

/// The page editor, for both creation and editing.
#[derive(Template)]
#[template(path = "admin/pages/content/form.html")]
pub struct ContentPageFormView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Page heading （固定ページの編集 / ページの登録， …).
    pub heading: String,
    /// Where the form posts.
    pub action: String,
    /// The form values: the stored page, the submitted values
    /// after a refused save, or blanks for a new page.
    pub values: ContentPageInput,
    /// The page's type: `standard` or `landing`
    /// (CMS_SPEC §9). Carried by a hidden field so the
    /// create submit and the preview know the type; the
    /// editor offers no control that changes it — a page's
    /// type is fixed at creation.
    pub page_layout: String,
    /// Whether the slug may be typed (creation only — after
    /// that it is locked, spec D4; system pages never show a
    /// slug field at all, TC-04).
    pub slug_editable: bool,
    /// Whether this page is a seeded system page.
    pub is_system: bool,
    /// Revisions URL for an existing page.
    pub revisions_url: Option<String>,
    /// Delete form URL: existing draft custom pages only.
    pub delete_url: Option<String>,
    /// Page-type field error (creation only).
    pub layout_error: Option<String>,
    /// Slug field error.
    pub slug_error: Option<String>,
    /// Title field error.
    pub title_error: Option<String>,
    /// Body field error.
    pub body_error: Option<String>,
    /// Format field error.
    pub format_error: Option<String>,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`); empty on
    /// the create form, which has no record to guard.
    pub form_ticket: String,
    /// Meta description field error.
    pub meta_error: Option<String>,
    /// Status field error.
    pub status_error: Option<String>,
}

/// The editor's fixed chrome: where the form posts, and which
/// affordances the page's kind and status allow. Grouped so
/// the view's constructor stays small.
pub struct ContentFormChrome {
    /// The page's type (creation preset, or the stored
    /// page's own type when editing).
    pub page_layout: String,
    /// Page heading （固定ページの編集 / ページの登録， …).
    pub heading: String,
    /// Where the form posts.
    pub action: String,
    /// Whether the slug may be typed (creation only).
    pub slug_editable: bool,
    /// Whether this page is a seeded system page.
    pub is_system: bool,
    /// Revisions URL for an existing page.
    pub revisions_url: Option<String>,
    /// Delete form URL: existing draft custom pages only.
    pub delete_url: Option<String>,
}

impl ContentPageFormView {
    /// Builds the editor around one set of values, with no
    /// field errors.
    pub fn new(layout: AdminLayout, chrome: ContentFormChrome, values: ContentPageInput) -> Self {
        Self {
            layout,
            nav: "content",
            heading: chrome.heading,
            action: chrome.action,
            values,
            page_layout: chrome.page_layout,
            slug_editable: chrome.slug_editable,
            is_system: chrome.is_system,
            revisions_url: chrome.revisions_url,
            delete_url: chrome.delete_url,
            form_ticket: String::new(),
            layout_error: None,
            slug_error: None,
            title_error: None,
            body_error: None,
            format_error: None,
            meta_error: None,
            status_error: None,
        }
    }
}

/// One row of the revision history.
pub struct ContentRevisionRow {
    /// When this state was replaced (`YYYY/MM/DD HH:MM`).
    pub created_label: String,
    /// Title of the snapshotted state.
    pub title: String,
    /// Name of the admin whose save recorded the snapshot.
    pub editor_name: String,
    /// Restore form URL.
    pub restore_url: String,
}

/// A page's revision history.
#[derive(Template)]
#[template(path = "admin/pages/content/revisions.html")]
pub struct ContentRevisionListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The page whose history this is.
    pub page_title: String,
    /// Editor URL, for the back link.
    pub edit_url: String,
    /// Snapshots, newest first.
    pub rows: Vec<ContentRevisionRow>,
}
