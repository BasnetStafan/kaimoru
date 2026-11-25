//! View structs for the product detail page.

use askama::Template;

use super::header::Header;
use super::product_card::ProductCard;
use crate::models::Product;
use crate::models::review::{DistributionRow, RatingAggregate, Review};
use crate::models::variant::{AxisWithOptions, ProductVariant};

/// The variation picker state of one product: its axes (with
/// options) and its combinations in cartesian order. An
/// axis-less product has one default variant and no picker.
pub struct VariantPicker {
    /// Axes with options, in display order.
    pub axes: Vec<AxisWithOptions>,
    /// Combinations in cartesian (grid) order.
    pub variants: Vec<ProductVariant>,
}

impl VariantPicker {
    /// Whether the picker renders at all: only a product with
    /// more than one combination has a choice to offer.
    pub fn has_picker(&self) -> bool {
        self.variants.len() > 1
    }

    /// The combination the page starts on: the first sellable
    /// one in cartesian order. Needs the product for its stock
    /// mode, so the views compute it where both are at hand;
    /// [`VariantPicker::initial_stock`] serves axis-less reads.
    pub fn initial_variant(&self) -> Option<&ProductVariant> {
        self.variants.iter().find(|v| v.stock > 0 && v.is_enabled)
    }

    /// The starting combination's ID (0 when none is in stock;
    /// the add form is disabled in that state anyway).
    pub fn initial_variant_id(&self) -> i64 {
        self.initial_variant().map(|v| v.id).unwrap_or(0)
    }

    /// The starting combination's stock: the detail page's
    /// initial stock label and quantity cap.
    pub fn initial_stock(&self) -> i32 {
        self.initial_variant().map(|v| v.stock).unwrap_or(0)
    }

