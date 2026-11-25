//! Delivery domain types and rules: the slot catalogue,
//! delivery-window and date validation, shipping-details
//! validation, and the mail/display formatters for a
//! delivery request.

use crate::messages::CARRIER_SELECTION_INVALID_MESSAGE;
use crate::models::carrier::carrier_for;
use chrono::{Datelike, NaiveDate};

/// Days after the JST order date of the earliest selectable
/// delivery date: a packing day and a carrier-pickup day stand
/// between ordering and any date the shop can honestly accept.
pub const DELIVERY_EARLIEST_OFFSET_DAYS: i64 = 2;

/// Days after the JST order date of the latest selectable
/// delivery date: requests further out than a fortnight are
/// guesses nobody can plan around.
pub const DELIVERY_LATEST_OFFSET_DAYS: i64 = 14;

/// One delivery time slot of the catalogue: a stable code (what
/// the order stores) and its Japanese label (what the shopper
/// sees, snapshotted onto the order at placement).
pub struct DeliverySlot {
    /// Stable slot code, stored on the order.
    pub code: &'static str,
    /// Japanese label shown to shoppers.
    pub label: &'static str,
}

/// The delivery time slots, in display order: the
/// carrier-standard bands, with no preference first. A model
/// constant, not operator data — the list is a presentation
/// constant of Japanese logistics.
pub const DELIVERY_SLOTS: &[DeliverySlot] = &[
    DeliverySlot {
        code: "none",
        label: "指定なし",
    },
    DeliverySlot {
        code: "am",
        label: "午前中",
    },
    DeliverySlot {
        code: "t1416",
        label: "14時〜16時",
    },
    DeliverySlot {
        code: "t1618",
        label: "16時〜18時",
    },
    DeliverySlot {
        code: "t1820",
        label: "18時〜20時",
    },
    DeliverySlot {
        code: "t1921",
        label: "19時〜21時",
    },
];

/// Returns the catalogue label for a slot code, or `None` for a
/// code the catalogue does not know (a tampered form value).
pub fn delivery_slot_label(code: &str) -> Option<&'static str> {
    DELIVERY_SLOTS
        .iter()
        .find(|s| s.code == code)
        .map(|s| s.label)
}

/// Why the shipping details posted with a ship transition
/// failed validation.
#[derive(Debug, PartialEq, Eq)]
pub enum ShippingDetailsError {
    /// The carrier code is outside the catalogue.
    UnknownCarrier,
    /// The tracking number is over length or carries
    /// characters a carrier number never has.
    BadTrackingNumber,
}

impl ShippingDetailsError {
    /// The operator-facing message for this failure.
    pub fn message(&self) -> &'static str {
        match self {
            Self::UnknownCarrier => CARRIER_SELECTION_INVALID_MESSAGE,
            Self::BadTrackingNumber => {
                "伝票番号は32文字以内の半角英数字とハイフンで入力してください。"
            }
        }
    }
}

/// Validates the shipping details posted with a ship
/// transition: an empty carrier means the operator named none;
/// a present one must be in the catalogue. The tracking number
/// is trimmed; an empty one means none was given. Carrier
/// formats differ, so the number's check is deliberately light
/// — length and character class only, never a per-carrier
/// pattern that would reject a real number.
///
/// # Errors
///
/// Returns [`ShippingDetailsError`] for an unknown carrier
/// code or a malformed tracking number.
pub fn validate_shipping_details(
    carrier_raw: &str,
    tracking_raw: &str,
) -> Result<(Option<&'static str>, Option<String>), ShippingDetailsError> {
    let carrier = match carrier_raw.trim() {
        "" => None,
        code => Some(
            carrier_for(code)
                .ok_or(ShippingDetailsError::UnknownCarrier)?
                .code,
        ),
    };
    let tracking = match tracking_raw.trim() {
        "" => None,
        number => {
            let valid = number.len() <= 32
                && number
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-');
            if !valid {
                return Err(ShippingDetailsError::BadTrackingNumber);
            }
            Some(number.to_string())
        }
    };
    Ok((carrier, tracking))
}

/// Returns the selectable delivery window for a JST "today":
/// the earliest and latest date, both inclusive calendar days.
pub fn delivery_window(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    delivery_window_with(
        today,
        DELIVERY_EARLIEST_OFFSET_DAYS,
        DELIVERY_LATEST_OFFSET_DAYS,
    )
}

