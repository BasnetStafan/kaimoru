//! Order models (Cart & Checkout Spec).

use crate::models::carrier::{carrier_for, tracking_url};
use crate::models::delivery::format_delivery_request;
use crate::models::jst::{jst, jst_datetime_label, jst_long_datetime_label};
use chrono::{DateTime, NaiveDate, Utc};

/// Which door an order entered through, stamped onto the order
/// at placement (migration 20261101000054): the normal
/// storefront checkout, or the shopping agent's human-approved
/// settlement. The database column is plain text with a CHECK
/// constraint; this enum is the only writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacedVia {
    /// Placed through the storefront checkout pages.
    Storefront,
    /// Placed through the shopping agent's settlement flow.
    Agent,
}

impl PlacedVia {
    /// The stored text value.
    pub fn as_str(&self) -> &'static str {
        match self {
            PlacedVia::Storefront => "storefront",
            PlacedVia::Agent => "agent",
        }
    }
}

/// An order as the member history and the completion page show it.
#[derive(Clone, sqlx::FromRow)]
pub struct Order {
    /// Surrogate ID; also the tail of the displayed order number.
    pub id: i64,
    /// Member who ordered; `None` for guest orders.
    pub user_id: Option<i64>,
    /// Contact email captured with the order.
    pub email: String,
    /// Delivery family name.
    pub family_name: String,
    /// Delivery given name.
    pub given_name: String,
    /// Delivery postal code.
    pub postal_code: String,
    /// Delivery prefecture.
    pub prefecture: String,
    /// Delivery city and ward.
    pub city: String,
    /// Delivery street address.
    pub address_line: String,
    /// Delivery building, when given.
    pub building: Option<String>,
    /// Delivery phone number.
    pub phone: String,
    /// Product subtotal in yen before discount and fees.
    pub subtotal_yen: i64,
    /// Coupon discount in yen.
    pub discount_yen: i64,
    /// Shipping fee in yen, net of any shipping discount.
    pub shipping_yen: i64,
    /// The part of the shipping fee a free-shipping coupon
    /// waived, in yen: the gross fee is `shipping_yen` plus
    /// this. Zero when no shipping coupon applied.
    pub shipping_discount_yen: i64,
    /// Cash-on-delivery fee in yen.
    pub cod_fee_yen: i64,
    /// Grand total in yen: subtotal less discount, plus shipping
    /// and any cash-on-delivery fee, whatever the payment method.
    pub total_yen: i64,
    /// Points the member spent on this order (1 pt = ¥1;
    /// EC V2 core 1). Points are a payment instrument: the
    /// total above never includes them, and the amount the
    /// payment method charges is `total_yen` minus this.
    pub points_used_yen: i64,
    /// Points this order earned, granted at shipment.
    pub points_earned: i64,
    /// Coupon code applied, when any.
    pub coupon_code: Option<String>,
    /// Fulfilment status key: `received`, `preparing`, `shipped`,
    /// `delivered` or `cancelled`. New rows take the database
    /// default (`received`); the back office moves the value
    /// through the fixed forward-only flow.
    pub status: String,
    /// Which door the order entered through: `storefront` for
    /// the normal checkout, `agent` for the shopping agent's
    /// settlement (see [`PlacedVia`]).
    pub placed_via: String,
    /// Requested delivery date; `None` when the shopper made no
    /// date request — NULL *is* the no-preference state.
    pub delivery_date: Option<NaiveDate>,
    /// Requested delivery slot code (see [`crate::models::delivery::DELIVERY_SLOTS`]).
    pub delivery_slot: String,
    /// The slot's Japanese label, snapshotted at placement so
    /// history never changes wording retroactively.
    pub delivery_slot_label: String,
    /// Delivery carrier code (see [`crate::models::carrier::CARRIERS`]); `None` when
    /// the operator shipped without naming one.
    pub carrier: Option<String>,
    /// Carrier tracking number as the operator entered it;
    /// `None` when no number was given at ship time.
    pub tracking_number: Option<String>,
    /// When the shipping notification mail was successfully
    /// sent; `None` means no successful send yet — for a
    /// shipped order that is the admin-visible unsent state.
    pub shipped_mail_sent_at: Option<DateTime<Utc>>,
    /// When the order was placed.
    pub created_at: DateTime<Utc>,
    /// Gift noshi code (see [`crate::models::gift::NOSHI_KINDS`]);
    /// `none` when no noshi was chosen.
    pub noshi_code: String,
    /// The noshi label snapshotted at placement.
    pub noshi_label: String,
    /// The gift name inscription （名入れ）, when given.
    pub noshi_name: Option<String>,
    /// The gift message-card text, when given.
    pub gift_message: Option<String>,
}

