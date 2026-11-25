//! Persistence for the member address book.
//!
//! Every statement is scoped by `user_id`: ownership is the
//! WHERE clause itself, the same pattern as the mypage order
//! queries, so another member's address id reads as no row.
//! Writes that must keep the one-default invariant atomic take
//! a transaction from the service.

use sqlx::PgPool;

use crate::models::{AddressInput, MemberAddress};
use crate::pii::{decrypt_member_address, encrypt_field, encrypt_field_opt};

/// The column list every address read shares.
const ADDRESS_COLUMNS: &str = "id, user_id, label, family_name, given_name, \
    postal_code, prefecture, city, address_line, building, phone, \
    is_default, created_at, updated_at";

/// All of a member's addresses: the default first, then oldest
/// first (the order the picker and the list show).
pub async fn list_for_user(pool: &PgPool, user_id: i64) -> Result<Vec<MemberAddress>, sqlx::Error> {
    let mut addresses = sqlx::query_as::<_, MemberAddress>(&format!(
        "SELECT {ADDRESS_COLUMNS} FROM member_addresses \
         WHERE user_id = $1 ORDER BY is_default DESC, created_at, id"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    for address in &mut addresses {
        decrypt_member_address(address);
    }
    Ok(addresses)
}

/// One of the member's own addresses by id; `None` when the id
/// is unknown OR belongs to another member — the two are
/// indistinguishable by construction.
pub async fn find_owned(
    pool: &PgPool,
    user_id: i64,
    id: i64,
) -> Result<Option<MemberAddress>, sqlx::Error> {
    let mut address = sqlx::query_as::<_, MemberAddress>(&format!(
        "SELECT {ADDRESS_COLUMNS} FROM member_addresses \
         WHERE user_id = $1 AND id = $2"
    ))
    .bind(user_id)
    .bind(id)
    .fetch_optional(pool)
    .await?;
    if let Some(address) = &mut address {
        decrypt_member_address(address);
    }
    Ok(address)
}

/// The member's address count, read inside the create
/// transaction so the cap check and the insert are one atomic
/// decision.
pub async fn count_for_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM member_addresses WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await
}

/// Inserts an address; returns its id.
pub async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    input: &AddressInput,
    is_default: bool,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO member_addresses \
         (user_id, label, family_name, given_name, postal_code, prefecture, \
          city, address_line, building, phone, is_default) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING id",
    )
    .bind(user_id)
    .bind(&input.label)
    .bind(encrypt_field(&input.family_name))
    .bind(encrypt_field(&input.given_name))
    .bind(encrypt_field(&input.postal_code))
    .bind(encrypt_field(&input.prefecture))
    .bind(encrypt_field(&input.city))
    .bind(encrypt_field(&input.address_line))
    .bind(encrypt_field_opt(input.building.as_deref()))
    .bind(encrypt_field(&input.phone))
    .bind(is_default)
    .fetch_one(&mut **tx)
    .await
}

/// Returns the address's current `updated_at` with the row
/// locked, owner-scoped: the address save's staleness guard
/// compares its form ticket's stamp against this value
/// inside the update transaction (the product guard's
/// shape). A foreign or missing id reads as `None`, exactly
/// as the write itself would find no row.
pub async fn updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, sqlx::Error> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM member_addresses WHERE user_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(user_id)
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

/// Replaces an address's fields (the default flag is managed
/// separately, so an edit can never drop it by accident).
/// Returns whether a row was updated.
pub async fn update_fields(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    id: i64,
    input: &AddressInput,
) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(
        "UPDATE member_addresses SET label = $3, family_name = $4, \
         given_name = $5, postal_code = $6, prefecture = $7, city = $8, \
         address_line = $9, building = $10, phone = $11, updated_at = now() \
         WHERE user_id = $1 AND id = $2",
    )
    .bind(user_id)
    .bind(id)
    .bind(&input.label)
    .bind(encrypt_field(&input.family_name))
    .bind(encrypt_field(&input.given_name))
    .bind(encrypt_field(&input.postal_code))
    .bind(encrypt_field(&input.prefecture))
    .bind(encrypt_field(&input.city))
    .bind(encrypt_field(&input.address_line))
    .bind(encrypt_field_opt(input.building.as_deref()))
    .bind(encrypt_field(&input.phone))
    .execute(&mut **tx)
    .await?;
    Ok(rows.rows_affected() > 0)
}

/// Deletes one of the member's addresses. Returns whether a
/// row was deleted.
pub async fn delete_owned(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    id: i64,
) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query("DELETE FROM member_addresses WHERE user_id = $1 AND id = $2")
        .bind(user_id)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(rows.rows_affected() > 0)
}

/// Unsets the member's default flag wherever it currently is.
pub async fn clear_default(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE member_addresses SET is_default = false \
         WHERE user_id = $1 AND is_default",
    )
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Marks one of the member's addresses as the default and
/// returns how many rows were marked: zero means the id is
/// missing or belongs to another member, which the caller
/// turns into its NotFound refusal.
pub async fn mark_default(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    id: i64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE member_addresses SET is_default = true, updated_at = now() \
         WHERE user_id = $1 AND id = $2",
    )
    .bind(user_id)
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

/// The member's oldest remaining address id — the promotion
/// target when the default is deleted.
pub async fn oldest_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id FROM member_addresses WHERE user_id = $1 \
         ORDER BY created_at, id LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
}

/// Deletes every saved address of one member (account
/// withdrawal): saved addresses are personal data with
/// no remaining business purpose (Account Deletion
/// spec 3.4's principle).
pub async fn delete_all_for_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM member_addresses WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map(|_| ())
}
