//! User repository.
//! Owns every query against the users table and the completion
//! transaction that turns a pending registration into an account.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::models::{PendingRegistration, User};
use crate::pii::{
    blind_index_email, decrypt_field, decrypt_user, encrypt_field, encrypt_field_opt,
};

/// Columns read for display and session purposes.
/// The password hash is never in this list.
const USER_COLUMNS: &str = "id, email, family_name, given_name, family_name_kana, \
    given_name_kana, phone, postal_code, prefecture, city, address_line, building, mail_magazine, \
 withdrawn_at, has_password";

/// Finds a member by ID, for example when loading the header greeting.
pub async fn find_by_id(pool: &PgPool, id: i64) -> Result<Option<User>, sqlx::Error> {
    let mut user =
        sqlx::query_as::<_, User>(&format!("SELECT {USER_COLUMNS} FROM users WHERE id = $1"))
            .bind(id)
            .fetch_optional(pool)
            .await?;
    if let Some(user) = &mut user {
        decrypt_user(user);
    }
    Ok(user)
}

/// Returns the stored password hash for an email, for login checks only.
/// Hash reads stay on the authentication paths: this lookup,
/// `password_hash_for_id`, and the credential-fingerprint read
/// (`credential_fields`) are the only ones.
pub async fn password_hash_for_email(
    pool: &PgPool,
    email: &str,
) -> Result<Option<(i64, String)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, String)>("SELECT id, password_hash FROM users WHERE email_bidx = $1")
        .bind(blind_index_email(email))
        .fetch_optional(pool)
        .await
}

/// Whether an account already exists for an email.
pub async fn exists_by_email(pool: &PgPool, email: &str) -> Result<bool, sqlx::Error> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE email_bidx = $1")
        .bind(blind_index_email(email))
        .fetch_one(pool)
        .await?;
    Ok(row.0 > 0)
}

/// Creates the account from a verified pending registration.
///
/// One transaction: insert the user, issue the welcome coupon,
/// delete the pending row — if any step fails, none happened, so
/// a half-created account can never exist.
///
/// # Errors
///
/// Returns [`sqlx::Error`] on database failure, including the
/// unique email constraint when the address was registered in
/// the meantime.
pub async fn create_from_pending(
    pool: &PgPool,
    pending: &PendingRegistration,
    welcome_coupon_code: Option<&str>,
    cart_session_key: Option<&str>,
) -> Result<i64, sqlx::Error> {
    create_account(
        pool,
        pending,
        welcome_coupon_code,
        cart_session_key,
        pending.password_hash.as_deref().unwrap_or_default(),
        true,
        None,
    )
    .await
}

/// Creates the account for an SNS registration: the same
/// transaction as [`create_from_pending`], but the account
/// starts passwordless (the stored hash is of an unguessable
/// random secret nobody knows, so password sign-in can never
/// succeed until a password is set) and the provider identity
/// is linked in the same transaction.
///
/// # Errors
///
/// Returns [`sqlx::Error`] on database failure, including the
/// unique constraints when the email or the identity was
/// registered in the meantime.
#[allow(clippy::too_many_arguments)]
pub async fn create_from_pending_sns(
    pool: &PgPool,
    pending: &PendingRegistration,
    welcome_coupon_code: Option<&str>,
    cart_session_key: Option<&str>,
    random_password_hash: &str,
    provider: &str,
    provider_user_id: &str,
    provider_email: Option<&str>,
) -> Result<i64, sqlx::Error> {
    create_account(
        pool,
        pending,
        welcome_coupon_code,
        cart_session_key,
        random_password_hash,
        false,
        Some((provider, provider_user_id, provider_email)),
    )
    .await
}

