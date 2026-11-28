//! The MCP tools' parameter types and their boundary
//! validation. The tool handlers themselves live in the
//! parent module, next to the `ServerHandler` impl the
//! `#[tool_handler]` macro completes. Validation lives here,
//! at the protocol boundary — a bad parameter is a protocol
//! error naming the parameter, never a silent fallback.

use rmcp::{ErrorData as McpError, schemars::JsonSchema};
use serde::{Deserialize, Serialize};

use crate::models::agent_approval::{ApprovalSnapshot, ApprovalSnapshotLine};
use crate::services::api_cart_service::{ApiCart, ApiCartLine};
use crate::services::mcp_order_draft_service::DraftAddress;
use crate::validation;

/// Parameters for `search_products`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SearchProductsParams {
    /// Keyword text matched against product names and
    /// categories (multi-term AND). Empty or omitted browses
    /// the whole catalog.
    pub query: Option<String>,
    /// Category name, exactly as `list_categories` reports it.
    pub category: Option<String>,
    /// Lowest price to keep, in integer yen (inclusive).
    pub min_price_yen: Option<i64>,
    /// Highest price to keep, in integer yen (inclusive).
    pub max_price_yen: Option<i64>,
    /// One of: recommended, newest, price_asc, price_desc,
    /// rating. Defaults to recommended.
    pub sort: Option<String>,
    /// Keep only products with stock on hand.
    pub in_stock_only: Option<bool>,
    /// 1-based page number. Defaults to 1.
    pub page: Option<i64>,
}

/// Parameters for `get_product`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetProductParams {
    /// The product id, from a search or list result.
    pub id: i64,
}

/// Parameters for the cart tools that address one cart.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct CartIdParams {
    /// The cart handle, from `create_cart`.
    pub cart_id: String,
}

/// Parameters for `add_cart_item`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct AddCartItemParams {
    /// The cart handle, from `create_cart`.
    pub cart_id: String,
    /// The product id, from a search or list result.
    pub product_id: i64,
    /// The combination to add; required when the product
    /// has combinations, omitted for a plain product.
    pub variant_id: Option<i64>,
    /// Units to add (1 to 999999).
    pub quantity: i32,
}

/// Parameters for `update_cart_item`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct UpdateCartItemParams {
    /// The cart handle, from `create_cart`.
    pub cart_id: String,
    /// The line's variant id.
    pub variant_id: i64,
    /// The line's new quantity (at least 1; stock-capped
    /// by the cart service).
    pub quantity: i32,
}

/// Parameters for `remove_cart_item`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RemoveCartItemParams {
    /// The cart handle, from `create_cart`.
    pub cart_id: String,
    /// The line's variant id.
    pub variant_id: i64,
}

/// One MCP cart as the cart tools return it: the API cart's
/// lines and totals, plus the handle they belong to. Totals
/// come from the store's one totals implementation — the
/// same numbers the cart page shows.
#[derive(Serialize, JsonSchema)]
pub(crate) struct McpCart {
    /// The cart handle.
    pub cart_id: String,
    /// The cart's lines, in cart order.
    pub lines: Vec<ApiCartLine>,
    /// Total units across all lines.
    pub item_count: i64,
    /// Sum of purchasable line totals, in yen.
    pub subtotal_yen: i64,
    /// Shipping fee in yen (free at the store's threshold).
    pub shipping_yen: i64,
    /// Grand total in yen: subtotal plus shipping.
    pub total_yen: i64,
}

impl McpCart {
    /// Wraps an API cart view under its handle.
    pub(crate) fn new(cart_id: String, cart: ApiCart) -> Self {
        Self {
            cart_id,
            lines: cart.lines,
            item_count: cart.item_count,
            subtotal_yen: cart.subtotal_yen,
            shipping_yen: cart.shipping_yen,
            total_yen: cart.total_yen,
        }
    }
}

/// The browser handoff the `get_checkout_link` tool returns:
/// a person opens the URL and the cart becomes theirs.
#[derive(Serialize, JsonSchema)]
pub(crate) struct McpCheckoutLink {
    /// The cart handle the link hands off.
    pub cart_id: String,
    /// The single-use claim URL to open in a browser.
    pub checkout_url: String,
    /// When the link stops working, as an RFC 3339 timestamp.
    pub expires_at: String,
}

/// Parameters for `create_order_draft`: the cart to
/// freeze, the idempotency key that makes the create safe
/// to retry, and the delivery address the draft orders
/// to — field for field the guest checkout's delivery
/// step, because the draft is placed as a guest order.
// No Debug derive: the address is personal data, and a
// parameter struct is one log line away from leaking it.
#[derive(Deserialize, JsonSchema)]
pub(crate) struct CreateOrderDraftParams {
    /// The cart handle, from `create_cart`.
    pub cart_id: String,
    /// A caller-chosen key: repeating a create with the
    /// same key returns the same draft instead of drafting
    /// twice. At most 128 characters.
    pub idempotency_key: String,
    /// The delivery address the order will ship to.
    pub address: DraftAddressParams,
}