/// One row of the mypage hub's recent-orders block: the order
/// plus the summary of its lines (first product name, kind
/// count), loaded by one grouped query — never a query per
/// order.
#[derive(Clone)]
pub struct RecentOrder {
    /// The order itself.
    pub order: Order,
    /// The first line's snapshotted product name.
    pub first_product_name: String,
    /// How many lines (kinds of product) the order holds.
    pub item_kinds: i64,
}

impl RecentOrder {
    /// The items label: the first product's name, with
    /// `ほかN点` appended when the order holds more kinds.
    pub fn items_label(&self) -> String {
        if self.item_kinds > 1 {
            format!("{} ほか{}点", self.first_product_name, self.item_kinds - 1)
        } else {
            self.first_product_name.clone()
        }
    }
}

impl Order {
    /// The customer-facing order number, for example
    /// `AS-20261101-0005`: store prefix, placement date, padded ID.
    /// Whether the order was placed through the shopping
    /// agent's settlement rather than the storefront checkout.
    pub fn is_agent_placed(&self) -> bool {
        self.placed_via == PlacedVia::Agent.as_str()
    }

    pub fn order_number(&self) -> String {
        order_number_for(self.id, self.created_at)
    }

    /// Returns this order's status as members see it; the first
    /// state displays the long-standing 注文受付済み wording.
    pub fn status_label(&self) -> &'static str {
        status_label(&self.status)
    }

    /// Returns this order's delivery request as every surface
    /// shows it, from the stored date and label snapshot.
    /// The gift choice's display lines (のし / メッセージ）,
    /// empty when the shopper chose no gift options.
    pub fn gift_lines(&self) -> Vec<String> {
        crate::models::gift::gift_display_lines(
            &self.noshi_code,
            &self.noshi_label,
            self.noshi_name.as_deref(),
            self.gift_message.as_deref(),
        )
    }

    pub fn delivery_request_display(&self) -> String {
        format_delivery_request(self.delivery_date, &self.delivery_slot_label)
    }

    /// The grand total formatted for display.
    pub fn formatted_total(&self) -> String {
        crate::pricing::display_price(self.total_yen)
    }

    /// The amount the payment method charges for this
    /// order: the total minus the points spent (EC V2
    /// core 1). Equals the total when no points were used.
    pub fn payable_yen(&self) -> i64 {
        self.total_yen - self.points_used_yen
    }

    /// [`Self::payable_yen`], formatted for display.
    pub fn formatted_payable(&self) -> String {
        crate::pricing::display_price(self.payable_yen())
    }

    /// Placement date and time in the long Japanese form the
    /// order detail page shows, for example `2026年11月1日 17:44`.
    pub fn placed_long(&self) -> String {
        jst_long_datetime_label(self.created_at)
    }

    /// Product subtotal formatted for display.
    pub fn formatted_subtotal(&self) -> String {
        crate::pricing::display_price(self.subtotal_yen)
    }

    /// Coupon discount formatted for display.
    pub fn formatted_discount(&self) -> String {
        crate::pricing::display_price(self.discount_yen)
    }

    /// Shipping fee formatted for display.
    pub fn formatted_shipping(&self) -> String {
        crate::pricing::display_price(self.shipping_yen)
    }

    /// The gross shipping fee (before the shipping discount)
    /// formatted for display: itemised surfaces show this with
    /// the discount row beneath it, exactly as the checkout
    /// steps present the same figures.
    pub fn formatted_shipping_gross(&self) -> String {
        crate::pricing::display_price(self.shipping_yen + self.shipping_discount_yen)
    }

    /// The shipping discount formatted for display.
    pub fn formatted_shipping_discount(&self) -> String {
        crate::pricing::display_price(self.shipping_discount_yen)
    }

    /// COD fee formatted for display.
    pub fn formatted_cod_fee(&self) -> String {
        crate::pricing::display_price(self.cod_fee_yen)
    }

    /// The shipping carrier's Japanese label, when the
    /// operator named one at ship time.
    pub fn carrier_label(&self) -> Option<&'static str> {
        self.carrier
            .as_deref()
            .and_then(carrier_for)
            .map(|c| c.label)
    }

    /// The carrier's public tracking URL for this order's
    /// tracking number, when both exist and the carrier has a
    /// deep-linkable page.
    pub fn tracking_url(&self) -> Option<String> {
        match (&self.carrier, &self.tracking_number) {
            (Some(code), Some(number)) => tracking_url(code, number),
            _ => None,
        }
    }

    /// The shipping-mail stamp in the long Japanese form the
    /// back office shows, for example `2026年11月3日 17:44`;
    /// `None` while no send has succeeded.
    pub fn shipped_mail_sent_long(&self) -> Option<String> {
        self.shipped_mail_sent_at.map(jst_long_datetime_label)
    }

    /// Placement date and time in local display form.
    pub fn placed_display(&self) -> String {
        jst_datetime_label(self.created_at)
    }
}

