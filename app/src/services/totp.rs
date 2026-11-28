//! TOTP two-factor core (RFC 6238): enrolment secrets, code
//! generation and verification, the otpauth URI and its QR
//! rendering, and recovery codes.
//!
//! Pure functions only — storage and hashing for recovery
//! codes live in [`crate::services::totp_service`]. The hash
//! is HMAC-SHA-1 because that is what authenticator apps
//! implement by default; the `sha2` crate cannot substitute.

use hmac::{Hmac, Mac};
use rand::Rng;
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// The issuer label authenticator apps show for this store,
/// in the otpauth URI's issuer parameter and label prefix.
pub const ISSUER: &str = "AgenticStore";

/// Code lifetime in seconds: the RFC default every
/// authenticator app assumes.
pub const PERIOD_SECS: u64 = 30;

/// Code length in digits.
pub const DIGITS: u32 = 6;

/// How many recovery codes one regeneration issues.
pub const RECOVERY_CODE_COUNT: usize = 10;

/// Secret length in bytes: 160 bits, the RFC 4226
/// recommendation for HMAC-SHA-1.
const SECRET_BYTES: usize = 20;

/// Generates a fresh enrolment secret: CSPRNG bytes in the
/// base32 form authenticator apps accept as a manual key.
pub fn generate_secret() -> String {
    let mut bytes = [0u8; SECRET_BYTES];
    rand::thread_rng().fill(&mut bytes);
    base32_encode(&bytes)
}

/// Encodes bytes as RFC 4648 base32, uppercase, no padding
/// (the form authenticator apps accept).
pub fn base32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity(bytes.len() * 8 / 5 + 1);
    for chunk in bytes.chunks(5) {
        let mut buf = [0u8; 5];
        buf[..chunk.len()].copy_from_slice(chunk);
        let n = u64::from_be_bytes([buf[0], buf[1], buf[2], buf[3], buf[4], 0, 0, 0]);
        let chars = match chunk.len() {
            1 => 2,
            2 => 4,
            3 => 5,
            4 => 7,
            _ => 8,
        };
        for i in 0..chars {
            out.push(ALPHABET[((n >> (59 - 5 * i)) & 0x1f) as usize] as char);
        }
    }
    out
}

/// Decodes RFC 4648 base32; padding, spaces and lowercase are
/// tolerated so a key the member copied by hand still parses.
/// `None` on any character outside the alphabet.
pub fn base32_decode(text: &str) -> Option<Vec<u8>> {
    let mut values = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '=' || ch.is_whitespace() {
            continue;
        }
        let value = match ch.to_ascii_uppercase() {
            c @ 'A'..='Z' => c as u32 - 'A' as u32,
            c @ '2'..='7' => c as u32 - '2' as u32 + 26,
            _ => return None,
        };
        values.push(value);
    }
    let mut out = Vec::with_capacity(values.len() * 5 / 8);
    for chunk in values.chunks(8) {
        let mut n: u64 = 0;
        for &value in chunk {
            n = (n << 5) | u64::from(value);
        }
        let bits = chunk.len() * 5;
        let bytes = bits / 8;
        n <<= 64 - bits.max(8);
        for i in 0..bytes {
            out.push((n >> (56 - 8 * i)) as u8);
        }
    }
    Some(out)
}

/// The HOTP value (RFC 4226) of a counter, as a zero-padded
/// 6-digit string.
fn hotp(secret: &[u8], counter: u64) -> String {
    let mut mac = HmacSha1::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    // Dynamic truncation: the low nibble of the last byte picks
    // a 4-byte window, its top bit masked off.
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = (u32::from(digest[offset]) & 0x7f) << 24
        | (u32::from(digest[offset + 1]) & 0xff) << 16
        | (u32::from(digest[offset + 2]) & 0xff) << 8
        | (u32::from(digest[offset + 3]) & 0xff);
    format!("{:06}", binary % 10u32.pow(DIGITS))
}

/// The TOTP code for a base32 secret at a Unix time.
pub fn code_at(secret_base32: &str, unix_time: u64) -> Option<String> {
    let secret = base32_decode(secret_base32)?;
    Some(hotp(&secret, unix_time / PERIOD_SECS))
}

