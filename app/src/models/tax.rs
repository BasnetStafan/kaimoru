//! Consumption-tax models: classes, dated rates and the
//! per-class breakdown of an order total.
//!
//! Stored prices are tax-INCLUSIVE; a class never changes a
//! price, it only names how much of an inclusive amount is tax.
//! `exempt` （非課税） and `out_of_scope` （不課税） are distinct
//! classes that merely share a 0% rate today.

use crate::config::CONFIG;
use chrono::NaiveDate;

/// One consumption-tax class （標準税率 / 軽減税率 / 非課税 / 不課税）.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct TaxClass {
    /// Surrogate ID.
    pub id: i64,
    /// Stable code: `standard`, `reduced`, `exempt`, `out_of_scope`.
    pub code: String,
    /// Japanese display name.
    pub name: String,
}

/// A tax class with the rate in force on one date, resolved from
/// the dated `tax_rates` rows.
#[derive(Clone, Debug)]
pub struct ClassRate {
    /// The class.
    pub class: TaxClass,
    /// Rate in whole percent (10 means 10%).
    pub rate_percent: i32,
}

/// One row of a tax breakdown: the taxable base and tax amount
/// of one class inside one total. The base is tax-inclusive, so
/// `tax_yen` is a part of it, never an addition to it.
#[derive(Clone, Debug, Default)]
pub struct TaxBreakdown {
    /// Class code snapshot.
    pub tax_class_code: String,
    /// Class name snapshot.
    pub tax_class_name: String,
    /// Rate percent snapshot.
    pub rate_percent: i32,
    /// Tax-inclusive base in yen.
    pub taxable_base_yen: i64,
    /// The tax contained in the base, in yen.
    pub tax_yen: i64,
}

impl TaxBreakdown {
    /// The base formatted for display.
    pub fn formatted_base(&self) -> String {
        crate::pricing::display_price(self.taxable_base_yen)
    }

    /// The tax amount formatted for display.
    pub fn formatted_tax(&self) -> String {
        crate::pricing::display_price(self.tax_yen)
    }
}

/// One dated rate row, for the administration overview.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct TaxRate {
    /// Row ID.
    pub id: i64,
    /// The class the rate belongs to.
    pub tax_class_id: i64,
    /// Rate percent.
    pub rate_percent: i32,
    /// First day the rate applies.
    pub effective_from: NaiveDate,
    /// Last day the rate applies; `None` = open-ended.
    pub effective_to: Option<NaiveDate>,
}

impl TaxRate {
    /// The effective-from date for display.
    pub fn from_label(&self) -> String {
        self.effective_from.format("%Y-%m-%d").to_string()
    }

    /// The effective-to date for display; open-ended as 〜.
    pub fn to_label(&self) -> String {
        self.effective_to
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "〜".to_string())
    }
}

/// A stored per-class breakdown row of one placed order.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct OrderTaxBreakdown {
    /// Class code snapshot.
    pub tax_class_code: String,
    /// Class name snapshot.
    pub tax_class_name: String,
    /// Rate percent snapshot.
    pub rate_percent: i32,
    /// Tax-inclusive base in yen.
    pub taxable_base_yen: i64,
    /// The tax contained in the base, in yen.
    pub tax_yen: i64,
}

impl OrderTaxBreakdown {
    /// The base formatted for display.
    pub fn formatted_base(&self) -> String {
        crate::pricing::display_price(self.taxable_base_yen)
    }

    /// The tax amount formatted for display.
    pub fn formatted_tax(&self) -> String {
        crate::pricing::display_price(self.tax_yen)
    }
}

/// Display order of the classes in a breakdown: standard first,
/// then reduced, then the two zero classes.
pub fn class_rank(code: &str) -> usize {
    match code {
        "standard" => 0,
        "reduced" => 1,
        "exempt" => 2,
        "out_of_scope" => 3,
        _ => 4,
    }
}

/// How a tax fraction is rounded: the shop-level setting
/// (`tax_rounding` in the typed config), applied once per tax
/// class to the class subtotal — the invoice-system rule —
/// never per line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaxRounding {
    /// 切り捨て: truncate toward zero.
    Floor,
    /// 四捨五入: round half up.
    Round,
    /// 切り上げ: round up.
    Ceil,
}

