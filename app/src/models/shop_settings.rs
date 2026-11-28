//! Shop settings model.
//! The operator-editable settings of the shop, held as one
//! typed row (CMS spec C-1): the shop's words, plus — since
//! the shipping/delivery settings domain — the fees and the
//! delivery window an operator legitimately tunes. Developer
//! configuration still never lives here: payment flags and
//! secrets stay in config and environment variables.

/// The shop's settings: the single `shop_settings` row.
#[derive(Clone, sqlx::FromRow)]
pub struct ShopSettings {
    /// Shop name, shown in the header logo, page titles and footer.
    pub shop_name: String,
    /// One-line catchphrase; empty when the shop has none.
    pub tagline: String,
    /// Plain-text shop introduction; also the meta description
    /// fallback when no SEO default is set.
    pub shop_description: String,
    /// Public contact address; empty hides the footer's contact
    /// line (no channel chosen yet is a valid state).
    pub contact_email: String,
    /// Contact page URL: an internal `/…` path or an `https:` URL.
    pub contact_url: String,
    /// Support note shown beside the contact line (hours and the like).
    pub support_note: String,
    /// Extra plain-text line in the footer; empty shows nothing.
    pub footer_note: String,
    /// Copyright holder; empty falls back to the shop name.
    pub copyright_holder: String,
    /// Top page `<title>` override; empty uses the name pattern.
    pub top_title: String,
    /// Default meta description; empty falls back to the shop
    /// description.
    pub default_meta_description: String,
    /// Mail signature for the notification mails' `{signature}`
    /// tag; empty falls back to the shop name.
    pub mail_signature: String,
    /// Flat shipping fee in integer yen (tax included).
    pub shipping_flat_yen: i64,
    /// Product subtotal at or above which shipping is free,
    /// in integer yen.
    pub free_shipping_threshold_yen: i64,
    /// Cash-on-delivery fee in integer yen (tax included),
    /// flat per order.
    pub cod_fee_yen: i64,
    /// Days after the order date of the earliest selectable
    /// delivery date.
    pub delivery_earliest_offset_days: i32,
    /// Days after the order date of the latest selectable
    /// delivery date.
    pub delivery_latest_offset_days: i32,
    /// Comma-joined delivery slot codes the shop does not
    /// offer; empty offers the whole catalogue.
    pub disabled_delivery_slots: String,
    /// Comma-joined carrier codes the shop does not use;
    /// empty offers the whole catalogue.
    pub disabled_carriers: String,
    /// Qualified-invoice registration number
    /// (適格請求書発行事業者登録番号, `T` + 13 digits); empty =
    /// not registered, in which case printed documents omit
    /// it entirely. The value is the owner's fact (D-5) —
    /// never defaulted or invented here.
    pub invoice_registration_number: String,
    /// Whether the points programme is live (EC V2 core 1;
    /// internal points design record). Off until the operator turns it
    /// on: no earn, no spend, no points UI.
    pub points_enabled: bool,
    /// Points earned per 100 yen of the earn base (1 = 1%).
    pub points_earn_percent: i32,
    /// Days a points grant stays spendable.
    pub points_expiry_days: i32,
    /// Whether the storefront is closed for maintenance
    ///.
    pub maintenance_mode: bool,
    /// The closed page's message; empty renders the
    /// code default.
    pub maintenance_message: String,
}

/// The fee triple the pricing path consumes, lifted out of
/// the settings row: [`crate::services::cart_service::totals`]
/// takes it as a parameter (the `TaxParams` pattern), so the
/// pure totals computation never reads the database itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeeSettings {
    /// Flat shipping fee in integer yen.
    pub shipping_flat_yen: i64,
    /// Free-shipping threshold in integer yen.
    pub free_shipping_threshold_yen: i64,
    /// Cash-on-delivery fee in integer yen.
    pub cod_fee_yen: i64,
}

impl FeeSettings {
    /// The long-standing hard-coded fees: the fallback when
    /// no settings row can be read, and the pricing module's
    /// constants are these same values.
    pub fn fallback() -> Self {
        Self {
            shipping_flat_yen: crate::pricing::SHIPPING_FLAT_YEN,
            free_shipping_threshold_yen: crate::pricing::FREE_SHIPPING_THRESHOLD_YEN,
            cod_fee_yen: crate::pricing::COD_FEE_YEN,
        }
    }
}

/// The points triple the order/checkout paths consume,
/// lifted out of the settings row (the `FeeSettings`
/// pattern, so the pure computations never read the
/// database).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointsSettings {
    /// Whether the programme is live.
    pub enabled: bool,
    /// Points earned per 100 yen of the earn base.
    pub earn_percent: i32,
    /// Days a grant stays spendable.
    pub expiry_days: i32,
}