/// Returns the order number for an ID and placement instant:
/// store prefix, placement date, padded ID. Shared by
/// [`Order::order_number`] and the admin order list.
pub fn order_number_for(id: i64, created_at: DateTime<Utc>) -> String {
    // The date is the Japan-calendar day of placement: formatting
    // the raw UTC instant gave an order placed at 00:30 JST the
    // previous day's date, disagreeing with the times shown beside
    // it. Numbers are derived, never stored, so all historical
    // numbers shift consistently.
    format!(
        "AS-{}-{:04}",
        created_at.with_timezone(&jst()).format("%Y%m%d"),
        id
    )
}

/// Parses a shopper-typed order number into the order ID it
/// names, or `None` when the text is not shaped like one. The
/// date segment is shape-checked only; the caller compares the
/// full derived number against the loaded order, which is what
/// actually proves the date segment.
pub fn parse_order_number(text: &str) -> Option<i64> {
    let normalized = text.trim().to_uppercase();
    let mut parts = normalized.split('-');
    if parts.next()? != "AS" {
        return None;
    }
    let date = parts.next()?;
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let id: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || id <= 0 {
        return None;
    }
    Some(id)
}

/// A fulfilment status as members see it: the received state
/// keeps the long-standing 注文受付済み wording.
pub fn status_label(status: &str) -> &'static str {
    match status {
        "preparing" => "準備中",
        "shipped" => "発送済み",
        "delivered" => "配達完了",
        "cancelled" => "キャンセル",
        _ => "注文受付済み",
    }
}

/// Returns a fulfilment status as the back office names it: the
/// same flow, with the shorter 受付済み for the first state.
pub fn admin_status_label(status: &str) -> &'static str {
    match status {
        "received" => "受付済み",
        other => status_label(other),
    }
}

/// Returns the derived cash-on-delivery state for a status.
///
/// The money is collected at the door, so the state follows the
/// parcel: 代引き（未収） in flight, 代引き（回収済み） once
/// delivered, 代引き（対象外） when cancelled. Nothing is stored;
/// this derivation is the single source shared by the admin list
/// and detail, so the two screens cannot disagree.
pub fn payment_status_label(status: &str) -> &'static str {
    match status {
        "delivered" => "代引き（回収済み）",
        "cancelled" => "代引き（対象外）",
        _ => "代引き（未収）",
    }
}

/// Returns the legal next statuses from `status` in the fixed
/// forward-only flow: received → preparing → shipped → delivered,
/// with cancellation only before shipping.
///
/// The admin form offers exactly these and the service re-checks
/// against the same list, because a posted form is never trusted.
/// A terminal or unknown status yields an empty list.
pub fn legal_next_statuses(status: &str) -> Vec<&'static str> {
    match status {
        "received" => vec!["preparing", "cancelled"],
        "preparing" => vec!["shipped", "cancelled"],
        "shipped" => vec!["delivered"],
        _ => vec![],
    }
}

/// One line of a placed order, with name and price frozen at
/// purchase time (later catalogue changes never rewrite history).
#[derive(Clone, sqlx::FromRow)]
pub struct OrderLine {
    /// Product ID.
    pub product_id: i64,
    /// Product name as it was when ordered.
    pub product_name: String,
    /// The combination label as it was when ordered; empty for
    /// the default variant and for orders placed before
    /// variants existed.
    pub variant_label: String,
    /// Unit price in yen as it was when ordered.
    pub unit_price_yen: i64,
    /// Quantity ordered.
    pub quantity: i32,
    /// Tax class code snapshot; `None` on orders placed before
    /// tax classes existed.
    pub tax_class_code: Option<String>,
    /// Tax rate percent snapshot of the order's date.
    pub rate_percent: Option<i32>,
}

impl OrderLine {
    /// Line total in yen from the frozen snapshot.
    pub fn line_total_yen(&self) -> i64 {
        self.unit_price_yen * i64::from(self.quantity)
    }

    /// The line total formatted for display.
    pub fn formatted_line_total(&self) -> String {
        crate::pricing::display_price(self.line_total_yen())
    }
}

/// One line of a placed order for the detail page: the frozen
/// snapshot (name, price) joined with the product's *current*
/// thumbnail, which is display-only. A deleted product leaves the
/// snapshot intact with no image.
#[derive(Clone, sqlx::FromRow)]
pub struct OrderLineDetail {
    /// Product ID.
    pub product_id: i64,
    /// Product name as it was when ordered.
    pub product_name: String,
    /// The combination label as it was when ordered; empty
    /// when the line has no variation.
    pub variant_label: String,
    /// Unit price in yen as it was when ordered.
    pub unit_price_yen: i64,
    /// Quantity ordered.
    pub quantity: i32,
    /// Tax class code snapshot; `None` on pre-tax orders.
    pub tax_class_code: Option<String>,
    /// Tax rate percent snapshot of the order's date.
    pub rate_percent: Option<i32>,
    /// Current catalogue image path, when the product still exists.
    pub image_path: Option<String>,
}

