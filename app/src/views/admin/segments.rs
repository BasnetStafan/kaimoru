//! Customer segment screens.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::repositories::admin::segment_repository::CustomerSegment;

/// One segment row with its live member count and a
/// human-readable conditions summary.
pub struct SegmentRow {
    /// The definition.
    pub segment: CustomerSegment,
    /// Live matching member count.
    pub member_count: i64,
    /// Conditions in words; すべて when unconditional.
    pub conditions_label: String,
}

/// The segments list + create form page.
#[derive(Template)]
#[template(path = "admin/pages/segments.html")]
pub struct SegmentListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Existing segments with counts.
    pub rows: Vec<SegmentRow>,
    /// Form value echo: name.
    pub form_name: String,
    /// Form value echo: tag.
    pub form_tag: String,
    /// Form value echo: min spend.
    pub form_min_spend: String,
    /// Form value echo: min orders.
    pub form_min_orders: String,
    /// Form value echo: last order before.
    pub form_before: String,
    /// Form value echo: last order after.
    pub form_after: String,
    /// Validation errors, already joined for display.
    pub form_error: String,
}

impl SegmentRow {
    /// Builds a row, summarising the conditions in words.
    pub fn new(segment: CustomerSegment, member_count: i64) -> Self {
        let mut parts: Vec<String> = Vec::new();
        if !segment.tag.is_empty() {
            parts.push(format!("タグ「{}」", segment.tag));
        }
        if let Some(v) = segment.min_total_spend_yen {
            parts.push(format!("累計購入 {v}円以上"));
        }
        if let Some(v) = segment.min_order_count {
            parts.push(format!("注文 {v}回以上"));
        }
        if let Some(d) = segment.last_order_before {
            parts.push(format!("最終注文 {} まで", d.format("%Y/%m/%d")));
        }
        if let Some(d) = segment.last_order_after {
            parts.push(format!("最終注文 {} 以降", d.format("%Y/%m/%d")));
        }
        let conditions_label = if parts.is_empty() {
            "すべて（条件なし）".to_string()
        } else {
            parts.join("、")
        };
        Self {
            segment,
            member_count,
            conditions_label,
        }
    }
}
