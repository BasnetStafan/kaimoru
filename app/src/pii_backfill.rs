//! PII encryption backfill: the one-off rollout job that
//! encrypts plaintext member PII rows in place. It runs
//! from pool setup at startup and is a no-op once every
//! row carries ciphertext.

use crate::pii::{
    blind_index_email, decrypt_with, encrypt_field, encrypt_field_opt, encrypt_with, master_key,
};
use sqlx::PgPool;

/// Encrypts, in place, every pre-encryption row of the four
/// personal-data tables, and proves the configured key can
/// read what earlier boots wrote. Idempotent: rows already
/// carrying the version marker are skipped by the sentinel
/// predicates, so every boot after the first is a no-op.
/// Only counts are logged — never values.
///
/// # Errors
///
/// Returns [`sqlx::Error`] when the database fails or the
/// configured key cannot decrypt an existing value: the boot
/// must fail rather than serve a store it cannot read.
pub async fn backfill(pool: &PgPool) -> Result<(), sqlx::Error> {
    // Round-trip under the configured key, then prove the key
    // against one value an earlier boot actually stored.
    let probe = encrypt_with(master_key(), "pii-self-test");
    decrypt_with(master_key(), &probe).map_err(|_| key_mismatch_error())?;
    let existing: Option<String> =
        sqlx::query_scalar("SELECT email FROM users WHERE email LIKE 'v1:%' LIMIT 1")
            .fetch_optional(pool)
            .await?
            .flatten();
    if let Some(stored) = existing {
        decrypt_with(master_key(), &stored).map_err(|_| key_mismatch_error())?;
    }

    let users = backfill_users(pool).await?;
    let addresses = backfill_member_addresses(pool).await?;
    let orders = backfill_orders(pool).await?;
    let pending = backfill_pending(pool).await?;
    if users + addresses + orders + pending > 0 {
        tracing::info!(
            "pii backfill encrypted: users={users} addresses={addresses} orders={orders} pending={pending}"
        );
    }
    Ok(())
}

/// The boot-failing error for a key that cannot read the
/// store's existing ciphertext.
fn key_mismatch_error() -> sqlx::Error {
    sqlx::Error::Configuration(
        "PII_ENCRYPTION_KEY does not decrypt the stored personal data".into(),
    )
}

