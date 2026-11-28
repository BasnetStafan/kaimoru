//! Admin account management (owners only).
//!
//! The list, create and edit screens for admin accounts, plus
//! the owner-triggered password reset. The permission guard
//! already refuses non-owners on these paths; every handler
//! repeats the owner check, so the surface stays owner-only
//! even if a route is ever mounted outside the guard's table.
//! The business rails (no self-edits here, the last active
//! owner is protected) live in
//! [`crate::services::admin::admin_account_service`].

use axum::{
    Form, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, record_not_found, render, set_flash};
use crate::error::AppError;
use crate::models::{
    ADMIN_AREAS, AREA_CONTENT, AREA_MAIL, AREA_PRODUCTS, AREA_SALES, AREA_SETTINGS, AREA_STORE,
    AdminIdentity, AdminSummary,
};
use crate::services::admin::admin_account_service;
use crate::session::CurrentAdmin;
use crate::validation::{FieldError, field_error};
use crate::views::admin::admins::{AdminFormView, AdminListView, AreaChoice};
use crate::views::admin::notfound::bare_403_html;

/// Routes for the account management surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/admins", get(list).post(create))
        .route("/admin/admins/new", get(new_page))
        .route("/admin/admins/{id}/edit", get(edit_page))
        .route("/admin/admins/{id}", post(update))
        .route("/admin/admins/{id}/password", post(reset))
}

/// The styled 403 for a non-owner who somehow reaches a
/// handler (see the module docs): the same page the
/// permission guard serves.
fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, Html(bare_403_html())).into_response()
}

/// The account list.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    if !admin.is_owner() {
        return forbidden();
    }
    match admin_account_service::list_admins(&pool).await {
        Ok(rows) => render(AdminListView {
            layout: layout_for(&pool, &admin, &session).await,
            nav: "admins",
            rows,
        }),
        Err(e) => db_error("admins", e),
    }
}

/// The create form, blank.
pub async fn new_page(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    if !admin.is_owner() {
        return forbidden();
    }
    render(
        form_view(
            &pool,
            &admin,
            &session,
            "管理者を登録",
            "/admin/admins".to_string(),
            true,
            FormValues {
                email: String::new(),
                display_name: String::new(),
                role: "staff".to_string(),
                permissions: Vec::new(),
                is_active: true,
            },
            None,
            false,
        )
        .await,
    )
}

/// The create + edit forms' shared body. Password inputs post
/// only on create; the active flag only on edit; the six
/// permission checkboxes post as `perm_<area>` fields.
#[derive(Deserialize)]
pub struct AccountForm {
    /// Email as typed (create only; the edit form has no
    /// email input — the sign-in ID never changes).
    email: Option<String>,
    /// Display name as typed.
    display_name: String,
    /// Initial password as typed (create only). Never logged,
    /// never echoed back into a page.
    #[serde(rename = "password")]
    pw: Option<String>,
    /// Initial password again (create only).
    #[serde(rename = "password_confirm")]
    pw_confirm: Option<String>,
    /// Role as posted: `owner` or `staff`.
    role: String,
    /// Active flag as posted by the edit form's radios:
    /// `active` or `disabled`.
    is_active: Option<String>,
    /// The sales checkbox.
    perm_sales: Option<String>,
    /// The products checkbox.
    perm_products: Option<String>,
    /// The content checkbox.
    perm_content: Option<String>,
    /// The mail checkbox.
    perm_mail: Option<String>,
    /// The store checkbox.
    perm_store: Option<String>,
    /// The settings checkbox.
    perm_settings: Option<String>,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the form, which the
    /// edit submit treats as stale. The create submit
    /// ignores it.
    #[serde(default)]
    form_ticket: String,
}

impl AccountForm {
    /// The permission keys whose checkbox was ticked.
    fn permissions(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.perm_sales.is_some() {
            out.push(AREA_SALES.to_string());
        }
        if self.perm_products.is_some() {
            out.push(AREA_PRODUCTS.to_string());
        }
        if self.perm_content.is_some() {
            out.push(AREA_CONTENT.to_string());
        }
        if self.perm_mail.is_some() {
            out.push(AREA_MAIL.to_string());
        }
        if self.perm_store.is_some() {
            out.push(AREA_STORE.to_string());
        }
        if self.perm_settings.is_some() {
            out.push(AREA_SETTINGS.to_string());
        }
        out
    }
}

