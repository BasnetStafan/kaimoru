//! SQL `LIKE` pattern construction for search filters.
//!
//! These helpers build query patterns from operator or
//! member input; they are query construction, not form
//! field validation, so they live outside `validation`.

/// Escapes the LIKE wildcards in a search term so the term can
/// only match literally: the backslash first (it is the default
/// LIKE escape character), then `%` and `_`.
pub fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Builds the contains-pattern (`%term%`) for a search box from
/// the raw typed text, or `None` when nothing but whitespace was
/// typed. Wildcards in the text are escaped via [`escape_like`],
/// so the pattern matches the typed characters literally.
pub fn like_pattern(query: &str) -> Option<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(format!("%{}%", escape_like(trimmed)))
    }
}
