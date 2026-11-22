//! HMAC-SHA-256 helpers, shared by the two features that sign
//! or verify with a server secret: verification codes (hashed
//! at rest, keyed by the code secret) and the KOMOJU webhook
//! (the signature over the raw body, keyed by the webhook
//! secret). One implementation, so the two can never drift.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Computes HMAC-SHA-256 over the concatenated parts.
pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

/// Computes HMAC-SHA-256 over the concatenated parts, hex encoded.
pub fn hmac_sha256_hex(key: &[u8], parts: &[&[u8]]) -> String {
    hex_encode(&hmac_sha256(key, parts))
}

/// Verifies a hex HMAC-SHA-256 signature over `data`.
///
/// The expected value is hex-decoded and checked with the MAC's
/// own verifier, which compares in constant time: a forged or
/// re-encoded body fails here, never partially. Malformed hex
/// is simply not a signature.
pub fn verify_hmac_sha256_hex(key: &[u8], data: &[u8], expected_hex: &str) -> bool {
    let Ok(expected) = hex_decode(expected_hex) else {
        return false;
    };
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.verify_slice(&expected).is_ok()
}

/// Encodes bytes as lowercase hex.
pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes lowercase or uppercase hex; `Err` on any non-hex
/// character or odd length.
fn hex_decode(text: &str) -> Result<Vec<u8>, ()> {
    if !text.len().is_multiple_of(2) {
        return Err(());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| ()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4231_test_case_1() {
        // The canonical HMAC-SHA-256 known answer: key of twenty
        // 0x0b bytes over "Hi There".
        let key = [0x0b_u8; 20];
        assert_eq!(
            hmac_sha256_hex(&key, &[b"Hi There"]),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn verify_accepts_only_the_exact_bytes() {
        let key = b"webhook-secret";
        let body = br#"{"id":"evt_1","type":"payment.captured"}"#;
        let signature = hmac_sha256_hex(key, &[body]);
        assert!(verify_hmac_sha256_hex(key, body, &signature));
        // The same JSON re-encoded with different spacing is a
        // different byte string and must fail (TC-06's core).
        let reencoded = br#"{ "id": "evt_1", "type": "payment.captured" }"#;
        assert!(!verify_hmac_sha256_hex(key, reencoded, &signature));
        // A wrong key and malformed hex both fail closed.
        assert!(!verify_hmac_sha256_hex(b"other", body, &signature));
        assert!(!verify_hmac_sha256_hex(key, body, "not-hex"));
        assert!(!verify_hmac_sha256_hex(key, body, ""));
    }

    #[test]
    fn parts_concatenate_in_order() {
        let joined = hmac_sha256_hex(b"k", &[b"email", b":", b"123456"]);
        let whole = hmac_sha256_hex(b"k", &[b"email:123456"]);
        assert_eq!(joined, whole);
    }
}
