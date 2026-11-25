//! SNS login controller.
//! The provider round trip (start / callback), the SNS
//! registration steps, the My Page link management, and the
//! post-sign-in link confirmation. Business rules live in the
//! SNS and auth services; this layer moves the browser
//! between the steps and refuses anything the session cannot
//! prove. An identity is only ever created in an authenticated
//! session or atomically with a new account whose email the
//! provider (or this store's own code) verified - never by
//! silently merging on an email match.

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, not_found_fallback, render};
use crate::controllers::storefront::login::complete_login;
use crate::controllers::storefront::signup::{confirm_view, details_view, load_flow_pending};
use crate::error::AppError;
use crate::messages::{
    SIGNUP_COMPLETED_NOTICE, SNS_ACCOUNT_USED_MESSAGE, SNS_LINK_FAILED_MESSAGE,
    SNS_UNLINK_FAILED_MESSAGE, TERMS_AGREEMENT_REQUIRED_MESSAGE, TOO_MANY_REQUESTS_MESSAGE,
    sns_account_linked_message,
};

use crate::services::cart_service;
use crate::services::member_account_service::{self, DetailsForm};
use crate::services::sns_providers::SnsProvider;
use crate::services::sns_service::{self, SnsClaims, SnsFlow, SnsLinkPending, SnsPendingProfile};
use crate::session::{
    CurrentUser, SESSION_FLASH, SESSION_PENDING_ID, SESSION_SNS_FLOW, SESSION_SNS_LINK_PENDING,
    SESSION_SNS_PENDING, SESSION_USER_ID, current_user,
};
use crate::views::storefront::signup::{DetailsErrors, DetailsValues};
use crate::views::storefront::sns::{
    SnsEmailView, SnsLinkPendingInfo, SnsLinkRequiredView, SnsRow, SnsSettingsView,
};

/// How long an authorization round trip may take, in seconds.
const FLOW_WINDOW_SECS: i64 = 600;

/// How long a link-pending marker stays confirmable, in
/// seconds.
const LINK_PENDING_WINDOW_SECS: i64 = 1800;

/// The one failure message for a failed SNS sign-in: the
/// detail (provider error, bad state, failed exchange) is
/// logged server-side and never shown.
const SNS_FAILURE: &str = "SNSログインに失敗しました。時間をおいてもう一度お試しください。";

/// Routes owned by the SNS controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/auth/{provider}/start", get(auth_start))
        .route("/auth/{provider}/callback", get(auth_callback))
        .route(
            "/signup/sns/email",
            get(sns_email_page).post(sns_email_submit),
        )
        .route(
            "/signup/sns/details",
            get(sns_details_page).post(sns_details_submit),
        )
        .route("/signup/sns/confirm", post(sns_confirm_submit))
        .route("/mypage/sns", get(sns_settings_page))
        .route("/mypage/sns/link-pending", post(link_pending_submit))
        .route("/mypage/sns/{provider}/unlink", post(unlink_submit))
}

/// Query parameters of the flow start.
#[derive(Deserialize)]
pub struct StartQuery {
    /// Return path requested by the page, validated before use.
    next: Option<String>,
    /// Link intent marker, set by the SNS settings page.
    link: Option<String>,
}

/// GET /auth/{provider}/start: begins an authorization round
/// trip. Unavailable providers and unknown slugs are a plain
/// 404 - the buttons for them are never rendered either.
async fn auth_start(
    session: Session,
    State(pool): State<PgPool>,
    Path(provider_slug): Path<String>,
    Query(query): Query<StartQuery>,
) -> Response {
    let Some(provider) = SnsProvider::from_slug(&provider_slug) else {
        return not_found_fallback().await;
    };
    if !sns_service::is_available(provider) {
        return not_found_fallback().await;
    }
    let me = current_user(&pool, &session).await;
    // Link intent exists only for a signed-in member who asked
    // for it from the settings page; a guest's link parameter
    // is ignored, never honoured.
    let link_user_id = if query.link.as_deref() == Some("1") {
        me.as_ref().map(|user| user.id)
    } else {
        None
    };
    if me.is_some() && link_user_id.is_none() {
        // Already signed in and not linking: nothing to do.
        return Redirect::to("/mypage").into_response();
    }
    let Some(redirect_uri) = sns_service::redirect_uri(provider) else {
        return not_found_fallback().await;
    };
    let secrets = sns_service::new_flow_secrets();
    let flow = SnsFlow {
        provider: provider.slug().to_string(),
        state: secrets.state.clone(),
        nonce: secrets.nonce.clone(),
        code_verifier: secrets.code_verifier.clone(),
        next: super::login::safe_next(query.next.as_deref()),
        link_user_id,
        started_at: chrono::Utc::now().timestamp(),
    };
    let _ = session.insert(SESSION_SNS_FLOW, flow).await;
    Redirect::to(&sns_service::authorize_url(
        provider,
        &secrets,
        &redirect_uri,
    ))
    .into_response()
}