/// Returns the selectable delivery window under operator-set
/// offsets AND the shop's registered holidays: the latest
/// date stays a plain calendar offset, while the earliest
/// counts only non-holiday days — the shop needs its working
/// days between the order and the promise. With no holidays
/// registered the result equals
/// [`delivery_window_with`] exactly.
pub fn delivery_window_with_holidays(
    today: NaiveDate,
    earliest_offset_days: i64,
    latest_offset_days: i64,
    holidays: &[NaiveDate],
) -> (NaiveDate, NaiveDate) {
    let latest = today + chrono::Duration::days(latest_offset_days);
    let mut day = today;
    let mut counted = 0i64;
    while counted < earliest_offset_days {
        day += chrono::Duration::days(1);
        if !holidays.contains(&day) {
            counted += 1;
        }
    }
    (day, latest)
}

/// Returns the selectable delivery dates under operator-set
/// offsets and holidays: the window's days with registered
/// holidays removed — a closed day is never offered.
pub fn delivery_dates_with_holidays(
    today: NaiveDate,
    earliest_offset_days: i64,
    latest_offset_days: i64,
    holidays: &[NaiveDate],
) -> Vec<NaiveDate> {
    let (earliest, latest) =
        delivery_window_with_holidays(today, earliest_offset_days, latest_offset_days, holidays);
    let mut dates = Vec::new();
    let mut day = earliest;
    while day <= latest {
        if !holidays.contains(&day) {
            dates.push(day);
        }
        day += chrono::Duration::days(1);
    }
    dates
}

/// Returns the earliest selectable date's label under
/// operator-set offsets and holidays.
pub fn earliest_delivery_label_with_holidays(
    today: NaiveDate,
    earliest_offset_days: i64,
    latest_offset_days: i64,
    holidays: &[NaiveDate],
) -> String {
    delivery_date_label(
        delivery_window_with_holidays(today, earliest_offset_days, latest_offset_days, holidays).0,
    )
}

/// Returns the window's range label under operator-set
/// offsets and holidays.
pub fn delivery_window_range_label_with_holidays(
    today: NaiveDate,
    earliest_offset_days: i64,
    latest_offset_days: i64,
    holidays: &[NaiveDate],
) -> String {
    let (earliest, latest) =
        delivery_window_with_holidays(today, earliest_offset_days, latest_offset_days, holidays);
    format!(
        "{}〜{}",
        delivery_date_label(earliest),
        delivery_date_label(latest)
    )
}

/// Returns the selectable delivery window for a JST "today"
/// under operator-set offsets (the shop settings' delivery
/// section): both dates inclusive calendar days.
pub fn delivery_window_with(
    today: NaiveDate,
    earliest_offset_days: i64,
    latest_offset_days: i64,
) -> (NaiveDate, NaiveDate) {
    (
        today + chrono::Duration::days(earliest_offset_days),
        today + chrono::Duration::days(latest_offset_days),
    )
}

/// Returns the selectable delivery dates for a JST "today", in
/// order: every calendar day of the window, weekends and
/// holidays included (no business-day calendar in V1).
pub fn delivery_dates(today: NaiveDate) -> Vec<NaiveDate> {
    delivery_dates_with(
        today,
        DELIVERY_EARLIEST_OFFSET_DAYS,
        DELIVERY_LATEST_OFFSET_DAYS,
    )
}

/// Returns the selectable delivery dates under operator-set
/// offsets: every calendar day of the window, in order.
pub fn delivery_dates_with(
    today: NaiveDate,
    earliest_offset_days: i64,
    latest_offset_days: i64,
) -> Vec<NaiveDate> {
    let (earliest, latest) = delivery_window_with(today, earliest_offset_days, latest_offset_days);
    let mut dates = Vec::new();
    let mut day = earliest;
    while day <= latest {
        dates.push(day);
        day += chrono::Duration::days(1);
    }
    dates
}

/// Returns the earliest selectable delivery date's picker
/// label for a JST "today", for example `11月5日（木）`: the
/// date behind the storefront's delivery estimate, taken from
/// the same window the checkout picker offers, so a displayed
/// promise can never disagree with what checkout accepts.
pub fn earliest_delivery_label(today: NaiveDate) -> String {
    earliest_delivery_label_with(
        today,
        DELIVERY_EARLIEST_OFFSET_DAYS,
        DELIVERY_LATEST_OFFSET_DAYS,
    )
}

