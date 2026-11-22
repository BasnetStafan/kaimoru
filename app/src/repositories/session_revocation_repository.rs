//! Session repository.
//!
//! Owns the one query the session store's API cannot express:
//! finding every session of one identity, so a password change or
//! reset can revoke them. Both session layers write to the store's
//! one table, told apart by the key their records carry.
//!
//! The store's API works one session ID at a time, and the owner
//! ID lives inside each record's data — so rows are decoded here
//! with the store's own serializer (rmp_serde over [`Record`]) and
//! matched on the ID value itself. Decoding beats byte-pattern
//! matching: the MessagePack integer encoding varies with the ID's
//! magnitude, and a byte pattern could appear inside another value.
//! A row that fails to decode is skipped with a warning — never
//! deleted on a guess — and session IDs are never logged: a
//! session ID is a live credential.

use sqlx::PgPool;
use tower_sessions::session::Record;

/// The member ID key inside a session record's data: the same name
/// the session layer writes, repeated as a literal because this
/// repository reads the stored form, below the session layer.
const USER_ID_KEY: &str = "user_id";

/// The admin ID key inside an admin session record's data,
/// repeated as a literal for the same reason as [`USER_ID_KEY`].
const ADMIN_ID_KEY: &str = "admin_id";

/// Deletes every session belonging to one member, keeping one
/// session ID when given (the session performing a password change
/// survives; a reset keeps none). Returns the number deleted.
///
/// Guest and admin sessions cannot match: neither carries a
/// member ID.
pub async fn delete_sessions_for_user(
    pool: &PgPool,
    user_id: i64,
    keep_session_id: Option<&str>,
) -> Result<u64, sqlx::Error> {
    delete_sessions_for_key(pool, USER_ID_KEY, user_id, keep_session_id).await
}

/// Deletes every session belonging to one admin, keeping one
/// session ID when given (the session performing the password
/// change survives). Member sessions can never match.
pub async fn delete_sessions_for_admin(
    pool: &PgPool,
    admin_id: i64,
    keep_session_id: Option<&str>,
) -> Result<u64, sqlx::Error> {
    delete_sessions_for_key(pool, ADMIN_ID_KEY, admin_id, keep_session_id).await
}

/// The shared revocation walk: decode every stored record and
/// delete the ones whose `key` value is exactly `owner_id`.
///
/// Decoding every row is O(table size); revocation runs only on a
/// password change or reset, so the exact match is worth more
/// than an index.
///
/// # Errors
///
/// Returns [`sqlx::Error`] when the session table cannot be read
/// or the delete fails.
async fn delete_sessions_for_key(
    pool: &PgPool,
    key: &str,
    owner_id: i64,
    keep_session_id: Option<&str>,
) -> Result<u64, sqlx::Error> {
    let rows: Vec<(String, Vec<u8>)> =
        sqlx::query_as("SELECT id, data FROM tower_sessions.session")
            .fetch_all(pool)
            .await?;
    let mut doomed: Vec<String> = Vec::new();
    let mut undecodable = 0usize;
    for (row_id, data) in rows {
        if keep_session_id == Some(row_id.as_str()) {
            continue;
        }
        match rmp_serde::from_slice::<Record>(&data) {
            Ok(record) => {
                // The ID is stored as a JSON number by the
                // session layer; match it exactly.
                if record.data.get(key).and_then(|v| v.as_i64()) == Some(owner_id) {
                    doomed.push(row_id);
                }
            }
            Err(_) => undecodable += 1,
        }
    }
    if undecodable > 0 {
        tracing::warn!(
            undecodable,
            "session rows skipped during revocation: an undecodable record is never deleted on a guess"
        );
    }
    if doomed.is_empty() {
        return Ok(0);
    }
    let deleted = sqlx::query("DELETE FROM tower_sessions.session WHERE id = ANY($1)")
        .bind(&doomed)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(deleted)
}
