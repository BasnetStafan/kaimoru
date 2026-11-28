//! Serialization for JSON data embedded in a page's
//! `<script type="application/json">` block.
//!
//! The text is admin- or shop-typed (variation names, option
//! values), so a value containing `</script>` must not be able
//! to end the block early: `<` is escaped after serialization,
//! which JSON string syntax accepts as the same character.

/// Serializes a value to JSON safe for embedding in a script
/// block: the standard serialization, with `<` written as its
/// unicode escape. Falls back to `null` on a serialization
/// failure (the values embedded in this app always serialize).
pub fn json_for_script<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "null".to_string())
        .replace('<', "\\u003c")
}