/// Returns the earliest selectable date's label under
/// operator-set offsets.
pub fn earliest_delivery_label_with(
    today: NaiveDate,
    earliest_offset_days: i64,
    latest_offset_days: i64,
) -> String {
    delivery_date_label(delivery_window_with(today, earliest_offset_days, latest_offset_days).0)
}

/// Returns the selectable delivery window's picker labels as
/// a range, for example `11月5日（木）〜11月17日（火）`.
pub fn delivery_window_range_label(today: NaiveDate) -> String {
    delivery_window_range_label_with(
        today,
        DELIVERY_EARLIEST_OFFSET_DAYS,
        DELIVERY_LATEST_OFFSET_DAYS,
    )
}

/// Returns the window's range label under operator-set
/// offsets.
pub fn delivery_window_range_label_with(
    today: NaiveDate,
    earliest_offset_days: i64,
    latest_offset_days: i64,
) -> String {
    let (earliest, latest) = delivery_window_with(today, earliest_offset_days, latest_offset_days);
    format!(
        "{}〜{}",
        delivery_date_label(earliest),
        delivery_date_label(latest)
    )
}

/// Returns a date as the picker shows it, for example
/// `11月7日（金）`: month, day and weekday, no year.
pub fn delivery_date_label(date: NaiveDate) -> String {
    format!(
        "{}月{}日（{}）",
        date.month(),
        date.day(),
        weekday_kanji(date.weekday())
    )
}

/// Returns a date in the long form the confirm step and the
/// order pages show, for example `2026年11月7日（金）`.
pub fn format_delivery_date_long(date: NaiveDate) -> String {
    format!("{}年{}", date.year(), delivery_date_label(date))
}

/// Returns the weekday's kanji for the picker labels.
fn weekday_kanji(weekday: chrono::Weekday) -> &'static str {
    use chrono::Weekday::{Fri, Mon, Sat, Sun, Thu, Tue, Wed};
    match weekday {
        Mon => "月",
        Tue => "火",
        Wed => "水",
        Thu => "木",
        Fri => "金",
        Sat => "土",
        Sun => "日",
    }
}

/// Returns the one rendering of a delivery request that every
/// surface shares — confirm step, complete page, both order
/// details — built from the date and the slot label (the stored
/// snapshot for placed orders, the catalogue label before
/// placement).
pub fn format_delivery_request(date: Option<NaiveDate>, slot_label: &str) -> String {
    match date {
        None => "指定なし（最短でお届け）".to_string(),
        Some(day) if slot_label == "指定なし" => {
            format!("{}・時間帯指定なし", format_delivery_date_long(day))
        }
        Some(day) => format!("{}{}", format_delivery_date_long(day), slot_label),
    }
}

/// Returns the confirmation mail's delivery lines for a
/// request: the two labelled lines when a date was requested,
/// the single no-preference line otherwise.
pub fn delivery_mail_lines(date: Option<NaiveDate>, slot_label: &str) -> String {
    match date {
        None => "お届け希望日時：指定なし（最短でお届け）\n".to_string(),
        Some(day) => format!(
            "お届け希望日：{}\nお届け時間帯：{}\n",
            format_delivery_date_long(day),
            slot_label
        ),
    }
}

/// Why a delivery request failed validation.
#[derive(Debug, PartialEq, Eq)]
pub enum DeliveryRequestError {
    /// A time slot was chosen without a date.
    SlotNeedsDate,
    /// The date fell before the placement-time window's
    /// earliest day (the midnight boundary, or a stale
    /// carried value).
    WindowChanged,
    /// Anything else the catalogue and window do not admit.
    Invalid,
}

impl DeliveryRequestError {
    /// The shopper-facing message for this failure.
    pub fn message(&self) -> &'static str {
        match self {
            Self::SlotNeedsDate => "時間帯を選択する場合は、お届け希望日も選択してください。",
            Self::WindowChanged => {
                "お届け希望日の受付期間が変わりました。もう一度選択してください。"
            }
            Self::Invalid => "お届け希望日時の選択が正しくありません。",
        }
    }
}

/// Validates a posted delivery request against the slot
/// catalogue and the window computed from `today`: returns the
/// parsed date (`None` for no preference), the slot code and
/// its catalogue label. Tampered values are rejected, never
/// clamped to the nearest valid value.
///
/// # Errors
///
/// Returns the matching [`DeliveryRequestError`] for a slot
/// without a date, a date before the window, a date past the
/// window, an unparseable date or an unknown slot code.
pub fn validate_delivery_request(
    date_raw: &str,
    slot_raw: &str,
    today: NaiveDate,
) -> Result<(Option<NaiveDate>, &'static str, &'static str), DeliveryRequestError> {
    validate_delivery_request_with(
        date_raw,
        slot_raw,
        today,
        DELIVERY_EARLIEST_OFFSET_DAYS,
        DELIVERY_LATEST_OFFSET_DAYS,
        "",
    )
}