/// Handles the create submit.
pub async fn create(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<AccountForm>,
) -> Response {
    if !admin.is_owner() {
        return forbidden();
    }
    // The stored form of the sign-in ID: trimmed and
    // lowercased, the same normalisation the login applies.
    let email = form.email.as_deref().unwrap_or("").trim().to_lowercase();
    let display_name = form.display_name.trim().to_string();
    let pw = form.pw.as_deref().unwrap_or("");
    let pw_confirm = form.pw_confirm.as_deref().unwrap_or("");
    let values = FormValues {
        email: email.clone(),
        display_name: display_name.clone(),
        role: form.role.clone(),
        permissions: form.permissions(),
        is_active: true,
    };
    let errors =
        admin_account_service::validate_create(&email, &display_name, &form.role, pw, pw_confirm);
    if !errors.is_empty() {
        let mut view = form_view(
            &pool,
            &admin,
            &session,
            "管理者を登録",
            "/admin/admins".to_string(),
            true,
            values,
            None,
            false,
        )
        .await;
        apply_errors(&mut view, &errors);
        return render(view);
    }
    match admin_account_service::create_admin(
        &pool,
        admin.id,
        &email,
        &display_name,
        &form.role,
        &form.permissions(),
        pw,
    )
    .await
    {
        Ok(id) => {
            set_flash(&session, "管理者を登録しました").await;
            Redirect::to(&format!("/admin/admins/{id}/edit")).into_response()
        }
        Err(AppError::Validation(errors)) => {
            let mut view = form_view(
                &pool,
                &admin,
                &session,
                "管理者を登録",
                "/admin/admins".to_string(),
                true,
                values,
                None,
                false,
            )
            .await;
            apply_errors(&mut view, &errors);
            render(view)
        }
        Err(e) => db_error("admins", e),
    }
}

/// The edit form for one account.
pub async fn edit_page(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(id_raw): Path<String>,
) -> Response {
    if !admin.is_owner() {
        return forbidden();
    }
    let Some(target) = load_target(&pool, &id_raw).await else {
        return record_not_found(&pool, &admin, &session).await;
    };
    render(edit_view(&pool, &admin, &session, &target, values_of(&target)).await)
}

/// The form-ticket scope of one staff account.
fn admin_scope(id: i64) -> String {
    format!("admin:{id}")
}

/// Handles the edit submit: name, role, permissions, active.
pub async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(id_raw): Path<String>,
    Form(form): Form<AccountForm>,
) -> Response {
    if !admin.is_owner() {
        return forbidden();
    }
    let Some(target) = load_target(&pool, &id_raw).await else {
        return record_not_found(&pool, &admin, &session).await;
    };
    // Spend the form ticket before validating: a submit
    // without a valid ticket was not rendered from the
    // current record, so it is refused as stale. A ticket
    // whose stamp has gone stale is refused by the
    // service's check instead (the guarded arm below).
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &admin_scope(target.id),
        &form.form_ticket,
    )
    .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/admins/{}/edit", target.id)).into_response();
    };
    let display_name = form.display_name.trim().to_string();
    // An absent flag (a crafted post) keeps the stored state;
    // the rendered form always posts one of the two values.
    let is_active = form
        .is_active
        .as_deref()
        .map(|v| v == "active")
        .unwrap_or(target.is_active);
    let values = FormValues {
        email: target.email.clone(),
        display_name: display_name.clone(),
        role: form.role.clone(),
        permissions: form.permissions(),
        is_active,
    };
    let errors = admin_account_service::validate_update(&display_name, &form.role);
    if !errors.is_empty() {
        let mut view = edit_view(&pool, &admin, &session, &target, values).await;
        apply_errors(&mut view, &errors);
        return render(view);
    }
    match admin_account_service::update_admin(
        &pool,
        admin.id,
        &target,
        &display_name,
        &form.role,
        &form.permissions(),
        is_active,
        expected,
    )
    .await
    {
        Ok(()) => {
            set_flash(&session, "管理者を更新しました").await;
            Redirect::to(&format!("/admin/admins/{}/edit", target.id)).into_response()
        }
        Err(AppError::Validation(errors)) => {
            let mut view = edit_view(&pool, &admin, &session, &target, values).await;
            apply_errors(&mut view, &errors);
            render(view)
        }
        // The staleness guard's conflict goes back to a
        // fresh edit page (current values, explanatory
        // flash), not the submitted-values re-render the
        // other conflicts use.
        Err(AppError::Conflict(message)) if message == crate::messages::STALE_FORM_MESSAGE => {
            set_flash(&session, &message).await;
            Redirect::to(&format!("/admin/admins/{}/edit", target.id)).into_response()
        }
        Err(AppError::Conflict(message)) => {
            let mut view = edit_view(&pool, &admin, &session, &target, values).await;
            view.form_error = Some(message);
            render(view)
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("admins", e),
    }
}

/// The password-reset form body (the edit page's second form).
#[derive(Deserialize)]
pub struct ResetForm {
    /// The new password as typed. Never logged, never echoed.
    #[serde(rename = "new_password")]
    new_pw: String,
    /// The new password again, as typed.
    new_password_confirm: String,
}