/// Verifies a submitted code against a base32 secret at a Unix
/// time, accepting the previous and next step as well (±30s of
/// clock skew and typing time). Candidates are compared with
/// the session module's constant-time equality, so the answer
/// leaks nothing about how much of the code matched.
pub fn verify(secret_base32: &str, submitted: &str, unix_time: u64) -> bool {
    verify_step(secret_base32, submitted, unix_time).is_some()
}

/// The step counter a submitted code matches, when it matches
/// one of the accepted steps (previous, current, next). The
/// login challenge records the returned step as consumed, so
/// the same code cannot be replayed inside its window
/// (SEC-5); plain verification is [`verify`].
pub fn verify_step(secret_base32: &str, submitted: &str, unix_time: u64) -> Option<u64> {
    let submitted = submitted.trim();
    if submitted.len() != DIGITS as usize || !submitted.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let secret = base32_decode(secret_base32)?;
    let counter = unix_time / PERIOD_SECS;
    [counter.wrapping_sub(1), counter, counter + 1]
        .into_iter()
        .find(|step| crate::session::constant_time_eq(&hotp(&secret, *step), submitted))
}

/// The otpauth URI an authenticator app enrols from. The
/// account label and issuer are percent-encoded, so the URI
/// (and anything rendered from it) carries no raw member data.
pub fn otpauth_uri(secret_base32: &str, account_email: &str) -> String {
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={}&period={}",
        percent_encode(ISSUER),
        percent_encode(account_email),
        secret_base32,
        percent_encode(ISSUER),
        DIGITS,
        PERIOD_SECS
    )
}

/// Percent-encodes everything outside the RFC 3986 unreserved
/// set, for otpauth URI parts.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Renders a URI as an inline SVG QR code, generated
/// server-side by the qrcode crate: no provider JavaScript,
/// no external image. `None` when the URI cannot be encoded.
pub fn qr_svg(uri: &str) -> Option<String> {
    let code = qrcode::QrCode::new(uri.as_bytes()).ok()?;
    Some(
        code.render::<qrcode::render::svg::Color>()
            .min_dimensions(220, 220)
            .dark_color(qrcode::render::svg::Color("#084C46"))
            .light_color(qrcode::render::svg::Color("#FFFFFF"))
            .build(),
    )
}

