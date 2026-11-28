//! The member address book (Address Book spec §5.1): list,
//! add, edit, delete and set-default under `/mypage/addresses`.
//!
//! Every handler takes the member from the [`CurrentUser`]
//! extractor (guests are redirected to login first), and every
//! read and write is scoped to that member in the repository —
//! another member's address id answers with the same styled
//! 404 as a nonexistent one.
use askama::Template;
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

use super::{header_for, render};
use crate::config::CONFIG;
use crate::services::member_address_service::{self, AddressError, AddressFormValues};
use crate::session::CurrentUser;
use crate::views::storefront::addresses::{
    AddressBookView, AddressDeleteView, AddressFormErrors, AddressFormView, AddressMissingView,
};
use crate::views::storefront::checkout::PREFECTURES;

/// Routes owned by the address book controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/mypage/addresses", get(list))
        .route("/mypage/addresses/new", get(new_form).post(create))
        .route("/mypage/addresses/{id}/edit", get(edit_form).post(update))
        .route(
            "/mypage/addresses/{id}/delete",
            get(delete_form).post(delete),
        )
        .route("/mypage/addresses/{id}/default", post(set_default))
}

/// The address form body: the checkout delivery fields plus
/// the label and the make-default checkbox (present as a
/// field only when checked, as browsers submit checkboxes).
#[derive(Deserialize)]
pub struct AddressForm {
    /// The member's label for the address.
    #[serde(default)]
    label: String,
    /// Recipient family name.
    #[serde(default)]
    family_name: String,
    /// Recipient given name.
    #[serde(default)]
    given_name: String,
    /// Postal code as typed.
    #[serde(default)]
    postal_code: String,
    /// Prefecture.
    #[serde(default)]
    prefecture: String,
    /// City / ward.
    #[serde(default)]
    city: String,
    /// Street address.
    #[serde(default)]
    address_line: String,
    /// Building / room.
    #[serde(default)]
    building: String,
    /// Phone number as typed.
    #[serde(default)]
    phone: String,
    /// The make-default checkbox's value when checked.
    #[serde(default)]
    make_default: Option<String>,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the form, which the
    /// edit submit treats as stale. The create submit
    /// ignores it: a new address has no record to guard.
    #[serde(default)]
    form_ticket: String,
}

impl AddressForm {
    /// The raw values the service validates.
    fn values(&self) -> AddressFormValues {
        AddressFormValues {
            label: self.label.clone(),
            family_name: self.family_name.clone(),
            given_name: self.given_name.clone(),
            postal_code: self.postal_code.clone(),
            prefecture: self.prefecture.clone(),
            city: self.city.clone(),
            address_line: self.address_line.clone(),
            building: self.building.clone(),
            phone: self.phone.clone(),
        }
    }
}

/// The styled 404 every unknown-or-foreign address id gets.
async fn address_not_found(pool: &PgPool, session: &Session) -> Response {
    let header = header_for(pool, session, false).await;
    let body = AddressMissingView { header }.render().unwrap_or_default();
    (StatusCode::NOT_FOUND, Html(body)).into_response()
}

/// A plain 500 for database failures, logged by the caller.
fn db_error() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
}

/// The form-ticket scope of one address book entry.
fn address_scope(id: i64) -> String {
    format!("address:{id}")
}

/// Mints the edit form's ticket for `address` at its current
/// stamp.
async fn address_ticket(session: &Session, address: &crate::models::MemberAddress) -> String {
    crate::services::form_ticket_service::issue(
        session,
        &address_scope(address.id),
        address.updated_at.timestamp_micros(),
    )
    .await
}

