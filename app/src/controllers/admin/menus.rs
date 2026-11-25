//! Menus (CMS spec C-4).
//!
//! The operator's editor for the layout's two link lists —
//! the header menu and the footer menu. The rules live in
//! `crate::services::admin::menu_service` — lengths, the
//! link rule, the last-visible-item guard, and the single
//! audited write. Handlers here own HTTP concerns only.
//!
//! The form is read as raw (name, value) pairs rather than
//! a derived struct: each menu's rows repeat indexed field
//! names (`items[footer][0][label]`, ...), and
//! serde_urlencoded cannot build a `Vec` from repeated keys
//! (the product form's precedent).
//!
//! Success answers with a redirect back to this page (the
//! admin surface's Post/Redirect/Get pattern).

use std::collections::BTreeMap;

use axum::{
    Form, Router,
    extract::State,
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, key_parts, layout_for, render, set_flash};
use crate::models::{FOOTER_KEY, HEADER_KEY};
use crate::services::admin::menu_service::{self, MenuItemInput, MenusInput};
use crate::session::CurrentAdmin;
use crate::views::admin::menus::MenusView;

/// Routes for the menus surface.
pub fn router() -> Router<PgPool> {
    Router::new().route("/admin/content/menus", get(page).post(update))
}

/// The submitted form as raw (name, value) pairs, in order.
type FormPairs = Vec<(String, String)>;

/// Builds one menu's input rows from the raw pairs. The
/// editor posts indexed fields (`items[footer][0][label]`,
/// ...); indices order the rows, and a checkbox's presence
/// — not its value — is its state.
fn rows_from(pairs: &FormPairs, menu_key: &str) -> Vec<MenuItemInput> {
    let mut rows: BTreeMap<usize, MenuItemInput> = BTreeMap::new();
    for (key, value) in pairs {
        let parts = key_parts(key);
        if parts.len() != 4 || parts[0] != "items" || parts[1] != menu_key {
            continue;
        }
        let Ok(index) = parts[2].parse::<usize>() else {
            continue;
        };
        let row = rows.entry(index).or_default();
        match parts[3].as_str() {
            "id" => row.id = value.clone(),
            "label" => row.label = value.clone(),
            "url" => row.url = value.clone(),
            "sort" => row.sort_order = value.clone(),
            "visible" => row.visible = true,
            "delete" => row.delete = true,
            _ => {}
        }
    }
    rows.into_values().collect()
}

/// Carries the submitted pairs into the service's input
/// shape: each menu's indexed rows, in index order.
fn input_from(pairs: &FormPairs) -> MenusInput {
    MenusInput {
        header: rows_from(pairs, HEADER_KEY),
        footer: rows_from(pairs, FOOTER_KEY),
    }
}

/// GET /admin/content/menus: the editor, freshly rendered
/// with the stored values.
pub async fn page(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    match menu_service::get_menus(&pool).await {
        Ok(menus) => {
            let mut view = MenusView::new(
                layout_for(&pool, &admin, &session).await,
                MenusInput::from(&menus),
            );
            view.form_ticket = crate::services::form_ticket_service::issue(
                &session,
                "menus",
                menu_service::fingerprint(&menus),
            )
            .await;
            render(view)
        }
        Err(e) => db_error("menus", e),
    }
}

/// Handles the menus save.
pub async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(pairs): Form<FormPairs>,
) -> Response {
    let input = input_from(&pairs);
    let current = match menu_service::get_menus(&pool).await {
        Ok(menus) => menus,
        Err(e) => return db_error("menus", e),
    };
    // Field errors first, all at once: a refused submit
    // shows every problem together, never one per round trip.
    let validated = match menu_service::validate_menus(&input, &current) {
        Ok(validated) => validated,
        Err(errors) => {
            let mut view = MenusView::new(layout_for(&pool, &admin, &session).await, input);
            view.apply_errors(errors);
            view.form_ticket = crate::services::form_ticket_service::issue(
                &session,
                "menus",
                menu_service::fingerprint(&current),
            )
            .await;
            return render(view);
        }
    };
    let ticket = pairs
        .iter()
        .find(|(key, _)| key == "form_ticket")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    let Some(expected) =
        crate::services::form_ticket_service::consume(&session, "menus", &ticket).await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to("/admin/content/menus").into_response();
    };
    match menu_service::update_menus(&pool, admin.id, &validated, expected).await {
        Ok(()) => {
            set_flash(&session, "メニュー設定を保存しました").await;
            Redirect::to("/admin/content/menus").into_response()
        }
        Err(crate::error::AppError::Conflict(_)) => {
            set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
            Redirect::to("/admin/content/menus").into_response()
        }
        Err(e) => db_error("menus", e),
    }
}
