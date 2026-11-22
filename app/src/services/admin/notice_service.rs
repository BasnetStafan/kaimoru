//! Admin notice rules.
//!
//! A notice carries a title, a body, an optional display period
//! and a visibility flag. It appears on the top page when the flag
//! is on and today (JST) is inside the period — the flag and a
//! future start date are the staging mechanism; there is no draft
//! state beyond them.

use chrono::NaiveDate;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::AdminNotice;
use crate::repositories::notice_repository;
use crate::services::admin::audit_service;
use crate::services::notice_service;
use crate::validation::FieldError;

/// The raw notice form as submitted.
#[derive(Clone, Default)]
pub struct NoticeFormInput {
    /// Headline as typed.
    pub title: String,
    /// Announcement body as typed.
    pub body: String,
    /// Display period start as typed (`YYYY-MM-DD` from the
    /// date input); empty shows the notice immediately.
    pub display_from: String,
    /// Display period end as typed; empty never expires.
    pub display_to: String,
    /// The 表示する checkbox.
    pub is_visible: bool,
}

/// A notice form that passed validation, ready to store.
#[derive(Debug)]
pub struct ValidatedNotice {
    /// Trimmed headline.
    pub title: String,
    /// Trimmed body.
    pub body: String,
    /// Period start; `None` shows immediately.
    pub display_from: Option<NaiveDate>,
    /// Period end; `None` never expires.
    pub display_to: Option<NaiveDate>,
    /// The visibility flag.
    pub is_visible: bool,
}

/// Parses one optional date field: empty is `None` (an open
/// period end), a real calendar date is `Some`, anything else
/// earns the field its error — a hand-crafted value must not
/// silently become an open end.
fn parse_optional_date(
    raw: &str,
    field: &'static str,
    message: &str,
    errors: &mut Vec<FieldError>,
) -> Option<NaiveDate> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    match NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        Ok(d) => Some(d),
        Err(_) => {
            errors.push(FieldError::new(field, message));
            None
        }
    }
}

/// Validates the notice form: headline and body required with
/// their character limits (counted in characters, not bytes),
/// both period ends optional, and the end never earlier than the
/// start. Every error at once, each on its own field.
pub fn validate_notice(input: &NoticeFormInput) -> Result<ValidatedNotice, Vec<FieldError>> {
    let mut errors = Vec::new();
    let title = input.title.trim().to_string();
    if title.is_empty() {
        errors.push(FieldError::new("title", "タイトルを入力してください"));
    } else if title.chars().count() > 100 {
        errors.push(FieldError::new(
            "title",
            "タイトルは100文字以内で入力してください",
        ));
    }
    let body = input.body.trim().to_string();
    if body.is_empty() {
        errors.push(FieldError::new("body", "本文を入力してください"));
    } else if body.chars().count() > 2000 {
        errors.push(FieldError::new(
            "body",
            "本文は2000文字以内で入力してください",
        ));
    }
    let display_from = parse_optional_date(
        &input.display_from,
        "display_from",
        "表示開始日が正しくありません",
        &mut errors,
    );
    let display_to = parse_optional_date(
        &input.display_to,
        "display_to",
        "表示終了日が正しくありません",
        &mut errors,
    );
    // The end must not precede the start when both are given; the
    // error belongs to the end field — the one the operator can
    // move without touching the start.
    if let (Some(from), Some(to)) = (display_from, display_to)
        && to < from
    {
        errors.push(FieldError::new(
            "display_to",
            "終了日は開始日以降の日付を選択してください",
        ));
    }
    if errors.is_empty() {
        Ok(ValidatedNotice {
            title,
            body,
            display_from,
            display_to,
            is_visible: input.is_visible,
        })
    } else {
        Err(errors)
    }
}

/// The derived display state of one notice on a given day — the
/// admin list's 状態 pill. Derived from the flag and the period,
/// never stored: a stored copy would go stale at midnight. The
/// SQL twin of this definition is the list query's status
/// predicate in the notice repository; the two must agree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoticeState {
    /// Flag on and today inside the period: live on the top page.
    Visible,
    /// Flag off: hidden regardless of the period.
    Hidden,
    /// Flag on but today outside the period.
    OutOfPeriod,
}

