//! Shared form validation: one home for the field rules every
//! form must agree on.
//!
//! Before this module the app had two email validators with
//! different answers and digit rules with three behaviours. The
//! rules here are the strict variants; the Japanese messages stay
//! with each caller, because the same failure reads differently
//! on different forms.

/// One field's validation failure.
///
/// Services return a list of these so a form can show every
/// problem at once, each under its own field.
#[derive(Debug, Clone)]
pub struct FieldError {
    /// The form field name the error belongs to.
    pub field: &'static str,
    /// The message to show under that field.
    pub message: String,
}

impl FieldError {
    /// Creates one field error for `field` with the given message:
    /// the way every service builds the errors its form returns.
    pub fn new(field: &'static str, message: &str) -> Self {
        FieldError {
            field,
            message: message.to_string(),
        }
    }
}

/// The message recorded for `field`, if any: the lookup every
/// form's per-field error display shares.
pub fn field_error(errors: &[FieldError], field: &str) -> Option<String> {
    errors
        .iter()
        .find(|e| e.field == field)
        .map(|e| e.message.clone())
}

/// Records the outcome of a uniqueness check in a form's error
/// list: a taken value adds the "already used" message, a failed
/// check adds the "could not confirm" message, and a free value
/// adds nothing.
///
/// A failed check must never read as "the value is free": the
/// form could not look, so the operator retries instead of the
/// app registering a duplicate it never verified. The check
/// itself is the caller's (a repository `*_taken` call); only
/// its outcome is interpreted here.
pub fn push_uniqueness_check<E>(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    result: Result<bool, E>,
    taken_message: &str,
    failed_message: &str,
) {
    match result {
        Ok(true) => errors.push(FieldError::new(field, taken_message)),
        Ok(false) => {}
        Err(_) => errors.push(FieldError::new(field, failed_message)),
    }
}

/// The one email shape check: exactly one `@`, a non-empty local
/// part, a dot inside the domain (never first or last), and no
/// whitespace anywhere.
///
/// Full RFC validation is not attempted: sign-up proves the
/// address receives mail by sending it a verification code.
pub fn is_valid_email(email: &str) -> bool {
    let mut parts = email.split('@');
    let (Some(local), Some(domain)) = (parts.next(), parts.next()) else {
        return false;
    };
    parts.next().is_none()
        && !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email.contains(char::is_whitespace)
}

/// Returns the digits of a postal code when the input is exactly
/// seven digits once hyphens are removed.
///
/// Only `-` is stripped; letters or spaces make the input invalid
/// instead of being silently removed.
pub fn postal_code_digits(text: &str) -> Option<String> {
    digits_after_hyphen_strip(text).filter(|digits| digits.len() == 7)
}

/// Returns the digits of a phone number when the input is 10 or
/// 11 digits once hyphens are removed (landline or mobile length).
/// The same strip rule as [`postal_code_digits`].
pub fn phone_digits(text: &str) -> Option<String> {
    digits_after_hyphen_strip(text).filter(|digits| (10..=11).contains(&digits.len()))
}

/// Removes hyphens and returns the rest when it is all ASCII
/// digits, or `None` when anything else is present.
fn digits_after_hyphen_strip(text: &str) -> Option<String> {
    let digits: String = text.chars().filter(|c| *c != '-').collect();
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        Some(digits)
    } else {
        None
    }
}

/// Whether a string is a valid slug: 1-50 characters of ASCII
/// lowercase letters, digits and hyphens, never starting or
/// ending with a hyphen. Slugs are the machine-facing identifiers
/// of categories and tags (URLs, agent matching), so the shape is
/// deliberately the URL-safe subset; callers lowercase the input
/// before checking, making this the canonical form.
pub fn is_valid_slug(text: &str) -> bool {
    let len = text.chars().count();
    (1..=50).contains(&len)
        && !text.starts_with('-')
        && !text.ends_with('-')
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Whether an operator-entered link target is allowed
/// an internal path (`/…`, never
/// protocol-relative `//…`), an `https:` URL, or a
/// same-page fragment (`#…`). Script schemes, `data:`,
/// plain `http:` and bare domains are refused, and any
/// whitespace or control character refuses the URL — a
/// browser would strip those out of a scheme and could be
/// left holding `javascript:`.
///
/// This is the strict rule for link *fields* (the home
/// slots here, menus next); Markdown body links follow the
/// wider allowlist in `crate::util::markdown`, which also permits
/// `http:` and `mailto:`.
pub fn link_url_allowed(url: &str) -> bool {
    let url = url.trim();
    if url.is_empty() {
        return false;
    }
    if url.chars().any(|c| c <= ' ') {
        return false;
    }
    if url.starts_with('#') {
        return true;
    }
    if url.starts_with('/') {
        return !url.starts_with("//");
    }
    url.to_ascii_lowercase().starts_with("https://")
}

/// Whether a string is full-width katakana (plus the long vowel
/// mark and the middle dot used in names).
pub fn is_katakana(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| ('\u{30A1}'..='\u{30F6}').contains(&c) || c == 'ー' || c == '・')
}

