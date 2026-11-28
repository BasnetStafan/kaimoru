//! Menu models (CMS spec C-4).
//!
//! The two link lists the base layout renders — the header
//! menu and the footer menu — as operator-edited rows. The
//! header's session-functional links (cart, login, my page)
//! are state, not links, and stay in the template; these
//! rows carry words and switches only.

/// The header menu's key in `menu_items`.
pub const HEADER_KEY: &str = "header";

/// The footer menu's key in `menu_items`.
pub const FOOTER_KEY: &str = "footer";

/// The menu keys, in the admin form's display order. The
/// set is fixed in code — a menu exists because the layout
/// renders it.
pub const MENU_KEYS: [&str; 2] = [HEADER_KEY, FOOTER_KEY];

/// Returns a menu's Japanese name for the admin form.
pub fn menu_label(key: &str) -> &'static str {
    match key {
        "header" => "ヘッダーメニュー",
        "footer" => "フッターメニュー",
        _ => "",
    }
}

/// One menu link row: a label, its target, and the
/// visibility and position the operator controls.
#[derive(Clone, sqlx::FromRow)]
pub struct MenuItem {
    /// Row id; the admin form carries it so a save can tell
    /// stored rows from new ones.
    pub id: i64,
    /// Owning menu: [`HEADER_KEY`] or [`FOOTER_KEY`].
    pub menu_key: String,
    /// Link text: plain text, escaped at render.
    pub label: String,
    /// Link target: an internal path, a page fragment, or
    /// an `https:` URL — the save-side rule is
    /// `link_url_allowed` in `crate::validation`.
    pub url: String,
    /// Whether the link renders.
    pub is_visible: bool,
    /// Render position within the menu; lower renders first.
    pub sort_order: i32,
}

impl MenuItem {
    /// Whether the link is external: an `https:` URL, which
    /// the layout renders with `rel="noopener"`. Internal
    /// paths and fragments are the store's own.
    pub fn is_external(&self) -> bool {
        self.url.to_ascii_lowercase().starts_with("https://")
    }

    /// The footer menu with no database behind it: the five
    /// links the layout hard-coded before C-4, in their
    /// order. A page renders this footer when the rows
    /// cannot be read (the 404 fallback builds its header
    /// with no database work at all). The header menu's
    /// fallback is the empty list — it holds no links until
    /// the operator adds one — so it needs no function.
    pub fn fallback_footer() -> Vec<MenuItem> {
        [
            ("送料・お支払いについて", "/guide"),
            ("お問い合わせ", "/contact"),
            ("利用規約", "/terms"),
            ("プライバシーポリシー", "/privacy"),
            ("特定商取引法に基づく表記", "/legal"),
        ]
        .iter()
        .enumerate()
        .map(|(index, (label, url))| MenuItem {
            id: 0,
            menu_key: FOOTER_KEY.to_string(),
            label: label.to_string(),
            url: url.to_string(),
            is_visible: true,
            sort_order: (index as i32 + 1) * 10,
        })
        .collect()
    }
}