impl NoticeState {
    /// The pill's Japanese label.
    pub fn label(self) -> &'static str {
        match self {
            NoticeState::Visible => "表示中",
            NoticeState::Hidden => "非表示",
            NoticeState::OutOfPeriod => "期間外",
        }
    }
}

/// Derives one notice's display state for `today` (JST).
/// Period ends are inclusive on both sides: a notice whose
/// start and end are both today is live today.
pub fn display_state(notice: &AdminNotice, today: NaiveDate) -> NoticeState {
    if !notice.is_visible {
        return NoticeState::Hidden;
    }
    let started = notice.display_from.is_none_or(|d| d <= today);
    let not_ended = notice.display_to.is_none_or(|d| today <= d);
    if started && not_ended {
        NoticeState::Visible
    } else {
        NoticeState::OutOfPeriod
    }
}

/// Returns one page of the admin list with its filtered total.
pub async fn list_notices(
    pool: &PgPool,
    query: Option<&str>,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AdminNotice>, i64), AppError> {
    // The repository binds a finished LIKE pattern, built by
    // the one shared escaper so `%`/`_` in the typed text match
    // literally, as on every other admin list.
    let pattern = query.and_then(crate::util::like_pattern::like_pattern);
    notice_repository::list_admin(
        pool,
        pattern.as_deref(),
        status,
        crate::models::jst::jst_today(),
        limit,
        offset,
    )
    .await
}

/// The duplicated-content groups for the admin list's
/// duplicate marker.
pub async fn duplicate_counts(pool: &PgPool) -> Result<Vec<(String, String, i64)>, AppError> {
    notice_repository::duplicate_counts(pool).await
}

/// Returns one notice by ID, for the edit form and the edit
/// submit's existence check.
pub async fn find_notice(pool: &PgPool, id: i64) -> Result<Option<AdminNotice>, AppError> {
    notice_repository::find_admin(pool, id).await
}