/// The account-creation transaction shared by the email and
/// SNS completions.
#[allow(clippy::too_many_arguments)]
async fn create_account(
    pool: &PgPool,
    pending: &PendingRegistration,
    welcome_coupon_code: Option<&str>,
    cart_session_key: Option<&str>,
    password_hash: &str,
    has_password: bool,
    identity: Option<(&str, &str, Option<&str>)>,
) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, email_bidx, password_hash, family_name, given_name, \
         family_name_kana, given_name_kana, birth_date, gender, postal_code, \
         prefecture, city, address_line, building, phone, mail_magazine, has_password) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17) \
         RETURNING id",
    )
    .bind(encrypt_field(&pending.email))
    .bind(blind_index_email(&pending.email))
    .bind(password_hash)
    .bind(encrypt_field(
        pending.family_name.as_deref().unwrap_or_default(),
    ))
    .bind(encrypt_field(
        pending.given_name.as_deref().unwrap_or_default(),
    ))
    .bind(encrypt_field(
        pending.family_name_kana.as_deref().unwrap_or_default(),
    ))
    .bind(encrypt_field(
        pending.given_name_kana.as_deref().unwrap_or_default(),
    ))
    .bind(pending.birth_date)
    .bind(&pending.gender)
    .bind(encrypt_field(
        pending.postal_code.as_deref().unwrap_or_default(),
    ))
    .bind(encrypt_field(
        pending.prefecture.as_deref().unwrap_or_default(),
    ))
    .bind(encrypt_field(pending.city.as_deref().unwrap_or_default()))
    .bind(encrypt_field(
        pending.address_line.as_deref().unwrap_or_default(),
    ))
    .bind(encrypt_field_opt(pending.building.as_deref()))
    .bind(encrypt_field(pending.phone.as_deref().unwrap_or_default()))
    .bind(pending.mail_magazine)
    .bind(has_password)
    .fetch_one(&mut *tx)
    .await?;

    if let Some(code) = welcome_coupon_code {
        sqlx::query("INSERT INTO user_coupons (user_id, code) VALUES ($1, $2)")
            .bind(user_id)
            .bind(code)
            .execute(&mut *tx)
            .await?;
    }

    if let Some((provider, provider_user_id, provider_email)) = identity {
        crate::repositories::user_identity_repository::insert_in_tx(
            &mut tx,
            user_id,
            provider,
            provider_user_id,
            provider_email,
        )
        .await?;
    }

    sqlx::query("DELETE FROM pending_registrations WHERE id = $1")
        .bind(pending.id)
        .execute(&mut *tx)
        .await?;

    // The address typed at registration also becomes the
    // member's first address-book entry (the default), so the
    // checkout picker can offer it from the first purchase.
    // Only a complete address is copied: a registration that
    // skipped the address (an SNS completion may) starts with
    // an empty book, exactly as before.
    let field = |value: &Option<String>| value.as_deref().unwrap_or_default().trim().to_owned();
    let registration_address = crate::models::AddressInput {
        label: String::new(),
        family_name: field(&pending.family_name),
        given_name: field(&pending.given_name),
        postal_code: field(&pending.postal_code),
        prefecture: field(&pending.prefecture),
        city: field(&pending.city),
        address_line: field(&pending.address_line),
        building: pending
            .building
            .as_ref()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()),
        phone: field(&pending.phone),
    };
    if !registration_address.family_name.is_empty()
        && !registration_address.given_name.is_empty()
        && !registration_address.postal_code.is_empty()
        && !registration_address.prefecture.is_empty()
        && !registration_address.city.is_empty()
        && !registration_address.address_line.is_empty()
        && !registration_address.phone.is_empty()
    {
        crate::repositories::member_address_repository::insert(
            &mut tx,
            user_id,
            &registration_address,
            true,
        )
        .await?;
    }

    // The guest session's cart becomes the new member's cart,
    // inside this same transaction.
    if let Some(key) = cart_session_key {
        crate::repositories::cart_repository::rekey_session_to_user(&mut tx, key, user_id).await?;
    }

    tx.commit().await?;
    Ok(user_id)
}

/// The stored Argon2id hash for one member, for password changes.
pub async fn password_hash_for_id(
    pool: &PgPool,
    user_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
}

/// The credential columns of one member row, in the order
/// (password hash, email, TOTP secret, TOTP enabled-at,
/// withdrawn-at), for the CSRF credential fingerprint (see
/// [`crate::session::credential_state`]).
///
/// Authentication use only, like the password hash reads:
/// these values feed a hash and are never rendered or logged.
/// Recovery codes are deliberately absent — they live in
/// their own table, so reissuing them does not move the
/// fingerprint.
pub async fn credential_fields(
    pool: &PgPool,
    user_id: i64,
) -> Result<
    Option<(
        String,
        String,
        Option<String>,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
    )>,
    sqlx::Error,
> {
    sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
        ),
    >(
        "SELECT password_hash, email, totp_secret, totp_enabled_at, withdrawn_at \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map(|row| {
        row.map(|(hash, email, secret, enabled_at, withdrawn_at)| {
            (
                hash,
                decrypt_field(&email),
                secret,
                enabled_at,
                withdrawn_at,
            )
        })
    })
}

/// Replaces a member's password hash.
pub async fn update_password_hash(
    pool: &PgPool,
    user_id: i64,
    password_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
        .bind(user_id)
        .bind(password_hash)
        .execute(pool)
        .await?;
    Ok(())
}

/// Normalised profile values for the mypage edit form.
pub struct ProfileUpdate {
    /// Family name in kanji.
    pub family_name: String,
    /// Given name in kanji.
    pub given_name: String,
    /// Family name in katakana.
    pub family_name_kana: String,
    /// Given name in katakana.
    pub given_name_kana: String,
    /// Postal code, digits only.
    pub postal_code: String,
    /// Prefecture name.
    pub prefecture: String,
    /// City and ward.
    pub city: String,
    /// Street address.
    pub address_line: String,
    /// Building name, when given.
    pub building: Option<String>,
    /// Phone number, digits only.
    pub phone: String,
    /// Whether the member accepts the mail magazine.
    pub mail_magazine: bool,
}

