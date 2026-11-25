//! Menus, storefront read side (CMS spec C-4).
//!
//! The layout's two link lists change only when the
//! operator saves them, so the rows are read through a
//! brief in-process cache — the home content shape: an
//! admin save invalidates the cache at once, and the TTL
//! only bounds how long a change made outside the admin
//! can linger. Visibility is a stored row property, not a
//! time decision, so filtering the cached rows at render
//! is exact (no window is ever cached shut or open).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{FOOTER_KEY, HEADER_KEY, MenuItem};
use crate::repositories::menu_repository;

/// How long the menu rows may be served from memory before
/// the database is asked again.
const MENU_TTL: Duration = Duration::from_secs(60);

/// The cached menus and when they were read.
static MENU_CACHE: Mutex<Option<(Instant, Menus)>> = Mutex::new(None);

/// The layout's two menus, assembled: every item of each,
/// in render order, hidden ones included — the storefront
/// filters visibility at render ([`Menus::visible_items`])
/// and the admin editor needs the whole list.
#[derive(Clone)]
pub struct Menus {
    /// The header menu's items, in render order.
    pub header: Vec<MenuItem>,
    /// The footer menu's items, in render order.
    pub footer: Vec<MenuItem>,
}

impl Menus {
    /// The menus with no database behind them: the header
    /// empty (it holds no links until the operator adds
    /// one) and the footer at its pre-C-4 built-in links,
    /// so a page renders its navigation even when the rows
    /// cannot be read.
    pub fn fallback() -> Self {
        Self {
            header: Vec::new(),
            footer: MenuItem::fallback_footer(),
        }
    }

    /// Returns one menu's items by its key; an unknown key
    /// has no items.
    pub fn items(&self, menu_key: &str) -> &[MenuItem] {
        match menu_key {
            HEADER_KEY => &self.header,
            FOOTER_KEY => &self.footer,
            _ => &[],
        }
    }

    /// Returns one menu's visible items, in render order:
    /// what the layout prints.
    pub fn visible_items(&self, menu_key: &str) -> Vec<MenuItem> {
        self.items(menu_key)
            .iter()
            .filter(|item| item.is_visible)
            .cloned()
            .collect()
    }
}

/// Assembles the two menus from raw rows, each menu keeping
/// the repository's render order. Shared by the storefront
/// read and the admin editor's fresh read, so both sides
/// see the same lists.
pub fn assemble(items: Vec<MenuItem>) -> Menus {
    // One pass, moving each item: the two filtered collects
    // this replaces cloned every item it kept.
    let mut menus = Menus {
        header: Vec::new(),
        footer: Vec::new(),
    };
    for item in items {
        if item.menu_key == HEADER_KEY {
            menus.header.push(item);
        } else if item.menu_key == FOOTER_KEY {
            menus.footer.push(item);
        }
    }
    menus
}

/// Returns the current menus, from the brief cache when
/// fresh.
///
/// A failed read renders with [`Menus::fallback`] instead
/// of failing: the words layer must never take the store
/// down (the shop settings rule). The fallback is not
/// cached, so a recovered database shows on the next
/// request, not a TTL later.
pub async fn load(pool: &PgPool) -> Menus {
    if let Ok(cache) = MENU_CACHE.lock()
        && let Some((read_at, menus)) = cache.as_ref()
        && read_at.elapsed() < MENU_TTL
    {
        return menus.clone();
    }
    let menus = match read(pool).await {
        Ok(menus) => menus,
        Err(_) => return Menus::fallback(),
    };
    if let Ok(mut cache) = MENU_CACHE.lock() {
        *cache = Some((Instant::now(), menus.clone()));
    }
    menus
}

/// Reads the rows and assembles them; the one database path
/// behind [`load`].
async fn read(pool: &PgPool) -> Result<Menus, AppError> {
    let items = menu_repository::list_all(pool).await?;
    Ok(assemble(items))
}

/// Drops the cached rows, so the next [`load`] re-reads
/// them. The admin menu service calls this after a save
/// commits: the operator's new links are live on the next
/// page, not up to a TTL later.
pub fn invalidate_cache() {
    if let Ok(mut cache) = MENU_CACHE.lock() {
        *cache = None;
    }
}