/// Query parameters of the provider callback.
#[derive(Deserialize)]
pub struct CallbackQuery {
    /// The authorization code, on success.
    code: Option<String>,
    /// The state echoed by the provider.
    state: Option<String>,
    /// The provider's error value, on failure.
    error: Option<String>,
}

/// GET /auth/{provider}/callback: verifies the round trip and
/// branches on what the verified identity means - a known
/// identity signs in, a link intent links, a verified email
/// that is already registered asks for the owner's password
/// sign-in, a new verified email registers, and anything else
/// asks for an email to verify by code.
async fn auth_callback(
    session: Session,
    State(pool): State<PgPool>,
    Path(provider_slug): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let Some(provider) = SnsProvider::from_slug(&provider_slug) else {
        return not_found_fallback().await;
    };
    if query.error.is_some() {
        tracing::warn!(
            provider = provider.slug(),
            stage = "authorize",
            "sns callback returned an error"
        );
        let _ = session.remove::<SnsFlow>(SESSION_SNS_FLOW).await;
        return sns_fail(&session).await;
    }
    let Some(flow) = session
        .get::<SnsFlow>(SESSION_SNS_FLOW)
        .await
        .ok()
        .flatten()
    else {
        return sns_fail(&session).await;
    };
    // Single use: the flow is consumed whatever happens next,
    // so a state can never be replayed.
    let _ = session.remove::<SnsFlow>(SESSION_SNS_FLOW).await;
    let state_ok = query
        .state
        .as_deref()
        .is_some_and(|state| crate::session::constant_time_eq(state, &flow.state));
    let fresh = chrono::Utc::now().timestamp() - flow.started_at <= FLOW_WINDOW_SECS;
    if !state_ok || !fresh || flow.provider != provider.slug() {
        tracing::warn!(
            provider = provider.slug(),
            stage = "state",
            "sns callback state check failed"
        );
        return sns_fail(&session).await;
    }
    let Some(code) = query.code.filter(|code| !code.is_empty()) else {
        return sns_fail(&session).await;
    };
    let Some(redirect_uri) = sns_service::redirect_uri(provider) else {
        return sns_fail(&session).await;
    };
    let claims = match sns_service::exchange_and_verify(
        provider,
        &code,
        &flow.code_verifier,
        &flow.nonce,
        &redirect_uri,
    )
    .await
    {
        Ok(claims) => claims,
        Err(failure) => {
            tracing::warn!(
                provider = provider.slug(),
                stage = %failure,
                "sns sign-in failed"
            );
            return sns_fail(&session).await;
        }
    };

    // Link intent finishes in the session that started it.
    if let Some(link_user_id) = flow.link_user_id {
        return finish_link(&pool, &session, provider, &claims, link_user_id).await;
    }

    // A known identity signs its owner in; a new one
    // branches on the provider-verified email.
    match sns_service::resolve_sign_in(&pool, provider, &claims).await {
        sns_service::SignInResolution::SignedIn(user) => {
            complete_login(
                &pool,
                &session,
                &user,
                flow.next.as_deref().unwrap_or_default(),
            )
            .await
        }
        sns_service::SignInResolution::LinkRequired { email } => {
            // The address already belongs to an account:
            // never merge silently. The link waits for the
            // owner's password sign-in and confirmation.
            let marker = SnsLinkPending {
                provider: provider.slug().to_string(),
                provider_user_id: claims.sub.clone(),
                provider_email: email.clone(),
                parked_at: chrono::Utc::now().timestamp(),
            };
            let _ = session.insert(SESSION_SNS_LINK_PENDING, marker).await;
            render(SnsLinkRequiredView {
                header: header_for(&pool, &session, false).await,
                email,
            })
        }
        sns_service::SignInResolution::RegisterVerified(pending_id) => {
            // A new registration on the provider-verified
            // email: the born-verified pending row carries
            // it through the shared details/confirm steps.
            let _ = session.insert(SESSION_PENDING_ID, pending_id).await;
            park_profile(&session, provider, &claims, true).await;
            Redirect::to("/signup/sns/details").into_response()
        }
        sns_service::SignInResolution::NeedsEmail => {
            // No usable email (LINE without the email
            // permission, Yahoo before its review, an
            // unverified address): ask for one and verify
            // it with this store's own code.
            park_profile(&session, provider, &claims, false).await;
            Redirect::to("/signup/sns/email").into_response()
        }
        sns_service::SignInResolution::Failed => sns_fail(&session).await,
    }
}

