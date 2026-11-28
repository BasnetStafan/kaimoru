//! Product model: one item on sale in the store.
//!
//! Most fields map straight onto products table columns; a few
//! are computed at read time (stock is the variant sum, the tax
//! class is joined in), marked where they sit.

use chrono::{DateTime, Utc};

/// A product in the catalog, as stored in the products table.
/// The price is integer yen, never a float, so amounts are always exact.
#[derive(Clone, sqlx::FromRow)]
pub struct Product {
    /// Surrogate ID; the detail page lives at /products/{id}.
    pub id: i64,
    /// Display name of the product, in Japanese.
    pub name: String,
    /// Regular price in yen, tax included.
    pub price_yen: i64,
    /// Sale price in yen (tax included) while a sale is set.
    pub sale_price_yen: Option<i64>,
    /// When the sale price starts applying.
    pub sale_starts_at: Option<DateTime<Utc>>,
    /// When the sale price stops applying.
    pub sale_ends_at: Option<DateTime<Utc>>,
    /// Total units in stock: the sum over the product's enabled
    /// variants. Drives the card state label.
    pub stock: i32,
    /// Whether the product has more than one enabled variant
    /// (variation axes were defined for it). Cards link
    /// multi-variant products to the detail page instead of
    /// adding directly: a combination must never be chosen for
    /// the customer.
    pub has_variants: bool,
    /// Path of the product image under /static.
    pub image_path: String,
    /// The main image's stored alt text, loaded by the card,
    /// search and detail reads; empty when the operator set
    /// none — the name stands in (see [`Product::card_image_alt`]).
    pub image_alt: String,
    /// Category name shown in the category navigation.
    pub category: String,
    /// The managed category's ID; `None` only on legacy rows
    /// seeded outside the app. The detail page links the category
    /// only while the linked category is active.
    pub category_id: Option<i64>,
    /// Whether the product may carry the NEW promo label.
    pub is_new: bool,
    /// Manual promo percentage, 0 when unused: a label only,
    /// it never changes the price (an active sale's computed
    /// percentage takes precedence on the badge).
    pub discount_percent: i32,
    /// Whether the product is sold as a pre-order.
    pub is_preorder: bool,
    /// Whether the product is picked up at an event venue.
    pub is_event: bool,
    /// Whether the operator marked the product おすすめ.
    pub is_staff_pick: bool,
    /// Long description shown on the product detail page (the
    /// default / PC text).
    pub description: String,
    /// Operator-facing product code; shown as 商品番号 when set.
    pub product_code: Option<String>,
    /// Brand name, when the operator registered one.
    pub brand: String,
    /// Manufacturer name, when registered.
    pub manufacturer: String,
    /// Country of origin, when registered.
    pub country_of_origin: String,
    /// Specification text （商品仕様）, when registered.
    pub spec_text: String,
    /// Features text （商品の特徴）, when registered.
    pub features_text: String,
    /// Smartphone-only description; empty renders the default
    /// description on phones too.
    pub description_sp: String,
    /// The product's consumption-tax class.
    pub tax_class_id: i64,
    /// The tax class's stable code, joined by the catalog
    /// reads that render cards and the detail page; empty on
    /// reads that never join it (the admin side).
    #[sqlx(default)]
    pub tax_class_code: String,
    /// The tax class's Japanese name, joined like the code.
    #[sqlx(default)]
    pub tax_class_name: String,
    /// Whether stock is tracked: untracked products sell without
    /// caps or decrements (unlimited).
    pub track_stock: bool,
    /// Whether the product may be ordered at zero stock
    /// (backorder: ships when stock arrives).
    pub allow_backorder: bool,
    /// This product's low-stock threshold; `None` uses the
    /// shop-wide configured default.
    pub low_stock_threshold: Option<i32>,
    /// This product's points earn-rate override, in
    /// percent (points override layer; internal points design record):
    /// `None` uses the shop rate, `Some(0)` excludes the
    /// product from earning.
    pub points_earn_percent: Option<i32>,
    /// Weight in grams, when registered (shipping information).
    pub weight_grams: Option<i32>,
    /// Package length in millimetres, when registered.
    pub size_length_mm: Option<i32>,
    /// Package width in millimetres, when registered.
    pub size_width_mm: Option<i32>,
    /// Package height in millimetres, when registered.
    pub size_height_mm: Option<i32>,
    /// Whether the product ships refrigerated （クール便）.
    pub cool_delivery: bool,
    /// Whether the product can ship by mail delivery （メール便）.
    pub mail_delivery: bool,
    /// Whether this product always ships free.
    pub free_shipping: bool,
    /// Comma-separated prefecture names the product cannot be
    /// delivered to; empty means everywhere is fine.
    pub undeliverable_regions: String,
    /// Days until dispatch （発送予定日数）, when registered.
    pub lead_time_days: Option<i32>,
    /// URL slug; the detail page also answers /products/{slug}.
    pub slug: Option<String>,
    /// SEO title override for the detail page.
    pub seo_title: String,
    /// Meta description for the detail page.
    pub meta_description: String,
}

