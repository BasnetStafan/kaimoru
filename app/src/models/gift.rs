//! Gift option types and rules: the noshi catalogue, the
//! length caps of the two free-text fields, validation of a
//! posted gift choice, and the display/mail formatters every
//! surface shares. Gift options are free in V1 — no fee is
//! computed here, because paid wrapping is a business
//! decision, not a default.

/// One noshi kind of the catalogue: a stable code (what the
/// order stores) and its Japanese label (snapshotted onto
/// the order at placement, like the delivery slot's).
pub struct NoshiKind {
    /// Stable code, stored on the order.
    pub code: &'static str,
    /// Japanese label shown to shoppers.
    pub label: &'static str,
}

/// The noshi catalogue, in display order, "none" first. A
/// model constant in the delivery-slot style: a fixed
/// presentation list of Japanese gift custom, not operator
/// data.
pub const NOSHI_KINDS: &[NoshiKind] = &[
    NoshiKind {
        code: "none",
        label: "なし",
    },
    NoshiKind {
        code: "celebration",
        label: "御祝",
    },
    NoshiKind {
        code: "midyear",
        label: "御中元",
    },
    NoshiKind {
        code: "year_end",
        label: "御歳暮",
    },
    NoshiKind {
        code: "thanks",
        label: "御礼",
    },
    NoshiKind {
        code: "plain",
        label: "無地のし",
    },
];

/// Character cap of the name inscription （名入れ）: a name
/// printed on the noshi paper, not a letter.
pub const NOSHI_NAME_MAX_CHARS: usize = 20;

/// Character cap of the message-card text.
pub const GIFT_MESSAGE_MAX_CHARS: usize = 100;

/// The catalogue entry for a code, or `None` for a code the
/// catalogue does not know (a tampered form value).
pub fn noshi_kind(code: &str) -> Option<&'static NoshiKind> {
    NOSHI_KINDS.iter().find(|k| k.code == code)
}

/// A validated gift choice, ready to store.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidGift {
    /// The noshi code (`none` when no noshi).
    pub noshi_code: &'static str,
    /// The noshi label, snapshotted at placement.
    pub noshi_label: &'static str,
    /// The name inscription, when given.
    pub noshi_name: Option<String>,
    /// The message-card text, when given.
    pub gift_message: Option<String>,
}

impl ValidGift {
    /// Whether the shopper chose anything at all (a noshi,
    /// or a message card).
    pub fn any(&self) -> bool {
        self.noshi_code != "none" || self.gift_message.is_some()
    }
}

/// Why a posted gift choice failed validation; each variant
/// names the form field its message belongs under.
#[derive(Debug, PartialEq, Eq)]
pub enum GiftError {
    /// The noshi code is outside the catalogue.
    UnknownNoshi,
    /// A name inscription without a noshi to print it on.
    NameWithoutNoshi,
    /// The name inscription is over its cap.
    NameTooLong,
    /// The message-card text is over its cap.
    MessageTooLong,
}

impl GiftError {
    /// The checkout form field this error belongs under.
    pub fn field(&self) -> &'static str {
        match self {
            Self::UnknownNoshi => "noshi",
            Self::NameWithoutNoshi | Self::NameTooLong => "noshi_name",
            Self::MessageTooLong => "gift_message",
        }
    }

    /// The shopper-facing message for this failure.
    pub fn message(&self) -> String {
        match self {
            Self::UnknownNoshi => "のしの選択が正しくありません。".to_string(),
            Self::NameWithoutNoshi => {
                "名入れは、のしをお選びの場合のみご利用いただけます。".to_string()
            }
            Self::NameTooLong => {
                format!("名入れは{NOSHI_NAME_MAX_CHARS}文字以内で入力してください。")
            }
            Self::MessageTooLong => {
                format!("メッセージは{GIFT_MESSAGE_MAX_CHARS}文字以内で入力してください。")
            }
        }
    }
}

/// Validates a posted gift choice: the noshi code must be
/// in the catalogue (an empty value deserializes as `none`,
/// for sessions saved before the feature), the inscription
/// needs a noshi, and both texts are capped — a cap is an
/// error, never a silent truncation. Tampered values are
/// rejected, never clamped.
///
/// # Errors
///
/// Returns the first failing rule's [`GiftError`]; the
/// checkout form collects them per field via
/// `validate_gift_options_all` in the checkout validation.
pub fn validate_gift_options(
    noshi_raw: &str,
    name_raw: &str,
    message_raw: &str,
) -> Result<ValidGift, GiftError> {
    let code = if noshi_raw.trim().is_empty() {
        "none"
    } else {
        noshi_raw.trim()
    };
    let Some(kind) = noshi_kind(code) else {
        return Err(GiftError::UnknownNoshi);
    };
    let name = name_raw.trim();
    let noshi_name = if name.is_empty() {
        None
    } else {
        if kind.code == "none" {
            return Err(GiftError::NameWithoutNoshi);
        }
        if name.chars().count() > NOSHI_NAME_MAX_CHARS {
            return Err(GiftError::NameTooLong);
        }
        Some(name.to_string())
    };
    let message = message_raw.trim();
    let gift_message = if message.is_empty() {
        None
    } else {
        if message.chars().count() > GIFT_MESSAGE_MAX_CHARS {
            return Err(GiftError::MessageTooLong);
        }
        Some(message.to_string())
    };
    Ok(ValidGift {
        noshi_code: kind.code,
        noshi_label: kind.label,
        noshi_name,
        gift_message,
    })
}

