//! Contact handlers: the inquiry
//! form for guests and members, the honeypot that drops
//! bot submissions behind a normal-looking redirect, and
//! the guarded completion page. Submitted contents are
//! never logged.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Form, Router, routing::get};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use crate::controllers::storefront::{header_for, render};
use crate::services::inquiry_service::{self, InquiryInput, SubmitError};
use crate::session::{self, SESSION_INQUIRY_REF};
use crate::views::storefront::contact::{
    ContactCompleteView, ContactErrors, ContactValues, ContactView,
};

/// The contact routes.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/contact", get(show_form).post(submit))
        .route("/contact/complete", get(complete))
}

/// Renders the form, with any preserved values and errors.
async fn render_form(
    pool: &PgPool,
    session: &Session,
    values: ContactValues,
    errors: Vec<crate::validation::FieldError>,
    form_error: Option<String>,
) -> Response {
    let settings = crate::services::shop_settings_service::load(pool).await;
    render(ContactView {
        header: header_for(pool, session, false).await,
        support_note: (!settings.support_note.trim().is_empty())
            .then(|| settings.support_note.clone()),
        values,
        errors: ContactErrors::from_field_errors(&errors),
        form_error,
    })
}

/// `GET /contact`: the blank form, prefilled for a
/// logged-in member (both fields stay editable — the
/// reply must reach the address they actually read).
async fn show_form(State(pool): State<PgPool>, session: Session) -> Response {
    let mut values = ContactValues::default();
    if let Some(user) = session::current_user(&pool, &session).await {
        values.name = format!("{} {}", user.family_name, user.given_name);
        values.email = user.email;
    }
    render_form(&pool, &session, values, Vec::new(), None).await
}

/// The posted contact form.
#[derive(Deserialize)]
pub struct ContactForm {
    /// Name.
    pub name: String,
    /// Email.
    pub email: String,
    /// Category code (empty when unchosen).
    #[serde(default)]
    pub category: String,
    /// Order number (may be empty).
    #[serde(default)]
    pub order_number: String,
    /// Body.
    pub body: String,
    /// The honeypot: humans never see or fill it.
    #[serde(default)]
    pub website: String,
}

/// `POST /contact`: honeypot first (a filled trap means
/// a bot — answer with the normal redirect and store
/// nothing), then validation with all errors at once,
/// then the rate limits inside the service, then the
/// insert + acknowledgement mail.
async fn submit(
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    session: Session,
    headers: axum::http::HeaderMap,
    Form(form): Form<ContactForm>,
) -> Response {
    if !form.website.trim().is_empty() {
        tracing::info!(action = "inquiry.honeypot", "inquiry dropped by honeypot");
        return Redirect::to("/contact/complete").into_response();
    }
    let values = ContactValues {
        name: form.name.clone(),
        email: form.email.clone(),
        category: form.category.clone(),
        order_number: form.order_number.clone(),
        body: form.body.clone(),
    };
    let input = InquiryInput {
        name: form.name,
        email: form.email,
        category: form.category,
        order_number: form.order_number,
        body: form.body,
    };
    let errors = inquiry_service::validate(&input);
    if !errors.is_empty() {
        return render_form(&pool, &session, values, errors, None).await;
    }
    let member = session::current_user(&pool, &session).await;
    // The client IP for the per-IP limit, resolved by the
    // same gated helper as login/signup/suggest: the proxy
    // header counts only with TRUST_PROXY_HEADERS=1, so a
    // direct client cannot spoof its way into a fresh
    // bucket. The per-email DB limit applies either way.
    let ip = session::client_ip(&addr, &headers);
    match inquiry_service::submit(&pool, &ip, member.map(|m| m.id), &input).await {
        Ok(id) => {
            let _ = session
                .insert(SESSION_INQUIRY_REF, format!("INQ-{id:06}"))
                .await;
            Redirect::to("/contact/complete").into_response()
        }
        Err(SubmitError::RateLimited) => {
            render_form(
                &pool,
                &session,
                values,
                Vec::new(),
                Some(inquiry_service::RATE_LIMIT_MESSAGE.to_string()),
            )
            .await
        }
        Err(SubmitError::Db(e)) => {
            tracing::error!(action = "inquiry.create", error = %e, "inquiry insert failed");
            render_form(
                &pool,
                &session,
                values,
                Vec::new(),
                Some(
                    "送信に失敗しました。しばらく時間をおいてから再度お試しください。".to_string(),
                ),
            )
            .await
        }
    }
}

/// `GET /contact/complete`: shows the reference once,
/// from the session flash; a direct visit without a
/// submission goes back to the form.
async fn complete(State(pool): State<PgPool>, session: Session) -> Response {
    let reference: Option<String> = session.remove(SESSION_INQUIRY_REF).await.ok().flatten();
    match reference {
        Some(reference) => render(ContactCompleteView {
            header: header_for(&pool, &session, false).await,
            reference,
        }),
        None => Redirect::to("/contact").into_response(),
    }
}
