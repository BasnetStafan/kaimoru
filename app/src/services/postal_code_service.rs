//! Business rules for the postal code lookup: what a valid
//! code looks like, and the read itself.
//!
//! The data comes from the Japan Post CSV, imported by an
//! ops script; this service never writes it.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::PostalAddress;
use crate::repositories::postal_code_repository;

/// Returns true when `raw` is exactly seven ASCII digits.
///
/// The address forms strip hyphens in the browser before
/// calling, so anything else is a client error, not a miss.
pub fn is_valid_postal_code(raw: &str) -> bool {
    raw.len() == 7 && raw.bytes().all(|b| b.is_ascii_digit())
}

/// Returns every address row for a postal code; an empty
/// vec means the code is unknown, or the data has not been
/// imported yet.
pub async fn lookup(pool: &PgPool, postal_code: &str) -> Result<Vec<PostalAddress>, AppError> {
    postal_code_repository::find_by_code(pool, postal_code).await
}
