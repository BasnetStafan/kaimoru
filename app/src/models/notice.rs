//! Notice model.
//! One announcement shown in the top page news section.

use chrono::NaiveDate;

/// A store announcement from the notices table.
/// The date is preformatted as text by the query so templates never
/// need date handling.
#[derive(Clone, sqlx::FromRow)]
pub struct Notice {
    /// Publication date formatted as YYYY/MM/DD.
    pub date_text: String,
    /// Headline of the announcement; empty for rows written
    /// before the title column existed (migration 0019), which
    /// render as a plain dated line, exactly as before.
    pub title: String,
    /// Announcement text, in Japanese.
    pub body: String,
}

/// One visible notice row with its display window, before the
/// window is evaluated. The storefront caches these candidates
/// and applies the window against today on every request, so a
/// notice still appears and expires on its dates while cached.
#[derive(Clone, sqlx::FromRow)]
pub struct NoticeCandidate {
    /// Publication date formatted as YYYY/MM/DD: the display
    /// start, or the creation date when no start is set.
    pub date_text: String,
    /// Headline of the announcement.
    pub title: String,
    /// Announcement text, in Japanese.
    pub body: String,
    /// Display period start; `None` shows immediately.
    pub display_from: Option<NaiveDate>,
    /// Display period end; `None` never expires.
    pub display_to: Option<NaiveDate>,
}

impl NoticeCandidate {
    /// Whether the notice is live on the given JST date. This
    /// and the repository's storefront predicate are two
    /// spellings of one definition and must agree: both window
    /// ends inclusive, either end open.
    pub fn is_live_on(&self, today: NaiveDate) -> bool {
        self.display_from.is_none_or(|from| from <= today)
            && self.display_to.is_none_or(|to| to >= today)
    }

    /// Converts a live candidate into its storefront shape.
    pub fn to_notice(&self) -> Notice {
        Notice {
            date_text: self.date_text.clone(),
            title: self.title.clone(),
            body: self.body.clone(),
        }
    }
}