impl Product {
    /// The price in force right now, via the one effective-price
    /// rule ([`crate::pricing::effective_price_yen`]).
    pub fn effective_price_yen(&self) -> i64 {
        crate::pricing::effective_price_yen(
            self.price_yen,
            self.sale_price_yen,
            self.sale_starts_at,
            self.sale_ends_at,
            Utc::now(),
        )
    }

    /// Whether a sale is in force right now.
    pub fn sale_active(&self) -> bool {
        self.effective_price_yen() < self.price_yen
    }

    /// The sale's percentage off, rounded to a whole percent;
    /// 0 when no sale is in force.
    pub fn sale_percent_off(&self) -> i32 {
        if !self.sale_active() || self.price_yen <= 0 {
            return 0;
        }
        let off = (i64::from(100) * (self.price_yen - self.effective_price_yen()) / self.price_yen)
            as i32;
        off.max(1)
    }

    /// Returns the effective price formatted for display (e.g.
    /// "￥2,980"), via [`crate::pricing::display_price`].
    pub fn formatted_price(&self) -> String {
        crate::pricing::display_price(self.effective_price_yen())
    }

    /// The alt text for this product's card and quick-add
    /// image: the main image's stored alt, or the product name
    /// when the row carries none — the detail gallery's rule,
    /// shared by every surface that shows the main image.
    pub fn card_image_alt(&self) -> &str {
        if self.image_alt.trim().is_empty() {
            &self.name
        } else {
            &self.image_alt
        }
    }

    /// The card-sized derivative of the main image:
    /// the resized WebP copy the cards serve. The path is
    /// derived by [`crate::util::image_paths`]; when no derivative
    /// applies or the file is missing, the browser falls back
    /// to the original (the templates carry it alongside).
    pub fn thumb_image_path(&self) -> String {
        crate::util::image_paths::thumb_path(&self.image_path)
    }

    /// The detail-sized derivative of the main image
    ///: the resized WebP copy the detail page's main
    /// view serves; the original remains the lightbox view.
    pub fn medium_image_path(&self) -> String {
        crate::util::image_paths::medium_path(&self.image_path)
    }

    /// The regular price formatted, for the struck-through figure
    /// shown beside an active sale price.
    pub fn formatted_regular_price(&self) -> String {
        crate::pricing::display_price(self.price_yen)
    }

    /// The low-stock threshold that applies to this product: its
    /// own when set, the configured shop default otherwise.
    pub fn low_stock_threshold(&self) -> i32 {
        self.low_stock_threshold
            .unwrap_or(crate::config::CONFIG.low_stock_default as i32)
    }

    /// The points earn percent that applies to this
    /// product: its own override when set, the shop-wide
    /// rate otherwise.
    pub fn points_earn_percent_or(&self, shop_percent: i32) -> i32 {
        self.points_earn_percent.unwrap_or(shop_percent)
    }

    /// The card note naming a non-standard tax class, for
    /// the card and quick-add price lines: standard-rate
    /// products carry no note, reduced-rate ones the
    /// 軽減税率対象 marker, and the zero classes their name.
    pub fn tax_class_note(&self) -> Option<String> {
        match self.tax_class_code.as_str() {
            "reduced" => Some("※軽減税率対象".to_string()),
            "exempt" | "out_of_scope" => Some(if self.tax_class_name.is_empty() {
                if self.tax_class_code == "exempt" {
                    "非課税".to_string()
                } else {
                    "不課税".to_string()
                }
            } else {
                self.tax_class_name.clone()
            }),
            _ => None,
        }
    }

    /// The 商品番号 shown to customers: the operator's product
    /// code when registered, the database ID otherwise (the
    /// long-standing display).
    pub fn display_code(&self) -> String {
        match &self.product_code {
            Some(code) if !code.is_empty() => code.clone(),
            _ => self.id.to_string(),
        }
    }
}
