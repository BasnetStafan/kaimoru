//! Field-level encryption for personal data at rest.
//!
//! Email, names, postal addresses and phone numbers are stored
//! as AES-256-GCM ciphertext: a
//! database copy on its own — a dump, a snapshot, a read-only
//! leak — reveals nothing without the master key, which lives
//! only in the app's environment. Repositories encrypt on write
//! and decrypt on read, so models and everything above them
//! keep working with plaintext. Email lookups run through
//! [`blind_index_email`], an HMAC over the normalised address,
//! because ciphertext cannot be searched.

use std::sync::OnceLock;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use configuration::config::CONFIG;

use crate::models::restock::RestockSubscription;
use crate::models::{AdminCustomerDetail, MemberAddress, Order, PendingRegistration, User};
use crate::util::hmac_util::{hex_encode, hmac_sha256};

/// Storage version marker prefixed to every encrypted value.
const PREFIX: &str = "v1:";
/// Domain separator for the blind-index key derivation, so the
/// index key and the encryption key are never the same bytes.
const BLIND_INDEX_DOMAIN: &[u8] = b"pii-blind-index-v1";
/// GCM nonce length in bytes (the crate's default 96-bit).
const NONCE_LEN: usize = 12;

/// Returns the cached master key, loaded once from the
/// environment through the typed config (which panics when
/// the variable is missing or malformed — fail closed).
pub(crate) fn master_key() -> &'static [u8; 32] {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    KEY.get_or_init(|| CONFIG.pii_encryption_key())
}

/// Returns the blind-index key, derived from the master key
/// under a domain separator.
fn blind_key() -> &'static [u8; 32] {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    KEY.get_or_init(|| hmac_sha256(master_key(), &[BLIND_INDEX_DOMAIN]))
}

/// Encrypts one value with the given key. The stored form is
/// `v1:` + base64(nonce || ciphertext || tag), with a fresh
/// random nonce per value, so equal plaintexts never produce
/// equal stored forms. The empty string is stored as itself:
/// it carries no information, and wiped rows keep their
/// existing empty-field semantics.
pub(crate) fn encrypt_with(key: &[u8; 32], plaintext: &str) -> String {
    if plaintext.is_empty() {
        return String::new();
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .expect("AES-GCM encryption of an in-memory value does not fail");
    let mut stored = nonce.to_vec();
    stored.extend_from_slice(&ciphertext);
    format!("{PREFIX}{}", BASE64.encode(stored))
}

/// Decrypts one `v1:` value with the given key.
///
/// # Errors
///
/// Returns `Err` on malformed input or a GCM tag mismatch
/// (tampered data, or a value written under another key).
pub(crate) fn decrypt_with(key: &[u8; 32], stored: &str) -> Result<String, ()> {
    let body = stored.strip_prefix(PREFIX).ok_or(())?;
    let raw = BASE64.decode(body).map_err(|_| ())?;
    if raw.len() <= NONCE_LEN {
        return Err(());
    }
    let (nonce, ciphertext) = raw.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| ())?;
    String::from_utf8(plain).map_err(|_| ())
}

/// Encrypts one personal value for storage.
pub fn encrypt_field(plaintext: &str) -> String {
    encrypt_with(master_key(), plaintext)
}

/// Decrypts one stored personal value.
///
/// A value without the version marker is pre-encryption
/// plaintext: it is returned as-is (the boot [`crate::pii_backfill::backfill`]
/// rewrites it) and a warning is logged — never the value.
///
/// # Panics
///
/// Panics when a `v1:` value fails to decrypt: the data was
/// written under another key or was tampered with, and
/// serving it as-is would render ciphertext as a name.
pub fn decrypt_field(stored: &str) -> String {
    if stored.is_empty() || !stored.starts_with(PREFIX) {
        if !stored.is_empty() {
            tracing::warn!("pii: read a legacy plaintext value (backfill pending)");
        }
        return stored.to_string();
    }
    decrypt_with(master_key(), stored)
        .expect("stored PII must decrypt with the configured key: check PII_ENCRYPTION_KEY")
}

