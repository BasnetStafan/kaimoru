//! Coupon model (Cart & Checkout Spec).

use chrono::{DateTime, Utc};

/// A store coupon definition, as the evaluation engine reads
/// it: value type, scope, and usage caps (Coupon Shopify-Core
/// spec). `discount_yen` carries the value only for the fixed
/// type; percent definitions carry `percent` instead, and the
/// free-shipping type carries neither.
#[derive(Clone, sqlx::FromRow)]
pub struct Coupon {
    /// The code customers type.
    pub code: String,
    /// Value kind: `fixed`, `percent`, or `free_shipping`.
    pub discount_type: String,
    /// Discount in yen (fixed type only; 0 otherwise).
    pub discount_yen: i64,
    /// Discount percent, 1..=100 (percent type only).
    pub percent: Option<i32>,
    /// Minimum product subtotal for the code to apply.
    pub min_subtotal_yen: i64,
    /// Scope kind: `order`, `product`, or `variant`.
    pub scope: String,
    /// The scoped product (product scope only).
    pub product_id: Option<i64>,
    /// The scoped variant (variant scope only).
    pub variant_id: Option<i64>,
    /// Total redemption cap across all customers
    /// (`None` = unlimited).
    pub max_uses_total: Option<i32>,
    /// Per-customer redemption cap (`None` = unlimited;
    /// 1 is the store default, the pre-pack rule).
    pub max_uses_per_customer: Option<i32>,
    /// Whether the discount applies without a code.
    pub is_automatic: bool,
    /// The definition's display message.
    pub description: String,
}

impl Coupon {
    /// Returns the combination class this definition belongs
    /// to: free shipping is its own class, a product- or
    /// variant-scoped discount is the product class, and
    /// everything else is the order class. One coupon per
    /// class may apply to an order.
    pub fn class(&self) -> &'static str {
        if self.discount_type == "free_shipping" {
            "shipping"
        } else if self.scope == "order" {
            "order"
        } else {
            "product"
        }
    }

    /// Returns the discount in yen for a class basis (the
    /// scoped lines total for the product class, the subtotal
    /// after the product discount for the order class).
    /// Percent values round down to whole yen. The shipping
    /// class is priced by the engine from the fee itself, so
    /// it returns 0 here.
    pub fn value_for_basis(&self, basis_yen: i64) -> i64 {
        match self.discount_type.as_str() {
            "percent" => basis_yen * i64::from(self.percent.unwrap_or(0)) / 100,
            "fixed" => self.discount_yen.min(basis_yen),
            _ => 0,
        }
    }

    /// Returns the benefit as one display phrase (for example
    /// `￥500割引`, `10%割引`, `送料無料`).
    pub fn benefit_label(&self) -> String {
        match self.discount_type.as_str() {
            "percent" => format!("{}%割引", self.percent.unwrap_or(0)),
            "free_shipping" => "送料無料".to_string(),
            _ => format!("{}割引", crate::pricing::display_price(self.discount_yen)),
        }
    }
}

/// One coupon applied to a cart or order, with the amount it
/// contributed: a row of the checkout's applied list and of
/// the `coupon_redemptions` ledger.
#[derive(Clone, Debug)]
pub struct AppliedCoupon {
    /// The applied definition's code (the internal identifier
    /// for automatic discounts).
    pub code: String,
    /// The combination class: `product`, `order`, or
    /// `shipping`.
    pub class: String,
    /// The yen this coupon took off (the waived fee for the
    /// shipping class).
    pub discount_yen: i64,
    /// Whether it applied without a code.
    pub automatic: bool,
    /// The definition's display message.
    pub description: String,
}

impl AppliedCoupon {
    /// Returns the class's Japanese label for the checkout
    /// list （商品 / 注文 / 送料）.
    pub fn class_label(&self) -> &'static str {
        match self.class.as_str() {
            "product" => "商品",
            "shipping" => "送料",
            _ => "注文",
        }
    }

    /// Returns the amount formatted for display.
    pub fn amount_display(&self) -> String {
        crate::pricing::display_price(self.discount_yen)
    }
}

