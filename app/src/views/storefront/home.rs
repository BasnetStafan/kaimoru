//! Home view.
//! Data for the store top page template. Each field feeds one section of
//! the page, matching the top page spec.

use askama::Template;

use super::product_card::ProductCard;
use crate::models::{HeroImage, HomeSection, HomeSlot, Notice, canonical_key};
use crate::views::storefront::header::Header;

/// Renders `pages/home.html` inside the `base.html` layout block.
///
/// Data comes from the services layer via the home controller;
/// the template receives display-ready values only.
#[derive(Template)]
#[template(path = "storefront/pages/home.html")]
pub struct HomeView {
    /// Shared header state, including the login greeting and any
    /// one-time welcome message.
    pub header: Header,
    /// Category names for the category navigation.
    pub categories: Vec<String>,
    /// Whether more categories exist than the nav shows:
    /// the nav links to the category index only then.
    pub categories_more: bool,
    /// Products for the new arrivals section.
    pub new_products: Vec<ProductCard>,
    /// Products for the ranking section, in rank order.
    pub ranking_products: Vec<ProductCard>,
    /// Products for the event pickup row.
    pub event_products: Vec<ProductCard>,
    /// Products for the featured section: its pins only.
    /// Empty renders the section's banner instead.
    pub featured_products: Vec<ProductCard>,
    /// Products for the staff picks section.
    pub staff_products: Vec<ProductCard>,
    /// Announcements for the news section.
    pub notices: Vec<Notice>,
    /// The page's blocks in render order: the sections in
    /// their stored order, with the fixed blocks (hero,
    /// agent strip, event pickup) anchored after the
    /// sections they follow today. The template dispatches
    /// markup per block kind; markup never comes from the
    /// database.
    pub blocks: Vec<HomeBlock>,
    /// The hero slot's render data, when the hero is live.
    pub hero: Option<HeroView>,
}

/// One block of the top page in render order.
pub struct HomeBlock {
    /// Block kind: a section key (`categories`,
    /// `new_arrivals`, `ranking`, `featured`, `staff_picks`,
    /// `notices`) or
    /// a fixed block (`hero`, `agent`, `event`).
    pub key: &'static str,
    /// The section's display heading (its override, or the
    /// built-in default); empty for the fixed blocks, whose
    /// words live in the slot or the template.
    pub heading: String,
    /// The section's display description (its override, or
    /// the built-in default); only the featured block's
    /// banner renders it today.
    pub description: String,
    /// Link to the section's full listing page, set only
    /// when the section holds more products than its row
    /// displays (the see-more rule); empty renders no link.
    pub more_url: String,
}

impl HomeBlock {
    /// A block for one stored section, carrying its display
    /// heading.
    pub fn section(section: &HomeSection) -> Self {
        Self {
            key: canonical_key(&section.section_key),
            heading: section.display_heading(),
            description: section.display_description(),
            more_url: String::new(),
        }
    }

    /// A fixed block, whose markup carries its own words.
    pub fn fixed(key: &'static str) -> Self {
        Self {
            key,
            heading: String::new(),
            description: String::new(),
            more_url: String::new(),
        }
    }
}

/// The hero block's render data (CMS C-3): the slot's
/// headline and subtext — overlaid on the slides when the
/// hero has any — plus its slides (up to ten), each with
/// its own link.
pub struct HeroView {
    /// Headline text.
    pub headline: String,
    /// Subtext.
    pub text: String,
    /// Slides, in display order; empty renders the
    /// text-only hero.
    pub images: Vec<HeroImage>,
}

impl HeroView {
    /// Builds the render data from a live hero slot and
    /// its slide list.
    pub fn from_slot(slot: &HomeSlot, images: &[HeroImage]) -> Self {
        Self {
            headline: slot.headline.clone(),
            text: slot.body.clone(),
            images: images.to_vec(),
        }
    }
}
