//! View types for the agent checkout settlement
//! the one settlement surface the agent
//! thread and the cart page both render, from the one
//! server-side state — the two surfaces can never disagree
//! about where a settlement stands. In the thread the
//! surface walks the steps (offer, agreement policy,
//! address, payment method, final confirmation); on the
//! cart it only reports state and routes the owner into
//! the thread, because the settlement itself lives in the
//! conversation.

use askama::Template;

use crate::models::agent_approval::ApprovalSnapshot;
use crate::services::cart_service::CartTotals;

/// One line of the settlement surface: display strings only.
pub struct AgentSettlementLineView {
    /// Product name.
    pub name: String,
    /// Variant label (empty for a plain product).
    pub variant_label: String,
    /// Quantity the approval binds.
    pub quantity: i64,
    /// Line total, formatted.
    pub formatted_line_total: String,
}

/// One saved address offered on the address step.
pub struct SavedAddressView {
    /// Address-book row ID (the pick's posted value).
    pub id: i64,
    /// One-line summary: recipient and address.
    pub summary: String,
    /// Whether this is the member's default address (the
    /// one the step offers as the one-tap pick).
    pub is_default: bool,
}

/// One payment method offered on the payment step.
pub struct SettlementMethodView {
    /// Posted method slug.
    pub slug: &'static str,
    /// Customer-facing method name.
    pub label: &'static str,
    /// The note under the method name.
    pub note: String,
    /// Whether this is the walk's currently chosen method
    /// (the payment step marks it and prices by it).
    pub selected: bool,
}

/// The address step's field errors, one slot per field, in
/// the checkout form's idiom: `Some` renders under its
/// field.
#[derive(Default)]
pub struct SettlementAddressErrors {
    /// Error under the email field (guests only).
    pub email: Option<String>,
    /// Error under the family name field.
    pub family_name: Option<String>,
    /// Error under the given name field.
    pub given_name: Option<String>,
    /// Error under the postal code field.
    pub postal_code: Option<String>,
    /// Error under the prefecture field.
    pub prefecture: Option<String>,
    /// Error under the city field.
    pub city: Option<String>,
    /// Error under the street address field.
    pub address_line: Option<String>,
    /// Error under the phone field.
    pub phone: Option<String>,
}

impl SettlementAddressErrors {
    /// Builds the slots from (field, message) pairs — the
    /// shape the settlement state carries its last submit's
    /// validation failures in.
    pub fn from_pairs(pairs: &[(String, String)]) -> SettlementAddressErrors {
        let mut errors = SettlementAddressErrors::default();
        for (field, message) in pairs {
            let slot = match field.as_str() {
                "email" => &mut errors.email,
                "family_name" => &mut errors.family_name,
                "given_name" => &mut errors.given_name,
                "postal_code" => &mut errors.postal_code,
                "prefecture" => &mut errors.prefecture,
                "city" => &mut errors.city,
                "address_line" => &mut errors.address_line,
                "phone" => &mut errors.phone,
                _ => continue,
            };
            *slot = Some(message.clone());
        }
        errors
    }
}

/// The settlement surface's render state. `stage` splits
/// the shapes: the offer (two actions, no settlement yet),
/// the four steps of a live settlement, and the cart's
/// read-only progress report. Fields a stage does not use
/// stay empty.
pub struct AgentSettlementView {
    /// "offer" | "policy" | "address" | "payment" |
    /// "confirm" | "progress".
    pub stage: &'static str,
    /// Whether this render is the chat surface (the steps
    /// render only there; the cart reports and routes).
    pub in_chat: bool,
    /// The cart lines the approval binds.
    pub lines: Vec<AgentSettlementLineView>,
    /// Product subtotal, formatted.
    pub formatted_subtotal: String,
    /// Shipping fee, formatted.
    pub formatted_shipping: String,
    /// Cash-on-delivery fee, formatted.
    pub formatted_cod_fee: String,
    /// Whether the totals include (and list) the COD fee:
    /// only the confirmation of a COD settlement.
    pub show_cod_fee: bool,
    /// Grand total, formatted.
    pub formatted_total: String,
    /// Payment step: the grand total with an online method
    /// (no COD fee), formatted. The step switches the
    /// displayed total between this and the COD total on
    /// the client, with no round trip.
    pub formatted_total_online: String,
    /// Payment step: the grand total with cash on delivery
    /// (fee included), formatted.
    pub formatted_total_cod: String,
    /// Payment step: the chosen method's slug, when one
    /// stands chosen — the client switch's initial state
    /// and the proceed form's method field.
    pub chosen_method_slug: Option<&'static str>,
    /// An explanation the surface must show (a previous
    /// approval's death, a refused step).
    pub notice: Option<String>,
    /// The live approval's deadline, JST, while a
    /// settlement is in progress.
    pub expires_label: Option<String>,
    /// Whether the owner is a member (the address step's
    /// address book and email handling differ).
    pub is_member: bool,
    /// The member's saved addresses for the address step,
    /// default first.
    pub saved_addresses: Vec<SavedAddressView>,
    /// The default saved address's summary for the
    /// one-tap pick, when the member has one.
    pub default_address_summary: Option<String>,
    /// The default saved address's ID, for its pick form.
    pub default_address_id: Option<i64>,
    /// Typed-address values (draft or profile prefill).
    pub v_email: String,
    /// Typed-address values (draft or profile prefill).
    pub v_family_name: String,
    /// Typed-address values (draft or profile prefill).
    pub v_given_name: String,
    /// Typed-address values (draft or profile prefill).
    pub v_postal_code: String,
    /// Typed-address values (draft or profile prefill).
    pub v_prefecture: String,
    /// Typed-address values (draft or profile prefill).
    pub v_city: String,
    /// Typed-address values (draft or profile prefill).
    pub v_address_line: String,
    /// Typed-address values (draft or profile prefill).
    pub v_building: String,
    /// Typed-address values (draft or profile prefill).
    pub v_phone: String,
    /// The address step's field errors.
    pub errors: SettlementAddressErrors,
    /// The 47 prefectures for the address step's select.
    pub prefectures: &'static [&'static str],
    /// The payment step's offered methods.
    pub methods: Vec<SettlementMethodView>,
    /// Whether a payment method has been chosen yet: the
    /// payment step prices by the chosen method and only
    /// then offers the way on to the confirmation.
    pub method_chosen: bool,
    /// The chosen method's name, on the confirmation.
    pub method_label: Option<String>,
    /// Confirmation: recipient name line.
    pub delivery_name: Option<String>,
    /// Confirmation: postal + address lines.
    pub delivery_address: Option<String>,
    /// Confirmation: phone number.
    pub delivery_phone: Option<String>,
    /// Confirmation: contact email.
    pub delivery_email: Option<String>,
    /// The active hosted gateway's display name, for the
    /// copy that names the provider the shopper pays through.
    pub gateway_name: &'static str,
}

