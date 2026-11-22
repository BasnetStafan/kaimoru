//! Mypage controller.
//! The member-only account page.

use std::net::SocketAddr;

use axum::{
    Form, Router,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{RenderHtml, header_for, render};
use crate::config::CONFIG;
use crate::error::AppError;
use crate::models::CodeChallenge;
use crate::repositories::challenge_repository::ChallengeTable;
use crate::services::member_account_service::{self, DetailsForm};
use crate::services::{
    coupon_service, member_order_cancel_service, order_service, review_service, withdraw_service,
};
use crate::session::{
    CurrentUser, SESSION_CART_NOTICE, SESSION_EMAIL_CHANGE_ID, SESSION_ORDER_NOTICE,
    SESSION_WITHDRAWAL_ID, email_change_id, withdrawal_id,
};
use crate::views::storefront::mask_email;
use crate::views::storefront::mypage::{
    CouponListView, EmailChangeVerifyView, EmailChangeView, MypageView, OrderCancelConfirmView,
    OrderDetailView, OrderHistoryView, OrderMissingView, PasswordChangeView, ProfileEditView,
    WithdrawVerifyView, WithdrawView,
};

use crate::messages::{
    GENERIC_FAILURE_MESSAGE, PASSWORD_CONFIRM_MISMATCH_MESSAGE, PASSWORD_REQUIRED_MESSAGE,
    SERVER_ERROR_MESSAGE, VERIFICATION_CODE_RESENT_NOTICE,
};
use crate::views::storefront::signup::{DetailsErrors, DetailsValues};

/// Routes owned by the mypage controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/mypage", get(mypage))
        .route("/mypage/orders", get(orders_list))
        .route("/mypage/coupons", get(coupons_list))
        .route("/mypage/orders/{id}", get(order_detail))
        .route(
            "/mypage/orders/{id}/cancel",
            get(order_cancel_confirm).post(order_cancel_submit),
        )
        .route("/mypage/orders/{id}/reorder", post(order_reorder))
        .route("/mypage/orders/{id}/receipt", get(order_receipt))
        .route(
            "/mypage/orders/{id}/return-request",
            post(order_return_request),
        )
        .route("/mypage/edit", get(profile_edit).post(profile_edit_submit))
        .route(
            "/mypage/password",
            get(password_change).post(password_change_submit),
        )
        .route("/mypage/email", get(email_change).post(email_change_submit))
        .route(
            "/mypage/email/verify",
            get(email_verify).post(email_verify_submit),
        )
        .route("/mypage/email/resend", post(email_resend_submit))
        .route("/mypage/withdraw", get(withdraw_get).post(withdraw_submit))
        .route(
            "/mypage/withdraw/verify",
            get(withdraw_verify).post(withdraw_verify_submit),
        )
        .route("/mypage/withdraw/resend", post(withdraw_resend_submit))
}

/// Mypage query parameters.
#[derive(Deserialize, Default)]
pub struct MypageQuery {
    /// History page number.
    page: Option<String>,
}

