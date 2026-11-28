//! Return request workflow states.
//!
//! The walk is linear with an early exit: requested →
//! approved → awaiting_return → inspected → refunded, and
//! rejected from the two decision points. `refunded` is a
//! workflow outcome — the money moves only through the
//! order page's existing refund action.

/// The Japanese label for a status code.
pub fn status_label(code: &str) -> &'static str {
    match code {
        "requested" => "申請中",
        "approved" => "承認",
        "awaiting_return" => "返送待ち",
        "inspected" => "検品済",
        "refunded" => "返金済",
        "rejected" => "却下",
        _ => "不明",
    }
}

/// The statuses reachable from `code` in one step.
pub fn legal_next_statuses(code: &str) -> &'static [&'static str] {
    match code {
        "requested" => &["approved", "rejected"],
        "approved" => &["awaiting_return", "rejected"],
        "awaiting_return" => &["inspected"],
        "inspected" => &["refunded"],
        _ => &[],
    }
}

/// Whether the status is terminal (refunded or rejected).
pub fn is_terminal(code: &str) -> bool {
    matches!(code, "refunded" | "rejected")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_walk_is_linear_with_rejection_exits() {
        assert_eq!(legal_next_statuses("requested"), &["approved", "rejected"]);
        assert_eq!(legal_next_statuses("inspected"), &["refunded"]);
        assert!(legal_next_statuses("refunded").is_empty());
        assert!(is_terminal("rejected"));
        assert!(!is_terminal("approved"));
    }
}