/// The delivery address inside [`CreateOrderDraftParams`].
#[derive(Deserialize, JsonSchema)]
pub(crate) struct DraftAddressParams {
    /// Contact email the order confirms to.
    pub email: String,
    /// Recipient family name.
    pub family_name: String,
    /// Recipient given name.
    pub given_name: String,
    /// Postal code (hyphens optional).
    pub postal_code: String,
    /// Prefecture.
    pub prefecture: String,
    /// City / ward.
    pub city: String,
    /// Street address.
    pub address_line: String,
    /// Building, when given.
    pub building: Option<String>,
    /// Phone number (hyphens optional).
    pub phone: String,
}

/// Parameters for `get_order_draft`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct DraftIdParams {
    /// The draft id, from `create_order_draft`.
    pub draft_id: i64,
}

/// Parameters for `get_order_status`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct OrderIdParams {
    /// The order id, from `get_order_draft` once its
    /// draft is consumed.
    pub order_id: i64,
}

/// One line of a draft, from its frozen snapshot: what
/// the approval page listed at creation (or, after the
/// pay click, in the final binding).
#[derive(Serialize, JsonSchema)]
pub(crate) struct McpDraftLine {
    /// The line's variant id.
    pub variant_id: i64,
    /// The line's product id.
    pub product_id: i64,
    /// Product name at draft time.
    pub name: String,
    /// Combination label; empty for a plain product.
    pub variant_label: String,
    /// Unit price in integer yen, tax included.
    pub unit_price_yen: i64,
    /// Units in the draft.
    pub quantity: i64,
    /// Line total in integer yen.
    pub line_total_yen: i64,
}

impl From<&ApprovalSnapshotLine> for McpDraftLine {
    fn from(line: &ApprovalSnapshotLine) -> Self {
        Self {
            variant_id: line.variant_id,
            product_id: line.product_id,
            name: line.name.clone(),
            variant_label: line.variant_label.clone(),
            unit_price_yen: line.unit_price_yen,
            quantity: line.quantity,
            line_total_yen: line.line_total_yen,
        }
    }
}

/// One order draft as the order tools return it: the
/// frozen lines and totals from its snapshot, its status,
/// and — at creation only — the approval URL a person
/// opens to approve and pay. The URL is never shown again
/// (only its digest is stored), so `approval_url` is
/// present exactly on the create answer.
#[derive(Serialize, JsonSchema)]
pub(crate) struct McpOrderDraft {
    /// The draft's id.
    pub draft_id: i64,
    /// `pending`, `consumed`, `expired` or `void`.
    pub status: String,
    /// The placed order's id, once the draft is consumed.
    pub order_id: Option<i64>,
    /// The browser approval URL (create answer only).
    pub approval_url: Option<String>,
    /// When the approval window closes, RFC 3339.
    pub expires_at: String,
    /// The draft's lines, from its snapshot.
    pub lines: Vec<McpDraftLine>,
    /// Sum of line totals in yen.
    pub subtotal_yen: i64,
    /// Shipping fee in yen.
    pub shipping_yen: i64,
    /// Grand total in yen.
    pub total_yen: i64,
}

impl McpOrderDraft {
    /// Shapes a draft from its stored parts and snapshot.
    pub(crate) fn new(
        draft_id: i64,
        status: String,
        order_id: Option<i64>,
        approval_url: Option<String>,
        expires_at: String,
        snapshot: &ApprovalSnapshot,
    ) -> Self {
        Self {
            draft_id,
            status,
            order_id,
            approval_url,
            expires_at,
            lines: snapshot.lines.iter().map(McpDraftLine::from).collect(),
            subtotal_yen: snapshot.subtotal_yen,
            shipping_yen: snapshot.shipping_yen,
            total_yen: snapshot.total_yen,
        }
    }
}

/// One placed order's status as `get_order_status`
/// returns it: fulfilment status and shipment facts
/// only — never the buyer's details.
#[derive(Serialize, JsonSchema)]
pub(crate) struct McpOrderStatus {
    /// The order's id.
    pub order_id: i64,
    /// Fulfilment status: `received`, `preparing`,
    /// `shipped`, `delivered` or `cancelled`.
    pub status: String,
    /// Grand total in integer yen.
    pub total_yen: i64,
    /// When the order was placed, RFC 3339.
    pub created_at: String,
    /// When the order was delivered, RFC 3339, when it
    /// has been.
    pub delivered_at: Option<String>,
    /// Delivery carrier code, once shipped with one named.
    pub carrier: Option<String>,
    /// Carrier tracking number, once shipped with one.
    pub tracking_number: Option<String>,
}

