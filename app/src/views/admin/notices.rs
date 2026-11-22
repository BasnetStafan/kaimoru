//! Notice list and form views.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::models::AdminNotice;
use crate::services::admin::notice_service::{NoticeState, display_state};
use crate::views::admin::Page;

/// One status filter option of the notices list.
pub struct NoticeStatusOption {
    /// Filter value (empty = all).
    pub value: &'static str,
    /// Japanese label.
    pub label: &'static str,
    /// Whether it is the active filter.
    pub selected: bool,
}

/// One notice row of the admin list, display-ready: the pill
/// and the period text are derived here, once, so the template
/// only prints.
pub struct NoticeRow {
    /// Primary key.
    pub id: i64,
    /// Headline; empty only on rows predating the title column.
    pub title: String,
    /// Announcement text (the template truncates it visually).
    pub body: String,
    /// Period as `開始日 〜 終了日`, with すぐに / 無期限 for the
    /// open ends.
    pub period_text: String,
    /// The derived 状態 pill's label.
    pub state_label: &'static str,
    /// The pill's CSS modifier class.
    pub state_class: &'static str,
    /// How many notices share this row's title and body,
    /// itself included; 0 when the content is unique. Filled
    /// by `rows_for`, which owns the group data.
    pub duplicate_count: i64,
    /// Last edit, `YYYY/MM/DD HH:mm` JST.
    pub updated_text: String,
}

impl NoticeRow {
    /// Builds the display row for one notice on `today` (JST).
    pub fn of(notice: &AdminNotice, today: chrono::NaiveDate) -> NoticeRow {
        let state = display_state(notice, today);
        let date = |d: Option<chrono::NaiveDate>, open: &str| match d {
            Some(d) => d.format("%Y/%m/%d").to_string(),
            None => open.to_string(),
        };
        NoticeRow {
            id: notice.id,
            title: notice.title.clone(),
            body: notice.body.clone(),
            period_text: format!(
                "{} 〜 {}",
                date(notice.display_from, "すぐに"),
                date(notice.display_to, "無期限")
            ),
            duplicate_count: 0,
            state_label: state.label(),
            state_class: match state {
                NoticeState::Visible => "notice-visible",
                NoticeState::Hidden => "notice-hidden",
                NoticeState::OutOfPeriod => "notice-scheduled",
            },
            updated_text: crate::models::jst::jst_datetime_label(notice.updated_at),
        }
    }
}

/// Builds the rows for one list page. `duplicates` is the
/// repository's content-group list (title, body, group size).
pub fn rows_for(notices: &[AdminNotice], duplicates: &[(String, String, i64)]) -> Vec<NoticeRow> {
    let today = crate::models::jst::jst_today();
    notices
        .iter()
        .map(|n| {
            let mut row = NoticeRow::of(n, today);
            row.duplicate_count = duplicates
                .iter()
                .find(|(title, body, _)| title == &n.title && body == &n.body)
                .map(|(_, _, count)| *count)
                .unwrap_or(0);
            row
        })
        .collect()
}

/// The notices list page: searched, status-filtered and
/// paginated at 20 like every other admin list.
#[derive(Template)]
#[template(path = "admin/pages/notices/list.html")]
pub struct NoticeListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The rows of this page.
    pub rows: Vec<NoticeRow>,
    /// What the storefront's news section shows right now —
    /// the same rows `notice_service::latest` returns for the
    /// top page (dedupe included), as a checkable preview.
    pub preview: Vec<crate::models::notice::Notice>,
    /// Pagination state.
    pub page: Page,
    /// The title search text, echoed back into the form.
    pub q: String,
    /// Status filter options.
    pub status_options: Vec<NoticeStatusOption>,
    /// Whether any filter is active — selects the empty-state
    /// wording (no notices at all vs none matching).
    pub filtered: bool,
}

/// The shared create/edit notice form page.
#[derive(Template)]
#[template(path = "admin/pages/notices/form.html")]
pub struct NoticeFormView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Page heading （お知らせを登録 / お知らせを編集）.
    pub heading: String,
    /// Form POST target.
    pub action: String,
    /// Delete form target on the edit form; `None` on create (the
    /// destructive action sits on the editor, away from the save
    /// button).
    pub delete_action: Option<String>,
    /// Title value.
    pub title: String,
    /// Body value.
    pub body: String,
    /// Period start value (`YYYY-MM-DD` or empty).
    pub display_from: String,
    /// Period end value (`YYYY-MM-DD` or empty).
    pub display_to: String,
    /// The 表示する checkbox state.
    pub is_visible: bool,
    /// Title field error.
    pub title_error: Option<String>,
    /// Body field error.
    pub body_error: Option<String>,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`); empty on
    /// the create form, which has no record to guard.
    pub form_ticket: String,
    /// Period start field error.
    pub display_from_error: Option<String>,
    /// Period end field error.
    pub display_to_error: Option<String>,
}
