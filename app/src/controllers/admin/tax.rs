//! Tax administration.
//!
//! The operator's page for consumption-tax classes and their
//! dated rates (Phase 2). Rates are append-only history: a tax
//! change is registered as a new row with a future effective
//! date, and orders always resolve the rate of their own date.

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
use crate::error::AppError;
use crate::services::admin::tax_service::{self, RateFormInput};
use crate::session::CurrentAdmin;
use crate::views::admin::tax::TaxSettingsView;

/// Routes for the tax settings surface.
pub fn router() -> Router<PgPool> {
    Router::new().route("/admin/settings/tax", get(page).post(add_rate))
}

/// The add-rate form body, exactly as typed.
#[derive(Deserialize)]
pub struct RateForm {
    /// Tax class ID.
    tax_class_id: String,
    /// Rate percent.
    rate_percent: String,
    /// Effective-from date.
    effective_from: String,
}

/// GET the tax settings page.
async fn page(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    render_page(
        &pool,
        &admin,
        &session,
        &RateFormInput {
            tax_class_id: String::new(),
            rate_percent: String::new(),
            effective_from: String::new(),
        },
        None,
    )
    .await
}

/// POST a new rate: success redirects with a flash; failure
/// re-renders with every error under its field.
async fn add_rate(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<RateForm>,
) -> Response {
    let input = RateFormInput {
        tax_class_id: form.tax_class_id,
        rate_percent: form.rate_percent,
        effective_from: form.effective_from,
    };
    match tax_service::add_rate(&pool, admin.id, &input).await {
        Ok(()) => {
            set_flash(&session, "税率を登録しました").await;
            Redirect::to("/admin/settings/tax").into_response()
        }
        Err(AppError::Validation(errors)) => {
            let by_field = crate::services::admin::product_service::errors_by_field(&errors);
            render_page(&pool, &admin, &session, &input, Some(&by_field)).await
        }
        Err(e) => db_error("tax settings", e),
    }
}

/// Renders the page for the given form state.
async fn render_page(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    input: &RateFormInput,
    errors: Option<&std::collections::BTreeMap<&'static str, String>>,
) -> Response {
    let classes = match tax_service::overview(pool).await {
        Ok(v) => v,
        Err(e) => return db_error("tax settings", e),
    };
    let err = |field: &str| errors.and_then(|m| m.get(field).cloned());
    render(TaxSettingsView {
        layout: layout_for(pool, admin, session).await,
        nav: "tax_settings",
        classes,
        form_class_id: input.tax_class_id.clone(),
        form_rate: input.rate_percent.clone(),
        form_from: input.effective_from.clone(),
        class_error: err("tax_class_id"),
        rate_error: err("rate_percent"),
        from_error: err("effective_from"),
    })
}
