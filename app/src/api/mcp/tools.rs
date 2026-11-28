//! The MCP tools' boundary validation. The parameter and
//! result types live in the shared `mcp-defs` crate — both
//! transports serve them — and are re-exported here for
//! the handlers in the parent module. Validation lives in
//! this module, at the protocol boundary: a bad parameter
//! is a protocol error naming the parameter, never a
//! silent fallback.

use rmcp::ErrorData as McpError;

use crate::models::agent_approval::ApprovalSnapshotLine;
use crate::services::mcp_order_draft_service::DraftAddress;
use crate::validation;

pub(crate) use mcp_defs::{
    AddCartItemParams, CartIdParams, CreateOrderDraftParams, DraftIdParams, GetProductParams,
    McpCart, McpCheckoutLink, McpDraftLine, McpOrderDraft, McpOrderStatus, OrderIdParams,
    RemoveCartItemParams, SearchProductsParams, UpdateCartItemParams,
};

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
    use mcp_defs::DraftAddressParams;

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