impl AgentSettlementView {
    /// A blank surface for `stage` over the given snapshot:
    /// the lines every stage lists, with the step-specific
    /// fields filled in by the builder afterwards.
    pub fn new(
        stage: &'static str,
        in_chat: bool,
        snapshot: &ApprovalSnapshot,
    ) -> AgentSettlementView {
        AgentSettlementView {
            stage,
            in_chat,
            gateway_name: crate::services::payment_service::active_gateway_name(),
            lines: snapshot
                .lines
                .iter()
                .map(|line| AgentSettlementLineView {
                    name: line.name.clone(),
                    variant_label: line.variant_label.clone(),
                    quantity: line.quantity,
                    formatted_line_total: crate::pricing::display_price(line.line_total_yen),
                })
                .collect(),
            formatted_subtotal: crate::pricing::display_price(snapshot.subtotal_yen),
            formatted_shipping: crate::pricing::display_price(snapshot.shipping_yen),
            formatted_cod_fee: crate::pricing::display_price(snapshot.cod_fee_yen),
            show_cod_fee: false,
            formatted_total: crate::pricing::display_price(snapshot.total_yen),
            formatted_total_online: crate::pricing::display_price(snapshot.total_yen),
            formatted_total_cod: crate::pricing::display_price(snapshot.total_yen),
            chosen_method_slug: None,
            notice: None,
            expires_label: None,
            is_member: false,
            saved_addresses: Vec::new(),
            default_address_summary: None,
            default_address_id: None,
            v_email: String::new(),
            v_family_name: String::new(),
            v_given_name: String::new(),
            v_postal_code: String::new(),
            v_prefecture: String::new(),
            v_city: String::new(),
            v_address_line: String::new(),
            v_building: String::new(),
            v_phone: String::new(),
            errors: SettlementAddressErrors::default(),
            prefectures: &[],
            methods: Vec::new(),
            method_chosen: false,
            method_label: None,
            delivery_name: None,
            delivery_address: None,
            delivery_phone: None,
            delivery_email: None,
        }
    }

    /// Replaces the totals with a freshly computed set —
    /// the confirmation's method-dependent totals differ
    /// from the snapshot's no-method ones (the COD fee).
    /// Payment step: prices the surface by both method
    /// families at once — online methods share one total,
    /// cash on delivery adds its fee — and records which
    /// family the currently chosen method belongs to. The
    /// displayed total and COD-fee row follow the choice;
    /// the client re-derives both from the two totals when
    /// the shopper switches methods in place.
    pub fn fill_payment_totals(
        &mut self,
        online: &CartTotals,
        cod: &CartTotals,
        chosen: Option<crate::models::PaymentMethod>,
    ) {
        self.formatted_subtotal = crate::pricing::display_price(online.subtotal_yen);
        self.formatted_shipping = crate::pricing::display_price(online.shipping_yen);
        self.formatted_cod_fee = crate::pricing::display_price(cod.cod_fee_yen);
        self.formatted_total_online = crate::pricing::display_price(online.total_yen);
        self.formatted_total_cod = crate::pricing::display_price(cod.total_yen);
        self.show_cod_fee = chosen == Some(crate::models::PaymentMethod::Cod);
        self.formatted_total = crate::pricing::display_price(if self.show_cod_fee {
            cod.total_yen
        } else {
            online.total_yen
        });
        self.chosen_method_slug = chosen.map(|method| method.slug());
    }

    /// Copies the cart totals into the view's formatted
    /// money fields, and the cash-on-delivery row's
    /// visibility from whether a COD fee applies.
    pub fn fill_totals(&mut self, totals: &CartTotals) {
        self.formatted_subtotal = crate::pricing::display_price(totals.subtotal_yen);
        self.formatted_shipping = crate::pricing::display_price(totals.shipping_yen);
        self.formatted_cod_fee = crate::pricing::display_price(totals.cod_fee_yen);
        self.show_cod_fee = totals.cod_fee_yen > 0;
        self.formatted_total = crate::pricing::display_price(totals.total_yen);
    }
}

/// The htmx answer to a settlement step post: just the
/// settlement slot's new content (the shared partial is
/// the whole template).
#[derive(Template)]
#[template(path = "storefront/parts/agent_settlement.html")]
pub struct AgentSettlementFragmentView {
    /// The settlement surface, or `None` for an empty slot.
    pub settlement: Option<AgentSettlementView>,
}
