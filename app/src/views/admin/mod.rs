//! Admin view structs.
//! Each struct is the typed data one Askama template under
//! `templates/admin/` receives. Mirrors the storefront's views
//! module on the other surface.

pub mod account_settings;
pub mod admins;
pub mod analytics;
pub mod audit;
pub mod categories;
pub mod content_pages;
pub mod coupons;
pub mod customers;
pub mod dashboard;
pub mod holidays;
pub mod home_content;
pub mod inquiries;
pub mod login;
pub mod mail;
pub mod menus;
pub mod notfound;
pub mod notices;
pub mod orders;
pub mod payments;
pub mod products;
pub mod returns;
pub mod reviews;
pub mod rfm;
pub mod segments;
pub mod shop_settings;
pub mod tags;
pub mod tax;
pub mod totp;

/// Pagination state shared by every admin list: the current page,
/// the totals, and the list's base URL (with its filter query
/// string) that page links extend with `&page=N`. The window
/// shows up to five page numbers around the current one;
/// previous/next links appear only when they have somewhere to go.
pub struct Page {
    /// Current page number, 1-based.
    pub current: i64,
    /// Total number of pages (at least 1, even when empty).
    pub total_pages: i64,
    /// Total number of matching rows.
    pub total: i64,
    /// Base URL including existing filters, without `page`.
    /// Always ends so that appending `page=N` — or `&page=N`
    /// when a query string is present — is correct; callers
    /// build it via [`Page::base_with`].
    pub base: String,
}

impl Page {
    /// Builds the state for one list.
    pub fn new(current: i64, total: i64, per_page: i64, base: String) -> Page {
        let total_pages = crate::util::pagination::total_pages(total, per_page).max(1);
        Page {
            current: current.clamp(1, total_pages),
            total_pages,
            total,
            base,
        }
    }

    /// The base URL for a list path plus a raw query string
    /// (already URL-encoded by the caller, without `page`).
    pub fn base_with(path: &str, query: &str) -> String {
        if query.is_empty() {
            format!("{path}?")
        } else {
            format!("{path}?{query}&")
        }
    }

    /// The URL of page `n` of this list.
    pub fn url(&self, n: i64) -> String {
        format!("{}page={n}", self.base)
    }

    /// The previous page's URL, when there is one.
    pub fn prev_url(&self) -> Option<String> {
        (self.current > 1).then(|| self.url(self.current - 1))
    }

    /// The next page's URL, when there is one.
    pub fn next_url(&self) -> Option<String> {
        (self.current < self.total_pages).then(|| self.url(self.current + 1))
    }

    /// The page numbers to render: a window of up to five
    /// centred on the current page, clamped to the real range.
    pub fn window(&self) -> Vec<i64> {
        let start = (self.current - 2).max(1);
        let end = (start + 4).min(self.total_pages);
        let start = (end - 4).max(1);
        (start..=end).collect()
    }

    /// The window as ready-to-render links, so the pagination
    /// partial never does arithmetic or method juggling itself.
    pub fn links(&self) -> Vec<PageLink> {
        self.window()
            .iter()
            .map(|&n| PageLink {
                n,
                url: self.url(n),
                current: n == self.current,
            })
            .collect()
    }
}

/// Percent-encodes one query value (UTF-8 bytes), leaving
/// the RFC 3986 unreserved set untouched. Shared by the admin
/// list controllers (code review R3-11): products and orders
/// each carried a verbatim copy of this function.
pub fn urlencode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Builds a URL query string from (name, value) pairs,
/// skipping the pairs whose value is `None` and
/// percent-encoding every value with [`urlencode`]. The names
/// are the controllers' own fixed field names, never user
/// input. The shared tail of the admin lists' filter URLs
/// (code review R3-11).
pub fn filter_query(pairs: &[(&str, Option<&str>)]) -> String {
    pairs
        .iter()
        .filter_map(|(name, value)| value.map(|v| format!("{name}={}", urlencode(v))))
        .collect::<Vec<_>>()
        .join("&")
}

/// One numbered link of the pagination partial.
pub struct PageLink {
    /// Page number to show.
    pub n: i64,
    /// Its URL.
    pub url: String,
    /// Whether it is the current page.
    pub current: bool,
}
