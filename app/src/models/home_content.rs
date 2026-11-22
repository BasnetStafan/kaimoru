//! Home content models (CMS spec C-3).
//!
//! The top page's operator-edited words: two keyed slots —
//! the announcement bar and the hero — and the fixed set of
//! sections whose heading, visibility and order the operator
//! controls. Section markup always stays in the templates;
//! these rows carry words and switches only.

use chrono::{DateTime, Utc};

/// The announcement slot's key in `home_slots`.
pub const ANNOUNCEMENT_KEY: &str = "announcement";

/// The hero slot's key in `home_slots`.
pub const HERO_KEY: &str = "hero";

/// The top page's section keys, in their seeded (default)
/// render order. The set is fixed in code — a section exists
/// because a template renders it — and the database carries
/// only each one's heading override, visibility and sort
/// position.
pub const SECTION_KEYS: [&str; 7] = [
    "categories",
    "new_arrivals",
    "free_space",
    "ranking",
    "featured",
    "staff_picks",
    "notices",
];

/// The sections whose rows an operator can pin products
/// into (spec: HOME_PINS_AND_STAFF_PICKS). The staff picks
/// row is flag-driven and the notices row carries no
/// products, so neither takes pins.
pub const PIN_SECTION_KEYS: [&str; 4] = ["new_arrivals", "ranking", "event", "featured"];

/// Returns a section's default sort position: its place
/// among [`SECTION_KEYS`], spaced so insertions have room
/// between neighbours. Unknown keys sort last.
pub fn default_sort_order(key: &str) -> i32 {
    SECTION_KEYS
        .iter()
        .position(|known| *known == key)
        .map(|index| (index as i32 + 1) * 10)
        .unwrap_or(999)
}

/// Returns a section's built-in heading, the words the top
/// page carried before C-3, shown while its override is
/// empty.
pub fn default_heading(key: &str) -> &'static str {
    match key {
        "categories" => "カテゴリーから探す",
        "new_arrivals" => "新着商品",
        "free_space" => "AIエージェントにおまかせ",
        "ranking" => "ランキング",
        "featured" => "特集:今月のピックアップ",
        "staff_picks" => "スタッフおすすめ商品",
        "notices" => "お知らせ",
        _ => "",
    }
}

/// Returns a section's built-in description, shown while
/// its override is empty. Only the featured section
/// renders a description today: the sentence its banner
/// and feature page have always carried.
pub fn default_description(key: &str) -> &'static str {
    match key {
        "featured" => "スタッフが選んだ、今月のおすすめ商品をまとめました。",
        "free_space" => {
            "「予算5,000円で、評価の高いグッズを探して」と頼むだけ。エージェントが商品を比べてカートに入れます。購入の決定は、必ずあなたが承認してから。"
        }
        _ => "",
    }
}

/// Returns a section's Japanese name for the admin form.
pub fn section_label(key: &str) -> &'static str {
    match key {
        "categories" => "カテゴリー",
        "new_arrivals" => "新着商品",
        "free_space" => "フリースペース",
        "ranking" => "ランキング",
        "featured" => "特集",
        "staff_picks" => "スタッフおすすめ",
        "notices" => "お知らせ",
        _ => "",
    }
}

/// Returns the canonical key literal for a stored section
/// key, so templates can compare against literals. Only the
/// [`SECTION_KEYS`] exist; anything else maps to the empty
/// string and renders nothing.
pub fn canonical_key(key: &str) -> &'static str {
    match key {
        "categories" => "categories",
        "new_arrivals" => "new_arrivals",
        "free_space" => "free_space",
        "ranking" => "ranking",
        "featured" => "featured",
        "staff_picks" => "staff_picks",
        "notices" => "notices",
        _ => "",
    }
}

/// Returns the instant a fallback row pretends to carry:
/// fallback rows are never displayed, so the epoch stands in
/// for "never saved".
fn never_saved() -> DateTime<Utc> {
    DateTime::from_timestamp(0, 0).expect("the epoch is a valid timestamp")
}

/// One keyed slot row: the announcement bar or the hero.
///
/// The two slots share one shape: `headline` and
/// `body` are the slot's main words — for the announcement
/// the text lives in `body` and `headline` stays unused —
/// and `cta_label` / `cta_url` are its optional link
/// (the announcement's page link; the hero's own link
/// fields are retired — its slides carry the links).
#[derive(Clone, sqlx::FromRow)]
pub struct HomeSlot {
    /// Slot key: [`ANNOUNCEMENT_KEY`] or [`HERO_KEY`].
    pub key: String,
    /// Headline text (the hero's `h1`; unused by the
    /// announcement).
    pub headline: String,
    /// Body text: the hero's subtext, the announcement's
    /// message.
    pub body: String,
    /// Link label; empty together with the URL when the slot
    /// has no link.
    pub cta_label: String,
    /// Link URL; empty together with the label when the slot
    /// has no link.
    pub cta_url: String,
    /// Whether the operator has switched the slot on.
    pub is_enabled: bool,
    /// Display window start; `None` opens immediately.
    pub starts_at: Option<DateTime<Utc>>,
    /// Display window end; `None` never closes.
    pub ends_at: Option<DateTime<Utc>>,
    /// When the row was last saved.
    pub updated_at: DateTime<Utc>,
}