/// Validates a delivery request under the shop's settings:
/// the window comes from the operator offsets, and a slot the
/// shop disabled is refused like an unknown one (a stale or
/// hand-built form must not resurrect an unoffered band).
/// `disabled_slots_csv` is the settings' comma-joined codes.
pub fn validate_delivery_request_with(
    date_raw: &str,
    slot_raw: &str,
    today: NaiveDate,
    earliest_offset_days: i64,
    latest_offset_days: i64,
    disabled_slots_csv: &str,
) -> Result<(Option<NaiveDate>, &'static str, &'static str), DeliveryRequestError> {
    validate_delivery_request_full(
        date_raw,
        slot_raw,
        today,
        earliest_offset_days,
        latest_offset_days,
        disabled_slots_csv,
        &[],
    )
}

/// Validates a delivery request under the shop's settings
/// and its registered holidays: a holiday is refused like a
/// date outside the window — it was never offered.
#[allow(clippy::too_many_arguments)]
pub fn validate_delivery_request_with_holidays(
    date_raw: &str,
    slot_raw: &str,
    today: NaiveDate,
    earliest_offset_days: i64,
    latest_offset_days: i64,
    disabled_slots_csv: &str,
    holidays: &[NaiveDate],
) -> Result<(Option<NaiveDate>, &'static str, &'static str), DeliveryRequestError> {
    validate_delivery_request_full(
        date_raw,
        slot_raw,
        today,
        earliest_offset_days,
        latest_offset_days,
        disabled_slots_csv,
        holidays,
    )
}

/// The one validation body behind the window variants.
#[allow(clippy::too_many_arguments)]
fn validate_delivery_request_full(
    date_raw: &str,
    slot_raw: &str,
    today: NaiveDate,
    earliest_offset_days: i64,
    latest_offset_days: i64,
    disabled_slots_csv: &str,
    holidays: &[NaiveDate],
) -> Result<(Option<NaiveDate>, &'static str, &'static str), DeliveryRequestError> {
    // An empty slot is how pre-feature sessions deserialize;
    // it means no preference, exactly like the `none` code.
    let slot_code = if slot_raw.trim().is_empty() {
        "none"
    } else {
        slot_raw.trim()
    };
    // The returned code and label are the catalogue's own
    // 'static strings, never slices of the posted input.
    let Some(slot) = DELIVERY_SLOTS.iter().find(|s| s.code == slot_code) else {
        return Err(DeliveryRequestError::Invalid);
    };
    if slot_code != "none" && disabled_slots_csv.split(',').any(|c| c.trim() == slot_code) {
        return Err(DeliveryRequestError::Invalid);
    }
    let slot_code = slot.code;
    let slot_label = slot.label;
    let date_text = date_raw.trim();
    let date = if date_text.is_empty() {
        None
    } else {
        Some(
            NaiveDate::parse_from_str(date_text, "%Y-%m-%d")
                .map_err(|_| DeliveryRequestError::Invalid)?,
        )
    };
    // A time band only has meaning on a day.
    if slot_code != "none" && date.is_none() {
        return Err(DeliveryRequestError::SlotNeedsDate);
    }
    if let Some(day) = date {
        let (earliest, latest) = delivery_window_with_holidays(
            today,
            earliest_offset_days,
            latest_offset_days,
            holidays,
        );
        if day < earliest {
            return Err(DeliveryRequestError::WindowChanged);
        }
        if day > latest || holidays.contains(&day) {
            return Err(DeliveryRequestError::Invalid);
        }
    }
    Ok((date, slot_code, slot_label))
}

#[cfg(test)]
mod tests {

