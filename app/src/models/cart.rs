//! Cart models (Cart & Checkout Spec).

/// One row of a cart as pages display it: the product joined with
/// the quantity the visitor put in the cart.
#[derive(Clone, sqlx::FromRow)]
pub struct CartLine {
    /// Variant ID: the cart line identity. The same product in
    /// two combinations is two lines.
    pub variant_id: i64,
    /// Product ID (through the variant).
    pub product_id: i64,
    /// Product name.
    pub name: String,
    /// The combination label, derived live from its axes and
    /// options; empty for the default variant, and rendered
    /// only when non-empty.
    pub variant_label: String,
    /// Unit price in yen, tax included, as currently priced.
    pub unit_price_yen: i64,
    /// Current stock of the line variant.
    pub stock: i32,
    /// Publication status (`public`, `draft` or `archived`). A
    /// product can be taken off sale after entering the cart; the
    /// line must know, or it would count toward the totals of a
    /// product the store no longer sells.
    pub status: String,
    /// Product image path.
    pub image_path: String,
    /// Quantity in the cart.
    pub quantity: i32,
    /// Whether the shopping agent added this line:
    /// a run may replace its own lines, never the owner's.
    pub added_by_agent: bool,
    /// Whether the line's combination is enabled: a combination
    /// disabled after carting is off sale, exactly like a drafted
    /// product.
    pub variant_enabled: bool,
    /// Whether the product's publication window is open now: a
    /// window that closed after carting takes the line off sale.
    pub publish_open: bool,
    /// Whether the product tracks stock: untracked lines are
    /// unlimited, never sold out.
    pub track_stock: bool,
    /// Whether the product accepts backorders at zero stock.
    pub allow_backorder: bool,
    /// Whether this product always ships free.
    pub free_shipping: bool,
    /// The product's consumption-tax class code.
    pub tax_class_code: String,
    /// The tax class's Japanese name.
    pub tax_class_name: String,
    /// The class rate in force today, in whole percent. Display
    /// and cart totals use it; placement re-resolves by the
    /// order's date and snapshots its own value.
    pub tax_rate_percent: i32,
    /// Comma-separated prefectures the product cannot ship to.
    pub undeliverable_regions: String,
}

impl CartLine {
    /// Line total in yen: unit price times quantity.
    pub fn line_total_yen(&self) -> i64 {
        self.unit_price_yen * i64::from(self.quantity)
    }

    /// The line total formatted for display.
    pub fn formatted_line_total(&self) -> String {
        crate::pricing::display_price(self.line_total_yen())
    }

    /// The unit price formatted for display.
    pub fn formatted_unit_price(&self) -> String {
        crate::pricing::display_price(self.unit_price_yen)
    }

    /// Whether the product can still be bought: on sale (public,
    /// in its publication window, combination enabled) and either
    /// untracked, backorderable, or in stock. An off-sale line
    /// behaves exactly like a sold-out one — excluded from
    /// totals, skipped at placement.
    pub fn purchasable(&self) -> bool {
        self.on_sale() && (self.stock > 0 || !self.track_stock || self.allow_backorder)
    }

    /// Whether the line is on sale at all, ignoring stock: the
    /// product is public, inside its publication window, and the
    /// combination is enabled.
    pub fn on_sale(&self) -> bool {
        self.status == "public" && self.publish_open && self.variant_enabled
    }

    /// Whether the product was taken off sale (drafted, archived,
    /// window closed, combination disabled) since entering the
    /// cart; the cart page words these lines differently from
    /// sold-out ones.
    pub fn off_sale(&self) -> bool {
        !self.on_sale()
    }

    /// Whether the + stepper may add one more (below the stock
    /// cap, for tracked non-backorder lines).
    pub fn can_increment(&self) -> bool {
        self.on_sale() && (!self.track_stock || self.allow_backorder || self.quantity < self.stock)
    }

    /// Whether this line cannot ship to `prefecture` (the
    /// product's undeliverable-region list names it).
    pub fn undeliverable_to(&self, prefecture: &str) -> bool {
        !prefecture.is_empty()
            && self
                .undeliverable_regions
                .split(',')
                .any(|r| r.trim() == prefecture)
    }
}
