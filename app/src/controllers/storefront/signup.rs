//! Signup controller.
//! The sign-up flow only: email entry, code verification, member
//! details, confirmation and completion. Login and logout live in the
//! login controller. Rules live in the auth service; this layer moves
//! the browser between the steps.

use std::net::SocketAddr;

use axum::{
    Form, Router,
    extract::{ConnectInfo, State},
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::error::AppError;
use crate::messages::{
    SIGNUP_COMPLETED_NOTICE, TERMS_AGREEMENT_REQUIRED_MESSAGE, TOO_MANY_REQUESTS_MESSAGE,
    VERIFICATION_CODE_RESENT_NOTICE,
};
use crate::models::PendingRegistration;
use crate::services::cart_service;
use crate::services::member_account_service::{self, DetailsForm};
use crate::services::sns_service::SnsPendingProfile;
use crate::session::{
    SESSION_FLASH, SESSION_PENDING_ID, SESSION_SNS_PENDING, SESSION_USER_ID, pending_id,
};
use crate::views::storefront::signup::{
    ConfirmView, DetailsErrors, DetailsValues, DetailsView, SignupView, VerifyView,
};
use crate::views::storefront::sns::sns_block;
use crate::views::storefront::{PREFECTURES, mask_email};

/// Routes owned by the signup controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/signup", get(signup_page).post(signup_start))
        .route("/signup/verify", get(verify_page).post(verify_submit))
        .route("/signup/resend", post(resend_submit))
        .route("/signup/details", get(details_page).post(details_submit))
        .route("/signup/confirm", post(confirm_submit))
}

/// Form posted by the email entry step.
#[derive(Deserialize)]
pub struct EmailForm {
    email: String,
    email_confirm: String,
}

/// GET /signup: the email entry page.
async fn signup_page(session: Session, State(pool): State<PgPool>) -> Response {
    render(SignupView {
        header: header_for(&pool, &session, false).await,
        error: None,
        email: String::new(),
        sns: sns_block("", true),
    })
}

/// POST /signup: starts the sign-up and sends the verification code.
async fn signup_start(
    session: Session,
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<EmailForm>,
) -> Response {
    // Starting a sign-up sends a real email: throttle by IP and
    // by address before any mail is sent.
    let ip = crate::session::client_ip(&addr, &headers);
    if !crate::middleware::auth_throttle::signup_start_allowed(&form.email, &ip) {
        return render(SignupView {
            header: header_for(&pool, &session, false).await,
            error: Some(TOO_MANY_REQUESTS_MESSAGE.to_string()),
            email: form.email.clone(),
            sns: sns_block("", true),
        });
    }
    if form.email.trim() != form.email_confirm.trim() {
        return render(SignupView {
            header: header_for(&pool, &session, false).await,
            error: Some("確認用メールアドレスが一致しません。".to_string()),
            email: form.email.clone(),
            sns: sns_block("", true),
        });
    }
    match member_account_service::start_signup(&pool, &form.email).await {
        Ok(pending_id) => {
            let _ = session.insert(SESSION_PENDING_ID, pending_id).await;
            Redirect::to("/signup/verify").into_response()
        }
        Err(e) => render(SignupView {
            header: header_for(&pool, &session, false).await,
            error: Some(e.user_message()),
            email: form.email.clone(),
            sns: sns_block("", true),
        }),
    }
}

/// Form posted by the code entry step.
#[derive(Deserialize)]
pub struct CodeForm {
    code: String,
}

/// GET /signup/verify: the code entry page.
async fn verify_page(session: Session, State(pool): State<PgPool>) -> Response {
    let Some(pending) = load_flow_pending(&session, &pool).await else {
        return Redirect::to("/signup").into_response();
    };
    render(VerifyView {
        header: header_for(&pool, &session, false).await,
        error: None,
        notice: None,
        email_masked: mask_email(&pending.email),
    })
}

/// POST /signup/verify: checks the entered code.
async fn verify_submit(
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<CodeForm>,
) -> Response {
    let Some(pending) = load_flow_pending(&session, &pool).await else {
        return Redirect::to("/signup").into_response();
    };
    match member_account_service::verify_code(&pool, &pending, &form.code).await {
        Ok(()) => {
            // An SNS registration that verified its email by
            // code continues on the SNS details step.
            if sns_profile(&session).await.is_some() {
                Redirect::to("/signup/sns/details").into_response()
            } else {
                Redirect::to("/signup/details").into_response()
            }
        }
        Err(e) => render(VerifyView {
            header: header_for(&pool, &session, false).await,
            error: Some(e.user_message()),
            notice: None,
            email_masked: mask_email(&pending.email),
        }),
    }
}

