//! The member address book's rules (Address Book spec §5):
//! validation shared with the checkout, the per-member cap,
//! and the one-default invariant kept inside transactions.
//!
//! Address contents are personal data: log lines carry the
//! action, the address id and the member id only — never a
//! name, postal code, street address or phone number.

use sqlx::PgPool;

use crate::config::CONFIG;
use crate::messages::{PREFECTURE_REQUIRED_MESSAGE, label_max_length_message};

use crate::models::{AddressInput, LABEL_MAX_CHARS, MemberAddress};
use crate::repositories::member_address_repository;
use crate::services::order_service::{self, CheckoutFlow};
use crate::validation::FieldError;
use crate::validation::{phone_digits, postal_code_digits};

/// Why an address operation was refused before any write.
#[derive(Debug)]
pub enum AddressError {
    /// The id is unknown or another member's — the two are
    /// indistinguishable, exactly as the IDOR rule requires.
    NotFound,
    /// The book already holds the maximum addresses.
    CapReached,
    /// The address was updated after the edit form was
    /// rendered (the form-ticket guard): the save is refused
    /// so a stale form cannot silently revert newer values.
    Stale,
    /// A database failure.
    Db(sqlx::Error),
}

impl From<sqlx::Error> for AddressError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

/// The raw fields of the address form (and of the checkout
/// flow, when an address is saved from a placed order): the
/// service validates and normalises them into an
/// [`AddressInput`].
#[derive(Debug, Clone, Default)]
pub struct AddressFormValues {
    /// Label as typed (may be empty).
    pub label: String,
    /// Family name as typed.
    pub family_name: String,
    /// Given name as typed.
    pub given_name: String,
    /// Postal code as typed (hyphens allowed).
    pub postal_code: String,
    /// Prefecture as posted.
    pub prefecture: String,
    /// City as typed.
    pub city: String,
    /// Street address as typed.
    pub address_line: String,
    /// Building as typed (may be empty).
    pub building: String,
    /// Phone as typed (hyphens allowed).
    pub phone: String,
}

impl AddressFormValues {
    /// The values of an existing address, for the edit form.
    pub fn from_address(address: &MemberAddress) -> Self {
        Self {
            label: address.label.clone(),
            family_name: address.family_name.clone(),
            given_name: address.given_name.clone(),
            postal_code: address.postal_code.clone(),
            prefecture: address.prefecture.clone(),
            city: address.city.clone(),
            address_line: address.address_line.clone(),
            building: address.building.clone().unwrap_or_default(),
            phone: address.phone.clone(),
        }
    }

    /// The values of a placed order's delivery fields, for the
    /// checkout save path.
    pub fn from_flow(flow: &CheckoutFlow) -> Self {
        Self {
            label: String::new(),
            family_name: flow.family_name.clone(),
            given_name: flow.given_name.clone(),
            postal_code: flow.postal_code.clone(),
            prefecture: flow.prefecture.clone(),
            city: flow.city.clone(),
            address_line: flow.address_line.clone(),
            building: flow.building.clone(),
            phone: flow.phone.clone(),
        }
    }
}

