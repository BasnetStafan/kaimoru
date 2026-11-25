//! Points controller (EC V2 core 1).
//!
//! The member's points page: balance, soonest expiry and
//! the ledger history. Member-only throughout — the
//! [`CurrentUser`] extractor bounces guests to login.

use axum::{Router, extract::State, response::Response, routing::get};
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::models::points::{
    KIND_ADMIN_ADJUST, KIND_EARN, KIND_EARN_REVOKE, KIND_EXPIRE, KIND_SPEND, KIND_SPEND_REFUND,
};
use crate::repositories::point_repository;
use crate::services::point_service;
use crate::session::CurrentUser;
use crate::views::storefront::points::{PointHistoryRow, PointsPageView};

/// Routes owned by the points controller.
pub fn router() -> Router<PgPool> {
    Router::new().route("/mypage/points", get(points_page))
}

/// The Japanese label for a ledger movement kind.
fn kind_label(kind: &str) -> &'static str {
    match kind {
        KIND_EARN => "ポイント付与",
        KIND_SPEND => "ポイント利用",
        KIND_SPEND_REFUND => "ポイント返還",
        KIND_EARN_REVOKE => "付与取消",
        KIND_EXPIRE => "ポイント失効",
        KIND_ADMIN_ADJUST => "ポイント調整",
        _ => "ポイント",
    }
}

/// A signed points amount in display shape: +1,000 / −300.
fn points_display(points: i64) -> String {
    let digits = format!("{}", points.abs());
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    if points < 0 {
        format!("−{grouped}")
    } else {
        format!("+{grouped}")
    }
}

/// GET /mypage/points: the member's points home.
async fn points_page(
    CurrentUser(user): CurrentUser,
    State(pool): State<PgPool>,
    session: Session,
) -> Response {
    let balance = point_service::balance(&pool, user.id).await.unwrap_or(0);
    let next_expiry_label = match point_repository::next_expiry(&pool, user.id).await {
        Ok(Some((points, expires_at))) => Some(format!(
            "{} ポイント（{}まで）",
            points,
            crate::models::jst::jst_date_label(expires_at)
        )),
        _ => None,
    };
    let history = point_repository::history(&pool, user.id, 50)
        .await
        .unwrap_or_default();
    let rows = history
        .iter()
        .map(|row| PointHistoryRow {
            date_label: crate::models::jst::jst_long_datetime_label(row.created_at),
            kind_label: kind_label(&row.kind).to_string(),
            points_display: points_display(row.points),
            order_id: row.order_id,
        })
        .collect();
    render(PointsPageView {
        header: header_for(&pool, &session, false).await,
        balance,
        next_expiry_label,
        rows,
    })
}