/// The validated draft inputs the service call runs with.
pub(crate) struct ValidDraft {
    /// The idempotency key, trimmed.
    pub(crate) idempotency_key: String,
    /// The delivery address, fields trimmed.
    pub(crate) address: DraftAddress,
}

/// Length caps for the draft address — the guest
/// checkout's own caps (`order_service::validate_delivery`),
/// mirrored at the protocol boundary so a bad address is
/// refused here instead of at the approval page.
const DRAFT_NAME_MAX_CHARS: usize = 50;
const DRAFT_EMAIL_MAX_CHARS: usize = 254;
const DRAFT_ADDRESS_MAX_CHARS: usize = 100;
/// The idempotency key's length cap.
const IDEMPOTENCY_KEY_MAX_CHARS: usize = 128;

/// Validates the draft parameters at the boundary, with
/// the guest delivery step's shape rules: the same
/// required fields, the same email / postal / phone
/// shapes, the same length caps. A failure names the
/// parameter, in the protocol's terms.
pub(crate) fn validate_draft(params: &CreateOrderDraftParams) -> Result<ValidDraft, McpError> {
    let idempotency_key = params.idempotency_key.trim();
    if idempotency_key.is_empty() {
        return Err(McpError::invalid_params(
            "idempotency_key is required",
            None,
        ));
    }
    if idempotency_key.chars().count() > IDEMPOTENCY_KEY_MAX_CHARS {
        return Err(McpError::invalid_params(
            format!("idempotency_key must be at most {IDEMPOTENCY_KEY_MAX_CHARS} characters"),
            None,
        ));
    }
    let a = &params.address;
    let email = a.email.trim();
    if email.is_empty() {
        return Err(McpError::invalid_params("address.email is required", None));
    }
    if email.chars().count() > DRAFT_EMAIL_MAX_CHARS || !validation::is_valid_email(email) {
        return Err(McpError::invalid_params(
            "address.email must be a valid email address",
            None,
        ));
    }
    for (field, value, cap) in [
        (
            "address.family_name",
            a.family_name.as_str(),
            DRAFT_NAME_MAX_CHARS,
        ),
        (
            "address.given_name",
            a.given_name.as_str(),
            DRAFT_NAME_MAX_CHARS,
        ),
        (
            "address.prefecture",
            a.prefecture.as_str(),
            DRAFT_ADDRESS_MAX_CHARS,
        ),
        ("address.city", a.city.as_str(), DRAFT_ADDRESS_MAX_CHARS),
        (
            "address.address_line",
            a.address_line.as_str(),
            DRAFT_ADDRESS_MAX_CHARS,
        ),
    ] {
        if value.trim().is_empty() {
            return Err(McpError::invalid_params(
                format!("{field} is required"),
                None,
            ));
        }
        if value.chars().count() > cap {
            return Err(McpError::invalid_params(
                format!("{field} must be at most {cap} characters"),
                None,
            ));
        }
    }
    let building = a.building.as_deref().unwrap_or("");
    if building.chars().count() > DRAFT_ADDRESS_MAX_CHARS {
        return Err(McpError::invalid_params(
            format!("address.building must be at most {DRAFT_ADDRESS_MAX_CHARS} characters"),
            None,
        ));
    }
    if a.postal_code.trim().is_empty() {
        return Err(McpError::invalid_params(
            "address.postal_code is required",
            None,
        ));
    }
    if validation::postal_code_digits(&a.postal_code).is_none() {
        return Err(McpError::invalid_params(
            "address.postal_code must be a Japanese postal code",
            None,
        ));
    }
    if a.phone.trim().is_empty() {
        return Err(McpError::invalid_params("address.phone is required", None));
    }
    if validation::phone_digits(&a.phone).is_none() {
        return Err(McpError::invalid_params(
            "address.phone must be a valid phone number",
            None,
        ));
    }
    Ok(ValidDraft {
        idempotency_key: idempotency_key.to_string(),
        address: DraftAddress {
            email: email.to_string(),
            family_name: a.family_name.trim().to_string(),
            given_name: a.given_name.trim().to_string(),
            postal_code: a.postal_code.trim().to_string(),
            prefecture: a.prefecture.trim().to_string(),
            city: a.city.trim().to_string(),
            address_line: a.address_line.trim().to_string(),
            building: building.trim().to_string(),
            phone: a.phone.trim().to_string(),
        },
    })
}