/// The link-intent branch: inserts the identity for the
/// member who started the flow, or explains why not. The
/// session is otherwise untouched.
async fn finish_link(
    pool: &PgPool,
    session: &Session,
    provider: SnsProvider,
    claims: &SnsClaims,
    link_user_id: i64,
) -> Response {
    let back = Redirect::to("/mypage/sns").into_response();
    let Some(me) = current_user(pool, session).await else {
        return sns_fail(session).await;
    };
    if me.id != link_user_id {
        return sns_fail(session).await;
    }
    match sns_service::link_identity(pool, me.id, provider, claims).await {
        sns_service::LinkOutcome::AlreadyLinkedProvider => {
            flash(
                session,
                &format!(
                    "すでに{}アカウントが連携されています。",
                    provider.display_name()
                ),
            )
            .await;
            back
        }
        sns_service::LinkOutcome::AccountUsed => {
            flash(session, SNS_ACCOUNT_USED_MESSAGE).await;
            back
        }
        sns_service::LinkOutcome::Linked => {
            flash(
                session,
                &sns_account_linked_message(provider.display_name()),
            )
            .await;
            back
        }
        sns_service::LinkOutcome::Failed => {
            flash(session, SNS_LINK_FAILED_MESSAGE).await;
            back
        }
    }
}

/// Parks the verified provider identity in the session for
/// the registration steps.
async fn park_profile(session: &Session, provider: SnsProvider, claims: &SnsClaims, trusted: bool) {
    let profile = SnsPendingProfile {
        provider: provider.slug().to_string(),
        provider_user_id: claims.sub.clone(),
        provider_email: claims.email.clone(),
        display_name: claims.name.clone(),
        email_trusted: trusted,
    };
    let _ = session.insert(SESSION_SNS_PENDING, profile).await;
}

/// The generic SNS sign-in failure: one flash, the login page.
async fn sns_fail(session: &Session) -> Response {
    flash(session, SNS_FAILURE).await;
    Redirect::to("/login").into_response()
}

/// Sets the session flash message.
async fn flash(session: &Session, message: &str) {
    let _ = session.insert(SESSION_FLASH, message).await;
}

/// Form posted by the SNS email entry step.
#[derive(Deserialize)]
pub struct SnsEmailForm {
    email: String,
}

/// GET /signup/sns/email: asks for an email when the provider
/// reported none (or none it verified).
async fn sns_email_page(session: Session, State(pool): State<PgPool>) -> Response {
    let Some(profile) = super::signup::sns_profile(&session).await else {
        return Redirect::to("/signup").into_response();
    };
    if profile.email_trusted {
        return Redirect::to("/signup/sns/details").into_response();
    }
    let Some(provider) = SnsProvider::from_slug(&profile.provider) else {
        return Redirect::to("/signup").into_response();
    };
    render(SnsEmailView {
        header: header_for(&pool, &session, false).await,
        error: None,
        email: profile.provider_email.clone().unwrap_or_default(),
        provider_name: provider.display_name(),
    })
}

/// POST /signup/sns/email: runs the address through the
/// existing sign-up code machinery (same throttle, same code,
/// same verify page); the SNS profile stays parked for the
/// steps after verification.
async fn sns_email_submit(
    session: Session,
    State(pool): State<PgPool>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    Form(form): Form<SnsEmailForm>,
) -> Response {
    let Some(profile) = super::signup::sns_profile(&session).await else {
        return Redirect::to("/signup").into_response();
    };
    let Some(provider) = SnsProvider::from_slug(&profile.provider) else {
        return Redirect::to("/signup").into_response();
    };
    let back = async |message: String| SnsEmailView {
        header: header_for(&pool, &session, false).await,
        error: Some(message),
        email: form.email.clone(),
        provider_name: provider.display_name(),
    };
    // Sending a code sends a real email: the same throttle as
    // the sign-up start, before any mail is sent.
    let ip = crate::session::client_ip(&addr, &headers);
    if !crate::middleware::auth_throttle::signup_start_allowed(&form.email, &ip) {
        return render(back(TOO_MANY_REQUESTS_MESSAGE.to_string()).await);
    }
    match member_account_service::start_signup(&pool, &form.email).await {
        Ok(pending_id) => {
            let _ = session.insert(SESSION_PENDING_ID, pending_id).await;
            Redirect::to("/signup/verify").into_response()
        }
        Err(e) => render(back(e.user_message()).await),
    }
}