#[cfg(test)]
mod tests {
    use super::{
        is_katakana, is_valid_email, is_valid_slug, link_url_allowed, phone_digits,
        postal_code_digits,
    };

    #[test]
    fn email_shape_is_the_strict_rule_everywhere() {
        assert!(is_valid_email("a@b.co"));
        assert!(is_valid_email("member+tag@example.co.jp"));
        assert!(!is_valid_email("a@b"));
        assert!(!is_valid_email("@b.co"));
        assert!(!is_valid_email("a@.co"));
        assert!(!is_valid_email("a@b."));
        // Shapes the old sign-up validator accepted are refused now.
        assert!(!is_valid_email("a b@c.co"));
        assert!(!is_valid_email("a@@b.co"));
        assert!(!is_valid_email("a@b@c.d"));
    }

    #[test]
    fn postal_code_accepts_hyphens_only_as_separators() {
        assert_eq!(postal_code_digits("1500001").as_deref(), Some("1500001"));
        assert_eq!(postal_code_digits("150-0001").as_deref(), Some("1500001"));
        // Letters are not stripped away and forgiven.
        assert_eq!(postal_code_digits("150-00a1"), None);
        assert_eq!(postal_code_digits("150 0001"), None);
        assert_eq!(postal_code_digits("123"), None);
        assert_eq!(postal_code_digits(""), None);
    }

    #[test]
    fn phone_accepts_ten_or_eleven_digits() {
        assert_eq!(
            phone_digits("090-1234-5678").as_deref(),
            Some("09012345678")
        );
        assert_eq!(phone_digits("0312345678").as_deref(), Some("0312345678"));
        // Letters are refused even when the digit count would fit.
        assert_eq!(phone_digits("090-abcd-5678"), None);
        assert_eq!(phone_digits("123"), None);
    }

    #[test]
    fn slug_shape_is_the_url_safe_subset() {
        assert!(is_valid_slug("apparel"));
        assert!(is_valid_slug("large-capacity"));
        assert!(is_valid_slug("a1-2b"));
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("Apparel"));
        assert!(!is_valid_slug("-apparel"));
        assert!(!is_valid_slug("apparel-"));
        assert!(!is_valid_slug("アパレル"));
        assert!(!is_valid_slug("a b"));
        assert!(!is_valid_slug(&"a".repeat(51)));
    }

    #[test]
    fn katakana_names() {
        assert!(is_katakana("ヤマダ"));
        assert!(is_katakana("メアリー"));
        assert!(!is_katakana("やまだ"));
        assert!(!is_katakana(""));
    }

    #[test]
    fn link_url_rule_is_the_strict_section_7_shape() {
        assert!(link_url_allowed("/guide"));
        assert!(link_url_allowed("/search?category=apparel"));
        assert!(link_url_allowed("https://example.com/page"));
        assert!(link_url_allowed("#new"));
        // The shapes the rule exists to refuse.
        assert!(!link_url_allowed(""));
        assert!(!link_url_allowed("http://example.com"));
        assert!(!link_url_allowed("//evil.example"));
        assert!(!link_url_allowed("javascript:alert(1)"));
        assert!(!link_url_allowed("data:text/html;base64,x"));
        assert!(!link_url_allowed("example.com"));
        assert!(!link_url_allowed("/guide us"));
        assert!(!link_url_allowed("java\tscript:alert(1)"));
    }
}
