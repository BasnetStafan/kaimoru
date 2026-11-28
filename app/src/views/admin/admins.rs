//! Admin account management views.
//!
//! The list and the shared create/edit form for the
//! owner-facing account screens. Passwords are never carried
//! into either struct: the inputs render empty after every
//! submit, the login form's rule.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::models::AdminSummary;

/// The account list page.
#[derive(Template)]
#[template(path = "admin/pages/admins/list.html")]
pub struct AdminListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key: highlights 管理者管理.
    pub nav: &'static str,
    /// Every admin account, oldest first.
    pub rows: Vec<AdminSummary>,
}

/// One permission checkbox on the account form.
pub struct AreaChoice {
    /// The area key (an `AREA_*` constant); the checkbox posts
    /// as `perm_<key>`.
    pub key: &'static str,
    /// The Japanese label beside the checkbox.
    pub label: &'static str,
    /// Whether the account currently holds the area.
    pub checked: bool,
}

/// The account create/edit form. On the edit page it also
/// carries the password-reset form's errors (a second, small
/// form rendered below this one).
#[derive(Template)]
#[template(path = "admin/pages/admins/form.html")]
pub struct AdminFormView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key: highlights 管理者管理.
    pub nav: &'static str,
    /// Page heading: 管理者を登録 or 管理者を編集.
    pub heading: String,
    /// The main form's POST target.
    pub action: String,
    /// Whether this is the create form (email + initial
    /// password fields) rather than the edit form.
    pub is_new: bool,
    /// The email: the typed value on create, the stored
    /// sign-in ID (read-only) on edit.
    pub email: String,
    /// The display name input's value.
    pub display_name: String,
    /// The selected role: `owner` or `staff`.
    pub role: String,
    /// The permission checkboxes, in sidebar order.
    pub areas: Vec<AreaChoice>,
    /// The active flag's current state (edit form).
    pub is_active: bool,
    /// Whether the target is the signed-in admin themselves:
    /// the form renders read-only and points at the account
    /// settings page instead.
    pub is_self: bool,
    /// Form-level refusal (a business rule), when a submit hit one.
    pub form_error: Option<String>,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`); empty on
    /// the create form, which has no record to guard.
    pub form_ticket: String,
    /// Email field error (create).
    pub email_error: Option<String>,
    /// Display name field error.
    pub name_error: Option<String>,
    /// Password field error (create).
    pub password_error: Option<String>,
    /// Password confirmation field error (create).
    pub confirm_error: Option<String>,
    /// Role field error.
    pub role_error: Option<String>,
    /// The password-reset form's POST target (edit page only).
    pub reset_action: Option<String>,
    /// The reset form's form-level refusal, when a reset hit one.
    pub reset_error: Option<String>,
    /// The reset form's new-password field error.
    pub reset_new_error: Option<String>,
    /// The reset form's confirmation field error.
    pub reset_confirm_error: Option<String>,
}