/// The non-panicking variant of [`decrypt_field`], for
/// fail-closed callers (the TOTP secret read): `None` when a
/// `v1:` value cannot be decrypted, or when the value is not
/// a `v1:` ciphertext at all.
pub fn decrypt_field_checked(stored: &str) -> Option<String> {
    if !stored.starts_with(PREFIX) {
        return None;
    }
    decrypt_with(master_key(), stored).ok()
}

/// Whether a stored value carries the encrypted prefix, as
/// opposed to legacy plaintext written before encryption
/// existed. Callers that must not panic on a corrupt value
/// use this to tell "legacy plaintext, use as-is" apart from
/// "encrypted, must decrypt" before reaching for
/// [`decrypt_field_checked`].
pub fn is_encrypted_value(stored: &str) -> bool {
    stored.starts_with(PREFIX)
}

/// Encrypts an optional personal value; `None` stays `None`.
pub fn encrypt_field_opt(value: Option<&str>) -> Option<String> {
    value.map(encrypt_field)
}

/// Decrypts an optional stored value; `None` stays `None`.
pub fn decrypt_field_opt(value: Option<String>) -> Option<String> {
    value.map(|stored| decrypt_field(&stored))
}

/// Computes the blind index of an email address with the
/// given key: deterministic over the normalised address
/// (trimmed, lowercased — the same normalisation the auth
/// services apply), so equality lookups work without the
/// address ever being stored in readable form.
fn blind_index_with(key: &[u8; 32], email: &str) -> String {
    let normalized = email.trim().to_lowercase();
    hex_encode(&hmac_sha256(key, &[b"email:", normalized.as_bytes()]))
}

/// Computes the blind index stored in `users.email_bidx` and
/// `pending_registrations.email_bidx` for an email address.
pub fn blind_index_email(email: &str) -> String {
    blind_index_with(blind_key(), email)
}

/// Decrypts every personal field of a loaded member row, in
/// place. Repository read paths call this before the model
/// travels upward, so no caller ever sees ciphertext.
pub fn decrypt_user(user: &mut User) {
    user.email = decrypt_field(&user.email);
    user.family_name = decrypt_field(&user.family_name);
    user.given_name = decrypt_field(&user.given_name);
    user.family_name_kana = decrypt_field(&user.family_name_kana);
    user.given_name_kana = decrypt_field(&user.given_name_kana);
    user.postal_code = decrypt_field(&user.postal_code);
    user.prefecture = decrypt_field(&user.prefecture);
    user.city = decrypt_field(&user.city);
    user.address_line = decrypt_field(&user.address_line);
    user.building = decrypt_field_opt(std::mem::take(&mut user.building));
    user.phone = decrypt_field(&user.phone);
}

/// Decrypts every personal field of a pending registration.
pub fn decrypt_pending(pending: &mut PendingRegistration) {
    pending.email = decrypt_field(&pending.email);
    pending.family_name = decrypt_field_opt(std::mem::take(&mut pending.family_name));
    pending.given_name = decrypt_field_opt(std::mem::take(&mut pending.given_name));
    pending.family_name_kana = decrypt_field_opt(std::mem::take(&mut pending.family_name_kana));
    pending.given_name_kana = decrypt_field_opt(std::mem::take(&mut pending.given_name_kana));
    pending.postal_code = decrypt_field_opt(std::mem::take(&mut pending.postal_code));
    pending.prefecture = decrypt_field_opt(std::mem::take(&mut pending.prefecture));
    pending.city = decrypt_field_opt(std::mem::take(&mut pending.city));
    pending.address_line = decrypt_field_opt(std::mem::take(&mut pending.address_line));
    pending.building = decrypt_field_opt(std::mem::take(&mut pending.building));
    pending.phone = decrypt_field_opt(std::mem::take(&mut pending.phone));
}

/// Decrypts every personal field of an address-book entry.
pub fn decrypt_member_address(address: &mut MemberAddress) {
    address.family_name = decrypt_field(&address.family_name);
    address.given_name = decrypt_field(&address.given_name);
    address.postal_code = decrypt_field(&address.postal_code);
    address.prefecture = decrypt_field(&address.prefecture);
    address.city = decrypt_field(&address.city);
    address.address_line = decrypt_field(&address.address_line);
    address.building = decrypt_field_opt(std::mem::take(&mut address.building));
    address.phone = decrypt_field(&address.phone);
}

