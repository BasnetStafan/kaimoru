//! API key screens.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::models::jst::jst_datetime_label;
use crate::repositories::api_key_repository::ApiKeyRow;

/// One key row, display-ready. The secret never appears:
/// only its public prefix is stored in a showable form.
pub struct ApiKeyRowView {
    /// The key's ID (the revoke form's target).
    pub id: i64,
    /// Operator-chosen label.
    pub name: String,
    /// Public lookup prefix.
    pub prefix: String,
    /// Granted scope.
    pub scope: String,
    /// When the key was issued, JST.
    pub created_label: String,
    /// When the key last authenticated, JST; なし while never.
    pub last_used_label: String,
    /// Whether the key is still usable.
    pub active: bool,
    /// Status label: 有効 or 無効化済み.
    pub status_label: String,
}

impl ApiKeyRowView {
    /// Shapes a stored key into its display row.
    pub fn new(row: &ApiKeyRow) -> Self {
        let active = row.revoked_at.is_none();
        Self {
            id: row.id,
            name: row.name.clone(),
            prefix: row.prefix.clone(),
            scope: row.scope.clone(),
            created_label: jst_datetime_label(row.created_at),
            last_used_label: row
                .last_used_at
                .map(jst_datetime_label)
                .unwrap_or_else(|| "なし".to_string()),
            active,
            status_label: if active {
                "有効".to_string()
            } else {
                "無効化済み".to_string()
            },
        }
    }
}

/// The API keys list + issue form page.
#[derive(Template)]
#[template(path = "admin/pages/settings/api_keys.html")]
pub struct ApiKeysView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Existing keys, newest first.
    pub rows: Vec<ApiKeyRowView>,
    /// Form value echo: name.
    pub form_name: String,
    /// Form value echo: scope.
    pub form_scope: String,
    /// Validation error, empty when none.
    pub form_error: String,
    /// The full token of a key created by this very request.
    /// Shown exactly once, on this render only: it is never
    /// stored, so no later page can display it again.
    pub created_token: Option<String>,
}