impl PointsSettings {
    /// The spec defaults: the fallback when no settings row
    /// can be read (programme off, so a broken settings read
    /// never starts handing out points).
    pub fn fallback() -> Self {
        Self {
            enabled: false,
            earn_percent: 1,
            expiry_days: 180,
        }
    }
}

impl ShopSettings {
    /// Returns the points settings the order paths consume.
    pub fn points_settings(&self) -> PointsSettings {
        PointsSettings {
            enabled: self.points_enabled,
            earn_percent: self.points_earn_percent,
            expiry_days: self.points_expiry_days,
        }
    }

    /// Returns the settings the templates hard-coded before this
    /// table existed.
    ///
    /// The belt-and-braces read path: if the row is ever missing
    /// (or the read fails), the store keeps rendering with these
    /// values instead of failing every page.
    pub fn fallback() -> Self {
        Self {
            shop_name: "Agentic Store".to_string(),
            tagline: String::new(),
            shop_description: "AIエージェントと買い物ができるECストア".to_string(),
            contact_email: String::new(),
            contact_url: String::new(),
            support_note: String::new(),
            footer_note: String::new(),
            copyright_holder: String::new(),
            top_title: String::new(),
            default_meta_description:
                "Agentic Store — AIエージェントと買い物ができるECストア。商品検索、カート、代金引換でのご注文ができます。"
                    .to_string(),
            mail_signature: String::new(),
            shipping_flat_yen: crate::pricing::SHIPPING_FLAT_YEN,
            free_shipping_threshold_yen: crate::pricing::FREE_SHIPPING_THRESHOLD_YEN,
            cod_fee_yen: crate::pricing::COD_FEE_YEN,
            delivery_earliest_offset_days:
                crate::models::delivery::DELIVERY_EARLIEST_OFFSET_DAYS as i32,
            delivery_latest_offset_days: crate::models::delivery::DELIVERY_LATEST_OFFSET_DAYS
                as i32,
            disabled_delivery_slots: String::new(),
            disabled_carriers: String::new(),
            invoice_registration_number: String::new(),
            points_enabled: false,
            points_earn_percent: 1,
            points_expiry_days: 180,
            maintenance_mode: false,
            maintenance_message: String::new(),
        }
    }

    /// Returns the fee triple the pricing path consumes.
    pub fn fee_settings(&self) -> FeeSettings {
        FeeSettings {
            shipping_flat_yen: self.shipping_flat_yen,
            free_shipping_threshold_yen: self.free_shipping_threshold_yen,
            cod_fee_yen: self.cod_fee_yen,
        }
    }

    /// Returns the delivery window offsets in days, as the
    /// `(earliest, latest)` pair the delivery model consumes.
    pub fn delivery_offsets(&self) -> (i64, i64) {
        (
            i64::from(self.delivery_earliest_offset_days),
            i64::from(self.delivery_latest_offset_days),
        )
    }

    /// Whether a delivery slot code is disabled by the shop.
    /// The `none` slot is never disabled: a shopper must
    /// always be able to place an order without a preference.
    pub fn slot_disabled(&self, code: &str) -> bool {
        code != "none"
            && self
                .disabled_delivery_slots
                .split(',')
                .any(|c| c.trim() == code)
    }

    /// Whether a carrier code is disabled by the shop.
    pub fn carrier_disabled(&self, code: &str) -> bool {
        self.disabled_carriers.split(',').any(|c| c.trim() == code)
    }

    /// Returns the logo text: the shop name upper-cased, the
    /// stylised rendering the header has always used (a no-op
    /// for Japanese names, which have no letter case).
    pub fn logo_text(&self) -> String {
        self.shop_name.to_uppercase()
    }

    /// Returns the copyright holder: the stored holder, or the
    /// shop name when none is set.
    pub fn copyright_holder(&self) -> &str {
        if self.copyright_holder.is_empty() {
            &self.shop_name
        } else {
            &self.copyright_holder
        }
    }

    /// Returns the meta description for a page: the SEO default,
    /// or the shop description when no default is set.
    pub fn meta_description(&self) -> &str {
        if self.default_meta_description.is_empty() {
            &self.shop_description
        } else {
            &self.default_meta_description
        }
    }

    /// Returns the top page's `<title>`: the override when set,
    /// otherwise the long-standing `name | トップページ` pattern.
    pub fn top_page_title(&self) -> String {
        if self.top_title.is_empty() {
            format!("{} | トップページ", self.shop_name)
        } else {
            self.top_title.clone()
        }
    }

    /// Whether the footer's contact line renders at all: it
    /// needs at least an address or a page to point at.
    pub fn has_contact(&self) -> bool {
        !self.contact_email.is_empty() || !self.contact_url.is_empty()
    }
}