    /// The picker's data block as JSON: axes with their options,
    /// and each combination's id, signature and stock. The
    /// page's vanilla JS drives selection state from this alone.
    pub fn picker_json(&self, product: &Product) -> String {
        let axes: Vec<_> = self
            .axes
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.axis.id,
                    "name": a.axis.name,
                    "options": a.options.iter().map(|o| {
                        serde_json::json!({ "id": o.id, "value": o.value })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();
        let variants: Vec<_> = self
            .variants
            .iter()
            .map(|v| {
                serde_json::json!({
                    "id": v.id,
                    "signature": v.option_signature,
                    "stock": v.stock,
                    "price": v.price_yen,
                    "image": v.image_path,
                    "enabled": v.is_enabled,
                })
            })
            .collect();
        crate::util::json_for_script::json_for_script(&serde_json::json!({
            "axes": axes,
            "variants": variants,
            "trackStock": product.track_stock,
            "allowBackorder": product.allow_backorder,
            "threshold": product.low_stock_threshold(),
            "productPrice": product.effective_price_yen(),
        }))
    }
}

/// The stock line wording for one stock level, shared by the
/// product-total label and the selected combination's label.
/// It mirrors the picker script's wording exactly, so the
/// server-rendered first paint already says what the script
/// would: an untracked product is simply in stock, and a
/// backorderable one at zero is 入荷次第発送, never 売り切れ.
pub(crate) fn stock_wording(
    stock: i32,
    track_stock: bool,
    allow_backorder: bool,
    is_preorder: bool,
    threshold: i32,
) -> String {
    if !track_stock {
        "在庫あり".to_string()
    } else if is_preorder && stock > 0 {
        "予約受付中".to_string()
    } else if stock == 0 {
        if allow_backorder {
            "入荷次第発送".to_string()
        } else {
            "売り切れ".to_string()
        }
    } else if stock <= threshold {
        format!("残りわずか（あと{}点）", stock)
    } else {
        "在庫あり".to_string()
    }
}

/// One image of the detail page's gallery: the uploaded
/// original plus its alt text. The resized derivatives the
/// strip and the main view serve are derived from the
/// original's path by [`crate::util::image_paths`]; the
/// original itself remains the lightbox view and the
/// fallback when a derivative file is missing.
pub struct GalleryImage {
    /// The original upload's path, under /static.
    pub original: String,
    /// The image's stored alt text; empty when none was set.
    pub alt: String,
}

impl GalleryImage {
    /// The strip-sized derivative path for this image.
    pub fn thumb(&self) -> String {
        crate::util::image_paths::thumb_path(&self.original)
    }

    /// The main-view-sized derivative path for this image.
    pub fn medium(&self) -> String {
        crate::util::image_paths::medium_path(&self.original)
    }
}

/// Renders pages/products/detail.html.
#[derive(Template)]
#[template(path = "storefront/pages/products/detail.html")]
pub struct ProductDetailView {
    /// Shared header state.
    pub header: Header,
    /// The product being shown.
    pub product: Product,
    /// The product's tax class Japanese name, resolved with
    /// today's rate by the controller, for the spec table's
    /// 消費税区分 row and the price decomposition.
    pub tax_class_name: String,
    /// The tax class's rate in force today, in whole percent.
    pub tax_rate_percent: i32,
    /// The product's gallery images in display order, main
    /// first; a single entry renders the plain image with no
    /// thumbnail strip.
    pub images: Vec<GalleryImage>,
    /// The product's tags, for the spec table's tag row.
    pub tags: Vec<crate::models::Tag>,
    /// Whether the product's linked category is active: the
    /// category name links to its listing only while it is —
    /// a retired category renders as plain text, never a dead
    /// navigation into a hidden listing.
    pub category_active: bool,
    /// The variation picker state (axes + combinations).
    pub picker: VariantPicker,
    /// The product's computed rating, for the buy panel line and
    /// the review summary.
    pub rating: RatingAggregate,
    /// Same-category products for the bottom row (up to 4).
    pub related: Vec<ProductCard>,
    /// One page of the product's reviews, in display order.
    pub reviews: Vec<ReviewCardView>,
    /// Reviews across all pages; 0 renders the empty state.
    pub review_total: i64,
    /// Points one item earns at the effective rate (the
    /// product's override, else the shop rate), when the
    /// programme is on and the amount is positive: the
    /// buy-panel points line (points override layer).
    pub points_for_one: Option<i64>,
    /// Star distribution rows, 5 down to 1.
    pub distribution: Vec<DistributionRow>,
    /// The active review sort ("new", "high" or "low").
    pub review_sort: String,
    /// Current review page, 1-based.
    pub review_page: i64,
    /// Total review pages.
    pub review_total_pages: i64,
    /// The payment methods checkout currently offers, by
    /// display label: the buy panel names exactly these, from
    /// the same source the checkout step offers.
    pub payment_method_labels: Vec<String>,
    /// The flat shipping fee, formatted (e.g. "￥550") — the
    /// shop settings' value, so the panel cannot drift.
    pub shipping_fee_label: String,
    /// The earliest delivery date's label under the shop's
    /// offsets, precomputed by the controller.
    pub delivery_earliest_label: String,
    /// The delivery window's range label under the shop's
    /// offsets, precomputed by the controller.
    pub delivery_range_label: String,
    /// The free-shipping threshold, formatted (e.g. "￥5,000").
    pub free_shipping_threshold_label: String,
    /// The cash-on-delivery fee, formatted (e.g. "￥330").
    pub cod_fee_label: String,
    /// Whether the viewing member bought this product and has
    /// not reviewed it yet — the write button shows only then.
    pub can_write: bool,
    /// The viewing member's own review id, when it exists:
    /// drives the edit button and the own-review controls.
    pub own_review_id: Option<i64>,
    /// Whether the viewing member has favourited this product
    /// (the buybox toggle's state; guests always false).
    pub is_favorite: bool,
    /// Whether the viewer is a signed-in member: members get
    /// the toggle form, guests a login link.
    pub is_member: bool,
    /// The back-in-stock registration card, when the product
    /// has a registrable sold-out combination.
    pub restock: Option<crate::views::storefront::restock::RestockCard>,
}

impl ProductDetailView {
    /// The page title: the product's SEO title when set, else
    /// the shop name and product name as before.
    pub fn page_title(&self) -> String {
        if self.product.seo_title.trim().is_empty() {
            format!("{} | {}", self.header.shop.shop_name, self.product.name)
        } else {
            self.product.seo_title.clone()
        }
    }

    /// The page meta description: the product's own when set,
    /// else the shop-wide description.
    pub fn page_meta_description(&self) -> String {
        if self.product.meta_description.trim().is_empty() {
            self.header.shop.meta_description().to_string()
        } else {
            self.product.meta_description.clone()
        }
    }

    /// The 消費税区分 row's value: the class name with its
    /// rate for taxed classes, the name alone for the zero
    /// classes （非課税 / 不課税）.
    pub fn tax_class_display(&self) -> String {
        if self.tax_rate_percent > 0 {
            format!("{}（{}%）", self.tax_class_name, self.tax_rate_percent)
        } else {
            self.tax_class_name.clone()
        }
    }

    /// The tax-exclusive part of the price in force,
    /// formatted: the 本体価格 of the price decomposition.
    pub fn price_excl_label(&self) -> String {
        crate::pricing::display_price(crate::models::tax::exclusive_from_inclusive(
            self.product.effective_price_yen(),
            self.tax_rate_percent,
            crate::models::tax::TaxRounding::configured(),
        ))
    }

    /// The tax contained in the price in force, formatted:
    /// the 内消費税 of the price decomposition.
    pub fn price_tax_label(&self) -> String {
        crate::pricing::display_price(crate::models::tax::tax_from_inclusive(
            self.product.effective_price_yen(),
            self.tax_rate_percent,
            crate::models::tax::TaxRounding::configured(),
        ))
    }

    /// The configured rounding mode's name, for the picker
    /// script that keeps the decomposition in step with the
    /// selected combination's price.
    pub fn tax_rounding_name(&self) -> &'static str {
        match crate::models::tax::TaxRounding::configured() {
            crate::models::tax::TaxRounding::Floor => "floor",
            crate::models::tax::TaxRounding::Round => "round",
            crate::models::tax::TaxRounding::Ceil => "ceil",
        }
    }

    /// The main image's alt text: its stored alt, or the
    /// product name when the row carries none.
    pub fn main_image_alt(&self) -> String {
        match self.images.first() {
            Some(image) if !image.alt.trim().is_empty() => image.alt.clone(),
            _ => self.product.name.clone(),
        }
    }

    /// The stock the detail page's stock line refers to: the
    /// selected combination's stock on a variant product (the
    /// starting combination at first render), the product total
    /// otherwise — the two are the same number for a product
    /// with a single combination.
    pub fn label_stock(&self) -> i32 {
        if self.picker.has_picker() {
            self.picker.initial_stock()
        } else {
            self.product.stock
        }
    }

    /// The stock line under the price, with the exact stock-table
    /// wording. On a variant product it names the selected
    /// combination's stock; the picker script keeps it current.
    pub fn stock_label(&self) -> String {
        stock_wording(
            self.label_stock(),
            self.product.track_stock,
            self.product.allow_backorder,
            self.product.is_preorder,
            self.product.low_stock_threshold(),
        )
    }

    /// The buy panel's delivery line: the earliest delivery
    /// promise this product can honestly carry, or `None` when
    /// no promise applies (a sold-out product that takes no
    /// backorders). A product with a dispatch lead time shows
    /// that instead — the wording the admin form promises for
    /// the field — and a preorder shows the selection note,
    /// because the shop holds no release date to count from.
    pub fn delivery_line(&self) -> Option<String> {
        if self.product.track_stock && !self.product.allow_backorder && self.label_stock() == 0 {
            return None;
        }
        if self.product.is_preorder {
            return Some("予約商品です。お届け希望日時はご注文手続きで選択できます".to_string());
        }
        if let Some(days) = self.product.lead_time_days {
            return Some(format!("ご注文から約{}日で発送します", days));
        }
        Some(format!(
            "最短で{}にお届けできます",
            self.delivery_earliest_label
        ))
    }

    /// The selectable-window note under the delivery line:
    /// shown only beside the standard earliest-date promise,
    /// where the window it names is the one checkout offers.
    pub fn delivery_window_note(&self) -> Option<String> {
        if self.product.is_preorder || self.product.lead_time_days.is_some() {
            return None;
        }
        self.delivery_line()?;
        Some(format!(
            "お届け希望日は{}からご注文手続きで選択できます。離島・一部地域はさらに日数がかかる場合があります",
            self.delivery_range_label
        ))
    }

    /// Whether this listed review is the viewing member's own:
    /// only then do its edit and delete controls render.
    pub fn is_own(&self, card: &ReviewCardView) -> bool {
        self.own_review_id == Some(card.review.id)
    }
}

/// One review card on the product page: the review plus its
/// helpful-vote state for the current reader （参考になった）.
pub struct ReviewCardView {
    /// The review itself.
    pub review: Review,
    /// How many readers marked it helpful.
    pub helpful_count: i64,
    /// Whether this reader already voted (their button then
    /// renders as the voted state, with no way to vote twice).
    pub voted: bool,
}

impl ReviewCardView {
    /// The vote endpoint for this review.
    pub fn helpful_url(&self) -> String {
        format!(
            "/products/{}/reviews/{}/helpful",
            self.review.product_id, self.review.id
        )
    }
}

/// Renders pages/product_not_found.html with a 404 status.
#[derive(Template)]
#[template(path = "storefront/pages/product_not_found.html")]
pub struct NotFoundView {
    /// Shared header state.
    pub header: Header,
}