    #[test]
    fn delivery_window_spans_plus_two_to_plus_fourteen() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let (earliest, latest) = super::delivery_window(today);
        assert_eq!(
            earliest,
            chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap()
        );
        assert_eq!(
            latest,
            chrono::NaiveDate::from_ymd_opt(2026, 11, 17).unwrap()
        );
        assert_eq!(super::delivery_dates(today).len(), 13);
    }

    #[test]
    fn holidays_push_the_earliest_date_and_leave_the_latest() {
        // 2026-11-03 + 2 working days, with 11-04 a holiday:
        // the count skips it, so the earliest lands on 11-06;
        // the latest stays the plain +14 calendar date.
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        let holidays = [chrono::NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()];
        let (earliest, latest) = super::delivery_window_with_holidays(today, 2, 14, &holidays);
        assert_eq!(
            earliest,
            chrono::NaiveDate::from_ymd_opt(2026, 11, 6).unwrap()
        );
        assert_eq!(
            latest,
            chrono::NaiveDate::from_ymd_opt(2026, 11, 17).unwrap()
        );
        // The holiday itself is not among the offered dates,
        // and an empty holiday set is the plain window.
        let dates = super::delivery_dates_with_holidays(today, 2, 14, &holidays);
        assert!(!dates.contains(&holidays[0]));
        assert_eq!(
            super::delivery_window_with_holidays(today, 2, 14, &[]),
            super::delivery_window(today)
        );
        // A holiday date is refused by validation.
        assert!(
            super::validate_delivery_request_with_holidays(
                "2026-11-04",
                "none",
                today,
                2,
                14,
                "",
                &holidays,
            )
            .is_err()
        );
    }

    #[test]
    fn delivery_estimate_labels_come_from_the_window() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        assert_eq!(super::earliest_delivery_label(today), "11月5日（木）");
        assert_eq!(
            super::delivery_window_range_label(today),
            "11月5日（木）〜11月17日（火）"
        );
    }

    #[test]
    fn delivery_request_formats_each_shape() {
        // 2026-11-07 is a Saturday.
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
        assert_eq!(
            super::format_delivery_request(None, "指定なし"),
            "指定なし（最短でお届け）"
        );
        assert_eq!(
            super::format_delivery_request(Some(day), "指定なし"),
            "2026年11月7日（土）・時間帯指定なし"
        );
        assert_eq!(
            super::format_delivery_request(Some(day), "14時〜16時"),
            "2026年11月7日（土）14時〜16時"
        );
    }

    #[test]
    fn delivery_validation_rejects_tampering_and_pairing_breaks() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
        assert_eq!(
            super::validate_delivery_request("", "am", today),
            Err(super::DeliveryRequestError::SlotNeedsDate)
        );
        assert_eq!(
            super::validate_delivery_request("2026-11-04", "none", today),
            Err(super::DeliveryRequestError::WindowChanged)
        );
        assert_eq!(
            super::validate_delivery_request("2026-12-01", "none", today),
            Err(super::DeliveryRequestError::Invalid)
        );
        assert_eq!(
            super::validate_delivery_request("not-a-date", "none", today),
            Err(super::DeliveryRequestError::Invalid)
        );
        assert_eq!(
            super::validate_delivery_request("2026-11-07", "bogus", today),
            Err(super::DeliveryRequestError::Invalid)
        );
        let ok = super::validate_delivery_request("2026-11-07", "t1416", today).unwrap();
        assert_eq!(ok.1, "t1416");
        assert_eq!(ok.2, "14時〜16時");
    }

    #[test]
    fn shipping_details_accept_empty_and_valid_pairs() {
        assert_eq!(super::validate_shipping_details("", ""), Ok((None, None)));
        assert_eq!(
            super::validate_shipping_details("yamato", " 1234-5678-9012 "),
            Ok((Some("yamato"), Some("1234-5678-9012".to_string())))
        );
        // A number with no carrier, and a carrier with no
        // number, are both legitimate ship-time states.
        assert_eq!(
            super::validate_shipping_details("", "123456789012"),
            Ok((None, Some("123456789012".to_string())))
        );
        assert_eq!(
            super::validate_shipping_details("other", ""),
            Ok((Some("other"), None))
        );
    }

    #[test]
    fn shipping_details_reject_unknown_carrier_and_bad_numbers() {
        assert_eq!(
            super::validate_shipping_details("dhl", ""),
            Err(super::ShippingDetailsError::UnknownCarrier)
        );
        assert_eq!(
            super::validate_shipping_details("yamato", "1234 5678"),
            Err(super::ShippingDetailsError::BadTrackingNumber)
        );
        assert_eq!(
            super::validate_shipping_details("yamato", "あ1234"),
            Err(super::ShippingDetailsError::BadTrackingNumber)
        );
        let long = "1".repeat(33);
        assert_eq!(
            super::validate_shipping_details("yamato", &long),
            Err(super::ShippingDetailsError::BadTrackingNumber)
        );
    }
}
