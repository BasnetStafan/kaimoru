//! XML text escaping, shared by the two renderers that
//! build XML documents by hand: the sitemap and the blog
//! RSS feed. One implementation, so the two can never
//! drift.

/// Escapes the five XML-special characters in a text node
/// or attribute value. Sitemap locations are built from
/// our own origin plus ids and operator-entered slugs, and
/// feed titles and excerpts are operator text; a slug or a
/// title may legally contain characters XML forbids raw.
pub fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::escape_xml;

    #[test]
    fn escapes_all_five_xml_specials() {
        assert_eq!(
            escape_xml("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
    }

    #[test]
    fn leaves_plain_text_untouched() {
        assert_eq!(escape_xml("plain text 123"), "plain text 123");
    }
}
