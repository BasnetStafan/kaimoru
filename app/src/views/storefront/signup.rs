//! Sign-up views.
//! Data for the sign-up flow templates (email, verify, details,
//! confirm steps).

use askama::Template;

use crate::views::storefront::header::Header;

/// Values of the member details form, kept as text for re-rendering
/// after a validation error.
#[derive(Default, Clone)]
pub struct DetailsValues {
    /// Family name in kanji.
    pub family_name: String,
    /// Given name in kanji.
    pub given_name: String,
    /// Family name in katakana.
    pub family_name_kana: String,
    /// Given name in katakana.
    pub given_name_kana: String,
    /// Birth date text.
    pub birth_date: String,
    /// Gender text.
    pub gender: String,
    /// Postal code text.
    pub postal_code: String,
    /// Prefecture name.
    pub prefecture: String,
    /// City and ward.
    pub city: String,
    /// Street address.
    pub address_line: String,
    /// Building name.
    pub building: String,
    /// Phone number text.
    pub phone: String,
    /// Whether the mail magazine box was checked.
    pub mail_magazine: bool,
}

/// Renders pages/signup/email.html, the email entry step.
#[derive(Template)]
#[template(path = "storefront/pages/signup/email.html")]
pub struct SignupView {
    /// Shared header state.
    pub header: Header,
    /// Validation error to show, if any.
    pub error: Option<String>,
    /// Email entered before, kept after an error.
    pub email: String,
    /// The SNS button block (empty when no provider is
    /// available, which hides the block).
    pub sns: crate::views::storefront::sns::SnsBlock,
}

/// Renders pages/signup/verify.html, the code entry step.
#[derive(Template)]
#[template(path = "storefront/pages/signup/verify.html")]
pub struct VerifyView {
    /// Shared header state.
    pub header: Header,
    /// Error to show, if any.
    pub error: Option<String>,
    /// Notice to show, for example after a resend.
    pub notice: Option<String>,
    /// The destination email, masked for display.
    pub email_masked: String,
}

/// Renders pages/signup/details.html, the member details step.
#[derive(Template)]
#[template(path = "storefront/pages/signup/details.html")]
pub struct DetailsView {
    /// Whether this is the SNS variant: no password fields,
    /// the SNS details/confirm endpoints.
    pub sns_mode: bool,
    /// Shared header state.
    pub header: Header,
    /// Validation error to show, if any.
    pub error: Option<String>,
    /// The verified email, shown read-only.
    pub email: String,
    /// Current form values.
    pub values: DetailsValues,
    /// Per-field validation errors, shown under their fields.
    pub errors: DetailsErrors,
    /// Prefecture options for the select.
    pub prefectures: &'static [&'static str],
}

/// Per-field validation errors for the details form.
/// Each message renders in red directly under its own field.
#[derive(Default)]
pub struct DetailsErrors {
    /// Password error.
    pub password: Option<String>,
    /// Password confirmation error.
    pub password_confirm: Option<String>,
    /// Family name error.
    pub family_name: Option<String>,
    /// Given name error.
    pub given_name: Option<String>,
    /// Family name kana error.
    pub family_name_kana: Option<String>,
    /// Given name kana error.
    pub given_name_kana: Option<String>,
    /// Birth date error.
    pub birth_date: Option<String>,
    /// Postal code error.
    pub postal_code: Option<String>,
    /// Prefecture error.
    pub prefecture: Option<String>,
    /// City error.
    pub city: Option<String>,
    /// Address line error.
    pub address_line: Option<String>,
    /// Phone error.
    pub phone: Option<String>,
}

impl DetailsErrors {
    /// Builds the error set from the service layer's field errors.
    pub fn from_field_errors(list: &[crate::validation::FieldError]) -> Self {
        let mut errors = Self::default();
        for error in list {
            let slot = match error.field {
                "password" => &mut errors.password,
                "password_confirm" => &mut errors.password_confirm,
                "family_name" => &mut errors.family_name,
                "given_name" => &mut errors.given_name,
                "family_name_kana" => &mut errors.family_name_kana,
                "given_name_kana" => &mut errors.given_name_kana,
                "birth_date" => &mut errors.birth_date,
                "postal_code" => &mut errors.postal_code,
                "prefecture" => &mut errors.prefecture,
                "city" => &mut errors.city,
                "address_line" => &mut errors.address_line,
                "phone" => &mut errors.phone,
                _ => continue,
            };
            *slot = Some(error.message.clone());
        }
        errors
    }
}

/// Renders pages/signup/confirm.html, the confirmation step.
#[derive(Template)]
#[template(path = "storefront/pages/signup/confirm.html")]
pub struct ConfirmView {
    /// Whether this is the SNS variant (posts to the SNS
    /// confirm endpoint, no password row).
    pub sns_mode: bool,
    /// Shared header state.
    pub header: Header,
    /// Error to show, if any.
    pub error: Option<String>,
    /// The verified email.
    pub email: String,
    /// Family name in kanji.
    pub family_name: String,
    /// Given name in kanji.
    pub given_name: String,
    /// Family name in katakana.
    pub family_name_kana: String,
    /// Given name in katakana.
    pub given_name_kana: String,
    /// Birth date text, empty when not given.
    pub birth_date: String,
    /// Gender text, empty when not given.
    pub gender: String,
    /// Postal code.
    pub postal_code: String,
    /// Prefecture name.
    pub prefecture: String,
    /// City and ward.
    pub city: String,
    /// Street address.
    pub address_line: String,
    /// Building name, empty when not given.
    pub building: String,
    /// Phone number.
    pub phone: String,
    /// Mail magazine choice as display text.
    pub mail_magazine_text: String,
}
