//! Carrier domain: the shipping carriers the shop can
//! hand a parcel to, and their tracking URL shapes.

/// One delivery carrier the operator can name at ship time.
pub struct Carrier {
    /// Stable code stored on the order.
    pub code: &'static str,
    /// Japanese label shown to the operator and the customer.
    pub label: &'static str,
    /// The carrier's public tracking page, with `{number}`
    /// where the tracking number goes; `None` for a carrier
    /// with no deep-linkable tracking page.
    pub tracking_url_template: Option<&'static str>,
}

/// The carriers the ship form offers, in display order. The
/// labels and URL templates are presentation constants of
/// Japanese logistics — the same philosophy as
/// [`crate::models::delivery::DELIVERY_SLOTS`] — so the database stores only the code.
pub const CARRIERS: &[Carrier] = &[
    Carrier {
        code: "yamato",
        label: "ヤマト運輸",
        tracking_url_template: Some(
            "https://jizen.kuronekoyamato.co.jp/jizen/servlet/crjz.b.NQ0010?id={number}",
        ),
    },
    Carrier {
        code: "sagawa",
        label: "佐川急便",
        tracking_url_template: Some(
            "https://k2k.sagawa-exp.co.jp/p/web/okurijosearch.do?okurijoNo={number}",
        ),
    },
    Carrier {
        code: "japan_post",
        label: "日本郵便（ゆうパック）",
        tracking_url_template: Some(
            "https://trackings.post.japanpost.jp/services/srv/search/direct?reqCodeNo1={number}",
        ),
    },
    Carrier {
        code: "other",
        label: "その他",
        tracking_url_template: None,
    },
];

/// Returns the carrier for a code, or `None` for a code the
/// catalogue does not know (a tampered form value).
pub fn carrier_for(code: &str) -> Option<&'static Carrier> {
    CARRIERS.iter().find(|c| c.code == code)
}

/// Returns the carrier's public tracking URL for a number, or
/// `None` when the carrier has no deep-linkable page.
pub fn tracking_url(carrier_code: &str, tracking_number: &str) -> Option<String> {
    let template = carrier_for(carrier_code)?.tracking_url_template?;
    // Tracking numbers are validated to URL-safe characters
    // (letters, digits, hyphens) before they are stored, so a
    // plain substitution cannot break out of the query value.
    Some(template.replace("{number}", tracking_number))
}

#[cfg(test)]
mod tests {

    #[test]
    fn tracking_url_only_for_deep_linkable_carriers() {
        assert_eq!(
            super::tracking_url("sagawa", "1234-5678").as_deref(),
            Some("https://k2k.sagawa-exp.co.jp/p/web/okurijosearch.do?okurijoNo=1234-5678")
        );
        assert_eq!(super::tracking_url("other", "1234"), None);
        assert_eq!(super::tracking_url("bogus", "1234"), None);
    }
}