impl OrderLineDetail {
    /// Unit price formatted for display.
    pub fn formatted_unit_price(&self) -> String {
        crate::pricing::display_price(self.unit_price_yen)
    }

    /// Line total in yen from the frozen snapshot.
    pub fn line_total_yen(&self) -> i64 {
        self.unit_price_yen * i64::from(self.quantity)
    }

    /// The line total formatted for display.
    pub fn formatted_line_total(&self) -> String {
        crate::pricing::display_price(self.line_total_yen())
    }
}

#[cfg(test)]
mod tests {
    use super::Order;
    use chrono::{DateTime, Utc};

    /// An order placed at a known UTC instant: 2026-11-01 00:30 UTC
    /// is 09:30 the same day in Japan.
    fn order_at(instant: &str) -> Order {
        Order {
            id: 1,
            user_id: None,
            email: "guest@example.jp".to_string(),
            family_name: "山田".to_string(),
            given_name: "太郎".to_string(),
            postal_code: "1500001".to_string(),
            prefecture: "東京都".to_string(),
            city: "渋谷区".to_string(),
            address_line: "神南1-1-1".to_string(),
            building: None,
            phone: "09012345678".to_string(),
            subtotal_yen: 1_000,
            discount_yen: 0,
            shipping_yen: 550,
            shipping_discount_yen: 0,
            cod_fee_yen: 330,
            total_yen: 1_880,
            points_used_yen: 0,
            points_earned: 0,
            coupon_code: None,
            status: "received".to_string(),
            placed_via: "storefront".to_string(),
            delivery_date: None,
            delivery_slot: "none".to_string(),
            delivery_slot_label: "指定なし".to_string(),
            carrier: None,
            tracking_number: None,
            shipped_mail_sent_at: None,
            created_at: instant.parse::<DateTime<Utc>>().unwrap(),
            noshi_code: "none".to_string(),
            noshi_label: "なし".to_string(),
            noshi_name: None,
            gift_message: None,
        }
    }

    #[test]
    fn placed_times_render_in_jst_regardless_of_process_timezone() {
        // Under TZ=UTC (the ECS default) the strings must still be
        // JST wall time: the offset is applied explicitly.
        let order = order_at("2026-11-01T00:30:00Z");
        assert_eq!(order.placed_display(), "2026/11/01 09:30");
        assert_eq!(order.placed_long(), "2026年11月1日 09:30");
    }

    #[test]
    fn placed_times_roll_over_the_date_line_in_jst() {
        // 2026-11-01 16:00 UTC is already 2026-11-02 01:00 in Japan.
        let order = order_at("2026-11-01T16:00:00Z");
        assert_eq!(order.placed_display(), "2026/11/02 01:00");
    }

    #[test]
    fn order_numbers_parse_back() {
        assert_eq!(super::parse_order_number("AS-20261109-0123"), Some(123));
        assert_eq!(super::parse_order_number(" as-20261109-1 "), Some(1));
        assert_eq!(super::parse_order_number("AS-20261109-0000"), None);
        assert_eq!(super::parse_order_number("AS-2026110-0123"), None);
        assert_eq!(super::parse_order_number("AS-20261109"), None);
        assert_eq!(super::parse_order_number("XX-20261109-0123"), None);
        assert_eq!(super::parse_order_number("AS-20261109-0123-4"), None);
    }

    #[test]
    fn order_numbers_carry_the_jst_date() {
        // 2026-11-01 15:30 UTC is 2026-11-02 00:30 in Japan — the
        // number must agree with the displayed placement time.
        let order = order_at("2026-11-01T15:30:00Z");
        assert_eq!(order.order_number(), "AS-20261102-0001");
        // 2026-11-01 00:30 UTC is 09:30 the same day in Japan.
        let order = order_at("2026-11-01T00:30:00Z");
        assert_eq!(order.order_number(), "AS-20261101-0001");
    }

    #[test]
    fn payment_labels_follow_the_parcel() {
        // Uncollected until delivery, collected at the door, out
        // of scope once cancelled; every in-flight state (including
        // legacy `placed`) reads 未収.
        assert_eq!(super::payment_status_label("received"), "代引き（未収）");
        assert_eq!(super::payment_status_label("received"), "代引き（未収）");
        assert_eq!(super::payment_status_label("preparing"), "代引き（未収）");
        assert_eq!(super::payment_status_label("shipped"), "代引き（未収）");
        assert_eq!(
            super::payment_status_label("delivered"),
            "代引き（回収済み）"
        );
        assert_eq!(super::payment_status_label("cancelled"), "代引き（対象外）");
    }
}