/// GET /signup/sns/details: the SNS variant of the details
/// step - same form and validation minus the password.
async fn sns_details_page(session: Session, State(pool): State<PgPool>) -> Response {
    let (Some(profile), Some(pending)) = (
        super::signup::sns_profile(&session).await,
        load_flow_pending(&session, &pool).await,
    ) else {
        return Redirect::to("/signup").into_response();
    };
    if !pending.verified {
        return Redirect::to("/signup/verify").into_response();
    }
    // An email verified by code that turns out to belong to an
    // existing account becomes a link request, never a merge:
    // the pending row is dropped and the link marker parked.
    if !profile.email_trusted {
        match sns_service::email_has_account(&pool, &pending.email).await {
            Ok(true) => {
                sns_service::discard_pending(&pool, pending.id).await;
                let _ = session.remove::<i64>(SESSION_PENDING_ID).await;
                let _ = session
                    .remove::<SnsPendingProfile>(SESSION_SNS_PENDING)
                    .await;
                let marker = SnsLinkPending {
                    provider: profile.provider.clone(),
                    provider_user_id: profile.provider_user_id.clone(),
                    provider_email: pending.email.clone(),
                    parked_at: chrono::Utc::now().timestamp(),
                };
                let _ = session.insert(SESSION_SNS_LINK_PENDING, marker).await;
                return render(SnsLinkRequiredView {
                    header: header_for(&pool, &session, false).await,
                    email: pending.email.clone(),
                });
            }
            Ok(false) => {}
            Err(_) => {
                // The service logged the failure at its source.
                return Redirect::to("/signup").into_response();
            }
        }
    }
    render(
        details_view(
            &pool,
            &session,
            &pending,
            None,
            DetailsValues::default(),
            DetailsErrors::default(),
        )
        .await,
    )
}

/// Form posted by the SNS details step: the details form
/// without a password.
#[derive(Deserialize)]
pub struct SnsDetailsPost {
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
    /// Mail-magazine checkbox: posts `true` when checked.
    #[serde(default)]
    mail_magazine: String,
    /// Terms checkbox: posts `true` when checked.
    #[serde(default)]
    agree_terms: String,
}