/// The manual-entry form of a base32 secret: groups of four,
/// the way authenticator apps present keys for typing.
pub fn grouped_secret(secret_base32: &str) -> String {
    secret_base32
        .as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Generates one recovery code's plaintext: ten hex characters
/// shown as `XXXXX-XXXXX`. The plaintext is shown to the
/// member exactly once; only its hash is ever stored.
pub fn generate_recovery_code() -> String {
    let mut bytes = [0u8; 5];
    rand::thread_rng().fill(&mut bytes);
    let hex = crate::util::hmac_util::hex_encode(&bytes).to_uppercase();
    format!("{}-{}", &hex[..5], &hex[5..])
}

/// Generates a full set of recovery codes.
pub fn generate_recovery_codes() -> Vec<String> {
    (0..RECOVERY_CODE_COUNT)
        .map(|_| generate_recovery_code())
        .collect()
}

/// Normalises a submitted recovery code for hashing: hyphens
/// and spaces removed, uppercase. Anything that is not then
/// ten hex characters is not a recovery code.
pub fn normalize_recovery_code(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| !matches!(c, '-' | ' ' | '　'))
        .collect::<String>()
        .to_uppercase();
    (cleaned.len() == 10 && cleaned.bytes().all(|b| b.is_ascii_hexdigit())).then_some(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RFC 6238 Appendix B SHA-1 test secret: the ASCII
    /// string "12345678901234567890".
    const RFC_SECRET_B32: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn base32_matches_rfc4648_vectors() {
        assert_eq!(base32_encode(b""), "");
        assert_eq!(base32_encode(b"f"), "MY");
        assert_eq!(base32_encode(b"fo"), "MZXQ");
        assert_eq!(base32_encode(b"foo"), "MZXW6");
        assert_eq!(base32_encode(b"foob"), "MZXW6YQ");
        assert_eq!(base32_encode(b"fooba"), "MZXW6YTB");
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI");
        assert_eq!(
            base32_encode(b"12345678901234567890"),
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
        );
    }

    #[test]
    fn base32_round_trips_and_tolerates_paste_noise() {
        for raw in [&b"f"[..], b"foob", b"12345678901234567890", &[7u8; 20]] {
            let encoded = base32_encode(raw);
            assert_eq!(base32_decode(&encoded).as_deref(), Some(raw));
        }
        assert_eq!(base32_decode("mzxw6 ytb").as_deref(), Some(&b"fooba"[..]));
        assert_eq!(base32_decode("MZXW6===").as_deref(), Some(&b"foo"[..]));
        assert_eq!(base32_decode("MZXW!"), None);
        assert_eq!(base32_decode("MZXW1"), None);
    }

    #[test]
    fn rfc6238_sha1_vectors_truncated_to_six_digits() {
        // The RFC's 8-digit SHA-1 values, truncated to this
        // app's 6 digits. Every value below was cross-checked
        // against two independent HMAC-SHA-1 implementations
        // (Python hmac and OpenSSL) over counter = time / 30.
        let cases = [
            (59_u64, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
            (20_000_000_000, "353130"),
        ];
        for (time, expected) in cases {
            assert_eq!(code_at(RFC_SECRET_B32, time).as_deref(), Some(expected));
            assert!(verify(RFC_SECRET_B32, expected, time));
        }
    }

    #[test]
    fn verify_accepts_neighbouring_steps_only() {
        let code = code_at(RFC_SECRET_B32, 59).unwrap();
        assert!(verify(RFC_SECRET_B32, &code, 59));
        assert!(verify(RFC_SECRET_B32, &code, 59 + 29));
        assert!(!verify(RFC_SECRET_B32, &code, 59 + 31));
        assert!(!verify(RFC_SECRET_B32, &code, 59 + 61));
        assert!(!verify(RFC_SECRET_B32, "28708", 59));
        assert!(!verify(RFC_SECRET_B32, "2870820", 59));
        assert!(!verify(RFC_SECRET_B32, "abcdef", 59));
        assert!(!verify(RFC_SECRET_B32, &code, 59 + 3600));
    }

    #[test]
    fn verify_step_reports_the_matched_counter() {
        // The code for t=59 is step 1 (59 / 30); the same code
        // presented inside its window still matches step 1,
        // which is what lets the caller refuse the replay.
        assert_eq!(verify_step(RFC_SECRET_B32, "287082", 59), Some(1));
        assert_eq!(verify_step(RFC_SECRET_B32, "287082", 89), Some(1));
        assert_eq!(verify_step(RFC_SECRET_B32, "287082", 3600), None);
        assert_eq!(verify_step(RFC_SECRET_B32, "abcdef", 59), None);
    }

    #[test]
    fn generated_secrets_are_160_bit_base32() {
        let secret = generate_secret();
        assert_eq!(secret.len(), 32);
        assert_eq!(base32_decode(&secret).map(|v| v.len()), Some(20));
        assert_ne!(secret, generate_secret());
    }

    #[test]
    fn otpauth_uri_encodes_the_account() {
        let uri = otpauth_uri("ABC234", "a+b@example.jp");
        assert!(uri.starts_with("otpauth://totp/AgenticStore:a%2Bb%40example.jp?secret=ABC234"));
        assert!(uri.contains("&issuer=AgenticStore"));
        assert!(uri.contains("&algorithm=SHA1&digits=6&period=30"));
    }

    #[test]
    fn recovery_codes_normalise_and_validate() {
        let code = generate_recovery_code();
        assert_eq!(code.len(), 11);
        let plain = normalize_recovery_code(&code).unwrap();
        assert_eq!(plain.len(), 10);
        assert_eq!(normalize_recovery_code(&plain.to_lowercase()), Some(plain));
        assert_eq!(normalize_recovery_code("287082"), None);
        assert_eq!(normalize_recovery_code("ZZZZZ-ZZZZZ"), None);
        assert_eq!(generate_recovery_codes().len(), RECOVERY_CODE_COUNT);
    }

    #[test]
    fn qr_svg_renders() {
        let svg = qr_svg("otpauth://totp/x").expect("small URI encodes");
        assert!(svg.contains("<svg"));
    }
}
