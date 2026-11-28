//! Views for the member address book: the mypage list, the
//! shared add/edit form, and the delete confirmation. The
//! address text block itself is a shared template part
//! (`storefront/parts/address_lines.html`), used by these
//! pages and by the checkout picker alike.

use askama::Template;

use crate::models::MemberAddress;
use crate::services::member_address_service::AddressFormValues;
use crate::validation::FieldError;
use crate::views::storefront::header::Header;

/// Per-field errors for the address form, mirroring
/// [`crate::views::storefront::checkout::CheckoutErrors`].
#[derive(Default)]
pub struct AddressFormErrors {
    /// Error for the label field.
    pub label: Option<String>,
    /// Error for the family name field.
    pub family_name: Option<String>,
    /// Error for the given name field.
    pub given_name: Option<String>,
    /// Error for the postal code field.
    pub postal_code: Option<String>,
    /// Error for the prefecture field.
    pub prefecture: Option<String>,
    /// Error for the city field.
    pub city: Option<String>,
    /// Error for the street address field.
    pub address_line: Option<String>,
    /// Error for the phone field.
    pub phone: Option<String>,
}

impl AddressFormErrors {
    /// Builds the per-field errors from the service's list.
    pub fn from_field_errors(list: &[FieldError]) -> Self {
        let mut errors = Self::default();
        for e in list {
            let slot = match e.field {
                "label" => &mut errors.label,
                "family_name" => &mut errors.family_name,
                "given_name" => &mut errors.given_name,
                "postal_code" => &mut errors.postal_code,
                "prefecture" => &mut errors.prefecture,
                "city" => &mut errors.city,
                "address_line" => &mut errors.address_line,
                "phone" => &mut errors.phone,
                _ => continue,
            };
            *slot = Some(e.message.clone());
        }
        errors
    }
}

/// The address book list at `/mypage/addresses`.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/addresses/list.html")]
pub struct AddressBookView {
    /// Page header.
    pub header: Header,
    /// The member's addresses, default first.
    pub addresses: Vec<MemberAddress>,
    /// Whether the book is at its cap (the add button renders
    /// disabled with the note).
    pub at_cap: bool,
    /// The configured cap, for the at-cap note's count.
    pub max_addresses: i64,
}

/// The shared add/edit address form.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/addresses/form.html")]
pub struct AddressFormView {
    /// Page header.
    pub header: Header,
    /// The page heading （新しいお届け先 / お届け先の編集）.
    pub heading: String,
    /// The form's POST target.
    pub form_action: String,
    /// The submit button's label.
    pub submit_label: String,
    /// Current field values.
    pub values: AddressFormValues,
    /// Per-field errors from the last submit.
    pub errors: AddressFormErrors,
    /// A form-level error (the cap), when one occurred.
    pub form_error: Option<String>,
    /// The 47 prefectures for the select.
    pub prefectures: &'static [&'static str],
    /// Whether the make-default checkbox starts checked.
    pub make_default_checked: bool,
    /// Whether the address being edited is already the
    /// default: the checkbox renders checked and disabled,
    /// with the note that the default moves from the list.
    pub default_locked: bool,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`); empty on
    /// the create form, which has no record to guard.
    pub form_ticket: String,
}

/// The styled 404 for an unknown or foreign address id:
/// the address book's counterpart of the mypage order
/// missing page, worded for addresses.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/addresses/missing.html")]
pub struct AddressMissingView {
    /// Page header.
    pub header: Header,
}

/// The delete confirmation page: the address, and what
/// happens to the default when this address is it.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/addresses/delete.html")]
pub struct AddressDeleteView {
    /// Page header.
    pub header: Header,
    /// The address to delete.
    pub address: MemberAddress,
}
