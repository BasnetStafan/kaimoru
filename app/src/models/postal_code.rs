//! One row of the postal code lookup table, as loaded from
//! the Japan Post data by the import script.

/// One address a postal code resolves to.
///
/// A code covering several towns yields one row per town;
/// an empty `town` means the code resolves at city level.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PostalAddress {
    /// Prefecture name, e.g. 東京都.
    pub prefecture: String,
    /// City, ward, town or village name, e.g. 千代田区.
    pub city: String,
    /// Town name; empty when the code has no town part.
    pub town: String,
}