/// Handles the owner-triggered password reset for one account.
pub async fn reset(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(id_raw): Path<String>,
    Form(form): Form<ResetForm>,
) -> Response {
    if !admin.is_owner() {
        return forbidden();
    }
    let Some(target) = load_target(&pool, &id_raw).await else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let errors = admin_account_service::validate_reset(&form.new_pw, &form.new_password_confirm);
    if !errors.is_empty() {
        let mut view = edit_view(&pool, &admin, &session, &target, values_of(&target)).await;
        view.reset_new_error = field_error(&errors, "new_password");
        view.reset_confirm_error = field_error(&errors, "new_password_confirm");
        return render(view);
    }
    match admin_account_service::reset_password(&pool, admin.id, target.id, &form.new_pw).await {
        Ok(()) => {
            set_flash(&session, "パスワードをリセットしました").await;
            Redirect::to(&format!("/admin/admins/{}/edit", target.id)).into_response()
        }
        Err(AppError::Conflict(message)) => {
            let mut view = edit_view(&pool, &admin, &session, &target, values_of(&target)).await;
            view.reset_error = Some(message);
            render(view)
        }
        Err(AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("admins", e),
    }
}

/// Loads the edit target from a raw path segment, parsed by
/// hand (the admin convention): a non-numeric segment or an
/// unknown ID both resolve to `None`, and the caller answers
/// the styled record 404.
async fn load_target(pool: &PgPool, id_raw: &str) -> Option<AdminSummary> {
    let id: i64 = id_raw.parse().ok()?;
    admin_account_service::find_admin(pool, id)
        .await
        .ok()
        .flatten()
}

/// The stored values of one account, as form values.
fn values_of(target: &AdminSummary) -> FormValues {
    FormValues {
        email: target.email.clone(),
        display_name: target.display_name.clone(),
        role: target.role.clone(),
        permissions: target.permissions.clone(),
        is_active: target.is_active,
    }
}

/// The edit page's view for one account and one value set.
async fn edit_view(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    target: &AdminSummary,
    values: FormValues,
) -> AdminFormView {
    let mut view = form_view(
        pool,
        admin,
        session,
        "管理者を編集",
        format!("/admin/admins/{}", target.id),
        false,
        values,
        Some(format!("/admin/admins/{}/password", target.id)),
        target.id == admin.id,
    )
    .await;
    // The ticket binds this render to the account's current
    // stamp (read from the repository: `AdminSummary` does
    // not carry `updated_at`).
    // A failed stamp read is logged and reads as 0, a value
    // no real stamp equals, so the ticket fails closed (the
    // rule `profile_stamp` documents on the member side).
    let stamp =
        match crate::repositories::admin::admin_repository::updated_at(pool, target.id).await {
            Ok(stamp) => stamp.map(|t| t.timestamp_micros()).unwrap_or(0),
            Err(e) => {
                tracing::error!("admin stamp read failed: {e}");
                0
            }
        };
    view.form_ticket =
        crate::services::form_ticket_service::issue(session, &admin_scope(target.id), stamp).await;
    view
}

/// Copies a validation list into the form view's error slots.
fn apply_errors(view: &mut AdminFormView, errors: &[FieldError]) {
    view.email_error = field_error(errors, "email");
    view.name_error = field_error(errors, "display_name");
    view.password_error = field_error(errors, "password");
    view.confirm_error = field_error(errors, "password_confirm");
    view.role_error = field_error(errors, "role");
}

/// The values a form render carries: the stored account's on
/// a fresh render, the submitted ones after a failed submit
/// (so the operator fixes the input instead of retyping it —
/// passwords are the exception and never come back).
struct FormValues {
    /// The email value.
    email: String,
    /// The display name value.
    display_name: String,
    /// The selected role.
    role: String,
    /// The ticked permission keys.
    permissions: Vec<String>,
    /// The active flag's state.
    is_active: bool,
}

/// Builds the account form view with every error slot empty.
#[allow(clippy::too_many_arguments)]
async fn form_view(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    heading: &str,
    action: String,
    is_new: bool,
    values: FormValues,
    reset_action: Option<String>,
    is_self: bool,
) -> AdminFormView {
    AdminFormView {
        layout: layout_for(pool, admin, session).await,
        nav: "admins",
        heading: heading.to_string(),
        action,
        is_new,
        email: values.email,
        display_name: values.display_name,
        role: values.role,
        areas: ADMIN_AREAS
            .iter()
            .map(|(key, label)| AreaChoice {
                key,
                label,
                checked: values.permissions.iter().any(|p| p == key),
            })
            .collect(),
        is_active: values.is_active,
        is_self,
        form_error: None,
        form_ticket: String::new(),
        email_error: None,
        name_error: None,
        password_error: None,
        confirm_error: None,
        role_error: None,
        reset_action,
        reset_error: None,
        reset_new_error: None,
        reset_confirm_error: None,
    }
}