/// Validates raw address fields with the checkout's own rules
/// — the same `validate_delivery` the delivery step runs, on a
/// flow built from these fields — plus the two rules that are
/// the book's own: the label's length cap and the prefecture
/// being one of the 47 the form offers. On success returns the
/// normalised input (digit-only postal code and phone).
pub fn validate_address(values: &AddressFormValues) -> Result<AddressInput, Vec<FieldError>> {
    let flow = CheckoutFlow {
        family_name: values.family_name.clone(),
        given_name: values.given_name.clone(),
        postal_code: values.postal_code.clone(),
        prefecture: values.prefecture.clone(),
        city: values.city.clone(),
        address_line: values.address_line.clone(),
        building: values.building.clone(),
        phone: values.phone.clone(),
        ..CheckoutFlow::default()
    };
    // The flow above carries no delivery request (the address
    // book stores none), so the delivery-window half of
    // validate_delivery can never fire here: the fallback
    // settings are exact for this call, not an approximation.
    let mut errors = order_service::validate_delivery(
        &flow,
        true,
        &crate::models::ShopSettings::fallback(),
        &[],
    );
    // The checkout constrains the prefecture through its
    // <select>; the book's form does too, but a forged post
    // must not store a value the picker would render as-is.
    if !values.prefecture.trim().is_empty()
        && !crate::views::storefront::checkout::PREFECTURES.contains(&values.prefecture.trim())
    {
        errors.push(FieldError {
            field: "prefecture",
            message: PREFECTURE_REQUIRED_MESSAGE.to_string(),
        });
    }
    if values.label.chars().count() > LABEL_MAX_CHARS {
        errors.push(FieldError {
            field: "label",
            message: label_max_length_message(LABEL_MAX_CHARS),
        });
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(AddressInput {
        label: values.label.trim().to_string(),
        family_name: values.family_name.trim().to_string(),
        given_name: values.given_name.trim().to_string(),
        postal_code: postal_code_digits(&values.postal_code).unwrap_or_default(),
        prefecture: values.prefecture.trim().to_string(),
        city: values.city.trim().to_string(),
        address_line: values.address_line.trim().to_string(),
        building: {
            let building = values.building.trim();
            if building.is_empty() {
                None
            } else {
                Some(building.to_string())
            }
        },
        phone: phone_digits(&values.phone).unwrap_or_default(),
    })
}

/// All of a member's addresses, in display order.
pub async fn list(pool: &PgPool, user_id: i64) -> Result<Vec<MemberAddress>, sqlx::Error> {
    member_address_repository::list_for_user(pool, user_id).await
}

/// One of the member's own addresses (the checkout picker's
/// ownership-checked load is this same scoped read).
pub async fn find_owned(
    pool: &PgPool,
    user_id: i64,
    id: i64,
) -> Result<Option<MemberAddress>, sqlx::Error> {
    member_address_repository::find_owned(pool, user_id, id).await
}

/// Saves a new address. The first address a member saves
/// becomes the default automatically; `make_default` asks for
/// the same on later saves. The cap is checked inside the
/// transaction, so two racing creates cannot both pass it.
pub async fn create(
    pool: &PgPool,
    user_id: i64,
    input: &AddressInput,
    make_default: bool,
) -> Result<i64, AddressError> {
    let mut tx = pool.begin().await?;
    let count = member_address_repository::count_for_user(&mut tx, user_id).await?;
    if count >= CONFIG.max_addresses.max(0) {
        return Err(AddressError::CapReached);
    }
    let is_default = make_default || count == 0;
    if is_default {
        member_address_repository::clear_default(&mut tx, user_id).await?;
    }
    let id = member_address_repository::insert(&mut tx, user_id, input, is_default).await?;
    tx.commit().await?;
    tracing::info!(
        user_id,
        address_id = id,
        action = "address.create",
        "address saved"
    );
    Ok(id)
}

/// Replaces an address's fields. The default flag is only ever
/// gained here (when `make_default` is set); losing it happens
/// by setting another address as default, or by deletion —
/// never as a silent side effect of an edit.
pub async fn update(
    pool: &PgPool,
    user_id: i64,
    id: i64,
    input: &AddressInput,
    make_default: bool,
    expected: i64,
) -> Result<(), AddressError> {
    let mut tx = pool.begin().await?;
    // The form-ticket staleness guard: `expected` is the
    // address's `updated_at` (epoch microseconds) recorded
    // server-side when the edit form was rendered. The row
    // is locked for the comparison, so no concurrent save
    // can slip between the check and the write. A missing
    // row falls through: the write below reports NotFound
    // exactly as before.
    let current = member_address_repository::updated_at_for_update(&mut tx, user_id, id).await?;
    if let Some(current) = current
        && current.timestamp_micros() != expected
    {
        return Err(AddressError::Stale);
    }
    let updated = member_address_repository::update_fields(&mut tx, user_id, id, input).await?;
    if !updated {
        return Err(AddressError::NotFound);
    }
    if make_default {
        member_address_repository::clear_default(&mut tx, user_id).await?;
        member_address_repository::mark_default(&mut tx, user_id, id).await?;
    }
    tx.commit().await?;
    tracing::info!(
        user_id,
        address_id = id,
        action = "address.update",
        "address updated"
    );
    Ok(())
}

/// Deletes an address. Deleting the default promotes the
/// oldest remaining address in the same transaction, so a non-empty book always has exactly one default.
pub async fn delete(pool: &PgPool, user_id: i64, id: i64) -> Result<(), AddressError> {
    let existing = member_address_repository::find_owned(pool, user_id, id)
        .await?
        .ok_or(AddressError::NotFound)?;
    let was_default = existing.is_default;
    let mut tx = pool.begin().await?;
    member_address_repository::delete_owned(&mut tx, user_id, id).await?;
    if was_default
        && let Some(next) = member_address_repository::oldest_id(&mut tx, user_id).await?
    {
        member_address_repository::mark_default(&mut tx, user_id, next).await?;
    }
    tx.commit().await?;
    tracing::info!(
        user_id,
        address_id = id,
        action = "address.delete",
        "address deleted"
    );
    Ok(())
}

/// Makes one of the member's addresses the default, atomically.
pub async fn set_default(pool: &PgPool, user_id: i64, id: i64) -> Result<(), AddressError> {
    let mut tx = pool.begin().await?;
    member_address_repository::clear_default(&mut tx, user_id).await?;
    // The ownership guard rides the mutation itself: the
    // mark only lands on a row this member owns, so a
    // missing or foreign id marks nothing — and the
    // refusal rolls the clear back with it, leaving the
    // old default standing (the update flow's shape).
    let marked = member_address_repository::mark_default(&mut tx, user_id, id).await?;
    if marked == 0 {
        return Err(AddressError::NotFound);
    }
    tx.commit().await?;
    tracing::info!(
        user_id,
        address_id = id,
        action = "address.set_default",
        "default address changed"
    );
    Ok(())
}

/// What became of a checkout save attempt: the
/// order has already placed either way; only the at-cap case
/// is surfaced to the customer, on the completion page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutSaveOutcome {
    /// The address is in the book now.
    Saved,
    /// The book was full; the save was skipped.
    AtCap,
    /// The save failed (logged); checkout is unaffected.
    Failed,
}

/// Saves a placed order's delivery address to the member's
/// book, when the member asked for it at checkout. Failures
/// never reach the customer as errors — the order is placed;
/// the worst case is the address is not in the book.
pub async fn save_from_checkout(
    pool: &PgPool,
    user_id: i64,
    flow: &CheckoutFlow,
) -> CheckoutSaveOutcome {
    let values = AddressFormValues::from_flow(flow);
    let input = match validate_address(&values) {
        Ok(input) => input,
        Err(_) => return CheckoutSaveOutcome::Failed,
    };
    match create(pool, user_id, &input, false).await {
        Ok(_) => CheckoutSaveOutcome::Saved,
        Err(AddressError::CapReached) => CheckoutSaveOutcome::AtCap,
        Err(e) => {
            tracing::error!(user_id, error = ?e, "checkout address save failed");
            CheckoutSaveOutcome::Failed
        }
    }
}