impl TaxRounding {
    /// Parses the configured value; anything unknown is the
    /// floor, the Japanese invoice convention and the default.
    pub fn from_config(value: &str) -> Self {
        match value {
            "round" => Self::Round,
            "ceil" => Self::Ceil,
            _ => Self::Floor,
        }
    }

    /// The configured rounding mode of this shop.
    pub fn configured() -> Self {
        Self::from_config(&CONFIG.tax_rounding)
    }

    /// Divides with this rounding: `numer / denom` for
    /// non-negative amounts (money here is never negative at
    /// this point).
    pub fn div_round(self, numer: i64, denom: i64) -> i64 {
        if denom <= 0 || numer <= 0 {
            return 0;
        }
        match self {
            Self::Floor => numer / denom,
            Self::Ceil => (numer + denom - 1) / denom,
            Self::Round => (numer + denom / 2) / denom,
        }
    }
}

/// The tax contained in a tax-inclusive base at `rate_percent`:
/// `base × rate / (100 + rate)` — floor-of-tax: the tax fraction
/// itself is rounded, once per class, with the shop rounding
/// (before that, the net part
/// was rounded and subtracted instead). Zero-rate classes
/// contain no tax.
pub fn tax_from_inclusive(base_yen: i64, rate_percent: i32, mode: TaxRounding) -> i64 {
    if base_yen <= 0 || rate_percent <= 0 {
        return 0;
    }
    let rate = i64::from(rate_percent);
    mode.div_round(base_yen * rate, 100 + rate)
}

/// Converts a tax-exclusive amount to the stored tax-inclusive
/// one at `rate_percent` (the admin price-entry helper).
pub fn inclusive_from_exclusive(exclusive_yen: i64, rate_percent: i32, mode: TaxRounding) -> i64 {
    if rate_percent <= 0 {
        return exclusive_yen;
    }
    let rate = i64::from(rate_percent);
    mode.div_round(exclusive_yen * (100 + rate), 100)
}

/// Converts a tax-inclusive amount back to its tax-exclusive
/// part at `rate_percent` (the figure the admin form shows
/// beside the stored inclusive price).
pub fn exclusive_from_inclusive(inclusive_yen: i64, rate_percent: i32, mode: TaxRounding) -> i64 {
    if rate_percent <= 0 {
        return inclusive_yen;
    }
    let rate = i64::from(rate_percent);
    mode.div_round(inclusive_yen * 100, 100 + rate)
}

#[cfg(test)]
mod tests {
    use super::{TaxRounding, tax_from_inclusive};

    #[test]
    fn floor_of_tax_reference_values() {
        // Reference values for floor-of-tax:
        // ¥500 at 8% shows ¥37 of included tax (the old
        // net-of-rounded-net rule showed ¥38); ¥5,100 at 10%
        // shows ¥463 (was ¥464).
        assert_eq!(tax_from_inclusive(500, 8, TaxRounding::Floor), 37);
        assert_eq!(tax_from_inclusive(5_100, 10, TaxRounding::Floor), 463);
    }

    #[test]
    fn floor_of_tax_exact_division_and_zero_cases() {
        // 1,080 × 8 / 108 divides exactly: ¥80 under both rules.
        assert_eq!(tax_from_inclusive(1_080, 8, TaxRounding::Floor), 80);
        assert_eq!(tax_from_inclusive(0, 10, TaxRounding::Floor), 0);
        assert_eq!(tax_from_inclusive(500, 0, TaxRounding::Floor), 0);
    }

    #[test]
    fn shop_rounding_applies_to_the_tax_fraction() {
        // 500 × 8 / 108 = 37.03...: floor and round-half-up give
        // 37, ceil gives 38 — the mode rounds the tax itself now,
        // not the net part.
        assert_eq!(tax_from_inclusive(500, 8, TaxRounding::Round), 37);
        assert_eq!(tax_from_inclusive(500, 8, TaxRounding::Ceil), 38);
    }
}