/// Decrypts every personal field of an order's delivery
/// snapshot.
pub fn decrypt_order(order: &mut Order) {
    order.email = decrypt_field(&order.email);
    order.family_name = decrypt_field(&order.family_name);
    order.given_name = decrypt_field(&order.given_name);
    order.postal_code = decrypt_field(&order.postal_code);
    order.prefecture = decrypt_field(&order.prefecture);
    order.city = decrypt_field(&order.city);
    order.address_line = decrypt_field(&order.address_line);
    order.building = decrypt_field_opt(std::mem::take(&mut order.building));
    order.phone = decrypt_field(&order.phone);
    order.noshi_name = decrypt_field_opt(std::mem::take(&mut order.noshi_name));
    order.gift_message = decrypt_field_opt(std::mem::take(&mut order.gift_message));
}

/// Decrypts the notification address of a back-in-stock
/// subscription, in place.
pub fn decrypt_restock(subscription: &mut RestockSubscription) {
    subscription.email = decrypt_field(&subscription.email);
}

/// Decrypts every personal field of an admin customer
/// detail row, in place.
pub fn decrypt_customer_detail(detail: &mut AdminCustomerDetail) {
    detail.email = decrypt_field(&detail.email);
    detail.family_name = decrypt_field(&detail.family_name);
    detail.given_name = decrypt_field(&detail.given_name);
    detail.family_name_kana = decrypt_field(&detail.family_name_kana);
    detail.given_name_kana = decrypt_field(&detail.given_name_kana);
    detail.phone = decrypt_field(&detail.phone);
    detail.postal_code = decrypt_field(&detail.postal_code);
    detail.prefecture = decrypt_field(&detail.prefecture);
    detail.city = decrypt_field(&detail.city);
    detail.address_line = decrypt_field(&detail.address_line);
    detail.building = decrypt_field_opt(std::mem::take(&mut detail.building));
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: [u8; 32] = [7_u8; 32];

    #[test]
    fn round_trip_restores_the_value() {
        let stored = encrypt_with(&TEST_KEY, "basnet@example.com");
        assert!(stored.starts_with(PREFIX));
        assert_eq!(
            decrypt_with(&TEST_KEY, &stored).expect("round trip"),
            "basnet@example.com"
        );
    }

    #[test]
    fn equal_plaintexts_encrypt_differently() {
        // The random nonce is what stops ciphertext comparison:
        // a leak must not reveal that two rows share an email.
        assert_ne!(
            encrypt_with(&TEST_KEY, "same@example.com"),
            encrypt_with(&TEST_KEY, "same@example.com")
        );
    }

    #[test]
    fn tampered_ciphertext_fails_the_tag_check() {
        let mut bytes = BASE64
            .decode(
                encrypt_with(&TEST_KEY, "090-0000-0000")
                    .strip_prefix(PREFIX)
                    .expect("prefix"),
            )
            .expect("base64");
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        let forged = format!("{PREFIX}{}", BASE64.encode(bytes));
        assert!(decrypt_with(&TEST_KEY, &forged).is_err());
        assert!(decrypt_with(&[9_u8; 32], &encrypt_with(&TEST_KEY, "x")).is_err());
    }

    #[test]
    fn empty_and_legacy_values_pass_through() {
        assert_eq!(encrypt_with(&TEST_KEY, ""), "");
        assert_eq!(decrypt_field(""), "");
        assert_eq!(decrypt_field("plain@example.com"), "plain@example.com");
    }

    #[test]
    fn blind_index_is_deterministic_and_normalised() {
        let a = blind_index_with(&TEST_KEY, "  User@Example.com ");
        let b = blind_index_with(&TEST_KEY, "user@example.com");
        assert_eq!(a, b);
        assert_ne!(a, blind_index_with(&TEST_KEY, "other@example.com"));
        assert_eq!(a.len(), 64);
    }
}
