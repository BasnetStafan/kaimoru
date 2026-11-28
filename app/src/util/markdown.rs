//! Markdown rendering for content pages (CMS spec D2).
//!
//! Page bodies are a Markdown subset rendered with
//! `pulldown-cmark`. Safety is by construction, never by a
//! sanitiser's configuration: raw HTML in the source is turned
//! back into plain text (so it renders as escaped characters),
//! and link/image destinations are restricted to a scheme
//! allowlist — checked at save time by the admin service and
//! re-checked here at render, so a body that predates a rule
//! change still cannot smuggle a `javascript:` URL onto a page.

use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, html};

/// Options for the page-body subset: tables and strikethrough
/// are the only extensions on — legal pages want tables, and
/// neither extension can produce markup the renderer does not
/// control.
fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH
}

/// Whether a link or image destination may appear on a rendered
/// page: an internal absolute path, a same-page fragment, or an
/// `https:` / `http:` / `mailto:` URL. Protocol-relative URLs,
/// script schemes, bare relative paths and anything carrying
/// whitespace or control characters are refused — a browser
/// would strip those characters out of a scheme and could be
/// left holding `javascript:`.
pub fn url_allowed(url: &str) -> bool {
    let url = url.trim();
    if url.is_empty() {
        return true;
    }
    if url.chars().any(|c| c <= ' ') {
        return false;
    }
    if url.starts_with('/') {
        return !url.starts_with("//");
    }
    if url.starts_with('#') {
        return true;
    }
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https:") || lower.starts_with("http:") || lower.starts_with("mailto:")
}

/// Returns the first link or image destination in `source` that
/// the allowlist refuses, so the save-time error can name the
/// offending link. `None` means every destination is allowed.
pub fn find_invalid_url(source: &str) -> Option<String> {
    for event in Parser::new_ext(source, options()) {
        let dest = match &event {
            Event::Start(Tag::Link { dest_url, .. }) => dest_url,
            Event::Start(Tag::Image { dest_url, .. }) => dest_url,
            _ => continue,
        };
        if !url_allowed(dest) {
            return Some(dest.to_string());
        }
    }
    None
}

/// Renders one page body to HTML.
///
/// Raw HTML events become text events, which the HTML writer
/// escapes — the operator's `<b>` shows as literal characters,
/// never as markup. A link or image whose destination fails the
/// allowlist is re-emitted with an empty destination, so its
/// text survives but it points nowhere.
pub fn render_markdown(source: &str) -> String {
    let events = Parser::new_ext(source, options()).map(|event| match event {
        // Raw HTML is source text, not markup: hand it back as a
        // text event and the writer escapes it.
        Event::Html(html) => Event::Text(html),
        Event::InlineHtml(html) => Event::Text(html),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: checked_dest(dest_url),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: checked_dest(dest_url),
            title,
            id,
        }),
        other => other,
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}

/// Returns the destination unchanged when allowed, or an empty
/// destination when it is not (the defence-in-depth half of the
/// allowlist: save refuses these, render neutralises them).
fn checked_dest(dest: CowStr<'_>) -> CowStr<'_> {
    if url_allowed(&dest) {
        dest
    } else {
        CowStr::Borrowed("")
    }
}

#[cfg(test)]
mod tests {
    use super::{find_invalid_url, render_markdown, url_allowed};

    #[test]
    fn allowlist_accepts_the_documented_shapes() {
        assert!(url_allowed("https://example.com/x"));
        assert!(url_allowed("http://example.com"));
        assert!(url_allowed("mailto:shop@example.com"));
        assert!(url_allowed("/guide"));
        assert!(url_allowed("#section"));
        assert!(url_allowed(""));
    }

    #[test]
    fn allowlist_refuses_script_and_smuggled_schemes() {
        assert!(!url_allowed("javascript:alert(1)"));
        assert!(!url_allowed("JaVaScRiPt:alert(1)"));
        assert!(!url_allowed("java\tscript:alert(1)"));
        assert!(!url_allowed("data:text/html;base64,PHNjcmlwdD4="));
        assert!(!url_allowed("//evil.example/x"));
        assert!(!url_allowed("relative/path"));
    }

    #[test]
    fn raw_html_renders_as_text() {
        let html = render_markdown("前 <script>alert(1)</script> 後");
        assert!(html.contains("&lt;script&gt;"), "{html}");
        assert!(!html.contains("<script>alert"), "{html}");
    }

    #[test]
    fn headings_lists_and_links_render() {
        let html = render_markdown("## 見出し\n\n- 項目1\n- 項目2\n\n[案内](/guide)");
        assert!(html.contains("<h2>見出し</h2>"), "{html}");
        assert!(html.contains("<li>項目1</li>"), "{html}");
        assert!(html.contains("href=\"/guide\""), "{html}");
    }

    #[test]
    fn disallowed_link_is_neutralised_at_render() {
        let html = render_markdown("[x](javascript:alert(1))");
        assert!(!html.contains("javascript:"), "{html}");
        assert!(html.contains(">x</a>"), "{html}");
    }

    #[test]
    fn find_invalid_url_names_the_offender() {
        assert_eq!(
            find_invalid_url("[ok](/guide) [bad](javascript:alert(1))"),
            Some("javascript:alert(1)".to_string())
        );
        assert_eq!(
            find_invalid_url("![img](data:image/png;base64,x)"),
            Some("data:image/png;base64,x".to_string())
        );
        assert_eq!(find_invalid_url("[ok](https://example.com)"), None);
    }
}