/// Encrypts the remaining plaintext member rows.
async fn backfill_users(pool: &PgPool) -> Result<usize, sqlx::Error> {
    type Row = (
        i64,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        String,
    );
    let rows = sqlx::query_as::<_, Row>(
        "SELECT id, email, family_name, given_name, family_name_kana, \
         given_name_kana, postal_code, prefecture, city, address_line, \
         building, phone FROM users WHERE email NOT LIKE 'v1:%'",
    )
    .fetch_all(pool)
    .await?;
    let mut tx = pool.begin().await?;
    for (
        id,
        email,
        family,
        given,
        family_kana,
        given_kana,
        postal,
        prefecture,
        city,
        line,
        building,
        phone,
    ) in &rows
    {
        sqlx::query(
            "UPDATE users SET email = $2, email_bidx = $3, family_name = $4, \
             given_name = $5, family_name_kana = $6, given_name_kana = $7, \
             postal_code = $8, prefecture = $9, city = $10, address_line = $11, \
             building = $12, phone = $13 WHERE id = $1",
        )
        .bind(id)
        .bind(encrypt_field(email))
        .bind(blind_index_email(email))
        .bind(encrypt_field(family))
        .bind(encrypt_field(given))
        .bind(encrypt_field(family_kana))
        .bind(encrypt_field(given_kana))
        .bind(encrypt_field(postal))
        .bind(encrypt_field(prefecture))
        .bind(encrypt_field(city))
        .bind(encrypt_field(line))
        .bind(encrypt_field_opt(building.as_deref()))
        .bind(encrypt_field(phone))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}

/// Encrypts the remaining plaintext address-book rows.
async fn backfill_member_addresses(pool: &PgPool) -> Result<usize, sqlx::Error> {
    type Row = (
        i64,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        String,
    );
    let rows = sqlx::query_as::<_, Row>(
        "SELECT id, family_name, given_name, postal_code, prefecture, \
         city, address_line, building, phone FROM member_addresses \
         WHERE family_name NOT LIKE 'v1:%' AND family_name <> ''",
    )
    .fetch_all(pool)
    .await?;
    let mut tx = pool.begin().await?;
    for (id, family, given, postal, prefecture, city, line, building, phone) in &rows {
        sqlx::query(
            "UPDATE member_addresses SET family_name = $2, given_name = $3, \
             postal_code = $4, prefecture = $5, city = $6, address_line = $7, \
             building = $8, phone = $9 WHERE id = $1",
        )
        .bind(id)
        .bind(encrypt_field(family))
        .bind(encrypt_field(given))
        .bind(encrypt_field(postal))
        .bind(encrypt_field(prefecture))
        .bind(encrypt_field(city))
        .bind(encrypt_field(line))
        .bind(encrypt_field_opt(building.as_deref()))
        .bind(encrypt_field(phone))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}

/// Encrypts the remaining plaintext order snapshots.
async fn backfill_orders(pool: &PgPool) -> Result<usize, sqlx::Error> {
    type Row = (
        i64,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
    );
    // `noshi_name` / `gift_message` ride along: the read path
    // treats them as encrypted fields, so leaving them out
    // would keep pre-encryption plaintext in place forever,
    // masked by the legacy passthrough.
    let rows = sqlx::query_as::<_, Row>(
        "SELECT id, email, family_name, given_name, postal_code, \
         prefecture, city, address_line, building, phone, \
         noshi_name, gift_message FROM orders \
         WHERE email NOT LIKE 'v1:%' AND email <> ''",
    )
    .fetch_all(pool)
    .await?;
    let mut tx = pool.begin().await?;
    for (id, email, family, given, postal, prefecture, city, line, building, phone, noshi, gift) in
        &rows
    {
        sqlx::query(
            "UPDATE orders SET email = $2, family_name = $3, given_name = $4, \
             postal_code = $5, prefecture = $6, city = $7, address_line = $8, \
             building = $9, phone = $10, noshi_name = $11, gift_message = $12 \
             WHERE id = $1",
        )
        .bind(id)
        .bind(encrypt_field(email))
        .bind(encrypt_field(family))
        .bind(encrypt_field(given))
        .bind(encrypt_field(postal))
        .bind(encrypt_field(prefecture))
        .bind(encrypt_field(city))
        .bind(encrypt_field(line))
        .bind(encrypt_field_opt(building.as_deref()))
        .bind(encrypt_field(phone))
        .bind(encrypt_field_opt(noshi.as_deref()))
        .bind(encrypt_field_opt(gift.as_deref()))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}

/// Encrypts the remaining plaintext pending registrations.
async fn backfill_pending(pool: &PgPool) -> Result<usize, sqlx::Error> {
    type Row = (
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let rows = sqlx::query_as::<_, Row>(
        "SELECT id, email, family_name, given_name, family_name_kana, \
         given_name_kana, postal_code, prefecture, city, address_line, \
         building, phone FROM pending_registrations \
         WHERE email NOT LIKE 'v1:%'",
    )
    .fetch_all(pool)
    .await?;
    let mut tx = pool.begin().await?;
    for (
        id,
        email,
        family,
        given,
        family_kana,
        given_kana,
        postal,
        prefecture,
        city,
        line,
        building,
        phone,
    ) in &rows
    {
        sqlx::query(
            "UPDATE pending_registrations SET email = $2, email_bidx = $3, \
             family_name = $4, given_name = $5, family_name_kana = $6, \
             given_name_kana = $7, postal_code = $8, prefecture = $9, \
             city = $10, address_line = $11, building = $12, phone = $13 \
             WHERE id = $1",
        )
        .bind(id)
        .bind(encrypt_field(email))
        .bind(blind_index_email(email))
        .bind(encrypt_field_opt(family.as_deref()))
        .bind(encrypt_field_opt(given.as_deref()))
        .bind(encrypt_field_opt(family_kana.as_deref()))
        .bind(encrypt_field_opt(given_kana.as_deref()))
        .bind(encrypt_field_opt(postal.as_deref()))
        .bind(encrypt_field_opt(prefecture.as_deref()))
        .bind(encrypt_field_opt(city.as_deref()))
        .bind(encrypt_field_opt(line.as_deref()))
        .bind(encrypt_field_opt(building.as_deref()))
        .bind(encrypt_field_opt(phone.as_deref()))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len())
}
