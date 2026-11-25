//! A customer inquiry and the shop's replies (Contact spec
//! §4). Inquiries are personal data: their contents are never
//! logged, and the order number is stored exactly as typed —
//! it is matched to a real order only on the admin side, so
//! submitting one can never probe whether an order exists.

use chrono::{DateTime, Utc};

/// One inquiry category: the fixed triage list.
pub struct Category {
    /// Stable code stored on the row.
    pub code: &'static str,
    /// Japanese label.
    pub label: &'static str,
}

/// The five inquiry categories, in form order.
pub const CATEGORIES: [Category; 5] = [
    Category {
        code: "product",
        label: "商品について",
    },
    Category {
        code: "order",
        label: "ご注文・配送について",
    },
    Category {
        code: "returns",
        label: "返品・交換について",
    },
    Category {
        code: "payment",
        label: "お支払いについて",
    },
    Category {
        code: "other",
        label: "その他",
    },
];

/// Looks up a category by its code.
pub fn category_for(code: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.code == code)
}

/// The return/exchange reasons of the member order page's
/// 返品・交換の申請 form, in form order. A request is filed
/// as an ordinary inquiry of the `returns` category; the
/// reason rides at the head of its body so the operator
/// triages from the inbox without a second table.
pub const RETURN_REASONS: [Category; 5] = [
    Category {
        code: "defective",
        label: "商品の不良・破損",
    },
    Category {
        code: "wrong_item",
        label: "誤配送・違う商品が届いた",
    },
    Category {
        code: "not_as_expected",
        label: "サイズ・イメージ違い",
    },
    Category {
        code: "ordered_by_mistake",
        label: "注文間違い",
    },
    Category {
        code: "other",
        label: "その他",
    },
];

/// Looks up a return reason by its code.
pub fn return_reason_for(code: &str) -> Option<&'static Category> {
    RETURN_REASONS.iter().find(|c| c.code == code)
}

/// The label for a stored category code.
pub fn category_label(code: &str) -> &'static str {
    category_for(code).map(|c| c.label).unwrap_or("その他")
}

/// One inquiry status in the admin workflow.
pub struct Status {
    /// Stable code stored on the row.
    pub code: &'static str,
    /// Japanese label.
    pub label: &'static str,
}

/// The three inquiry statuses: freely settable in any
/// direction (a one-operator inbox needs no
/// forward-only machine).
pub const STATUSES: [Status; 3] = [
    Status {
        code: "open",
        label: "未対応",
    },
    Status {
        code: "in_progress",
        label: "対応中",
    },
    Status {
        code: "closed",
        label: "対応済み",
    },
];

/// Whether a status code is one of the three.
pub fn status_known(code: &str) -> bool {
    STATUSES.iter().any(|s| s.code == code)
}

/// The label for a stored status code.
pub fn status_label(code: &str) -> &'static str {
    STATUSES
        .iter()
        .find(|s| s.code == code)
        .map(|s| s.label)
        .unwrap_or("未対応")
}

/// One customer inquiry.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Inquiry {
    /// Row id; the 受付番号 derives from it.
    pub id: i64,
    /// The submitting member, when logged in.
    pub user_id: Option<i64>,
    /// Submitter name as typed.
    pub name: String,
    /// Reply address as typed.
    pub email: String,
    /// Category code.
    pub category: String,
    /// Order number as typed, when given.
    pub order_number: Option<String>,
    /// The inquiry body (plain text).
    pub body: String,
    /// Workflow status code.
    pub status: String,
    /// The operator's internal memo; never customer-visible.
    pub admin_memo: String,
    /// When the inquiry arrived.
    pub created_at: DateTime<Utc>,
    /// When the row last changed.
    pub updated_at: DateTime<Utc>,
}

impl Inquiry {
    /// The customer-facing reference: `INQ-` + the id
    /// zero-padded to six digits.
    pub fn reference(&self) -> String {
        format!("INQ-{:06}", self.id)
    }

    /// The category's Japanese label.
    pub fn category_label(&self) -> &'static str {
        category_label(&self.category)
    }

    /// The status's Japanese label.
    pub fn status_label(&self) -> &'static str {
        status_label(&self.status)
    }

    /// The body's first ~40 characters, for the admin list:
    /// newlines flattened, plain text (escaped at render).
    pub fn excerpt(&self) -> String {
        let flat: String = self
            .body
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .collect();
        let mut out: String = flat.chars().take(40).collect();
        if flat.chars().count() > 40 {
            out.push('…');
        }
        out
    }

    /// The received time in JST, admin-list format.
    pub fn received_jst(&self) -> String {
        super::jst::jst_datetime_label(self.created_at)
    }

    /// Whole JST calendar days since the inquiry arrived
    /// (0 = today): the list's triage signal, derived from the
    /// stored instant — no stored priority field exists.
    pub fn age_days(&self) -> i64 {
        let arrived = self
            .created_at
            .with_timezone(&super::jst::jst())
            .date_naive();
        (super::jst::jst_today() - arrived).num_days().max(0)
    }

    /// The age as list wording: 本日 / 昨日 / N日前.
    pub fn age_label(&self) -> String {
        match self.age_days() {
            0 => "本日".to_string(),
            1 => "昨日".to_string(),
            days => format!("{days}日前"),
        }
    }

    /// Whether the inquiry is stale: still 未対応 two or more
    /// days after it arrived.
    pub fn is_stale_open(&self) -> bool {
        self.status == "open" && self.age_days() >= 2
    }

    /// The status badge's tone class: 未対応 reads amber,
    /// 対応済み the teal done tone, 対応中 the plain badge.
    pub fn status_badge_class(&self) -> &'static str {
        match self.status.as_str() {
            "open" => "inq-open",
            "closed" => "inq-closed",
            _ => "",
        }
    }
}

/// One reply the shop sent, stored exactly as sent.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InquiryReply {
    /// Row id.
    pub id: i64,
    /// The inquiry answered.
    pub inquiry_id: i64,
    /// The admin who sent it.
    pub admin_id: i64,
    /// The subject as sent.
    pub subject: String,
    /// The body as sent.
    pub body: String,
    /// When it was sent.
    pub sent_at: DateTime<Utc>,
}

impl InquiryReply {
    /// The sent time in JST, detail-page format.
    pub fn sent_jst(&self) -> String {
        super::jst::jst_datetime_label(self.sent_at)
    }
}
