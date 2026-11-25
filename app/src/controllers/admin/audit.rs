//! The audit log viewer.
//!
//! Read-only by construction: this controller has no POST route,
//! and the repository exposes no update or delete for audit rows.
//! Filters are the target type, the action and an inclusive JST
//! date range — no free-text search over a log that contains no
//! personal data to search. The 管理者 (actor) filter is deferred:
//! with a single admin it could only ever select that admin, so
//! it waits until a second admin exists.

use axum::{
    Router,
    extract::{Query, State},
    http::header,
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::NaiveDate;
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, pagination, render};
use crate::config::CONFIG;
use crate::services::admin::audit_query_service;
use crate::session::CurrentAdmin;
use crate::views::admin::Page;
use crate::views::admin::audit::{
    ACTION_CODES, AuditListView, AuditRow, TargetOption, action_label, target_href, target_label,
};
use crate::views::admin::filter_query;

/// The target-type filter options: every type the audit
/// writers emit, with the same labels the rows render.
const TARGETS: [(&str, &str); 19] = [
    ("", "すべて"),
    ("product", "商品"),
    ("product_image", "商品画像"),
    ("order", "注文"),
    ("coupon", "クーポン"),
    ("review", "レビュー"),
    ("inquiry", "お問い合わせ"),
    ("notice", "お知らせ"),
    ("category", "カテゴリー"),
    ("tag", "タグ"),
    ("mail_template", "メールテンプレート"),
    ("notification_template", "通知メール"),
    ("mail_campaign", "メールキャンペーン"),
    ("content_page", "コンテンツページ"),
    ("home_content", "トップページ設定"),
    ("menu", "メニュー設定"),
    ("tax_class", "消費税区分"),
    ("shop_settings", "店舗設定"),
    ("admin", "管理者"),
];

/// Routes: the audit viewer and its CSV export.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/audit", get(list))
        .route("/admin/audit.csv", get(export_csv))
}

/// List query parameters.
#[derive(Deserialize, Default)]
pub struct ListQuery {
    /// Target type filter.
    target: Option<String>,
    /// Action filter (one of the unified dotted codes).
    action: Option<String>,
    /// Period start, `YYYY-MM-DD`.
    from: Option<String>,
    /// Period end, `YYYY-MM-DD` (inclusive of the whole day).
    to: Option<String>,
    /// Page number.
    page: Option<String>,
}

/// Parses one period input: empty is no bound, a real date is
/// the bound, anything else marks the range invalid.
fn parse_bound(raw: Option<&str>, invalid: &mut bool) -> Option<NaiveDate> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    match NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        Ok(d) => Some(d),
        Err(_) => {
            *invalid = true;
            None
        }
    }
}

