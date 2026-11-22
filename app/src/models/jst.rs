//! Japan Standard Time date labels: the app-wide
//! helpers that render stored UTC timestamps as JST
//! dates for display.

use chrono::{DateTime, NaiveDate, Utc};

/// Returns today's calendar date in Japan: the base every
/// delivery window is computed from.
pub fn jst_today() -> NaiveDate {
    chrono::Utc::now().with_timezone(&jst()).date_naive()
}

/// Formats an instant as a JST date-and-time label
/// (`2026/11/04 11:30`): the timestamp shape the admin lists
/// and detail pages share.
pub fn jst_datetime_label(at: DateTime<Utc>) -> String {
    at.with_timezone(&jst())
        .format("%Y/%m/%d %H:%M")
        .to_string()
}

/// Formats an instant as a JST date label (`2026/11/04`).
pub fn jst_date_label(at: DateTime<Utc>) -> String {
    at.with_timezone(&jst()).format("%Y/%m/%d").to_string()
}

/// Formats an instant as a JST date-and-time label in the long
/// Japanese form (`2026年11月4日 11:30`) the order and payment
/// detail pages show.
pub fn jst_long_datetime_label(at: DateTime<Utc>) -> String {
    at.with_timezone(&jst())
        .format("%Y年%-m月%-d日 %H:%M")
        .to_string()
}

/// The store's display timezone: Japan Standard Time.
///
/// Applied explicitly instead of `chrono::Local`: the process
/// timezone is UTC on ECS, so Local would show every order time
/// 9 hours early there.
pub(crate) fn jst() -> chrono::FixedOffset {
    chrono::FixedOffset::east_opt(9 * 3600).expect("9 hours is a valid offset")
}
