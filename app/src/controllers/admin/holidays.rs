//! Shop holidays.
//!
//! The operator's closed days: a small list page with an add
//! form and per-row delete (native confirm via admin.js, the
//! house pattern). Rules live in
//! `crate::services::admin::holiday_service`.

use axum::{
    Form, Router,
    extract::State,
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, render, set_flash};
use crate::services::admin::holiday_service;
use crate::session::CurrentAdmin;
use crate::views::admin::holidays::{HolidayRowView, HolidaysView};

/// Routes for the holidays page.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/settings/holidays", get(page).post(add))
        .route(
            "/admin/settings/holidays/delete",
            axum::routing::post(remove),
        )
}

/// Builds the page view around the stored list and the add
/// form's current values.
async fn build_view(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    date_value: String,
    label_value: String,
) -> Result<HolidaysView, crate::error::AppError> {
    let rows = holiday_service::list(pool).await?;
    let today = crate::models::jst::jst_today();
    Ok(HolidaysView {
        layout: layout_for(pool, admin, session).await,
        nav: "holidays",
        holidays: rows
            .iter()
            .map(|h| HolidayRowView {
                date_value: h.date.format("%Y-%m-%d").to_string(),
                date_label: crate::models::delivery::delivery_date_label(h.date),
                label: if h.label.is_empty() {
                    "—".to_string()
                } else {
                    h.label.clone()
                },
                past: h.date < today,
            })
            .collect(),
        date_value,
        label_value,
        date_error: None,
        label_error: None,
    })
}

/// The holidays page.
pub async fn page(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    match build_view(&pool, &admin, &session, String::new(), String::new()).await {
        Ok(view) => render(view),
        Err(e) => db_error("holidays", e),
    }
}

/// The add form body.
#[derive(Deserialize)]
pub struct HolidayForm {
    /// The date as typed (`YYYY-MM-DD` from the date input).
    holiday_date: String,
    /// The label as typed; optional in the post.
    label: Option<String>,
}

/// Handles the add: success redirects back with the flash;
/// errors re-render with the typed values kept.
pub async fn add(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<HolidayForm>,
) -> Response {
    let label = form.label.clone().unwrap_or_default();
    match holiday_service::add(&pool, admin.id, &form.holiday_date, &label).await {
        Ok(()) => {
            set_flash(&session, "休業日を登録しました").await;
            Redirect::to("/admin/settings/holidays").into_response()
        }
        Err(errors) => {
            let mut view = match build_view(&pool, &admin, &session, form.holiday_date, label).await
            {
                Ok(v) => v,
                Err(e) => return db_error("holidays", e),
            };
            for e in &errors {
                match e.field {
                    "holiday_date" => view.date_error = Some(e.message.clone()),
                    "label" => view.label_error = Some(e.message.clone()),
                    _ => {}
                }
            }
            render(view)
        }
    }
}

/// The delete form body.
#[derive(Deserialize)]
pub struct HolidayDeleteForm {
    /// The date to remove (`YYYY-MM-DD`).
    holiday_date: String,
}

/// Handles the delete. An unparseable or unregistered date
/// simply lands back on the page: deletion is idempotent
/// from the operator's point of view.
pub async fn remove(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<HolidayDeleteForm>,
) -> Response {
    if let Ok(date) = chrono::NaiveDate::parse_from_str(form.holiday_date.trim(), "%Y-%m-%d") {
        match holiday_service::remove(&pool, admin.id, date).await {
            Ok(true) => set_flash(&session, "休業日を削除しました").await,
            Ok(false) => {}
            Err(e) => return db_error("holidays", e),
        }
    }
    Redirect::to("/admin/settings/holidays").into_response()
}
