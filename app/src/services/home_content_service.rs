//! Home content, storefront read side (CMS spec C-3).
//!
//! The top page's slots and sections change only when the
//! operator saves them, so the rows are read through a brief
//! in-process cache — the shop settings shape: an admin save
//! invalidates the cache at once, and the TTL only bounds how
//! long a change made outside the admin can linger. The
//! display *decision* is never cached: whether a slot is
//! inside its window is evaluated against now on every call,
//! so an ended campaign disappears on its own.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Utc;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{
    ANNOUNCEMENT_KEY, HERO_KEY, HeroImage, HomePin, HomeSection, HomeSlot, SECTION_KEYS,
};
use crate::repositories::home_content_repository;

/// How long the slot and section rows may be served from
/// memory before the database is asked again.
const CONTENT_TTL: Duration = Duration::from_secs(60);

/// The cached home content and when it was read.
static CONTENT_CACHE: Mutex<Option<(Instant, HomeContent)>> = Mutex::new(None);

/// The top page's operator content, assembled: both slots
/// and every section, with the canonical section set merged
/// over whatever rows the database holds, so a missing row
/// renders at its seeded defaults instead of vanishing.
#[derive(Clone)]
pub struct HomeContent {
    /// The announcement bar slot.
    pub announcement: HomeSlot,
    /// The hero slot.
    pub hero: HomeSlot,
    /// Every section, in canonical key order; call
    /// [`HomeContent::ordered_sections`] for render order.
    pub sections: Vec<HomeSection>,
    /// Every pinned product, in section and list order.
    pub pins: Vec<HomePin>,
    /// The hero's slides, in display order.
    pub hero_images: Vec<HeroImage>,
}

/// The live announcement bar's render data: the message and
/// its optional link, whose label and URL are empty together
/// when the bar has none.
pub struct AnnouncementParts {
    /// The bar's message text.
    pub text: String,
    /// Link label; empty when the bar has no link.
    pub link_label: String,
    /// Link URL; empty when the bar has no link.
    pub link_url: String,
}

impl HomeContent {
    /// The content with no database behind it: the
    /// announcement off, the hero at its built-in copy, every
    /// section at its seeded defaults.
    pub fn fallback() -> Self {
        Self {
            announcement: HomeSlot::fallback_announcement(),
            hero: HomeSlot::fallback_hero(),
            sections: SECTION_KEYS
                .iter()
                .map(|key| HomeSection::default_for(key))
                .collect(),
            pins: Vec::new(),
            hero_images: Vec::new(),
        }
    }

    /// Returns the announcement bar to render now, or `None`
    /// when the slot is off, outside its window, or has no
    /// text — an enabled-but-empty bar never renders.
    pub fn live_announcement(&self) -> Option<AnnouncementParts> {
        if !self.announcement.is_live_at(Utc::now()) || self.announcement.body.is_empty() {
            return None;
        }
        Some(AnnouncementParts {
            text: self.announcement.body.clone(),
            link_label: self.announcement.cta_label.clone(),
            link_url: self.announcement.cta_url.clone(),
        })
    }

    /// Returns the hero slot when it renders now: switched
    /// on, inside its window, and carrying something to
    /// show — an image, a headline, a body, or a complete
    /// button (label and link together). An enabled-but-
    /// empty hero never renders, the announcement's rule.
    pub fn live_hero(&self) -> Option<&HomeSlot> {
        if !self.hero.is_live_at(Utc::now()) {
            return None;
        }
        let has_copy = !self.hero.headline.is_empty() || !self.hero.body.is_empty();
        if self.hero_images.is_empty() && !has_copy {
            return None;
        }
        Some(&self.hero)
    }

    /// Returns the sections in render order: by the stored
    /// sort position, ties broken by canonical key order so
    /// the page is deterministic whatever the operator typed.
    /// Returns one section's pinned product ids, in pin
    /// order.
    pub fn pin_ids(&self, key: &str) -> Vec<i64> {
        self.pins
            .iter()
            .filter(|pin| pin.section_key == key)
            .map(|pin| pin.product_id)
            .collect()
    }

    pub fn ordered_sections(&self) -> Vec<HomeSection> {
        let mut sections = self.sections.clone();
        sections.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| canonical_index(&a.section_key).cmp(&canonical_index(&b.section_key)))
        });
        sections
    }
}

/// Returns a section key's canonical position, for ordering
/// ties; unknown keys go last.
fn canonical_index(key: &str) -> usize {
    SECTION_KEYS
        .iter()
        .position(|known| *known == key)
        .unwrap_or(usize::MAX)
}

/// Assembles the storefront content from raw rows: the two
/// slots picked out by key (fallback values when a row is
/// missing) and the canonical section set merged over the
/// stored rows. Shared by the storefront read and the admin
/// editor's fresh read, so both sides see the same page.
pub fn assemble(
    slots: Vec<HomeSlot>,
    stored_sections: Vec<HomeSection>,
    pins: Vec<HomePin>,
    hero_images: Vec<HeroImage>,
) -> HomeContent {
    let announcement = slots
        .iter()
        .find(|slot| slot.key == ANNOUNCEMENT_KEY)
        .cloned()
        .unwrap_or_else(HomeSlot::fallback_announcement);
    let hero = slots
        .iter()
        .find(|slot| slot.key == HERO_KEY)
        .cloned()
        .unwrap_or_else(HomeSlot::fallback_hero);
    let sections = SECTION_KEYS
        .iter()
        .map(|key| {
            stored_sections
                .iter()
                .find(|section| section.section_key == *key)
                .cloned()
                .unwrap_or_else(|| HomeSection::default_for(key))
        })
        .collect();
    HomeContent {
        announcement,
        hero,
        sections,
        pins,
        hero_images,
    }
}

/// Returns the current home content, from the brief cache
/// when fresh.
///
/// A failed read renders with [`HomeContent::fallback`]
/// instead of failing: the words layer must never take the
/// store down (the shop settings rule). The fallback is not
/// cached, so a recovered database shows on the next
/// request, not a TTL later.
pub async fn load(pool: &PgPool) -> HomeContent {
    if let Ok(cache) = CONTENT_CACHE.lock()
        && let Some((read_at, content)) = cache.as_ref()
        && read_at.elapsed() < CONTENT_TTL
    {
        return content.clone();
    }
    let content = match read(pool).await {
        Ok(content) => content,
        Err(_) => return HomeContent::fallback(),
    };
    if let Ok(mut cache) = CONTENT_CACHE.lock() {
        *cache = Some((Instant::now(), content.clone()));
    }
    content
}

/// Reads the rows and assembles them; the one database path
/// behind [`load`].
async fn read(pool: &PgPool) -> Result<HomeContent, AppError> {
    let slots = home_content_repository::list_slots(pool).await?;
    let sections = home_content_repository::list_sections(pool).await?;
    let pins = home_content_repository::list_pins(pool).await?;
    let hero_images = home_content_repository::list_hero_images(pool).await?;
    Ok(assemble(slots, sections, pins, hero_images))
}

/// Drops the cached rows, so the next [`load`] re-reads
/// them. The admin home content service calls this after a
/// save commits: the operator's new words are live on the
/// next page, not up to a TTL later.
pub fn invalidate_cache() {
    if let Ok(mut cache) = CONTENT_CACHE.lock() {
        *cache = None;
    }
}