/// Renders the edit form as a stale-save refusal: the
/// current values (never the stale form's), the shared
/// refusal message, and a fresh ticket, so the re-rendered
/// form can be saved from the current record. The row is
/// reloaded first, so the fresh ticket is minted from the
/// record as it stands now — a save that landed during the
/// refused submit must not leave the re-rendered form
/// holding an already-stale ticket.
async fn stale_address_response(
    pool: &PgPool,
    session: &Session,
    user_id: i64,
    id: i64,
) -> Response {
    let Ok(Some(address)) = member_address_service::find_owned(pool, user_id, id).await else {
        return address_not_found(pool, session).await;
    };
    let form_ticket = address_ticket(session, &address).await;
    form_response(
        pool,
        session,
        "お届け先の編集",
        format!("/mypage/addresses/{}/edit", address.id),
        "変更を保存する",
        AddressFormValues::from_address(&address),
        AddressFormErrors::default(),
        Some(crate::messages::STALE_FORM_MESSAGE.to_string()),
        address.is_default,
        address.is_default,
        form_ticket,
    )
    .await
}

/// Builds the form view for either the add or the edit page.
#[allow(clippy::too_many_arguments)]
async fn form_response(
    pool: &PgPool,
    session: &Session,
    heading: &str,
    form_action: String,
    submit_label: &str,
    values: AddressFormValues,
    errors: AddressFormErrors,
    form_error: Option<String>,
    make_default_checked: bool,
    default_locked: bool,
    form_ticket: String,
) -> Response {
    render(AddressFormView {
        header: header_for(pool, session, false).await,
        heading: heading.to_string(),
        form_action,
        submit_label: submit_label.to_string(),
        values,
        errors,
        form_error,
        prefectures: &PREFECTURES,
        make_default_checked,
        default_locked,
        form_ticket,
    })
}

/// GET /mypage/addresses: the member's saved addresses.
async fn list(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    let addresses = match member_address_service::list(&pool, user.id).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(user_id = user.id, "address list failed: {e}");
            return db_error();
        }
    };
    let at_cap = addresses.len() as i64 >= CONFIG.max_addresses.max(0);
    render(AddressBookView {
        header: header_for(&pool, &session, false).await,
        addresses,
        at_cap,
        max_addresses: CONFIG.max_addresses.max(0),
    })
}

/// GET /mypage/addresses/new: the add form. The make-default
/// checkbox starts checked when the book is empty, since the
/// first address becomes the default anyway.
async fn new_form(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    let empty_book = match member_address_service::list(&pool, user.id).await {
        Ok(v) => v.is_empty(),
        Err(e) => {
            tracing::error!(user_id = user.id, "address list failed: {e}");
            return db_error();
        }
    };
    form_response(
        &pool,
        &session,
        "新しいお届け先",
        "/mypage/addresses/new".to_string(),
        "保存する",
        AddressFormValues::default(),
        AddressFormErrors::default(),
        None,
        empty_book,
        false,
        String::new(),
    )
    .await
}

/// POST /mypage/addresses/new: validate with the checkout's
/// rules, then save under the cap.
async fn create(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<AddressForm>,
) -> Response {
    let values = form.values();
    let input = match member_address_service::validate_address(&values) {
        Ok(input) => input,
        Err(errors) => {
            return form_response(
                &pool,
                &session,
                "新しいお届け先",
                "/mypage/addresses/new".to_string(),
                "保存する",
                values,
                AddressFormErrors::from_field_errors(&errors),
                None,
                form.make_default.is_some(),
                false,
                String::new(),
            )
            .await;
        }
    };
    match member_address_service::create(&pool, user.id, &input, form.make_default.is_some()).await
    {
        Ok(_) => Redirect::to("/mypage/addresses").into_response(),
        Err(AddressError::CapReached) => {
            form_response(
                &pool,
                &session,
                "新しいお届け先",
                "/mypage/addresses/new".to_string(),
                "保存する",
                values,
                AddressFormErrors::default(),
                Some(format!(
                    "アドレス帳の上限（{}件）に達しています。",
                    CONFIG.max_addresses.max(0)
                )),
                form.make_default.is_some(),
                false,
                String::new(),
            )
            .await
        }
        Err(e) => {
            tracing::error!(user_id = user.id, "address create failed: {e:?}");
            db_error()
        }
    }
}