/// The audit viewer page, newest first.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<ListQuery>,
) -> Response {
    let target = query
        .target
        .as_deref()
        .filter(|t| TARGETS.iter().any(|(v, _)| v == t))
        .unwrap_or("")
        .to_string();
    let action = query
        .action
        .as_deref()
        .filter(|a| ACTION_CODES.contains(a))
        .unwrap_or("")
        .to_string();
    // Sloppy period input (a non-date, or an end before the start)
    // yields the filtered-empty state rather than an error page:
    // nothing is at stake in a read.
    let mut range_invalid = false;
    let date_from = parse_bound(query.from.as_deref(), &mut range_invalid);
    let date_to = parse_bound(query.to.as_deref(), &mut range_invalid);
    if let (Some(from), Some(to)) = (date_from, date_to)
        && to < from
    {
        range_invalid = true;
    }
    let (page_no, limit, offset) = pagination(query.page.as_deref(), CONFIG.admin_per_page);
    let (rows, total) = if range_invalid {
        (Vec::new(), 0)
    } else {
        let target_param = if target.is_empty() {
            None
        } else {
            Some(target.as_str())
        };
        let action_param = if action.is_empty() {
            None
        } else {
            Some(action.as_str())
        };
        match audit_query_service::list_audit_logs(
            &pool,
            target_param,
            action_param,
            date_from,
            date_to,
            limit,
            offset,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => return db_error("audit", e),
        }
    };
    let jst = crate::models::jst::jst();
    let echoed = |raw: &Option<String>| {
        raw.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let from_value = echoed(&query.from).unwrap_or_default();
    let to_value = echoed(&query.to).unwrap_or_default();
    let base = Page::base_with(
        "/admin/audit",
        &filter_query(&[
            (
                "target",
                if target.is_empty() {
                    None
                } else {
                    Some(target.as_str())
                },
            ),
            (
                "action",
                if action.is_empty() {
                    None
                } else {
                    Some(action.as_str())
                },
            ),
            ("from", echoed(&query.from).as_deref()),
            ("to", echoed(&query.to).as_deref()),
        ]),
    );
    let filtered =
        !target.is_empty() || !action.is_empty() || !from_value.is_empty() || !to_value.is_empty();
    let export_qs = filter_query(&[
        (
            "target",
            if target.is_empty() {
                None
            } else {
                Some(target.as_str())
            },
        ),
        (
            "action",
            if action.is_empty() {
                None
            } else {
                Some(action.as_str())
            },
        ),
        ("from", echoed(&query.from).as_deref()),
        ("to", echoed(&query.to).as_deref()),
    ]);
    let export_url = if export_qs.is_empty() {
        "/admin/audit.csv".to_string()
    } else {
        format!("/admin/audit.csv?{export_qs}")
    };
    render(AuditListView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "audit",
        targets: TARGETS
            .iter()
            .map(|(v, l)| TargetOption {
                value: v,
                label: (*l).to_string(),
                selected: target == *v,
            })
            .collect(),
        actions: std::iter::once(TargetOption {
            value: "",
            label: "すべて".to_string(),
            selected: action.is_empty(),
        })
        .chain(ACTION_CODES.iter().map(|code| TargetOption {
            value: code,
            label: action_label(code).into_owned(),
            selected: action == *code,
        }))
        .collect(),
        from_value,
        to_value,
        logs: rows
            .iter()
            .map(|r| AuditRow {
                at: r
                    .created_at
                    .with_timezone(&jst)
                    .format("%Y/%m/%d %H:%M:%S")
                    .to_string(),
                admin_name: r.admin_name.clone(),
                action_label: action_label(&r.action).into_owned(),
                target_href: target_href(&r.target_type, r.target_id),
                target_text: match r.target_id {
                    Some(id) => format!("{} #{}", target_label(&r.target_type), id),
                    None => target_label(&r.target_type).to_string(),
                },
            })
            .collect(),
        filtered,
        export_url,
        page: Page::new(page_no, total, limit, base),
    })
}

/// Downloads the audit trail as CSV with the viewer's own
/// filters (target, action, JST date range): the same rows
/// the viewer would page through, in one file for the shop's
/// records. An invalid range exports nothing but the header,
/// exactly like the viewer's filtered-empty state.
pub async fn export_csv(
    State(pool): State<PgPool>,
    CurrentAdmin(_admin): CurrentAdmin,
    Query(query): Query<ListQuery>,
) -> Response {
    let target = query
        .target
        .as_deref()
        .filter(|t| TARGETS.iter().any(|(v, _)| v == t))
        .unwrap_or("");
    let action = query
        .action
        .as_deref()
        .filter(|a| ACTION_CODES.contains(a))
        .unwrap_or("");
    let mut range_invalid = false;
    let date_from = parse_bound(query.from.as_deref(), &mut range_invalid);
    let date_to = parse_bound(query.to.as_deref(), &mut range_invalid);
    if let (Some(from), Some(to)) = (date_from, date_to)
        && to < from
    {
        range_invalid = true;
    }
    let rows = if range_invalid {
        Vec::new()
    } else {
        match audit_query_service::export_audit_logs(
            &pool,
            (!target.is_empty()).then_some(target),
            (!action.is_empty()).then_some(action),
            date_from,
            date_to,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => return db_error("audit", e),
        }
    };
    let jst = crate::models::jst::jst();
    let csv_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.created_at
                    .with_timezone(&jst)
                    .format("%Y/%m/%d %H:%M:%S")
                    .to_string(),
                r.admin_name.clone(),
                action_label(&r.action).into_owned(),
                r.action.clone(),
                target_label(&r.target_type).to_string(),
                r.target_id.map(|id| id.to_string()).unwrap_or_default(),
            ]
        })
        .collect();
    let bytes = crate::util::csv_transfer::write_csv(
        &["日時", "管理者", "操作", "操作コード", "対象", "対象ID"],
        &csv_rows,
    );
    let today = crate::models::jst::jst_today();
    let disposition = format!(
        "attachment; filename=\"audit_{}.csv\"",
        today.format("%Y%m%d")
    );
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=UTF-8"),
            (header::CONTENT_DISPOSITION, disposition.as_str()),
        ],
        bytes,
    )
        .into_response()
}