/// One coupon copy a member holds, with its definition and use
/// state: a row of the mypage coupon section.
#[derive(Clone, sqlx::FromRow)]
pub struct MemberCoupon {
    /// The code the member types at checkout.
    pub code: String,
    /// The definition's short description (may be empty).
    pub description: String,
    /// Discount in yen (fixed type only; 0 otherwise).
    pub discount_yen: i64,
    /// Value kind: `fixed`, `percent`, or `free_shipping`.
    pub discount_type: String,
    /// Discount percent, 1..=100 (percent type only).
    pub percent: Option<i32>,
    /// Scope kind: `order`, `product`, or `variant`.
    pub scope: String,
    /// The scope target's display name (the product name, or
    /// product name + variant label); `None` for order scope.
    pub target_label: Option<String>,
    /// Minimum product subtotal for the code to apply
    /// (0 = no minimum).
    pub min_subtotal_yen: i64,
    /// When this copy was issued to the member.
    pub issued_at: DateTime<Utc>,
    /// When the copy was used; `None` while it is held.
    pub used_at: Option<DateTime<Utc>>,
    /// The order that used the copy, resolved by (member, code);
    /// `None` while the copy is held.
    pub used_order_id: Option<i64>,
    /// That order's placement instant; the order number
    /// derives from the ID and this instant together.
    pub used_order_created_at: Option<DateTime<Utc>>,
    /// Whether the definition is currently active.
    pub active: bool,
    /// The definition's window start, when set.
    pub starts_at: Option<DateTime<Utc>>,
    /// The definition's window end, when set.
    pub ends_at: Option<DateTime<Utc>>,
}

impl MemberCoupon {
    /// Whether the copy has been used.
    pub fn is_used(&self) -> bool {
        self.used_at.is_some()
    }

    /// Whether the definition can currently be used:
    /// active and inside its validity window. (The
    /// copy's own used state is separate: is_used.)
    pub fn is_available(&self) -> bool {
        if !self.active {
            return false;
        }
        let now = Utc::now();
        if let Some(start) = self.starts_at
            && now < start
        {
            return false;
        }
        if let Some(end) = self.ends_at
            && now >= end
        {
            return false;
        }
        true
    }

    /// Whether the copy is held but unusable (definition
    /// off, or outside its window) — the mypage
    /// 利用できません state (Coupon Admin spec §6).
    pub fn is_unavailable(&self) -> bool {
        !self.is_used() && !self.is_available()
    }

    /// The benefit formatted for display as a full phrase
    /// (for example `￥500割引`, `10%割引`, `送料無料`).
    pub fn discount_display(&self) -> String {
        match self.discount_type.as_str() {
            "percent" => format!("{}%割引", self.percent.unwrap_or(0)),
            "free_shipping" => "送料無料".to_string(),
            _ => format!("{}割引", crate::pricing::display_price(self.discount_yen)),
        }
    }

    /// Whether a minimum-spend condition applies.
    pub fn has_min_subtotal(&self) -> bool {
        self.min_subtotal_yen > 0
    }

    /// The minimum subtotal formatted for display.
    pub fn min_subtotal_display(&self) -> String {
        crate::pricing::display_price(self.min_subtotal_yen)
    }

    /// Returns the checkout dropdown label: the mypage wording
    /// (code, description, discount, minimum) on one line.
    pub fn option_label(&self) -> String {
        let mut parts = vec![self.code.clone()];
        if !self.description.is_empty() {
            parts.push(self.description.clone());
        }
        parts.push(self.discount_display());
        if let Some(target) = &self.target_label {
            parts.push(format!("対象：{target}"));
        }
        if self.has_min_subtotal() {
            parts.push(format!(
                "{}以上のお買い物で利用可能",
                self.min_subtotal_display()
            ));
        }
        parts.join(" ")
    }

    /// Returns the used date (`YYYY/MM/DD`, JST); empty while
    /// the copy is held.
    pub fn used_date_display(&self) -> String {
        match self.used_at {
            Some(at) => super::jst::jst_date_label(at),
            None => String::new(),
        }
    }

    /// The member-side path of the order that used this
    /// copy, when the using order was resolved.
    pub fn used_order_path(&self) -> Option<String> {
        self.used_order_id.map(|id| format!("/mypage/orders/{id}"))
    }

    /// Returns the displayed number of the order that used this
    /// copy, derived from the row itself. Empty when no using
    /// order was resolved; the template calls this only behind
    /// `used_order_path`.
    pub fn used_order_number(&self) -> String {
        match (self.used_order_id, self.used_order_created_at) {
            (Some(id), Some(created_at)) => crate::models::order::order_number_for(id, created_at),
            _ => String::new(),
        }
    }
}

