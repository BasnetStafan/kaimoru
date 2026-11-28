//! Sanitising for admin-authored HTML page bodies (CMS v2).
//!
//! HTML content pages are the one place the store renders
//! stored markup, so the stored text must already be safe:
//! the admin service runs every HTML body through
//! [`sanitize_html`] at save time, and the preview runs the
//! unsaved body through it too — which is what lets the
//! renderer emit an HTML page's body unescaped. Cleaning is
//! the `ammonia` crate's job, never a hand-rolled parser.

use std::borrow::Cow;
use std::sync::LazyLock;

use ammonia::Builder;

use crate::util::markdown;

/// The one sanitiser configuration, built once: an allowlist
/// shaped for page bodies. Structure and formatting survive;
/// anything that can run script, submit a form, or send the
/// reader off-site does not.
static SANITIZER: LazyLock<Builder<'static>> = LazyLock::new(|| {
    let mut builder = Builder::default();
    builder
        // Already ammonia defaults; named so the page-body
        // vocabulary this store promises is visible in one place.
        .add_tags([
            "section",
            "article",
            "header",
            "footer",
            "nav",
            "figure",
            "figcaption",
            "details",
            "summary",
            "img",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
        ])
        // Whole-content removal for the two tags whose text is
        // code, not prose: a stripped `<script>` must not leave
        // its code behind as visible text either. Every other
        // refused tag (script's cousins form, iframe, meta,
        // base, input, …) is simply absent from the tag
        // allowlist, so ammonia unwraps it.
        .clean_content_tags(["script", "style"].into_iter().collect())
        .add_generic_attributes(["class", "id", "style"])
        // `rel` is deliberately not allowlisted on links:
        // ammonia owns it (it stamps `noopener noreferrer`
        // itself when a link opens a new tab) and refuses a
        // configuration that tries to allow it.
        .add_tag_attributes("a", ["target"])
        // The destinations an HTML body may point at are the
        // Markdown allowlist's schemes and nothing wider —
        // ammonia's own defaults would also pass ftp:, tel:
        // and more.
        .url_schemes(["http", "https", "mailto"].into_iter().collect())
        .attribute_filter(|_tag, attr, value| filter_attribute(attr, value));
    builder
});

/// Applies the per-attribute vetoes the allowlist cannot
/// express: dangerous `style` values, and scheme-relative
/// URLs.
fn filter_attribute<'v>(attr: &str, value: &'v str) -> Option<Cow<'v, str>> {
    if attr == "style" {
        // Inline styles are layout, but a style value can also
        // smuggle a network fetch or a legacy script vector;
        // drop the whole attribute when it tries.
        let squashed: String = value
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_lowercase();
        if squashed.contains("url(")
            || squashed.contains("expression(")
            || squashed.contains("javascript:")
            || squashed.contains("-moz-binding")
        {
            return None;
        }
    }
    if (attr == "href" || attr == "src") && value.trim_start().starts_with("//") {
        // A scheme-relative URL inherits the page's scheme and
        // points off-site. Ammonia's relative-URL pass-through
        // would keep it, and the shared allowlist refuses it,
        // so the attribute goes.
        return None;
    }
    Some(Cow::Borrowed(value))
}

/// Returns `source` reduced to the page-body allowlist.
///
/// Only sanitised output is ever stored for an HTML page, so
/// callers never need to sanitise again at render time.
pub fn sanitize_html(source: &str) -> String {
    SANITIZER.clean(source).to_string()
}

/// Returns the first `href`/`src` destination in `source`
/// that the shared URL allowlist refuses, so the save-time
/// error can name it exactly like the Markdown path does.
///
/// This scanner only feeds that error message: it reads
/// attribute values out of the raw source (quoted or
/// unquoted, names case-insensitive). Cleaning stored HTML
/// is [`sanitize_html`]'s job, never this function's.
pub fn find_invalid_url_html(source: &str) -> Option<String> {
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let name_len = if attr_name_at(bytes, i, b"href") {
            4
        } else if attr_name_at(bytes, i, b"src") {
            3
        } else {
            i += 1;
            continue;
        };
        // The name must stand alone: a longer attribute that
        // merely ends in href/src (data-href) is a different
        // attribute, and href in srcset/hreflang is refused by
        // the `=` check below, never read as a destination.
        let stands_alone = i == 0 || !is_attr_name_char(bytes[i - 1]);
        let mut j = skip_ws(bytes, i + name_len);
        if !stands_alone || j >= bytes.len() || bytes[j] != b'=' {
            i += 1;
            continue;
        }
        j = skip_ws(bytes, j + 1);
        if j >= bytes.len() {
            break;
        }
        let (value, next) = if bytes[j] == b'"' || bytes[j] == b'\'' {
            let quote = bytes[j];
            let start = j + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != quote {
                end += 1;
            }
            (&source[start..end], end)
        } else {
            let start = j;
            let mut end = start;
            while end < bytes.len() && !is_ws(bytes[end]) && bytes[end] != b'>' {
                end += 1;
            }
            (&source[start..end], end)
        };
        if !markdown::url_allowed(value) {
            return Some(value.to_string());
        }
        i = next.max(i + 1);
    }
    None
}