/// GET /mypage/addresses/{id}/edit: the prefilled edit form.
async fn edit_form(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return address_not_found(&pool, &session).await;
    };
    let address = match member_address_service::find_owned(&pool, user.id, id).await {
        Ok(Some(a)) => a,
        Ok(None) => return address_not_found(&pool, &session).await,
        Err(e) => {
            tracing::error!(user_id = user.id, "address load failed: {e}");
            return db_error();
        }
    };
    let is_default = address.is_default;
    let form_ticket = address_ticket(&session, &address).await;
    form_response(
        &pool,
        &session,
        "お届け先の編集",
        format!("/mypage/addresses/{id}/edit"),
        "変更を保存する",
        AddressFormValues::from_address(&address),
        AddressFormErrors::default(),
        None,
        is_default,
        is_default,
        form_ticket,
    )
    .await
}

/// POST /mypage/addresses/{id}/edit: validate, then replace
/// the fields. The default flag can be gained here, never
/// silently lost (see the service).
async fn update(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
    Form(form): Form<AddressForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return address_not_found(&pool, &session).await;
    };
    let existing = match member_address_service::find_owned(&pool, user.id, id).await {
        Ok(Some(a)) => a,
        Ok(None) => return address_not_found(&pool, &session).await,
        Err(e) => {
            tracing::error!(user_id = user.id, "address load failed: {e}");
            return db_error();
        }
    };
    // Spend the form ticket before validating: a submit
    // without a valid ticket was not rendered from the
    // current record (a replayed form, an expired session),
    // so it is refused as stale without touching the row. A
    // ticket whose stamp has gone stale is refused by the
    // service's check instead — both land on the same
    // refusal render with the current values.
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &address_scope(id),
        &form.form_ticket,
    )
    .await
    else {
        return stale_address_response(&pool, &session, user.id, id).await;
    };
    let values = form.values();
    let input = match member_address_service::validate_address(&values) {
        Ok(input) => input,
        Err(errors) => {
            let form_ticket = address_ticket(&session, &existing).await;
            return form_response(
                &pool,
                &session,
                "お届け先の編集",
                format!("/mypage/addresses/{id}/edit"),
                "変更を保存する",
                values,
                AddressFormErrors::from_field_errors(&errors),
                None,
                form.make_default.is_some() || existing.is_default,
                existing.is_default,
                form_ticket,
            )
            .await;
        }
    };
    match member_address_service::update(
        &pool,
        user.id,
        id,
        &input,
        form.make_default.is_some(),
        expected,
    )
    .await
    {
        Ok(()) => Redirect::to("/mypage/addresses").into_response(),
        Err(AddressError::Stale) => stale_address_response(&pool, &session, user.id, id).await,
        Err(AddressError::NotFound) => address_not_found(&pool, &session).await,
        Err(e) => {
            tracing::error!(user_id = user.id, "address update failed: {e:?}");
            db_error()
        }
    }
}

/// GET /mypage/addresses/{id}/delete: the confirmation page.
async fn delete_form(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return address_not_found(&pool, &session).await;
    };
    match member_address_service::find_owned(&pool, user.id, id).await {
        Ok(Some(address)) => render(AddressDeleteView {
            header: header_for(&pool, &session, false).await,
            address,
        }),
        Ok(None) => address_not_found(&pool, &session).await,
        Err(e) => {
            tracing::error!(user_id = user.id, "address load failed: {e}");
            db_error()
        }
    }
}

/// POST /mypage/addresses/{id}/delete: delete, promoting the
/// oldest remaining address when the default goes (service).
async fn delete(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return address_not_found(&pool, &session).await;
    };
    match member_address_service::delete(&pool, user.id, id).await {
        Ok(()) => Redirect::to("/mypage/addresses").into_response(),
        Err(AddressError::NotFound) => address_not_found(&pool, &session).await,
        Err(e) => {
            tracing::error!(user_id = user.id, "address delete failed: {e:?}");
            db_error()
        }
    }
}

/// POST /mypage/addresses/{id}/default: switch the default.
async fn set_default(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return address_not_found(&pool, &session).await;
    };
    match member_address_service::set_default(&pool, user.id, id).await {
        Ok(()) => Redirect::to("/mypage/addresses").into_response(),
        Err(AddressError::NotFound) => address_not_found(&pool, &session).await,
        Err(e) => {
            tracing::error!(user_id = user.id, "address set-default failed: {e:?}");
            db_error()
        }
    }
}
