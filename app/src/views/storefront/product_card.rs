//! Shop-wide product card parts: the card itself and
//! the quick-add / buy-box fragments other pages embed.

use super::product::{VariantPicker, stock_wording};
use crate::models::Product;
use crate::models::review::RatingAggregate;

/// One product card wherever cards are listed (top page, search
/// results, the detail page's same-category row): the product plus
/// its computed review aggregate. The aggregate is attached by the
/// controller in one grouped query for the whole list, never one
/// query per card.
pub struct ProductCard {
    /// The product on the card.
    pub product: Product,
    /// Its rating from real reviews; no average at 0 reviews.
    pub rating: RatingAggregate,
    /// Whether the viewing member has favourited this product
    /// (guests always false), resolved in bulk for the page.
    pub is_favorite: bool,
    /// Whether the viewer is a signed-in member: members get
    /// the toggle form, guests a login link.
    pub is_member: bool,
    /// What the card's action area offers. Catalog pages always
    /// use the storefront behaviour; the shopping agent's result
    /// cards override it, because their picks are already cart
    /// lines (or addable only through the agent's own endpoint).
    pub action: ProductCardAction,
}

/// Renders parts/quick_add.html: the quick-add modal fragment
/// (product summary, the shared variation picker and the add
/// form) loaded by the listing cards' 選択する button.
#[derive(askama::Template)]
#[template(path = "storefront/parts/quick_add.html")]
pub struct QuickAddView {
    /// The product whose combinations can be added.
    pub product: crate::models::Product,
    /// The variation picker state (axes + combinations).
    pub picker: VariantPicker,
}

impl QuickAddView {
    /// The stock the fragment's stock line refers to: the
    /// selected combination's stock on a variant product, the
    /// product total otherwise (see [`crate::views::storefront::product::ProductDetailView`]).
    pub fn label_stock(&self) -> i32 {
        if self.picker.has_picker() {
            self.picker.initial_stock()
        } else {
            self.product.stock
        }
    }

    /// The stock line under the price, kept current by the
    /// picker script as the selection changes.
    pub fn stock_label(&self) -> String {
        stock_wording(
            self.label_stock(),
            self.product.track_stock,
            self.product.allow_backorder,
            self.product.is_preorder,
            self.product.low_stock_threshold(),
        )
    }
}

/// Renders parts/product_buybox.html on its own: the detail
/// page's buy area in its resting state, used by the
/// self-restoring "added" fragment on the detail page.
#[derive(askama::Template)]
#[template(path = "storefront/parts/product_buybox.html")]
pub struct BuyboxView {
    /// The product whose buy area this is.
    pub product: crate::models::Product,
    /// The variation picker state (axes + combinations).
    pub picker: VariantPicker,
    /// Whether the viewing member has favourited this product
    /// (the toggle's state; guests always false).
    pub is_favorite: bool,
    /// Whether the viewer is a signed-in member.
    pub is_member: bool,
}

/// What a product card's cart-action area offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductCardAction {
    /// The storefront default: the sold-out state, the variant
    /// chooser or the plain add form, by the product's own state.
    Storefront,
    /// Already in the viewer's cart (an agent pick): a disabled
    /// confirmation, never a live add.
    InCart,
    /// Add this variant through the shopping agent's add-anyway
    /// endpoint: a deliberate budget crossing, offered only for
    /// a variant the visitor's own run refused.
    AddAnyway {
        /// The refused variant the button posts.
        variant_id: i64,
    },
    /// Apply this one pick of the agent's latest proposal:
    /// the button posts to the apply-pick endpoint, which
    /// adds exactly this variant as an agent line. A proposal
    /// pick is never in the cart until the shopper clicks.
    ApplyPick {
        /// The proposed variant the button posts.
        variant_id: i64,
    },
}

impl ProductCard {
    /// Whether the card's action area is the agent's
    /// already-in-cart state.
    pub fn action_is_in_cart(&self) -> bool {
        matches!(self.action, ProductCardAction::InCart)
    }

    /// Whether the card's action area is the storefront default.
    pub fn action_is_storefront(&self) -> bool {
        matches!(self.action, ProductCardAction::Storefront)
    }

    /// The variant the card's add-anyway button posts, when the
    /// action is the agent's.
    pub fn add_anyway_variant(&self) -> Option<i64> {
        match self.action {
            ProductCardAction::AddAnyway { variant_id } => Some(variant_id),
            _ => None,
        }
    }

    /// The variant the card's apply-pick button posts, when
    /// the action is a chat proposal pick.
    pub fn apply_pick_variant(&self) -> Option<i64> {
        match self.action {
            ProductCardAction::ApplyPick { variant_id } => Some(variant_id),
            _ => None,
        }
    }
}