/// Creates one notice with its `notice.create` audit row.
/// The second return value is the duplicate warning: the new
/// notice is set to show content another visible notice
/// already shows (the storefront renders it once — the
/// warning exists so the operator notices the twin).
pub async fn create_notice(
    pool: &PgPool,
    admin_id: i64,
    valid: &ValidatedNotice,
) -> Result<(i64, bool), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let id = notice_repository::create_admin(
        &mut tx,
        &valid.title,
        &valid.body,
        valid.display_from,
        valid.display_to,
        valid.is_visible,
    )
    .await?;
    audit_service::record(&mut tx, admin_id, "notice.create", "notice", Some(id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    // The storefront caches the visible notice rows briefly;
    // drop the cache so the change shows at once.
    notice_service::invalidate_candidates_cache();
    let duplicated = if valid.is_visible {
        notice_repository::count_visible_same_content(pool, &valid.title, &valid.body, Some(id))
            .await?
            > 0
    } else {
        false
    };
    Ok((id, duplicated))
}

/// Updates one notice with its `notice.update` audit row.
/// Returns the duplicate warning, as [`create_notice`] does.
pub async fn update_notice(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    valid: &ValidatedNotice,
    expected: i64,
) -> Result<bool, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    // The form-ticket staleness guard (see `form_ticket_service`).
    let current = notice_repository::updated_at_for_update(&mut tx, id).await?;
    crate::services::form_ticket_service::ensure_fresh(current, expected)?;
    notice_repository::update_admin(
        &mut tx,
        id,
        &valid.title,
        &valid.body,
        valid.display_from,
        valid.display_to,
        valid.is_visible,
        admin_id,
    )
    .await?;
    audit_service::record(&mut tx, admin_id, "notice.update", "notice", Some(id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    // The storefront caches the visible notice rows briefly;
    // drop the cache so the change shows at once.
    notice_service::invalidate_candidates_cache();
    let duplicated = if valid.is_visible {
        notice_repository::count_visible_same_content(pool, &valid.title, &valid.body, Some(id))
            .await?
            > 0
    } else {
        false
    };
    Ok(duplicated)
}

/// Deletes one notice, writing the `notice.delete` audit row only
/// when a row was actually removed: deletion is idempotent — a
/// repeated delete of the same ID succeeds silently and leaves no
/// second audit row.
pub async fn delete_notice(pool: &PgPool, admin_id: i64, id: i64) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let removed = notice_repository::delete_admin(&mut tx, id).await?;
    if removed > 0 {
        audit_service::record(&mut tx, admin_id, "notice.delete", "notice", Some(id)).await?;
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    // The storefront caches the visible notice rows briefly;
    // drop the cache so the change shows at once.
    notice_service::invalidate_candidates_cache();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a form input from parts, visible by default.
    fn input(title: &str, body: &str, from: &str, to: &str) -> NoticeFormInput {
        NoticeFormInput {
            title: title.to_string(),
            body: body.to_string(),
            display_from: from.to_string(),
            display_to: to.to_string(),
            is_visible: true,
        }
    }

    /// Builds a stored notice for the state derivation.
    fn notice(is_visible: bool, from: Option<NaiveDate>, to: Option<NaiveDate>) -> AdminNotice {
        AdminNotice {
            id: 1,
            title: "t".to_string(),
            body: "b".to_string(),
            display_from: from,
            display_to: to,
            is_visible,
            updated_at: chrono::Utc::now(),
        }
    }

    /// A day in the middle of the test period.
    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("test date")
    }

    #[test]
    fn empty_title_and_body_error_together() {
        let errors = validate_notice(&input("", "", "", "")).unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|e| e.field == "title"));
        assert!(errors.iter().any(|e| e.field == "body"));
    }

    #[test]
    fn title_limit_counts_characters_not_bytes() {
        let ok_title = "あ".repeat(100);
        assert!(validate_notice(&input(&ok_title, "body", "", "")).is_ok());
        let long_title = "あ".repeat(101);
        let errors = validate_notice(&input(&long_title, "body", "", "")).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "title");
    }

    #[test]
    fn body_limit_is_two_thousand_characters() {
        let ok_body = "あ".repeat(2000);
        assert!(validate_notice(&input("t", &ok_body, "", "")).is_ok());
        let long_body = "あ".repeat(2001);
        let errors = validate_notice(&input("t", &long_body, "", "")).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "body");
    }

    #[test]
    fn end_before_start_errors_on_the_end_field() {
        let errors = validate_notice(&input("t", "b", "2026-12-09", "2026-12-05")).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "display_to");
        assert_eq!(
            errors[0].message,
            "終了日は開始日以降の日付を選択してください"
        );
        // Same-day start and end is a valid one-day period.
        assert!(validate_notice(&input("t", "b", "2026-12-05", "2026-12-05")).is_ok());
    }

    #[test]
    fn garbage_date_is_an_error_not_an_open_end() {
        let errors = validate_notice(&input("t", "b", "next week", "")).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "display_from");
    }

    #[test]
    fn state_derivation_follows_flag_then_period() {
        let today = day(2026, 11, 2);
        // Flag off hides even an in-period notice.
        assert_eq!(
            display_state(&notice(false, None, None), today),
            NoticeState::Hidden
        );
        // Open period, flag on: live.
        assert_eq!(
            display_state(&notice(true, None, None), today),
            NoticeState::Visible
        );
        // Starts tomorrow: not yet.
        assert_eq!(
            display_state(&notice(true, Some(day(2026, 11, 3)), None), today),
            NoticeState::OutOfPeriod
        );
        // Ended yesterday: no longer.
        assert_eq!(
            display_state(&notice(true, None, Some(day(2026, 11, 1))), today),
            NoticeState::OutOfPeriod
        );
        // Both ends exactly today: live (inclusive boundaries).
        assert_eq!(
            display_state(&notice(true, Some(today), Some(today)), today),
            NoticeState::Visible
        );
    }
}