/// One coupon definition as the admin manages it: the
/// full row of the `coupons` table (Coupon Admin spec §3).
#[derive(Clone, sqlx::FromRow)]
pub struct CouponDefinition {
    /// Row id.
    pub id: i64,
    /// Canonical (uppercase) code.
    pub code: String,
    /// Value kind: `fixed`, `percent`, or `free_shipping`.
    pub discount_type: String,
    /// Fixed discount in yen (0 for other types).
    pub discount_yen: i64,
    /// Discount percent, 1..=100 (percent type only).
    pub percent: Option<i32>,
    /// Scope kind: `order`, `product`, or `variant`.
    pub scope: String,
    /// The scoped product (product scope only).
    pub product_id: Option<i64>,
    /// The scoped variant (variant scope only).
    pub variant_id: Option<i64>,
    /// Total redemption cap (`None` = unlimited).
    pub max_uses_total: Option<i32>,
    /// Per-customer redemption cap (`None` = unlimited).
    pub max_uses_per_customer: Option<i32>,
    /// Whether the discount applies without a code.
    pub is_automatic: bool,
    /// Minimum pre-discount subtotal (0 = none).
    pub min_subtotal_yen: i64,
    /// Band/dropdown message.
    pub description: String,
    /// The on/off switch; deactivation is the only delete.
    pub active: bool,
    /// Window start, when set (open-ended otherwise).
    pub starts_at: Option<DateTime<Utc>>,
    /// Window end, when set.
    pub ends_at: Option<DateTime<Utc>>,
    /// Whether the top-page band shows this coupon
    /// (at most one definition carries the flag).
    pub band_featured: bool,
}

/// A definition row for the admin list: the definition
/// plus its redemption count.
#[derive(Clone, sqlx::FromRow)]
pub struct CouponDefinitionRow {
    /// Row id.
    pub id: i64,
    /// Canonical (uppercase) code.
    pub code: String,
    /// Value kind: `fixed`, `percent`, or `free_shipping`.
    pub discount_type: String,
    /// Fixed discount in yen (0 for other types).
    pub discount_yen: i64,
    /// Discount percent, 1..=100 (percent type only).
    pub percent: Option<i32>,
    /// Scope kind: `order`, `product`, or `variant`.
    pub scope: String,
    /// The scope target's display name; `None` for order
    /// scope.
    pub target_label: Option<String>,
    /// Total redemption cap (`None` = unlimited).
    pub max_uses_total: Option<i32>,
    /// Per-customer redemption cap (`None` = unlimited).
    pub max_uses_per_customer: Option<i32>,
    /// Whether the discount applies without a code.
    pub is_automatic: bool,
    /// Minimum pre-discount subtotal (0 = none).
    pub min_subtotal_yen: i64,
    /// Band/dropdown message.
    pub description: String,
    /// Whether the definition is active.
    pub active: bool,
    /// Window start, when set.
    pub starts_at: Option<DateTime<Utc>>,
    /// Window end, when set.
    pub ends_at: Option<DateTime<Utc>>,
    /// Whether the band shows this coupon.
    pub band_featured: bool,
    /// How many members have redeemed it.
    pub redemptions: i64,
}

/// The band's coupon: code, discount and the message
/// the band carries (Coupon Admin spec D4).
#[derive(Clone, sqlx::FromRow)]
pub struct BandCoupon {
    /// The featured coupon's code.
    pub code: String,
    /// Value kind: `fixed`, `percent`, or `free_shipping`.
    pub discount_type: String,
    /// Its discount in yen (fixed type only).
    pub discount_yen: i64,
    /// Its discount percent (percent type only).
    pub percent: Option<i32>,
    /// Its message (the band's lead text).
    pub description: String,
}

impl BandCoupon {
    /// Returns the benefit as one display phrase (for example
    /// `￥500割引`, `10%割引`, `送料無料`).
    pub fn benefit_label(&self) -> String {
        match self.discount_type.as_str() {
            "percent" => format!("{}%割引", self.percent.unwrap_or(0)),
            "free_shipping" => "送料無料".to_string(),
            _ => format!("{}割引", crate::pricing::display_price(self.discount_yen)),
        }
    }
}

/// A definition's writable fields, grouped for the
/// repository writes (Coupon Admin spec §3).
pub struct CouponDefinitionWrite {
    /// Canonical (uppercase) code.
    pub code: String,
    /// Value kind: `fixed`, `percent`, or `free_shipping`.
    pub discount_type: String,
    /// Discount in yen (fixed type only; 0 otherwise).
    pub discount_yen: i64,
    /// Discount percent, 1..=100 (percent type only).
    pub percent: Option<i32>,
    /// Scope kind: `order`, `product`, or `variant`.
    pub scope: String,
    /// The scoped product (product scope only).
    pub product_id: Option<i64>,
    /// The scoped variant (variant scope only).
    pub variant_id: Option<i64>,
    /// Total redemption cap (`None` = unlimited).
    pub max_uses_total: Option<i32>,
    /// Per-customer redemption cap (`None` = unlimited).
    pub max_uses_per_customer: Option<i32>,
    /// Whether the discount applies without a code.
    pub is_automatic: bool,
    /// Minimum subtotal in yen.
    pub min_subtotal_yen: i64,
    /// Message.
    pub description: String,
    /// Window start.
    pub starts_at: Option<DateTime<Utc>>,
    /// Window end.
    pub ends_at: Option<DateTime<Utc>>,
    /// Whether the band shows this coupon.
    pub band_featured: bool,
}
