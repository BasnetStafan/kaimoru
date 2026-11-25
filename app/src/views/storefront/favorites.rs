//! Favourites views: the toggle fragment and the mypage list.

use askama::Template;

use crate::views::storefront::header::Header;
use crate::views::storefront::product_card::ProductCard;

/// Renders parts/favorite_toggle.html: one product's heart
/// toggle. Members get the htmx form that swaps this fragment
/// in place; guests get a plain link to the login page (their
/// tap is a navigation, never a POST). Also rendered directly
/// by the toggle endpoint as the swap answer.
#[derive(Template)]
#[template(path = "storefront/parts/favorite_toggle.html")]
pub struct FavoriteToggleView {
    /// The product the toggle acts on.
    pub product_id: i64,
    /// Whether the viewing member has favourited it.
    pub is_favorite: bool,
    /// True renders the labelled button (the detail buybox);
    /// false the icon-only card overlay.
    pub labelled: bool,
    /// True renders the member form; false the guest link.
    pub member: bool,
}

/// Renders pages/mypage/favorites.html: the member's
/// favourited products as cards.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/favorites.html")]
pub struct FavoritesPageView {
    /// Shared header state.
    pub header: Header,
    /// The favourited products' cards, newest mark first.
    pub cards: Vec<ProductCard>,
}