/// POST /signup/sns/details: validates and stores the
/// details, then shows the confirmation page.
async fn sns_details_submit(
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<SnsDetailsPost>,
) -> Response {
    let (Some(_profile), Some(pending)) = (
        super::signup::sns_profile(&session).await,
        load_flow_pending(&session, &pool).await,
    ) else {
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

    let details = DetailsForm {
        password: String::new(),
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

    match member_account_service::save_details_sns(&pool, &pending, &details).await {
        Ok(()) => match member_account_service::load_pending(&pool, pending.id).await {
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
        },
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

/// POST /signup/sns/confirm: creates the passwordless account
/// with its identity linked, logs the new member in with a
/// regenerated session ID, and redirects to the top page -
/// the same landing as the email sign-up.
async fn sns_confirm_submit(session: Session, State(pool): State<PgPool>) -> Response {
    let (Some(profile), Some(pending)) = (
        super::signup::sns_profile(&session).await,
        load_flow_pending(&session, &pool).await,
    ) else {
        return Redirect::to("/signup").into_response();
    };
    let cart_key = cart_service::existing_guest_token(&session).await;
    match member_account_service::complete_signup_sns(
        &pool,
        &pending,
        cart_key.as_deref(),
        &profile.provider,
        &profile.provider_user_id,
        Some(&pending.email),
    )
    .await
    {
        Ok(user) => {
            let _ = session.remove::<i64>(SESSION_PENDING_ID).await;
            let _ = session
                .remove::<SnsPendingProfile>(SESSION_SNS_PENDING)
                .await;
            // A fresh session ID at the moment of login, as at
            // the email sign-up's completion.
            let _ = session.cycle_id().await;
            let _ = session.insert(SESSION_USER_ID, user.id).await;
            flash(&session, SIGNUP_COMPLETED_NOTICE).await;
            Redirect::to("/").into_response()
        }
        Err(e) => render(confirm_view(&pool, &session, &pending, Some(e.user_message())).await),
    }
}

/// GET /mypage/sns: the member's SNS links - one row per
/// linked or available provider - plus the link confirmation
/// banner when a link-pending marker waits for this account.
async fn sns_settings_page(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    let identities = sns_service::identities_for(&pool, user.id).await;
    let mut rows = Vec::new();
    for provider in SnsProvider::ALL {
        let linked = identities
            .iter()
            .find(|row| row.provider == provider.slug());
        match linked {
            Some(identity) => rows.push(SnsRow {
                provider: provider.slug(),
                display_name: provider.display_name(),
                linked: true,
                linked_email: identity.provider_email.clone(),
            }),
            None if sns_service::is_available(provider) => rows.push(SnsRow {
                provider: provider.slug(),
                display_name: provider.display_name(),
                linked: false,
                linked_email: None,
            }),
            None => {}
        }
    }
    let link_pending = link_pending_for(&session, &user.email)
        .await
        .and_then(|marker| {
            SnsProvider::from_slug(&marker.provider).map(|provider| SnsLinkPendingInfo {
                provider_name: provider.display_name(),
            })
        });
    render(SnsSettingsView {
        header: header_for(&pool, &session, true).await,
        rows,
        link_pending,
    })
}

/// The link-pending marker when it is live for this account:
/// unexpired and addressed to the signed-in member's email.
async fn link_pending_for(session: &Session, email: &str) -> Option<SnsLinkPending> {
    let marker = session
        .get::<SnsLinkPending>(SESSION_SNS_LINK_PENDING)
        .await
        .ok()
        .flatten()?;
    if marker.provider_email != email {
        return None;
    }
    if chrono::Utc::now().timestamp() - marker.parked_at > LINK_PENDING_WINDOW_SECS {
        return None;
    }
    Some(marker)
}

/// POST /mypage/sns/link-pending: completes the link the
/// callback parked, for the account owner who has since
/// signed in with a password. The identity must still be
/// unclaimed.
async fn link_pending_submit(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    let back = Redirect::to("/mypage/sns").into_response();
    let Some(marker) = link_pending_for(&session, &user.email).await else {
        flash(&session, "連携の有効期限が切れています。").await;
        return back;
    };
    let Some(provider) = SnsProvider::from_slug(&marker.provider) else {
        return back;
    };
    match sns_service::complete_pending_link(
        &pool,
        user.id,
        provider,
        &marker.provider_user_id,
        &marker.provider_email,
    )
    .await
    {
        sns_service::PendingLinkOutcome::Linked => {
            let _ = session
                .remove::<SnsLinkPending>(SESSION_SNS_LINK_PENDING)
                .await;
            flash(
                &session,
                &sns_account_linked_message(provider.display_name()),
            )
            .await;
        }
        sns_service::PendingLinkOutcome::AlreadyMine => {
            let _ = session
                .remove::<SnsLinkPending>(SESSION_SNS_LINK_PENDING)
                .await;
            flash(&session, "すでに連携されています。").await;
        }
        sns_service::PendingLinkOutcome::UsedByOther => {
            let _ = session
                .remove::<SnsLinkPending>(SESSION_SNS_LINK_PENDING)
                .await;
            flash(&session, SNS_ACCOUNT_USED_MESSAGE).await;
        }
        sns_service::PendingLinkOutcome::Failed => {
            flash(&session, SNS_LINK_FAILED_MESSAGE).await;
        }
    }
    back
}

/// POST /mypage/sns/{provider}/unlink: removes one link. A
/// passwordless account may not remove its only sign-in
/// method - it sets a password first.
async fn unlink_submit(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(provider_slug): Path<String>,
) -> Response {
    let back = Redirect::to("/mypage/sns").into_response();
    let Some(provider) = SnsProvider::from_slug(&provider_slug) else {
        return not_found_fallback().await;
    };
    match sns_service::unlink(&pool, &user, provider).await {
        sns_service::UnlinkOutcome::SoleMethod => {
            flash(&session, "パスワードを設定すると解除できます").await;
        }
        sns_service::UnlinkOutcome::Unlinked => {
            flash(
                &session,
                &format!("{}との連携を解除しました。", provider.display_name()),
            )
            .await;
        }
        sns_service::UnlinkOutcome::Failed => {
            flash(&session, SNS_UNLINK_FAILED_MESSAGE).await;
        }
    }
    back
}