/// Whether the bytes at `i` spell the attribute `name`,
/// case-insensitively.
fn attr_name_at(bytes: &[u8], i: usize, name: &[u8]) -> bool {
    bytes
        .get(i..i + name.len())
        .is_some_and(|found| found.eq_ignore_ascii_case(name))
}

/// Whether a byte can be part of an attribute name.
fn is_attr_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Whether a byte is HTML whitespace.
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0C)
}

/// Returns the first index at or after `i` that is not
/// whitespace.
fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && is_ws(bytes[i]) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::{find_invalid_url_html, sanitize_html};

    #[test]
    fn script_tag_and_its_code_are_gone() {
        let out = sanitize_html("<p>本文</p><script>alert(1)</script>");
        assert!(out.contains("<p>本文</p>"));
        assert!(!out.contains("alert"));
        assert!(!out.contains("<script"));
    }

    #[test]
    fn event_handler_attributes_are_stripped() {
        let out = sanitize_html("<p onclick=\"steal()\">本文</p>");
        assert!(!out.contains("onclick"));
        assert!(out.contains("本文"));
    }

    #[test]
    fn javascript_href_is_neutralised() {
        let out = sanitize_html("<a href=\"javascript:alert(1)\">リンク</a>");
        assert!(!out.contains("javascript:"));
        assert!(out.contains("リンク"));
    }

    #[test]
    fn meta_refresh_is_stripped() {
        let out = sanitize_html(
            "<meta http-equiv=\"refresh\" content=\"0; url=https://evil.example/\"><p>本文</p>",
        );
        assert!(!out.contains("refresh"));
        assert!(!out.contains("<meta"));
        assert!(out.contains("本文"));
    }

    #[test]
    fn form_is_unwrapped() {
        let out = sanitize_html("<form action=\"/checkout\"><p>フォーム</p></form>");
        assert!(!out.contains("<form"));
        assert!(out.contains("フォーム"));
    }

    #[test]
    fn styled_div_keeps_class_and_style() {
        let out = sanitize_html("<div class=\"x\" style=\"color:red\">本文</div>");
        assert!(out.contains("class=\"x\""), "got: {out}");
        assert!(out.contains("style=\"color:red\""), "got: {out}");
    }

    #[test]
    fn style_with_url_is_dropped_whole() {
        let out = sanitize_html("<div style=\"background:url(https://evil.example/x)\">本文</div>");
        assert!(!out.contains("url("), "got: {out}");
        assert!(out.contains("本文"));
    }

    #[test]
    fn table_and_colspan_survive() {
        let out = sanitize_html("<table><tr><td colspan=\"2\">セル</td></tr></table>");
        assert!(out.contains("<table>"), "got: {out}");
        assert!(out.contains("colspan=\"2\""), "got: {out}");
    }

    #[test]
    fn scheme_relative_href_does_not_survive() {
        let out = sanitize_html("<a href=\"//evil.example/x\">リンク</a>");
        assert!(!out.contains("//evil.example"), "got: {out}");
    }

    #[test]
    fn https_link_survives() {
        let out = sanitize_html("<a href=\"https://example.com/page\">リンク</a>");
        assert!(
            out.contains("href=\"https://example.com/page\""),
            "got: {out}"
        );
    }

    #[test]
    fn uploaded_image_path_survives() {
        let out = sanitize_html("<img src=\"/static/images/uploads/a.png\" alt=\"写真\">");
        assert!(
            out.contains("src=\"/static/images/uploads/a.png\""),
            "got: {out}"
        );
        assert!(out.contains("alt=\"写真\""), "got: {out}");
    }

    #[test]
    fn scanner_names_a_javascript_href() {
        assert_eq!(
            find_invalid_url_html("<a href=\"javascript:alert(1)\">x</a>"),
            Some("javascript:alert(1)".to_string())
        );
    }

    #[test]
    fn scanner_names_a_scheme_relative_href() {
        assert_eq!(
            find_invalid_url_html("<a href='//evil.example/x'>x</a>"),
            Some("//evil.example/x".to_string())
        );
    }

    #[test]
    fn scanner_accepts_allowed_destinations() {
        let source = "<a href=\"/guide\">案内</a><img src=/static/a.png>\
                      <a HREF=\"https://example.com\">x</a><a href=\"#top\">top</a>";
        assert_eq!(find_invalid_url_html(source), None);
    }

    #[test]
    fn scanner_ignores_lookalike_attributes() {
        let source = "<img srcset=\"javascript:alert(1) 1x\" data-href=\"javascript:x\">";
        assert_eq!(find_invalid_url_html(source), None);
    }
}