/// POST /signup/resend: sends a fresh code under the cooldown rules.
async fn resend_submit(session: Session, State(pool): State<PgPool>) -> Response {
    let Some(pending) = load_flow_pending(&session, &pool).await else {
        return Redirect::to("/signup").into_response();
    };
    let (error, notice) = match member_account_service::resend_code(&pool, &pending).await {
        Ok(()) => (None, Some(VERIFICATION_CODE_RESENT_NOTICE.to_string())),
        Err(e) => (Some(e.user_message()), None),
    };
    render(VerifyView {
        header: header_for(&pool, &session, false).await,
        error,
        notice,
        email_masked: mask_email(&pending.email),
    })
}

/// Form posted by the member details step.
#[derive(Deserialize)]
pub struct DetailsPost {
    password: String,
    /// Password confirmation, posted only by the browser form
    /// (details.html, non-SNS mode). Absent from suite/API
    /// posts, which keep the previous behaviour; when present
    /// it must match, checked in details_submit.
    #[serde(default)]
    password_confirm: Option<String>,
    family_name: String,
    given_name: String,
    family_name_kana: String,
    given_name_kana: String,
    birth_date: String,
    gender: String,
    postal_code: String,
    prefecture: String,
    city: String,
    address_line: String,
    building: String,
    phone: String,
    /// Mail-magazine checkbox: posts `true` when checked, so
    /// the value stays text like every other posted field.
    #[serde(default)]
    mail_magazine: String,
    /// Terms checkbox: posts `true` when checked.
    #[serde(default)]
    agree_terms: String,
}

/// GET /signup/details: the member details form, after verification.
async fn details_page(session: Session, State(pool): State<PgPool>) -> Response {
    let Some(pending) = load_flow_pending(&session, &pool).await else {
        return Redirect::to("/signup").into_response();
    };
    if !pending.verified {
        return Redirect::to("/signup/verify").into_response();
    }
    if sns_profile(&session).await.is_some() {
        return Redirect::to("/signup/sns/details").into_response();
    }
    render(DetailsView {
        sns_mode: false,
        header: header_for(&pool, &session, false).await,
        error: None,
        email: pending.email.clone(),
        values: DetailsValues::default(),
        errors: DetailsErrors::default(),
        prefectures: &PREFECTURES,
    })
}

/// POST /signup/details: validates and stores the details, then shows
/// the confirmation page. Validation failures come back with every
/// failing field's error placed under that field.
async fn details_submit(
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<DetailsPost>,
) -> Response {
    let Some(pending) = load_flow_pending(&session, &pool).await else {
        return Redirect::to("/signup").into_response();
    };

    // The entered values, kept so a failed submit loses nothing.
    let values = DetailsValues {
        family_name: form.family_name.clone(),
        given_name: form.given_name.clone(),
        family_name_kana: form.family_name_kana.clone(),
        given_name_kana: form.given_name_kana.clone(),
        birth_date: form.birth_date.clone(),
        gender: form.gender.clone(),
        postal_code: form.postal_code.clone(),
        prefecture: form.prefecture.clone(),
        city: form.city.clone(),
        address_line: form.address_line.clone(),
        building: form.building.clone(),
        phone: form.phone.clone(),
        mail_magazine: form.mail_magazine == "true",
    };

    if form.agree_terms != "true" {
        return render(
            details_view(
                &pool,
                &session,
                &pending,
                Some(TERMS_AGREEMENT_REQUIRED_MESSAGE.to_string()),
                values,
                DetailsErrors::default(),
            )
            .await,
        );
    }

    // The confirmation check: only when the browser form
    // posted the field (SNS signups never see it).
    if let Some(confirm) = &form.password_confirm
        && *confirm != form.password
    {
        let errors = DetailsErrors {
            password_confirm: Some("パスワードが一致しません。".to_string()),
            ..Default::default()
        };
        return render(details_view(&pool, &session, &pending, None, values, errors).await);
    }

    let details = DetailsForm {
        password: form.password.clone(),
        family_name: form.family_name.clone(),
        given_name: form.given_name.clone(),
        family_name_kana: form.family_name_kana.clone(),
        given_name_kana: form.given_name_kana.clone(),
        birth_date: form.birth_date.clone(),
        gender: form.gender.clone(),
        postal_code: form.postal_code.clone(),
        prefecture: form.prefecture.clone(),
        city: form.city.clone(),
        address_line: form.address_line.clone(),
        building: form.building.clone(),
        phone: form.phone.clone(),
        mail_magazine: form.mail_magazine == "true",
    };

    match member_account_service::save_details(&pool, &pending, &details).await {
        Ok(()) => {
            // Reload so the confirmation shows exactly what was stored.
            match member_account_service::load_pending(&pool, pending.id).await {
                Ok(stored) => render(confirm_view(&pool, &session, &stored, None).await),
                Err(e) => render(
                    details_view(
                        &pool,
                        &session,
                        &pending,
                        Some(e.user_message()),
                        values,
                        DetailsErrors::default(),
                    )
                    .await,
                ),
            }
        }
        Err(AppError::Validation(list)) => render(
            details_view(
                &pool,
                &session,
                &pending,
                None,
                values,
                DetailsErrors::from_field_errors(&list),
            )
            .await,
        ),
        Err(e) => render(
            details_view(
                &pool,
                &session,
                &pending,
                Some(e.user_message()),
                values,
                DetailsErrors::default(),
            )
            .await,
        ),
    }
}