/// Saves the profile edit for one member, inside the
/// caller's transaction: the mail-magazine flag on this row
/// and the consent record in `email_subscriptions` commit
/// as one unit.
pub async fn update_profile(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    update: &ProfileUpdate,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET family_name = $2, given_name = $3, \
         family_name_kana = $4, given_name_kana = $5, postal_code = $6, \
         prefecture = $7, city = $8, address_line = $9, building = $10, \
         phone = $11, mail_magazine = $12 WHERE id = $1",
    )
    .bind(user_id)
    .bind(encrypt_field(&update.family_name))
    .bind(encrypt_field(&update.given_name))
    .bind(encrypt_field(&update.family_name_kana))
    .bind(encrypt_field(&update.given_name_kana))
    .bind(encrypt_field(&update.postal_code))
    .bind(encrypt_field(&update.prefecture))
    .bind(encrypt_field(&update.city))
    .bind(encrypt_field(&update.address_line))
    .bind(encrypt_field_opt(update.building.as_deref()))
    .bind(encrypt_field(&update.phone))
    .bind(update.mail_magazine)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the member ID for an email, or None when no account
/// uses it — for flows that act on an existing account without
/// loading the password hash.
pub async fn id_for_email(pool: &PgPool, email: &str) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM users WHERE email_bidx = $1")
        .bind(blind_index_email(email))
        .fetch_optional(pool)
        .await
}

/// Replaces a member's email address. Callers re-check the address
/// is still free first; the unique constraint is the backstop if
/// two changes race.
pub async fn update_email(pool: &PgPool, user_id: i64, email: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET email = $2, email_bidx = $3 WHERE id = $1")
        .bind(user_id)
        .bind(encrypt_field(email))
        .bind(blind_index_email(email))
        .execute(pool)
        .await?;
    Ok(())
}

/// Whether a withdrawn account under this email hash ever
/// redeemed WELCOME500 (Account Deletion spec D4): the
/// once-per-email-ever check behind the sign-up grant.
/// Only the hash is compared — the address itself is gone.
pub async fn welcome_previously_redeemed(
    pool: &PgPool,
    email_hash: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM users u \
         JOIN user_coupons uc ON uc.user_id = u.id \
         WHERE u.welcome_email_hash = $1 AND u.withdrawn_at IS NOT NULL \
           AND uc.code = 'WELCOME500' AND uc.used_at IS NOT NULL)",
    )
    .bind(email_hash)
    .fetch_one(pool)
    .await
}

/// Locks the member row for a withdrawal and reports its
/// current email and withdrawal state, so the service can
/// re-check both inside the transaction.
/// The member's current `updated_at`: the stamp a profile
/// form ticket is minted with at render time.
pub async fn updated_at(pool: &PgPool, user_id: i64) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    let value =
        sqlx::query_scalar::<_, DateTime<Utc>>("SELECT updated_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(value)
}

/// Returns the member's current `updated_at` with the row
/// locked, so no concurrent save can slip between the check
/// and the write: the profile save's staleness guard
/// compares its form ticket's stamp against this value
/// inside the update transaction (the product guard's
/// shape, `product_updated_at_for_update`).
pub async fn updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    let value = sqlx::query_scalar::<_, DateTime<Utc>>(
        "SELECT updated_at FROM users WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

pub async fn lock_withdrawal_state(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<Option<(String, Option<chrono::DateTime<chrono::Utc>>)>, sqlx::Error> {
    sqlx::query_as::<_, (String, Option<chrono::DateTime<chrono::Utc>>)>(
        "SELECT email, withdrawn_at FROM users WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map(|row| row.map(|(email, at)| (decrypt_field(&email), at)))
}

/// Anonymises a member in place: the row stays
/// as the tombstone keying the retained orders, with the
/// marker address, a hash of a random value in place of
/// the member's credential, every profile field wiped,
/// and the D4 email hash recorded.
pub async fn scrub_for_withdrawal(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    marker_email: &str,
    password_hash: &str,
    email_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET email = $2, email_bidx = $5, password_hash = $3, \
         family_name = '', given_name = '', family_name_kana = '', \
         given_name_kana = '', birth_date = NULL, gender = NULL, \
         postal_code = '', prefecture = '', city = '', address_line = '', \
         building = NULL, phone = '', mail_magazine = false, \
         withdrawn_at = now(), welcome_email_hash = $4 \
         WHERE id = $1",
    )
    .bind(user_id)
    .bind(encrypt_field(marker_email))
    .bind(password_hash)
    .bind(email_hash)
    .bind(blind_index_email(marker_email))
    .execute(&mut **tx)
    .await
    .map(|_| ())
}

/// Sets the first password of a passwordless (SNS-registered)
/// account: stores the hash and flips has_password in one
/// update, so the account is never observable in between.
pub async fn set_password(
    pool: &PgPool,
    user_id: i64,
    password_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET password_hash = $2, has_password = TRUE WHERE id = $1")
        .bind(user_id)
        .bind(password_hash)
        .execute(pool)
        .await?;
    Ok(())
}

/// Sets the legacy `users.mail_magazine` flag (Session
/// 7B). The subscription table is the record of record for
/// sending; this flag only drives the checkbox the member
/// sees on the profile form, so the unsubscribe flow flips
/// it too and the form never shows a stale state.
pub async fn set_mail_magazine(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    value: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET mail_magazine = $1 WHERE id = $2")
        .bind(value)
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
