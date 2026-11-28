//! Password hashing, verification and policy.
//!
//! Hashing and verification live in exactly one place: sign-up,
//! login, profile-driven changes and password changes hash and
//! verify through here, and cannot drift apart. The member
//! composition rule is judged here too; the admin surface
//! deliberately applies its own length-only floor instead
//! (see `member_account_service`).
//!
//! Argon2id is deliberately CPU-heavy, so the async wrappers run
//! it on the blocking thread pool: on a Tokio worker it would
//! stall every other request sharing that worker.

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

use crate::error::AppError;
use crate::messages::PASSWORD_PROCESSING_FAILED_MESSAGE;

/// Hashes a password with Argon2id on the blocking thread pool.
pub async fn hash_password_async(password: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|_| AppError::Conflict(PASSWORD_PROCESSING_FAILED_MESSAGE.to_string()))?
}

/// Verifies a password against a stored hash on the blocking
/// thread pool, for the same reason as [`hash_password_async`].
/// A hashing-side failure verifies as false: login must refuse,
/// never panic or pass, when the stored value is unusable.
pub async fn verify_password_async(password: String, stored_hash: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&password, &stored_hash))
        .await
        .unwrap_or(false)
}

/// Hashes a password with Argon2id and a fresh random salt.
pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| AppError::Conflict(PASSWORD_PROCESSING_FAILED_MESSAGE.to_string()))
}

/// Verifies a password against a stored Argon2id hash.
pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    match PasswordHash::new(stored_hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// The one password rule, shared by sign-up and password
/// changes: 8-64 characters with at least one ASCII letter and
/// one ASCII digit. Returns the message for the first broken
/// part, or None when the password passes.
pub fn password_rule_error(password: &str) -> Option<&'static str> {
    // The rule speaks of 文字 (characters): count characters, not
    // bytes — a byte count would judge a Japanese password as far
    // longer than it reads.
    if !(8..=64).contains(&password.chars().count()) {
        return Some("パスワードは8文字以上64文字以内で入力してください。");
    }
    let has_letter = password.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());
    if !has_letter || !has_digit {
        return Some("パスワードは半角英字と半角数字をそれぞれ1文字以上含めてください。");
    }
    None
}

/// A real Argon2id hash of a throwaway password, computed once.
/// Login verifies against it when the email is unknown, so
/// unknown and known accounts cost the same time to refuse and
/// the response time never reveals that an account is missing.
pub fn dummy_hash() -> &'static str {
    use std::sync::LazyLock;
    static DUMMY: LazyLock<String> =
        LazyLock::new(|| hash_password("timing-equalization-dummy").unwrap_or_default());
    &DUMMY
}

#[cfg(test)]
mod tests {
    use super::{hash_password, password_rule_error, verify_password};

    #[test]
    fn hash_round_trip() {
        let hash = hash_password("abc12345").unwrap();
        assert!(verify_password("abc12345", &hash));
        assert!(!verify_password("abc12346", &hash));
        assert!(!verify_password("abc12345", "not-a-hash"));
    }

    #[test]
    fn rule_counts_characters_not_bytes() {
        // 12 Japanese characters + letter + digit: 14 characters
        // but 38 bytes. A byte count would refuse this as over
        // the length limit; the rule means characters, so it passes.
        assert!(password_rule_error("あいうえおかきくけこさしa1").is_none());
        assert!(password_rule_error("あいうえおかきくa1").is_none());
        assert!(password_rule_error("short1").is_some());
        assert!(password_rule_error("abcdefgh").is_some());
        assert!(password_rule_error("12345678").is_some());
        // 34 characters: over the old 32 cap, inside the
        // widened 64 cap; 66 is over.
        assert!(password_rule_error(&"a1".repeat(17)).is_none());
        assert!(password_rule_error(&"a1".repeat(33)).is_some());
    }
}