/// One hero slide: an uploaded image and the link a click
/// on it follows (the storefront overlays the hero copy on
/// the slides; there is no hero button). An empty link
/// leaves the slide unlinked.
#[derive(Clone, Debug, PartialEq, sqlx::FromRow)]
pub struct HeroImage {
    /// The image's uploads-store path.
    pub image_path: String,
    /// The click destination; empty when unlinked.
    pub link_url: String,
}

impl HomeSlot {
    /// Whether the slot renders at `now`: switched on and
    /// inside its window. The window is half-open — it opens
    /// at `starts_at` and has closed by `ends_at` — and
    /// either end may be open. This is evaluated on every
    /// request, never cached: an ended campaign disappears
    /// on its own.
    pub fn is_live_at(&self, now: DateTime<Utc>) -> bool {
        self.is_enabled
            && self.starts_at.is_none_or(|start| start <= now)
            && self.ends_at.is_none_or(|end| now < end)
    }

    /// The announcement slot with no row behind it: switched
    /// off and empty, so a missing row can never invent a
    /// bar the operator never wrote.
    pub fn fallback_announcement() -> Self {
        Self {
            key: ANNOUNCEMENT_KEY.to_string(),
            headline: String::new(),
            body: String::new(),
            cta_label: String::new(),
            cta_url: String::new(),
            is_enabled: false,
            starts_at: None,
            ends_at: None,
            updated_at: never_saved(),
        }
    }

    /// The hero slot with no row behind it: the copy the top
    /// page template carried before C-3, switched on — the
    /// page keeps its hero if the row is ever missing.
    pub fn fallback_hero() -> Self {
        Self {
            key: HERO_KEY.to_string(),
            headline: "好きなものを、もっと手軽に。".to_string(),
            body: "ふだんの買い物はそのままに。AIエージェントにも頼める、新しいECストア。"
                .to_string(),
            cta_label: "新着商品を見る".to_string(),
            cta_url: "#new".to_string(),
            is_enabled: true,
            starts_at: None,
            ends_at: None,
            updated_at: never_saved(),
        }
    }
}

/// One section row: an operator's heading override,
/// visibility switch and sort position for a section whose
/// markup lives in the top page template.
#[derive(Clone, sqlx::FromRow)]
pub struct HomeSection {
    /// Section key, one of [`SECTION_KEYS`].
    pub section_key: String,
    /// Heading override; empty shows the built-in heading.
    pub heading: String,
    /// Description override; empty shows the built-in
    /// description. Rendered by the featured section's
    /// banner and feature page.
    pub description: String,
    /// Whether the section renders.
    pub is_visible: bool,
    /// Render position; lower renders first.
    pub sort_order: i32,
    /// When the row was last saved.
    pub updated_at: DateTime<Utc>,
}

impl HomeSection {
    /// The heading the storefront shows: the operator's
    /// override, or the built-in default while it is empty.
    pub fn display_heading(&self) -> String {
        if self.heading.is_empty() {
            default_heading(&self.section_key).to_string()
        } else {
            self.heading.clone()
        }
    }

    /// The description the storefront shows: the operator's
    /// override, or the built-in default while it is empty.
    pub fn display_description(&self) -> String {
        if self.description.is_empty() {
            default_description(&self.section_key).to_string()
        } else {
            self.description.clone()
        }
    }

    /// The section at its seeded defaults — visible, no
    /// heading override, its canonical sort position — used
    /// when the row is missing, so the top page keeps every
    /// section even if a row is ever lost.
    pub fn default_for(key: &str) -> Self {
        Self {
            section_key: key.to_string(),
            heading: String::new(),
            description: String::new(),
            is_visible: true,
            sort_order: default_sort_order(key),
            updated_at: never_saved(),
        }
    }
}

/// One pinned product in one top page section: the
/// operator's choice of what leads the section's row,
/// ahead of its automatic selection.
#[derive(Clone, sqlx::FromRow)]
pub struct HomePin {
    /// Section key, one of [`PIN_SECTION_KEYS`].
    pub section_key: String,
    /// The pinned product.
    pub product_id: i64,
    /// Position inside the section's pin list; lower first.
    pub sort_order: i32,
}

/// One product as the pin editor offers it: every product,
/// any status — a draft may be pinned ahead of its launch
/// and simply renders once it is public.
#[derive(Clone, sqlx::FromRow)]
pub struct ProductOption {
    /// Product id.
    pub id: i64,
    /// Product name.
    pub name: String,
    /// Lifecycle status (`public`, `draft`, ...).
    pub status: String,
}

/// One stored pin joined with its product's name and
/// status, for the pin editor's rows.
#[derive(Clone, sqlx::FromRow)]
pub struct PinRow {
    /// Section key, one of [`PIN_SECTION_KEYS`].
    pub section_key: String,
    /// The pinned product.
    pub product_id: i64,
    /// Position inside the section's pin list; lower first.
    pub sort_order: i32,
    /// The pinned product's name.
    pub name: String,
    /// The pinned product's lifecycle status.
    pub status: String,
}