/// The sort keys the search tool accepts — the storefront
/// search's allowlist, mirrored for the protocol boundary so
/// an unknown key is refused instead of silently re-sorted.
const SORT_KEYS: [&str; 5] = ["recommended", "newest", "price_asc", "price_desc", "rating"];

/// The validated search inputs the service call runs with.
pub(crate) struct ValidSearch {
    /// 1-based page, at least 1.
    pub(crate) page: i64,
    /// The validated sort key.
    pub(crate) sort: String,
}

/// Validates the search parameters at the boundary.
pub(crate) fn validate_search(params: &SearchProductsParams) -> Result<ValidSearch, McpError> {
    let page = params.page.unwrap_or(1);
    if page < 1 {
        return Err(McpError::invalid_params("page must be 1 or greater", None));
    }
    for (name, value) in [
        ("min_price_yen", params.min_price_yen),
        ("max_price_yen", params.max_price_yen),
    ] {
        if let Some(value) = value
            && value < 0
        {
            return Err(McpError::invalid_params(
                format!("{name} must not be negative"),
                None,
            ));
        }
    }
    if let (Some(min), Some(max)) = (params.min_price_yen, params.max_price_yen)
        && min > max
    {
        return Err(McpError::invalid_params(
            "min_price_yen must not exceed max_price_yen",
            None,
        ));
    }
    let sort = params
        .sort
        .clone()
        .unwrap_or_else(|| "recommended".to_string());
    if !SORT_KEYS.contains(&sort.as_str()) {
        return Err(McpError::invalid_params(
            format!("sort must be one of: {}", SORT_KEYS.join(", ")),
            None,
        ));
    }
    Ok(ValidSearch { page, sort })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds params with only the fields a case sets.
    fn params() -> SearchProductsParams {
        SearchProductsParams {
            query: None,
            category: None,
            min_price_yen: None,
            max_price_yen: None,
            sort: None,
            in_stock_only: None,
            page: None,
        }
    }

    #[test]
    fn defaults_are_valid() {
        let valid = validate_search(&params()).expect("defaults validate");
        assert_eq!(valid.page, 1);
        assert_eq!(valid.sort, "recommended");
    }

    #[test]
    fn page_below_one_is_refused() {
        let mut p = params();
        p.page = Some(0);
        assert!(validate_search(&p).is_err());
    }

    #[test]
    fn negative_price_is_refused() {
        let mut p = params();
        p.min_price_yen = Some(-1);
        assert!(validate_search(&p).is_err());
    }

    #[test]
    fn inverted_price_range_is_refused() {
        let mut p = params();
        p.min_price_yen = Some(5000);
        p.max_price_yen = Some(100);
        assert!(validate_search(&p).is_err());
    }

    #[test]
    fn unknown_sort_is_refused() {
        let mut p = params();
        p.sort = Some("cheapest".to_string());
        assert!(validate_search(&p).is_err());
    }

    /// Builds draft params with a complete valid address.
    fn draft_params() -> CreateOrderDraftParams {
        CreateOrderDraftParams {
            cart_id: "mcp_0123456789abcdef0123456789abcdef".to_string(),
            idempotency_key: "order-1".to_string(),
            address: DraftAddressParams {
                email: "buyer@example.com".to_string(),
                family_name: "検証".to_string(),
                given_name: "太郎".to_string(),
                postal_code: "184-0004".to_string(),
                prefecture: "東京都".to_string(),
                city: "小金井市".to_string(),
                address_line: "本町1-1-1".to_string(),
                building: None,
                phone: "09012345678".to_string(),
            },
        }
    }

    #[test]
    fn valid_draft_passes_and_trims() {
        let mut p = draft_params();
        p.address.family_name = " 検証 ".to_string();
        let valid = validate_draft(&p).expect("valid draft passes");
        assert_eq!(valid.idempotency_key, "order-1");
        assert_eq!(valid.address.family_name, "検証");
        assert_eq!(valid.address.building, "");
    }

    #[test]
    fn draft_refuses_bad_email_and_missing_name() {
        let mut p = draft_params();
        p.address.email = "not-an-email".to_string();
        assert!(validate_draft(&p).is_err());
        let mut p = draft_params();
        p.address.family_name = "  ".to_string();
        assert!(validate_draft(&p).is_err());
    }

    #[test]
    fn draft_refuses_bad_postal_phone_and_empty_key() {
        let mut p = draft_params();
        p.address.postal_code = "1234".to_string();
        assert!(validate_draft(&p).is_err());
        let mut p = draft_params();
        p.address.phone = "abc".to_string();
        assert!(validate_draft(&p).is_err());
        let mut p = draft_params();
        p.idempotency_key = " ".to_string();
        assert!(validate_draft(&p).is_err());
    }
}