/// Builds the details view for a re-display: the form again, with the
/// entered values kept and the errors placed under their fields.
pub(crate) async fn details_view(
    pool: &PgPool,
    session: &Session,
    pending: &PendingRegistration,
    error: Option<String>,
    values: DetailsValues,
    errors: DetailsErrors,
) -> DetailsView {
    DetailsView {
        sns_mode: sns_profile(session).await.is_some(),
        header: header_for(pool, session, false).await,
        error,
        email: pending.email.clone(),
        values,
        errors,
        prefectures: &PREFECTURES,
    }
}

/// Builds the confirmation view from a stored pending registration.
pub(crate) async fn confirm_view(
    pool: &PgPool,
    session: &Session,
    pending: &PendingRegistration,
    error: Option<String>,
) -> ConfirmView {
    ConfirmView {
        sns_mode: sns_profile(session).await.is_some(),
        header: header_for(pool, session, false).await,
        error,
        email: pending.email.clone(),
        family_name: pending.family_name.clone().unwrap_or_default(),
        given_name: pending.given_name.clone().unwrap_or_default(),
        family_name_kana: pending.family_name_kana.clone().unwrap_or_default(),
        given_name_kana: pending.given_name_kana.clone().unwrap_or_default(),
        birth_date: pending
            .birth_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default(),
        gender: pending.gender.clone().unwrap_or_default(),
        postal_code: pending.postal_code.clone().unwrap_or_default(),
        prefecture: pending.prefecture.clone().unwrap_or_default(),
        city: pending.city.clone().unwrap_or_default(),
        address_line: pending.address_line.clone().unwrap_or_default(),
        building: pending.building.clone().unwrap_or_default(),
        phone: pending.phone.clone().unwrap_or_default(),
        mail_magazine_text: if pending.mail_magazine {
            "受け取る".to_string()
        } else {
            "受け取らない".to_string()
        },
    }
}

/// POST /signup/confirm: creates the account, logs the new member in
/// with a regenerated session ID, and redirects to the top page.
async fn confirm_submit(session: Session, State(pool): State<PgPool>) -> Response {
    let Some(pending) = load_flow_pending(&session, &pool).await else {
        return Redirect::to("/signup").into_response();
    };
    match member_account_service::complete_signup(
        &pool,
        &pending,
        cart_service::existing_guest_token(&session)
            .await
            .as_deref(),
    )
    .await
    {
        Ok(user) => {
            let _ = session.remove::<i64>(SESSION_PENDING_ID).await;
            // A fresh session ID at the moment of login prevents session
            // fixation: the pre-login ID can never become the logged in one.
            let _ = session.cycle_id().await;
            let _ = session.insert(SESSION_USER_ID, user.id).await;
            let _ = session.insert(SESSION_FLASH, SIGNUP_COMPLETED_NOTICE).await;
            Redirect::to("/").into_response()
        }
        Err(e) => render(confirm_view(&pool, &session, &pending, Some(e.user_message())).await),
    }
}

/// Loads the pending registration this session is working through.
pub(crate) async fn load_flow_pending(
    session: &Session,
    pool: &PgPool,
) -> Option<PendingRegistration> {
    let id = pending_id(session).await?;
    member_account_service::load_pending(pool, id).await.ok()
}

/// The SNS registration profile parked in this session, if any:
/// its presence is what makes a sign-up the SNS variant.
pub(crate) async fn sns_profile(session: &Session) -> Option<SnsPendingProfile> {
    session
        .get::<SnsPendingProfile>(SESSION_SNS_PENDING)
        .await
        .ok()
        .flatten()
}