/// Collects every gift validation error at once, for the
/// checkout form's per-field display: the code error is
/// fatal to the rest (the pairing rule needs a known
/// noshi), the two text caps are independent.
pub fn gift_option_errors(noshi_raw: &str, name_raw: &str, message_raw: &str) -> Vec<GiftError> {
    match validate_gift_options(noshi_raw, name_raw, message_raw) {
        Ok(_) => Vec::new(),
        Err(first) => {
            let mut errors = vec![first];
            // The message cap is independent of the noshi and
            // name rules: report it alongside, never instead.
            if message_raw.trim().chars().count() > GIFT_MESSAGE_MAX_CHARS
                && !errors.contains(&GiftError::MessageTooLong)
            {
                errors.push(GiftError::MessageTooLong);
            }
            errors
        }
    }
}

/// The display lines of a stored gift choice, shared by the
/// confirm step, the complete page and both order details:
/// empty when the shopper chose nothing.
pub fn gift_display_lines(
    noshi_code: &str,
    noshi_label: &str,
    noshi_name: Option<&str>,
    gift_message: Option<&str>,
) -> Vec<String> {
    let mut lines = Vec::new();
    if noshi_code != "none" {
        let mut line = format!("のし：{noshi_label}");
        if let Some(name) = noshi_name {
            line.push_str(&format!("（名入れ：{name}）"));
        }
        lines.push(line);
    }
    if let Some(message) = gift_message {
        lines.push(format!("メッセージカード：{message}"));
    }
    lines
}

/// The mail form of a stored gift choice: the display lines
/// under a ギフト heading, each line newline-terminated, or
/// empty when the shopper chose nothing (the mail then
/// shows no gift block at all).
pub fn gift_mail_lines(
    noshi_code: &str,
    noshi_label: &str,
    noshi_name: Option<&str>,
    gift_message: Option<&str>,
) -> String {
    let lines = gift_display_lines(noshi_code, noshi_label, noshi_name, gift_message);
    if lines.is_empty() {
        return String::new();
    }
    let mut out = String::from("ギフト：\n");
    for line in lines {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gift_validation_accepts_each_shape() {
        let none = validate_gift_options("", "", "").unwrap();
        assert_eq!(none.noshi_code, "none");
        assert!(!none.any());
        let full =
            validate_gift_options("celebration", "山田 太郎", "お誕生日おめでとうございます")
                .unwrap();
        assert_eq!(full.noshi_label, "御祝");
        assert_eq!(full.noshi_name.as_deref(), Some("山田 太郎"));
        assert!(full.any());
        // A message card alone is a gift choice too.
        let card_only = validate_gift_options("none", "", "ありがとう").unwrap();
        assert!(card_only.any());
    }

    #[test]
    fn gift_validation_rejects_tampering_pairing_and_overlong() {
        assert_eq!(
            validate_gift_options("bogus", "", ""),
            Err(GiftError::UnknownNoshi)
        );
        assert_eq!(
            validate_gift_options("none", "山田", ""),
            Err(GiftError::NameWithoutNoshi)
        );
        let long_name = "あ".repeat(NOSHI_NAME_MAX_CHARS + 1);
        assert_eq!(
            validate_gift_options("thanks", &long_name, ""),
            Err(GiftError::NameTooLong)
        );
        let long_message = "あ".repeat(GIFT_MESSAGE_MAX_CHARS + 1);
        assert_eq!(
            validate_gift_options("plain", "", &long_message),
            Err(GiftError::MessageTooLong)
        );
    }

    #[test]
    fn gift_lines_render_only_what_was_chosen() {
        assert!(gift_display_lines("none", "なし", None, None).is_empty());
        assert_eq!(
            gift_display_lines("midyear", "御中元", Some("山田"), None),
            vec!["のし：御中元（名入れ：山田）".to_string()]
        );
        assert_eq!(
            gift_mail_lines("none", "なし", None, Some("ありがとう")),
            "ギフト：\nメッセージカード：ありがとう\n"
        );
        assert_eq!(gift_mail_lines("none", "なし", None, None), "");
    }
}