/// GET /mypage: the member hub — the grouped menu, then the
/// recent-orders block. The [`CurrentUser`] extractor has
/// already redirected guests to the login page by the time
/// this handler runs.
async fn mypage(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    // The recent orders are this page's content: a failed load
    // is a 500, never an empty history.
    let recent_orders = match order_service::recent(&pool, user.id, CONFIG.recent_orders_count)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(user_id = user.id, "recent orders failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    // The coupon badge follows the same rule: a failed load is
    // a 500, never a fake "no coupons".
    let coupons = match coupon_service::member_coupons(&pool, user.id).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(user_id = user.id, "member coupons failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let unused_coupons = coupons
        .iter()
        .filter(|coupon| !coupon.is_used() && !coupon.is_unavailable())
        .count() as i64;
    // The points item exists only while the programme is
    // on; a failed balance read hides it rather than
    // showing a wrong figure (the page itself re-reads).
    let points_balance = if crate::services::shop_settings_service::load(&pool)
        .await
        .points_settings()
        .enabled
    {
        crate::services::point_service::balance(&pool, user.id)
            .await
            .ok()
    } else {
        None
    };
    render(MypageView {
        header: header_for(&pool, &session, false).await,
        user,
        recent_orders,
        unused_coupons,
        points_balance,
    })
}

/// GET /mypage/orders: the full order history, paginated —
/// the list the hub's recent-orders block points at.
async fn orders_list(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Query(query): Query<MypageQuery>,
) -> Response {
    // A failed load is a 500, never an empty history.
    let requested = query
        .page
        .as_deref()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(1)
        .max(1);
    let (orders, total) = match history_page(&pool, user.id, requested).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(user_id = user.id, "order history failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let total_pages = crate::util::pagination::total_pages(total, CONFIG.history_per_page).max(1);
    // An out-of-range page clamps to the last page (the
    // storefront convention) instead of rendering an empty
    // history under a real account.
    let (orders, page_no) = if total > 0 && requested > total_pages {
        match history_page(&pool, user.id, total_pages).await {
            Ok((rows, _)) => (rows, total_pages),
            Err(e) => {
                tracing::error!(user_id = user.id, "order history failed: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
                    .into_response();
            }
        }
    } else {
        (orders, requested)
    };
    render(OrderHistoryView {
        header: header_for(&pool, &session, false).await,
        orders,
        page: page_no,
        total_pages,
    })
}

/// GET /mypage/coupons: the member's coupon list — the page
/// the hub's badge-carrying menu row points at.
async fn coupons_list(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    // A failed load is a 500, never a fake "no coupons".
    let coupons = match coupon_service::member_coupons(&pool, user.id).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(user_id = user.id, "member coupons failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    render(CouponListView {
        header: header_for(&pool, &session, false).await,
        coupons,
    })
}

/// Loads one page of the member's order history.
async fn history_page(
    pool: &PgPool,
    user_id: i64,
    page_no: i64,
) -> Result<(Vec<crate::models::RecentOrder>, i64), AppError> {
    order_service::history(
        pool,
        user_id,
        CONFIG.history_per_page,
        page_no
            .saturating_sub(1)
            .saturating_mul(CONFIG.history_per_page),
    )
    .await
}

/// GET /mypage/orders/{id}: one order in full. The ID is parsed
/// by hand so that a non-numeric ID lands on the same 404 as a
/// missing or foreign order; ownership is enforced inside the
/// repository query.
async fn order_detail(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let header = header_for(&pool, &session, false).await;
    // A database failure is a 500; a missing or foreign order
    // stays the indistinguishable styled 404 — deliberate, so the
    // page never confirms another member's order exists.
    let found = match raw_id.parse::<i64>() {
        Ok(id) if id > 0 => order_service::find_order_for_user(&pool, id, user.id).await,
        _ => Ok(None),
    };
    let order = match found {
        Ok(Some(order)) => order,
        Ok(None) => {
            let body = OrderMissingView { header }
                .render_html()
                .unwrap_or_default();
            return (StatusCode::NOT_FOUND, axum::response::Html(body)).into_response();
        }
        Err(e) => {
            tracing::error!(user_id = user.id, "order detail failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    render_detail(pool, header, &session, &user, order, Default::default()).await
}

/// Loads the member's own order for the cancel handlers,
/// answering with the detail page's indistinguishable 404
/// for a missing or foreign ID. A database failure is a
/// 500, never a fake 404.
async fn load_own_order(
    pool: &PgPool,
    session: &Session,
    user_id: i64,
    raw_id: &str,
) -> Result<crate::models::Order, Box<Response>> {
    let found = match raw_id.parse::<i64>() {
        Ok(id) if id > 0 => order_service::find_order_for_user(pool, id, user_id).await,
        _ => Ok(None),
    };
    match found {
        Ok(Some(order)) => Ok(order),
        Ok(None) => {
            let body = OrderMissingView {
                header: header_for(pool, session, false).await,
            }
            .render_html()
            .unwrap_or_default();
            Err(Box::new(
                (StatusCode::NOT_FOUND, axum::response::Html(body)).into_response(),
            ))
        }
        Err(e) => {
            tracing::error!(user_id, "cancel order load failed: {e}");
            Err(Box::new(
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response(),
            ))
        }
    }
}

/// The refusal explanation flashed when a cancellation
/// arrives after the order has moved on.
const CANCEL_REFUSED_NOTICE: &str = "申し訳ありません。この注文はすでに出荷準備・発送に進んでいるため、キャンセルできませんでした。";

/// `GET /mypage/orders/{id}/receipt`: the printable 領収書
/// for the member's own order — a plain receipt with the
/// per-rate tax breakdown, never an 適格請求書 (no
/// registration number exists to print). Ownership is the
/// detail page's own rule, via the shared loader.
async fn order_receipt(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let order = match load_own_order(&pool, &session, user.id, &raw_id).await {
        Ok(order) => order,
        Err(response) => return *response,
    };
    match super::receipt_view(&pool, &order).await {
        Ok(view) => render(view),
        Err(e) => {
            tracing::error!(order_id = order.id, "receipt failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// `POST /mypage/orders/{id}/reorder`: もう一度購入 — adds
/// the order's lines to the member's cart through the human
/// add path and lands on the cart, where a flash reports
/// the outcome and names any line that could not be added
/// (off sale / sold out). A missing or foreign order gets
/// the detail page's styled 404, indistinguishable as ever.
async fn order_reorder(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let header = header_for(&pool, &session, false).await;
    let outcome = match raw_id.parse::<i64>() {
        Ok(id) if id > 0 => {
            let owner = crate::services::cart_service::owner_for(&pool, &session).await;
            order_service::reorder(&pool, &owner, user.id, id).await
        }
        _ => Ok(None),
    };
    let outcome = match outcome {
        Ok(Some(outcome)) => outcome,
        Ok(None) => {
            let body = OrderMissingView { header }
                .render_html()
                .unwrap_or_default();
            return (StatusCode::NOT_FOUND, axum::response::Html(body)).into_response();
        }
        Err(e) => {
            tracing::error!(user_id = user.id, "reorder failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let message = if outcome.skipped.is_empty() {
        format!("注文{}の商品をカートに追加しました。", outcome.order_number)
    } else if outcome.added > 0 {
        format!(
            "注文{}の商品をカートに追加しました。次の商品は販売終了または在庫切れのため追加できませんでした：{}",
            outcome.order_number,
            outcome.skipped.join("、")
        )
    } else {
        format!(
            "注文{}の商品はすべて販売終了または在庫切れのため、カートに追加できませんでした：{}",
            outcome.order_number,
            outcome.skipped.join("、")
        )
    };
    let _ = session.insert(SESSION_CART_NOTICE, message).await;
    axum::response::Redirect::to("/cart").into_response()
}

/// GET /mypage/orders/{id}/cancel: the confirmation page —
/// the contract of Order Cancellation spec section 5.2.
/// An order that has already moved on goes back to its
/// detail page (with the refusal note when it was once
/// cancellable), never to a stale confirmation.
async fn order_cancel_confirm(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let order = match load_own_order(&pool, &session, user.id, &raw_id).await {
        Ok(order) => order,
        Err(response) => return *response,
    };
    if order.status != "received" {
        if order.status != "cancelled" {
            let _ = session
                .insert(SESSION_ORDER_NOTICE, CANCEL_REFUSED_NOTICE)
                .await;
        }
        return axum::response::Redirect::to(&format!("/mypage/orders/{}", order.id))
            .into_response();
    }
    let payment =
        match crate::repositories::payment_repository::find_by_order_id(&pool, order.id).await {
            Ok(row) => row,
            Err(e) => {
                tracing::error!(order_id = order.id, "cancel payment load failed: {e}");
                None
            }
        };
    let raw_lines = match order_service::order_lines_with_images(&pool, order.id).await {
        Ok(lines) => lines,
        Err(e) => {
            tracing::error!(order_id = order.id, "cancel order lines failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let settings = crate::services::shop_settings_service::load(&pool).await;
    let consequence = member_order_cancel_service::consequence_text(
        &order,
        payment.as_ref(),
        Some(settings.contact_email.as_str()),
    );
    render(OrderCancelConfirmView {
        header: header_for(&pool, &session, false).await,
        order,
        lines: raw_lines,
        consequence: consequence.main,
        coupon_line: consequence.coupon_line,
        contact_line: consequence.contact_line,
    })
}

/// POST /mypage/orders/{id}/cancel: runs the member
/// cancellation (the service re-checks eligibility under
/// the order lock) and lands back on the order detail,
/// whose completion block states the outcome.
async fn order_cancel_submit(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let order = match load_own_order(&pool, &session, user.id, &raw_id).await {
        Ok(order) => order,
        Err(response) => return *response,
    };
    match member_order_cancel_service::cancel_order(&pool, user.id, order.id).await {
        Ok(member_order_cancel_service::MemberCancelOutcome::Cancelled(_))
        | Ok(member_order_cancel_service::MemberCancelOutcome::AlreadyCancelled) => {
            axum::response::Redirect::to(&format!("/mypage/orders/{}", order.id)).into_response()
        }
        Ok(member_order_cancel_service::MemberCancelOutcome::Refused) => {
            let _ = session
                .insert(SESSION_ORDER_NOTICE, CANCEL_REFUSED_NOTICE)
                .await;
            axum::response::Redirect::to(&format!("/mypage/orders/{}", order.id)).into_response()
        }
        Ok(member_order_cancel_service::MemberCancelOutcome::NotFound) => {
            let body = OrderMissingView {
                header: header_for(&pool, &session, false).await,
            }
            .render_html()
            .unwrap_or_default();
            (StatusCode::NOT_FOUND, axum::response::Html(body)).into_response()
        }
        Err(e) => {
            tracing::error!(order_id = order.id, "member cancel failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// Form posted by the profile edit page: the sign-up details
/// fields minus email, password and consent.
#[derive(Deserialize)]
pub struct ProfileForm {
    family_name: String,
    given_name: String,
    family_name_kana: String,
    given_name_kana: String,
    postal_code: String,
    prefecture: String,
    city: String,
    address_line: String,
    #[serde(default)]
    building: String,
    phone: String,
    #[serde(default)]
    mail_magazine: Option<String>,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the form, which the
    /// submit treats as stale.
    #[serde(default)]
    form_ticket: String,
}

impl ProfileForm {
    /// The service-layer form (no password is edited here).
    fn to_details(&self) -> DetailsForm {
        DetailsForm {
            password: String::new(),
            family_name: self.family_name.clone(),
            given_name: self.given_name.clone(),
            family_name_kana: self.family_name_kana.clone(),
            given_name_kana: self.given_name_kana.clone(),
            birth_date: String::new(),
            gender: String::new(),
            postal_code: self.postal_code.clone(),
            prefecture: self.prefecture.clone(),
            city: self.city.clone(),
            address_line: self.address_line.clone(),
            building: self.building.clone(),
            phone: self.phone.clone(),
            mail_magazine: self.mail_magazine.is_some(),
        }
    }

    /// The view values for re-rendering after an error.
    fn to_values(&self) -> DetailsValues {
        DetailsValues {
            family_name: self.family_name.clone(),
            given_name: self.given_name.clone(),
            family_name_kana: self.family_name_kana.clone(),
            given_name_kana: self.given_name_kana.clone(),
            birth_date: String::new(),
            gender: String::new(),
            postal_code: self.postal_code.clone(),
            prefecture: self.prefecture.clone(),
            city: self.city.clone(),
            address_line: self.address_line.clone(),
            building: self.building.clone(),
            phone: self.phone.clone(),
            mail_magazine: self.mail_magazine.is_some(),
        }
    }
}

/// The view values prefilled from the stored member.
fn values_of(user: &crate::models::User) -> DetailsValues {
    DetailsValues {
        family_name: user.family_name.clone(),
        given_name: user.given_name.clone(),
        family_name_kana: user.family_name_kana.clone(),
        given_name_kana: user.given_name_kana.clone(),
        birth_date: String::new(),
        gender: String::new(),
        postal_code: user.postal_code.clone(),
        prefecture: user.prefecture.clone(),
        city: user.city.clone(),
        address_line: user.address_line.clone(),
        building: user.building.clone().unwrap_or_default(),
        phone: user.phone.clone(),
        mail_magazine: user.mail_magazine,
    }
}

/// GET /mypage/edit: the profile edit form.
/// The form-ticket scope of the profile edit form: one
/// member, one record — the profile itself.
const PROFILE_TICKET_SCOPE: &str = "profile";

/// The member's current profile stamp: the `updated_at`
/// (epoch microseconds) a form ticket records for the
/// profile edit. A missing row — or a failed read, logged
/// here — reads as 0, a value no real stamp equals, so a
/// ticket minted from it can never validate a save: the
/// guard fails closed rather than letting an unguarded
/// save through.
async fn profile_stamp(pool: &PgPool, user_id: i64) -> i64 {
    match crate::repositories::user_repository::updated_at(pool, user_id).await {
        Ok(stamp) => stamp.map(|t| t.timestamp_micros()).unwrap_or(0),
        Err(e) => {
            tracing::error!(user_id, "profile stamp read failed: {e}");
            0
        }
    }
}

/// Renders the profile edit page as a stale-save refusal:
/// the current values (never the stale form's), the refusal
/// message, and a fresh ticket, so the re-rendered form can
/// be saved from the current record.
async fn render_profile_stale(
    pool: &PgPool,
    session: &Session,
    user: crate::models::User,
    message: String,
) -> Response {
    let fresh = member_account_service::load_user(pool, user.id)
        .await
        .unwrap_or(user);
    let form_ticket = crate::services::form_ticket_service::issue(
        session,
        PROFILE_TICKET_SCOPE,
        profile_stamp(pool, fresh.id).await,
    )
    .await;
    render(ProfileEditView {
        header: header_for(pool, session, false).await,
        email: fresh.email.clone(),
        values: values_of(&fresh),
        errors: DetailsErrors::default(),
        prefectures: &crate::views::storefront::PREFECTURES,
        error: Some(message),
        notice: None,
        form_ticket,
    })
}

async fn profile_edit(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    let form_ticket = crate::services::form_ticket_service::issue(
        &session,
        PROFILE_TICKET_SCOPE,
        profile_stamp(&pool, user.id).await,
    )
    .await;
    render(ProfileEditView {
        header: header_for(&pool, &session, false).await,
        email: user.email.clone(),
        values: values_of(&user),
        errors: DetailsErrors::default(),
        prefectures: &crate::views::storefront::PREFECTURES,
        error: None,
        notice: None,
        form_ticket,
    })
}

/// POST /mypage/edit: validates and saves the profile.
async fn profile_edit_submit(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<ProfileForm>,
) -> Response {
    let header = header_for(&pool, &session, false).await;
    let prefectures: &[&str] = &crate::views::storefront::PREFECTURES;
    // Spend the form ticket before anything else: a submit
    // without a valid ticket was not rendered from the
    // current record (a replayed form, an expired session),
    // so it is refused as stale without touching the row.
    // A ticket whose stamp has gone stale is refused by the
    // service's check instead — both land on the same
    // refusal page with the current values.
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        PROFILE_TICKET_SCOPE,
        &form.form_ticket,
    )
    .await
    else {
        return render_profile_stale(
            &pool,
            &session,
            user,
            crate::messages::STALE_FORM_MESSAGE.to_string(),
        )
        .await;
    };
    match member_account_service::update_profile(&pool, user.id, &form.to_details(), expected).await
    {
        Ok(()) => {
            let fresh = member_account_service::load_user(&pool, user.id)
                .await
                .unwrap_or(user);
            let form_ticket = crate::services::form_ticket_service::issue(
                &session,
                PROFILE_TICKET_SCOPE,
                profile_stamp(&pool, fresh.id).await,
            )
            .await;
            render(ProfileEditView {
                header,
                email: fresh.email.clone(),
                values: values_of(&fresh),
                errors: DetailsErrors::default(),
                prefectures,
                error: None,
                notice: Some("会員情報を更新しました。".to_string()),
                form_ticket,
            })
        }
        Err(AppError::Conflict(message)) => {
            render_profile_stale(&pool, &session, user, message).await
        }
        // Validation problems land under their fields; anything
        // else (above all a database failure) surfaces as the
        // generic page error — it used to be disguised as a
        // family-name field error.
        Err(AppError::Validation(field_errors)) => {
            let form_ticket = crate::services::form_ticket_service::issue(
                &session,
                PROFILE_TICKET_SCOPE,
                profile_stamp(&pool, user.id).await,
            )
            .await;
            render(ProfileEditView {
                header,
                email: user.email.clone(),
                values: form.to_values(),
                errors: DetailsErrors::from_field_errors(&field_errors),
                prefectures,
                error: None,
                notice: None,
                form_ticket,
            })
        }
        Err(e) => {
            let form_ticket = crate::services::form_ticket_service::issue(
                &session,
                PROFILE_TICKET_SCOPE,
                profile_stamp(&pool, user.id).await,
            )
            .await;
            render(ProfileEditView {
                header,
                email: user.email.clone(),
                values: form.to_values(),
                errors: DetailsErrors::default(),
                prefectures,
                error: Some(e.user_message()),
                notice: None,
                form_ticket,
            })
        }
    }
}

/// Form posted by the password change page.
#[derive(Deserialize)]
pub struct PasswordForm {
    #[serde(default)]
    current_password: String,
    new_password: String,
    new_password_confirm: String,
}

/// GET /mypage/password: the password change form.
async fn password_change(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    render(PasswordChangeView {
        header: header_for(&pool, &session, false).await,
        error: None,
        notice: None,
        has_password: user.has_password,
    })
}

/// POST /mypage/password: verifies the current password, then
/// replaces the hash.
async fn password_change_submit(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<PasswordForm>,
) -> Response {
    let header = header_for(&pool, &session, false).await;
    if form.new_password != form.new_password_confirm {
        return render(PasswordChangeView {
            header,
            error: Some(PASSWORD_CONFIRM_MISMATCH_MESSAGE.to_string()),
            notice: None,
            has_password: user.has_password,
        });
    }
    // The current session is passed so the change revokes every
    // other session of this member and keeps this one.
    let keep_session_id = session.id().map(|id| id.to_string());
    // Passwordless (SNS-registered) accounts set their first
    // password here instead: there is no current password to
    // verify, and the form does not render one.
    let result = if user.has_password {
        member_account_service::change_password(
            &pool,
            user.id,
            &form.current_password,
            &form.new_password,
            keep_session_id,
        )
        .await
    } else {
        member_account_service::set_first_password(
            &pool,
            user.id,
            &form.new_password,
            keep_session_id,
        )
        .await
    };
    match result {
        Ok(()) => render(PasswordChangeView {
            header,
            error: None,
            notice: Some(
                if user.has_password {
                    "パスワードを変更しました。"
                } else {
                    "パスワードを設定しました。"
                }
                .to_string(),
            ),
            has_password: true,
        }),
        Err(e) => render(PasswordChangeView {
            header,
            error: Some(e.user_message()),
            notice: None,
            has_password: user.has_password,
        }),
    }
}

/// Form posted by the email change start page.
#[derive(Deserialize)]
pub struct EmailChangeForm {
    current_password: String,
    new_email: String,
}

/// Form posted by the email change code step.
#[derive(Deserialize)]
pub struct EmailCodeForm {
    code: String,
}

/// GET /mypage/email: the email change start form. A separate
/// verified flow, not a profile field: the address is the login
/// ID, so it moves only after the current password is
/// re-confirmed and a code sent to the NEW address verifies.
async fn email_change(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    render(EmailChangeView {
        header: header_for(&pool, &session, false).await,
        current_email: user.email.clone(),
        new_email: String::new(),
        error: None,
    })
}

/// POST /mypage/email: checks the current password and the new
/// address in the service, then sends the code to the new
/// address and moves to the code step.
async fn email_change_submit(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<EmailChangeForm>,
) -> Response {
    let header = header_for(&pool, &session, false).await;
    match member_account_service::start_email_change(
        &pool,
        &user,
        &form.current_password,
        &form.new_email,
    )
    .await
    {
        Ok(change_id) => {
            let _ = session.insert(SESSION_EMAIL_CHANGE_ID, change_id).await;
            axum::response::Redirect::to("/mypage/email/verify").into_response()
        }
        Err(e) => render(EmailChangeView {
            header,
            current_email: user.email.clone(),
            new_email: form.new_email.clone(),
            error: Some(e.user_message()),
        }),
    }
}

/// GET /mypage/email/verify: the code entry page. Without a
/// change in progress there is nothing to verify, so the
/// visitor returns to the start of the flow.
async fn email_verify(
    CurrentUser(_user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    let Some(challenge) = load_flow_change(&session, &pool).await else {
        return axum::response::Redirect::to("/mypage/email").into_response();
    };
    render(EmailChangeVerifyView {
        header: header_for(&pool, &session, false).await,
        error: None,
        notice: None,
        email_masked: mask_email(&challenge.email),
        completed: false,
    })
}

/// POST /mypage/email/verify: checks the code and, on success,
/// completes the change in the same request. The member stays
/// signed in: the session's user ID does not change. The
/// completed state renders on this page, like
/// the password change's success notice.
async fn email_verify_submit(
    CurrentUser(_user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<EmailCodeForm>,
) -> Response {
    let header = header_for(&pool, &session, false).await;
    let Some(challenge) = load_flow_change(&session, &pool).await else {
        return axum::response::Redirect::to("/mypage/email").into_response();
    };
    let back =
        |error: Option<String>, notice: Option<String>, completed: bool| EmailChangeVerifyView {
            header,
            error,
            notice,
            email_masked: mask_email(&challenge.email),
            completed,
        };
    if let Err(e) = member_account_service::verify_challenge(
        &pool,
        ChallengeTable::EmailChanges,
        &challenge,
        &form.code,
    )
    .await
    {
        return render(back(Some(e.user_message()), None, false));
    }
    // Reload so completion reads the verified flag as stored.
    let Ok(stored) =
        member_account_service::load_challenge(&pool, ChallengeTable::EmailChanges, challenge.id)
            .await
    else {
        return render(back(Some(GENERIC_FAILURE_MESSAGE.to_string()), None, false));
    };
    match member_account_service::complete_email_change(&pool, &stored).await {
        Ok(()) => {
            let _ = session.remove::<i64>(SESSION_EMAIL_CHANGE_ID).await;
            render(back(
                None,
                Some("メールアドレスを変更しました。".to_string()),
                true,
            ))
        }
        Err(e) => render(back(Some(e.user_message()), None, false)),
    }
}

/// POST /mypage/email/resend: sends a fresh code to the new
/// address under the cooldown rules.
async fn email_resend_submit(
    CurrentUser(_user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    let header = header_for(&pool, &session, false).await;
    let Some(challenge) = load_flow_change(&session, &pool).await else {
        return axum::response::Redirect::to("/mypage/email").into_response();
    };
    let (error, notice) = match member_account_service::resend_challenge(
        &pool,
        ChallengeTable::EmailChanges,
        &challenge,
    )
    .await
    {
        Ok(()) => (None, Some(VERIFICATION_CODE_RESENT_NOTICE.to_string())),
        Err(e) => (Some(e.user_message()), None),
    };
    render(EmailChangeVerifyView {
        header,
        error,
        notice,
        email_masked: mask_email(&challenge.email),
        completed: false,
    })
}

/// Loads the email change this session is working through.
async fn load_flow_change(session: &Session, pool: &PgPool) -> Option<CodeChallenge> {
    let id = email_change_id(session).await?;
    member_account_service::load_challenge(pool, ChallengeTable::EmailChanges, id)
        .await
        .ok()
}

/// Serialised withdrawal confirmation form (パスワード
/// plus the understanding checkbox).
#[derive(Deserialize)]
pub struct WithdrawForm {
    /// The member's current password, re-entered.
    password: Option<String>,
    /// The understanding checkbox ("true" when ticked).
    confirm: Option<String>,
}

/// Shared renderer for the withdrawal page: the blocker
/// list when in-flight business prevents the ceremony,
/// otherwise the form (with any field errors).
async fn render_withdraw(
    pool: &PgPool,
    session: &Session,
    user_id: i64,
    password_error: Option<String>,
    confirm_error: Option<String>,
) -> Response {
    let blockers = match withdraw_service::blockers(pool, user_id).await {
        Ok(orders) => orders
            .iter()
            .map(|o| (o.id, o.order_number()))
            .collect::<Vec<(i64, String)>>(),
        Err(e) => {
            tracing::error!("withdrawal blockers load failed: {e}");
            Vec::new()
        }
    };
    // The ceremony's proof depends on the account: password
    // accounts re-enter the password; passwordless accounts
    // prove ownership with the email code, whose verified
    // state this page renders.
    let member = member_account_service::load_user(pool, user_id).await;
    let has_password = member.as_ref().is_some_and(|u| u.has_password);
    let code_verified = if has_password {
        false
    } else {
        load_verified_withdrawal_proof(session, pool, user_id)
            .await
            .is_some()
    };
    render(WithdrawView {
        header: header_for(pool, session, true).await,
        has_password,
        code_verified,
        blockers,
        password_error,
        confirm_error,
    })
}

/// GET /mypage/withdraw: the explanation page — or, when
/// in-flight business blocks the withdrawal, the list of
/// blocking orders instead of the form.
async fn withdraw_get(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    render_withdraw(&pool, &session, user.id, None, None).await
}

/// POST /mypage/withdraw: the ceremony.
/// Every field error returns at once; the password is
/// verified in the service; a successful commit revokes
/// every session the member holds and lands on the
/// public farewell page.
async fn withdraw_submit(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<WithdrawForm>,
) -> Response {
    // All field errors at once, house style. A missing
    // password is a field error; a wrong one comes back
    // from the service with the same wording, so the two
    // cannot be told apart from outside.
    let mut password_error = None;
    let mut confirm_error = None;
    if form.confirm.as_deref() != Some("true") {
        confirm_error = Some("退会の確認にチェックを入れてください".to_string());
    }
    // Passwordless accounts run the same ceremony on the
    // email code proof instead of a password.
    if !user.has_password {
        return withdraw_submit_code(user, &session, &pool, confirm_error).await;
    }
    let password_input = form.password.clone().unwrap_or_default();
    if password_input.is_empty() {
        password_error = Some(PASSWORD_REQUIRED_MESSAGE.to_string());
    }
    if password_error.is_some() || confirm_error.is_some() {
        return render_withdraw(&pool, &session, user.id, password_error, confirm_error).await;
    }
    match withdraw_service::withdraw(&pool, user.id, &password_input).await {
        Ok(withdraw_service::WithdrawOutcome::Done) => {
            // The service revoked the member's sessions in
            // the database; this request's own session must
            // also be deleted through its handle, or the
            // session middleware re-saves it at response
            // time and resurrects it (the admin logout
            // precedent). The farewell page is public.
            let _ = session.delete().await;
            axum::response::Redirect::to("/withdrawn").into_response()
        }
        Ok(withdraw_service::WithdrawOutcome::WrongPassword) => {
            render_withdraw(
                &pool,
                &session,
                user.id,
                Some("パスワードが正しくありません".to_string()),
                None,
            )
            .await
        }
        Ok(withdraw_service::WithdrawOutcome::ProofInvalid) => {
            // The password path never produces this; the
            // blocker view is the safe answer regardless.
            render_withdraw(&pool, &session, user.id, None, None).await
        }
        Ok(withdraw_service::WithdrawOutcome::Blocked(_)) => {
            // The state changed between the render and
            // the POST: the blocker view is the answer.
            render_withdraw(&pool, &session, user.id, None, None).await
        }
        Ok(withdraw_service::WithdrawOutcome::AlreadyWithdrawn)
        | Ok(withdraw_service::WithdrawOutcome::NotFound) => {
            // Idempotent landing: the
            // account is gone either way.
            axum::response::Redirect::to("/withdrawn").into_response()
        }
        Err(e) => {
            tracing::error!("withdrawal failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, SERVER_ERROR_MESSAGE).into_response()
        }
    }
}

/// The passwordless withdrawal ceremony (SNS-registered
/// accounts): the first POST - with the confirmation ticked -
/// starts the email code challenge to the account's own
/// address; once the code has verified, the POST runs the
/// withdrawal on that proof.
async fn withdraw_submit_code(
    user: crate::models::User,
    session: &Session,
    pool: &PgPool,
    confirm_error: Option<String>,
) -> Response {
    if confirm_error.is_some() {
        return render_withdraw(pool, session, user.id, None, confirm_error).await;
    }
    let Some(challenge) = load_verified_withdrawal_proof(session, pool, user.id).await else {
        // No verified proof yet: this POST starts the code
        // challenge and moves to the code entry step.
        return match member_account_service::start_withdrawal_code(pool, &user).await {
            Ok(challenge_id) => {
                let _ = session.insert(SESSION_WITHDRAWAL_ID, challenge_id).await;
                axum::response::Redirect::to("/mypage/withdraw/verify").into_response()
            }
            Err(e) => render_withdraw(pool, session, user.id, Some(e.user_message()), None).await,
        };
    };
    match withdraw_service::withdraw_with_code_proof(pool, user.id, &challenge).await {
        Ok(withdraw_service::WithdrawOutcome::Done) => {
            // Same session handling as the password path: the
            // service revoked the stored sessions; this
            // request's own session is deleted through its
            // handle so it cannot be re-saved.
            let _ = session.delete().await;
            axum::response::Redirect::to("/withdrawn").into_response()
        }
        Ok(withdraw_service::WithdrawOutcome::Blocked(_)) => {
            render_withdraw(pool, session, user.id, None, None).await
        }
        Ok(withdraw_service::WithdrawOutcome::ProofInvalid) => {
            let _ = session.remove::<i64>(SESSION_WITHDRAWAL_ID).await;
            render_withdraw(
                pool,
                session,
                user.id,
                Some("認証コードの確認が完了していません。".to_string()),
                None,
            )
            .await
        }
        Ok(withdraw_service::WithdrawOutcome::AlreadyWithdrawn)
        | Ok(withdraw_service::WithdrawOutcome::NotFound)
        | Ok(withdraw_service::WithdrawOutcome::WrongPassword) => {
            // WrongPassword cannot occur on this path (no
            // password is involved); the idempotent landing
            // covers the rest.
            axum::response::Redirect::to("/withdrawn").into_response()
        }
        Err(e) => {
            tracing::error!("withdrawal failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, SERVER_ERROR_MESSAGE).into_response()
        }
    }
}

/// GET /mypage/withdraw/verify: the email code step of the
/// passwordless withdrawal.
async fn withdraw_verify(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    if user.has_password {
        return axum::response::Redirect::to("/mypage/withdraw").into_response();
    }
    let Some(challenge) = load_flow_withdrawal(&session, &pool).await else {
        return axum::response::Redirect::to("/mypage/withdraw").into_response();
    };
    render(WithdrawVerifyView {
        header: header_for(&pool, &session, false).await,
        error: None,
        notice: None,
        email_masked: mask_email(&challenge.email),
    })
}

/// POST /mypage/withdraw/verify: checks the code. Success
/// returns to the ceremony page, which then asks only for
/// the final confirmation.
async fn withdraw_verify_submit(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<EmailCodeForm>,
) -> Response {
    if user.has_password {
        return axum::response::Redirect::to("/mypage/withdraw").into_response();
    }
    let Some(challenge) = load_flow_withdrawal(&session, &pool).await else {
        return axum::response::Redirect::to("/mypage/withdraw").into_response();
    };
    match member_account_service::verify_challenge(
        &pool,
        ChallengeTable::WithdrawalCodes,
        &challenge,
        &form.code,
    )
    .await
    {
        Ok(()) => axum::response::Redirect::to("/mypage/withdraw").into_response(),
        Err(e) => render(WithdrawVerifyView {
            header: header_for(&pool, &session, false).await,
            error: Some(e.user_message()),
            notice: None,
            email_masked: mask_email(&challenge.email),
        }),
    }
}

/// POST /mypage/withdraw/resend: sends a fresh code to the
/// account's own address under the cooldown rules.
async fn withdraw_resend_submit(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    if user.has_password {
        return axum::response::Redirect::to("/mypage/withdraw").into_response();
    }
    let Some(challenge) = load_flow_withdrawal(&session, &pool).await else {
        return axum::response::Redirect::to("/mypage/withdraw").into_response();
    };
    let (error, notice) = match member_account_service::resend_challenge(
        &pool,
        ChallengeTable::WithdrawalCodes,
        &challenge,
    )
    .await
    {
        Ok(()) => (None, Some(VERIFICATION_CODE_RESENT_NOTICE.to_string())),
        Err(e) => (Some(e.user_message()), None),
    };
    render(WithdrawVerifyView {
        header: header_for(&pool, &session, false).await,
        error,
        notice,
        email_masked: mask_email(&challenge.email),
    })
}

/// Loads the withdrawal challenge this session is working
/// through, in any state.
async fn load_flow_withdrawal(session: &Session, pool: &PgPool) -> Option<CodeChallenge> {
    let id = withdrawal_id(session).await?;
    member_account_service::load_challenge(pool, ChallengeTable::WithdrawalCodes, id)
        .await
        .ok()
}

/// Loads the session's withdrawal challenge when it is a
/// verified, unexpired proof for this member: the only state
/// the ceremony may run on.
async fn load_verified_withdrawal_proof(
    session: &Session,
    pool: &PgPool,
    user_id: i64,
) -> Option<CodeChallenge> {
    let challenge = load_flow_withdrawal(session, pool).await?;
    if challenge.user_id == user_id
        && challenge.verified
        && challenge.expires_at > chrono::Utc::now()
    {
        Some(challenge)
    } else {
        None
    }
}

/// The customer-facing split-shipment rows: shipped
/// parcels only, each with its carrier, tracking number
/// and contents. Shared by the member detail and the guest
/// lookup. A failed read degrades to no rows.
pub(crate) async fn public_shipment_rows(
    pool: &PgPool,
    order_id: i64,
) -> Vec<crate::views::storefront::mypage::PublicShipmentRow> {
    use crate::repositories::admin::shipment_repository as ship_repo;
    let rows = ship_repo::list_for_order(pool, order_id)
        .await
        .unwrap_or_default();
    let mut out = Vec::new();
    for row in rows.iter().filter(|r| r.shipped_at.is_some()) {
        let items = ship_repo::items_for_shipment(pool, row.id)
            .await
            .unwrap_or_default();
        let items_label = items
            .iter()
            .map(|item| format!("{} ×{}", item.product_name, item.quantity))
            .collect::<Vec<_>>()
            .join("、");
        out.push(crate::views::storefront::mypage::PublicShipmentRow {
            index: out.len() + 1,
            carrier_label: crate::models::carrier::carrier_for(&row.carrier)
                .map(|c| c.label.to_string())
                .unwrap_or_default(),
            tracking_number: row.tracking_number.clone(),
            items_label,
        });
    }
    out
}

/// Renders the order detail page for an order already
/// proven to be the member's own. Shared by the GET page
/// and the return-request post's error re-render, so both
/// assemble the page identically.
async fn render_detail(
    pool: PgPool,
    header: crate::views::storefront::header::Header,
    session: &Session,
    user: &crate::models::User,
    order: crate::models::Order,
    return_form: crate::views::storefront::mypage::ReturnFormState,
) -> Response {
    let raw_lines = match order_service::order_lines_with_images(&pool, order.id).await {
        Ok(lines) => lines,
        Err(e) => {
            tracing::error!(order_id = order.id, "order lines failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    // Each purchased line carries its review button: the member's
    // own review id per product, in one query for the whole order.
    let product_ids: Vec<i64> = raw_lines.iter().map(|line| line.product_id).collect();
    let own_reviews = review_service::own_review_ids(&pool, user.id, &product_ids).await;
    let lines = raw_lines
        .into_iter()
        .map(|line| crate::views::storefront::mypage::OrderLineRow {
            own_review_id: own_reviews.get(&line.product_id).copied(),
            line,
        })
        .collect();
    // The payment block: the order's payment row rendered
    // through the shared part. A failed read or a pre-payments
    // order renders the COD wording the template falls back to.
    let payment_row =
        match crate::repositories::payment_repository::find_by_order_id(&pool, order.id).await {
            Ok(row) => row,
            Err(e) => {
                tracing::error!(order_id = order.id, "order payment load failed: {e}");
                None
            }
        };
    let payment = payment_row.as_ref().map(|row| {
        crate::views::storefront::payment::PaymentBlockView::from_payment(row, &order.status)
    });
    // The cancellation area (Order Cancellation spec 5.1):
    // members cancel only a `received` order; `preparing`
    // shows the fixed note; a cancelled order shows the
    // completion block with its refund line.
    let cancel = crate::views::storefront::mypage::OrderCancelBlock {
        can_cancel: order.status == "received",
        preparing_note: order.status == "preparing",
        cancelled: (order.status == "cancelled").then(|| {
            crate::views::storefront::mypage::CancelledBlock {
                refund_line: member_order_cancel_service::cancelled_refund_line(
                    payment_row.as_ref(),
                ),
            }
        }),
    };
    // A refused cancellation's explanation arrives as a
    // one-shot flash and is consumed by this render.
    let notice: Option<String> = session.remove(SESSION_ORDER_NOTICE).await.ok().flatten();
    let shipments = public_shipment_rows(&pool, order.id).await;
    // The return workflow's current state, when the
    // shop has opened one for this order. A failed read
    // hides the line rather than failing the page.
    let return_status_label =
        match crate::repositories::admin::return_repository::latest_status_for_order(
            &pool, order.id,
        )
        .await
        {
            Ok(Some(code)) => Some(crate::models::return_request::status_label(&code).to_string()),
            Ok(None) => None,
            Err(e) => {
                tracing::error!(order_id = order.id, "return status load failed: {e}");
                None
            }
        };
    let tax_breakdown =
        match crate::repositories::order_repository::tax_breakdowns(&pool, order.id).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!(order_id = order.id, "order tax breakdown failed: {e}");
                Vec::new()
            }
        };
    render(OrderDetailView {
        header,
        order,
        lines,
        payment,
        cancel,
        notice,
        tax_breakdown,
        return_form,
        return_status_label,
        shipments,
    })
}

/// The 返品・交換の申請 form (member order detail).
#[derive(serde::Deserialize)]
pub struct ReturnRequestForm {
    /// Reason code from the fixed catalogue.
    #[serde(default)]
    reason: String,
    /// Free-text detail.
    #[serde(default)]
    detail: String,
}

/// `POST /mypage/orders/{id}/return-request`: files the
/// member's return/exchange request as an ordinary inquiry
/// (category `returns`, the order number attached — the
/// admin inbox links it back to this order). Intake only:
/// no refund or status changes; the operator answers from
/// the inbox. Ownership is the detail page's own rule.
async fn order_return_request(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(raw_id): Path<String>,
    Form(form): Form<ReturnRequestForm>,
) -> Response {
    let order = match load_own_order(&pool, &session, user.id, &raw_id).await {
        Ok(order) => order,
        Err(response) => return *response,
    };
    let errors =
        crate::services::inquiry_service::validate_return_request(&form.reason, &form.detail);
    if !errors.is_empty() {
        let header = header_for(&pool, &session, false).await;
        let state = crate::views::storefront::mypage::ReturnFormState {
            reason: form.reason.clone(),
            detail: form.detail.clone(),
            errors: crate::views::storefront::mypage::ReturnFormErrors::from_field_errors(&errors),
            form_error: None,
        };
        return render_detail(pool, header, &session, &user, order, state).await;
    }
    let ip = crate::session::client_ip(&addr, &headers);
    let name = format!("{} {}", order.family_name, order.given_name);
    let outcome = crate::services::inquiry_service::submit_return_request(
        &pool,
        &ip,
        &crate::services::inquiry_service::ReturnRequest {
            user_id: user.id,
            name: &name,
            email: &user.email,
            order: &order,
            reason_code: &form.reason,
            detail: &form.detail,
        },
    )
    .await;
    match outcome {
        Ok(id) => {
            let _ = session
                .insert(
                    SESSION_ORDER_NOTICE,
                    format!(
                        "返品・交換の申請を受け付けました。受付番号は {} です。",
                        crate::services::inquiry_service::reference_for(id)
                    ),
                )
                .await;
            Redirect::to(&format!("/mypage/orders/{}", order.id)).into_response()
        }
        Err(crate::services::inquiry_service::SubmitError::RateLimited) => {
            let header = header_for(&pool, &session, false).await;
            let state = crate::views::storefront::mypage::ReturnFormState {
                reason: form.reason.clone(),
                detail: form.detail.clone(),
                errors: Default::default(),
                form_error: Some(crate::services::inquiry_service::RATE_LIMIT_MESSAGE.to_string()),
            };
            render_detail(pool, header, &session, &user, order, state).await
        }
        Err(crate::services::inquiry_service::SubmitError::Db(e)) => {
            tracing::error!(order_id = order.id, "return request failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}
