//! API keys: issuing and revocation for external MCP
//! clients. The full token is rendered exactly once, on the
//! create response itself — a redirect would lose it, and
//! it is stored nowhere, so this page deliberately renders
//! from the POST instead of redirecting (the one exception
//! to the back office's Post/Redirect/Get default, for the
//! same reason every major API provider shows a new key
//! inline). Revocation follows the normal PRG shape.

use axum::{
    Form, Router,
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, record_not_found, render, set_flash};
use crate::models::AdminIdentity;
use crate::services::api_key_service;
use crate::session::CurrentAdmin;
use crate::views::admin::api_keys::{ApiKeyRowView, ApiKeysView};

/// Routes for the API keys surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/settings/api-keys", get(list).post(create))
        .route("/admin/settings/api-keys/{id}/revoke", post(revoke))
}

/// Builds the page view from the current keys plus a form
/// state (echoes, error, and the once-shown token).
async fn build_view(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    form_name: &str,
    form_scope: &str,
    form_error: String,
    created_token: Option<String>,
) -> Result<ApiKeysView, crate::error::AppError> {
    let rows = api_key_service::list_keys(pool)
        .await?
        .iter()
        .map(ApiKeyRowView::new)
        .collect();
    Ok(ApiKeysView {
        layout: layout_for(pool, admin, session).await,
        nav: "api_keys",
        rows,
        form_name: form_name.to_string(),
        form_scope: form_scope.to_string(),
        form_error,
        created_token,
    })
}

/// The list + issue form.
pub async fn list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    match build_view(&pool, &admin, &session, "", "", String::new(), None).await {
        Ok(view) => render(view),
        Err(e) => db_error("api keys", e),
    }
}

/// Issues one key; the success render carries the full
/// token, shown this once. Validation failures re-render
/// with the typed values kept (back-office rule).
pub async fn create(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let get = |name: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let name = get("name");
    let scope = get("scope");
    match api_key_service::create_key(&pool, admin.id, &name, &scope).await {
        Ok((_row, token)) => match build_view(
            &pool,
            &admin,
            &session,
            "",
            &scope,
            String::new(),
            Some(token),
        )
        .await
        {
            Ok(view) => render(view),
            Err(e) => db_error("api keys", e),
        },
        Err(crate::error::AppError::Conflict(message)) => {
            match build_view(&pool, &admin, &session, &name, &scope, message, None).await {
                Ok(view) => render(view),
                Err(e) => db_error("api keys", e),
            }
        }
        Err(e) => db_error("api key create", e),
    }
}

/// Revokes one key, audited. An unknown or already-revoked
/// key answers with the admin 404, like segment deletion.
pub async fn revoke(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match api_key_service::revoke_key(&pool, admin.id, id).await {
        Ok(true) => {
            set_flash(&session, "APIキーを無効化しました").await;
            Redirect::to("/admin/settings/api-keys").into_response()
        }
        Ok(false) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("api key revoke", e),
    }
}
