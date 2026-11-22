//! A member's saved delivery address (the address book).
//!
//! The field set is the checkout delivery form's own, minus the
//! order-contact email: an address is a place and a recipient,
//! never a contact. Values are stored in the normalised forms
//! the checkout validation produces (digit-only postal code and
//! phone). Orders never reference this table — placement copies
//! the chosen address into the order's own snapshot columns, so
//! editing or deleting a saved address cannot rewrite history.

use chrono::{DateTime, Utc};

/// The label's length cap in characters: enough
/// for 自宅 / 実家 / 勤務先 / a relative's name, short enough to
/// stay a picker label.
pub const LABEL_MAX_CHARS: usize = 20;

/// One saved delivery address.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MemberAddress {
    /// Row id.
    pub id: i64,
    /// Owning member.
    pub user_id: i64,
    /// The member's own name for the address (may be empty).
    pub label: String,
    /// Recipient family name.
    pub family_name: String,
    /// Recipient given name.
    pub given_name: String,
    /// Postal code, seven digits.
    pub postal_code: String,
    /// Prefecture (one of the 47 the delivery form offers).
    pub prefecture: String,
    /// City / ward.
    pub city: String,
    /// Street address.
    pub address_line: String,
    /// Building / room, when given.
    pub building: Option<String>,
    /// Phone number, digits only.
    pub phone: String,
    /// Whether this is the member's default address.
    pub is_default: bool,
    /// When the address was saved.
    pub created_at: DateTime<Utc>,
    /// When the address last changed.
    pub updated_at: DateTime<Utc>,
}

/// The validated, normalised fields of an address write: what
/// the service hands the repository after the shared checkout
/// validation has run. Postal code and phone are digit-only;
/// an empty building is `None`.
#[derive(Debug, Clone)]
pub struct AddressInput {
    /// The member's label (may be empty).
    pub label: String,
    /// Recipient family name.
    pub family_name: String,
    /// Recipient given name.
    pub given_name: String,
    /// Postal code, seven digits.
    pub postal_code: String,
    /// Prefecture.
    pub prefecture: String,
    /// City / ward.
    pub city: String,
    /// Street address.
    pub address_line: String,
    /// Building / room, when given.
    pub building: Option<String>,
    /// Phone number, digits only.
    pub phone: String,
}

impl MemberAddress {
    /// The name a list or picker shows for this address: the
    /// member's label when one is set, else the recipient name.
    pub fn display_name(&self) -> String {
        if self.label.trim().is_empty() {
            format!("{} {}", self.family_name, self.given_name)
        } else {
            self.label.clone()
        }
    }

    /// The address as one line, in the order-detail format:
    /// prefecture, city, street and building run together.
    pub fn street_line(&self) -> String {
        match &self.building {
            Some(building) if !building.trim().is_empty() => format!(
                "{}{}{} {}",
                self.prefecture, self.city, self.address_line, building
            ),
            _ => format!("{}{}{}", self.prefecture, self.city, self.address_line),
        }
    }

    /// The recipient line: family and given name with 様.
    pub fn recipient(&self) -> String {
        format!("{} {} 様", self.family_name, self.given_name)
    }
}
