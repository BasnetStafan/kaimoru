//! Money constants and price display for the app.
//!
//! The display formatter lives here with the other money rules.
//! It is pure — the caller supplies the currency symbol — and
//! [`display_price`] binds the configured symbol in one place.
//! Money is integer yen everywhere: floats cannot represent
//! prices exactly, and the rounding errors would be visible
//! to customers.

use crate::config::CONFIG;

/// Formats an integer yen amount with the configured yen symbol
/// (for example `¥1,000`).
pub fn display_price(price_yen: i64) -> String {
    format_with_symbol(price_yen, &CONFIG.yen_symbol)
}

/// The formatted total tax over a stored order's breakdown
/// rows: the sum every order-shaped view (receipt, order
/// detail, order lookup, admin order pages) displays.
pub fn display_tax_total(rows: &[crate::models::tax::OrderTaxBreakdown]) -> String {
    display_price(rows.iter().map(|r| r.tax_yen).sum())
}

/// Formats an integer yen amount with an explicit symbol.
///
/// Digits are grouped in thousands. The symbol `円` follows the
/// amount, as Japanese prices write it; any other symbol precedes it.
fn format_with_symbol(price_yen: i64, symbol: &str) -> String {
    let grouped = group_thousands(price_yen);

    if symbol == "円" {
        format!("{grouped}{symbol}")
    } else {
        format!("{symbol}{grouped}")
    }
}

/// Groups an integer into thousands with commas, keeping a leading minus sign.
fn group_thousands(value: i64) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let digits = value.abs().to_string();
    let mut grouped = String::new();

    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }

    format!("{sign}{grouped}")
}

/// Flat shipping fee in yen (tax included): the DEFAULT the
/// shop settings start from — the live value is the operator-
/// editable `shop_settings.shipping_flat_yen`, read through
/// [`crate::models::FeeSettings`]; this constant is only the
/// fallback when no settings row can be read.
pub const SHIPPING_FLAT_YEN: i64 = 550;
/// Product subtotal at or above which shipping is free:
/// the settings default, like [`SHIPPING_FLAT_YEN`].
pub const FREE_SHIPPING_THRESHOLD_YEN: i64 = 5_000;
/// Cash-on-delivery fee in yen (tax included), flat per
/// order: the settings default, like [`SHIPPING_FLAT_YEN`].
pub const COD_FEE_YEN: i64 = 330;

/// The price a product sells at right now: the sale price while
/// the sale window is open and the sale price undercuts the
/// regular price, the regular price otherwise. The one
/// effective-price rule; cards, detail, cart and placement all
/// price through it (in SQL where the query owns the value, the
/// same expression is written out — see `crate::repositories::EFFECTIVE_PRICE_SQL`).
pub fn effective_price_yen(
    price_yen: i64,
    sale_price_yen: Option<i64>,
    sale_starts_at: Option<chrono::DateTime<chrono::Utc>>,
    sale_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> i64 {
    match sale_price_yen {
        Some(sale)
            if sale >= 1
                && sale < price_yen
                && sale_starts_at.is_none_or(|t| t <= now)
                && sale_ends_at.is_none_or(|t| t > now) =>
        {
            sale
        }
        _ => price_yen,
    }
}

#[cfg(test)]
mod tests {
    use super::format_with_symbol;

    #[test]
    fn groups_thousands() {
        assert_eq!(format_with_symbol(2_980, "¥"), "¥2,980");
        assert_eq!(format_with_symbol(1_234_567, "¥"), "¥1,234,567");
        assert_eq!(format_with_symbol(500, "¥"), "¥500");
        assert_eq!(format_with_symbol(0, "¥"), "¥0");
    }

    #[test]
    fn keeps_the_minus_sign() {
        assert_eq!(format_with_symbol(-1_500, "¥"), "¥-1,500");
    }

    #[test]
    fn yen_word_symbol_follows_the_amount() {
        assert_eq!(format_with_symbol(500, "円"), "500円");
    }
}
