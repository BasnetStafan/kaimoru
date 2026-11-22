//! Admin shop settings rules.
//!
//! Two sections, two validators, one stored row: the shop's
//! words (name, contact, footer, SEO defaults) and its
//! shipping/delivery settings (fees, delivery window, offered
//! slots and carriers). Each validator starts from the stored
//! row and replaces only its own section's fields, so saving
//! one section can never reset the other. Payment flags and
//! secrets are not fields here at all. Lengths are counted in
//! characters, not bytes, and every failing field reports at
//! once.

use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{EMAIL_FORMAT_MESSAGE, max_length_message};

use crate::models::ShopSettings;
use crate::repositories::shop_settings_repository;
use crate::services::{admin::audit_service, shop_settings_service};
use crate::validation::{FieldError, is_valid_email};

/// Shop name limit, in characters.
const SHOP_NAME_MAX_CHARS: usize = 50;
/// Tagline limit, in characters.
const TAGLINE_MAX_CHARS: usize = 80;
/// Shop description limit, in characters.
const DESCRIPTION_MAX_CHARS: usize = 500;
/// Support note limit, in characters.
const SUPPORT_NOTE_MAX_CHARS: usize = 200;
/// Footer note limit, in characters.
const FOOTER_NOTE_MAX_CHARS: usize = 200;
/// Copyright holder limit, in characters.
const COPYRIGHT_HOLDER_MAX_CHARS: usize = 50;
/// Top title limit, in characters.
const TOP_TITLE_MAX_CHARS: usize = 80;
/// Meta description limit, in characters: the same cap the
/// content pages use for theirs.
const META_DESCRIPTION_MAX_CHARS: usize = 160;
/// Mail signature limit, in characters: a signature is a
/// few lines of identity text, not a second template.
const MAIL_SIGNATURE_MAX_CHARS: usize = 1000;

/// The raw shop settings form as submitted: every value a
/// string, exactly as typed.
#[derive(Clone, Default)]
pub struct ShopSettingsInput {
    /// Shop name as typed.
    pub shop_name: String,
    /// Tagline as typed.
    pub tagline: String,
    /// Shop description as typed.
    pub shop_description: String,
    /// Contact email as typed.
    pub contact_email: String,
    /// Contact URL as typed.
    pub contact_url: String,
    /// Support note as typed.
    pub support_note: String,
    /// Footer note as typed.
    pub footer_note: String,
    /// Copyright holder as typed.
    pub copyright_holder: String,
    /// Top title as typed.
    pub top_title: String,
    /// Default meta description as typed.
    pub default_meta_description: String,
    /// Mail signature as typed.
    pub mail_signature: String,
    /// Qualified-invoice registration number as typed
    ///; empty = not registered.
    pub invoice_registration_number: String,
    /// Maintenance mode: the storefront closure
    /// toggle.
    pub maintenance_mode: bool,
    /// The maintenance page's message.
    pub maintenance_message: String,
}

impl From<&ShopSettings> for ShopSettingsInput {
    /// The form's initial values: the stored settings, verbatim.
    fn from(settings: &ShopSettings) -> Self {
        Self {
            shop_name: settings.shop_name.clone(),
            tagline: settings.tagline.clone(),
            shop_description: settings.shop_description.clone(),
            contact_email: settings.contact_email.clone(),
            contact_url: settings.contact_url.clone(),
            support_note: settings.support_note.clone(),
            footer_note: settings.footer_note.clone(),
            copyright_holder: settings.copyright_holder.clone(),
            top_title: settings.top_title.clone(),
            default_meta_description: settings.default_meta_description.clone(),
            mail_signature: settings.mail_signature.clone(),
            invoice_registration_number: settings.invoice_registration_number.clone(),
            maintenance_mode: settings.maintenance_mode,
            maintenance_message: settings.maintenance_message.clone(),
        }
    }
}

/// Pushes the length error for one field when its trimmed value
/// is over its cap.
fn check_length(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    label: &str,
    value: &str,
    max: usize,
) {
    if value.chars().count() > max {
        errors.push(FieldError::new(field, &max_length_message(label, max)));
    }
}

/// Validates the settings form: the shop name is the only
/// required field; the contact address, when given, must be a
/// valid email shape; the contact URL, when given, must be an
/// internal path or an `https:` URL — never a script scheme or
/// a protocol-relative URL the footer would render as a link.
/// Values are trimmed before they are checked and stored.
pub fn validate_settings(
    input: &ShopSettingsInput,
    current: &ShopSettings,
) -> Result<ShopSettings, Vec<FieldError>> {
    let settings = ShopSettings {
        // The shipping/delivery section rides through from the
        // stored row: this form does not carry it.
        shipping_flat_yen: current.shipping_flat_yen,
        free_shipping_threshold_yen: current.free_shipping_threshold_yen,
        cod_fee_yen: current.cod_fee_yen,
        delivery_earliest_offset_days: current.delivery_earliest_offset_days,
        delivery_latest_offset_days: current.delivery_latest_offset_days,
        disabled_delivery_slots: current.disabled_delivery_slots.clone(),
        disabled_carriers: current.disabled_carriers.clone(),
        // The points section rides through the same way.
        points_enabled: current.points_enabled,
        points_earn_percent: current.points_earn_percent,
        points_expiry_days: current.points_expiry_days,
        shop_name: input.shop_name.trim().to_string(),
        tagline: input.tagline.trim().to_string(),
        shop_description: input.shop_description.trim().to_string(),
        contact_email: input.contact_email.trim().to_string(),
        contact_url: input.contact_url.trim().to_string(),
        support_note: input.support_note.trim().to_string(),
        footer_note: input.footer_note.trim().to_string(),
        copyright_holder: input.copyright_holder.trim().to_string(),
        top_title: input.top_title.trim().to_string(),
        default_meta_description: input.default_meta_description.trim().to_string(),
        mail_signature: input.mail_signature.trim().to_string(),
        invoice_registration_number: input.invoice_registration_number.trim().to_string(),
        maintenance_mode: input.maintenance_mode,
        maintenance_message: input.maintenance_message.trim().to_string(),
    };
    let mut errors = Vec::new();
    if settings.shop_name.is_empty() {
        errors.push(FieldError::new("shop_name", "店舗名を入力してください。"));
    }
    // The registration number is structured: `T` followed by
    // exactly 13 digits. Empty is legitimate (not registered);
    // anything else must match the national format exactly.
    let number = &settings.invoice_registration_number;
    if !number.is_empty()
        && !(number.len() == 14
            && number.starts_with('T')
            && number[1..].bytes().all(|b| b.is_ascii_digit()))
    {
        errors.push(FieldError::new(
            "invoice_registration_number",
            "登録番号はTに続く13桁の数字で入力してください（未登録の場合は空欄）。",
        ));
    }
    check_length(
        &mut errors,
        "maintenance_message",
        "メンテナンスメッセージ",
        &settings.maintenance_message,
        500,
    );
    check_length(
        &mut errors,
        "shop_name",
        "店舗名",
        &settings.shop_name,
        SHOP_NAME_MAX_CHARS,
    );
    check_length(
        &mut errors,
        "tagline",
        "キャッチコピー",
        &settings.tagline,
        TAGLINE_MAX_CHARS,
    );
    check_length(
        &mut errors,
        "shop_description",
        "店舗紹介",
        &settings.shop_description,
        DESCRIPTION_MAX_CHARS,
    );
    if !settings.contact_email.is_empty() && !is_valid_email(&settings.contact_email) {
        errors.push(FieldError::new("contact_email", EMAIL_FORMAT_MESSAGE));
    }
    if !settings.contact_url.is_empty()
        && !(settings.contact_url.starts_with("https://")
            || (settings.contact_url.starts_with('/') && !settings.contact_url.starts_with("//")))
    {
        errors.push(FieldError::new(
            "contact_url",
            "URLは「/」で始まるパスか、https://で始まるURLを入力してください。",
        ));
    }
    check_length(
        &mut errors,
        "support_note",
        "サポート案内",
        &settings.support_note,
        SUPPORT_NOTE_MAX_CHARS,
    );
    check_length(
        &mut errors,
        "footer_note",
        "フッター補足",
        &settings.footer_note,
        FOOTER_NOTE_MAX_CHARS,
    );
    check_length(
        &mut errors,
        "copyright_holder",
        "著作権者名",
        &settings.copyright_holder,
        COPYRIGHT_HOLDER_MAX_CHARS,
    );
    check_length(
        &mut errors,
        "top_title",
        "トップページのタイトル",
        &settings.top_title,
        TOP_TITLE_MAX_CHARS,
    );
    check_length(
        &mut errors,
        "default_meta_description",
        "メタディスクリプション",
        &settings.default_meta_description,
        META_DESCRIPTION_MAX_CHARS,
    );
    check_length(
        &mut errors,
        "mail_signature",
        "メール署名",
        &settings.mail_signature,
        MAIL_SIGNATURE_MAX_CHARS,
    );
    if errors.is_empty() {
        Ok(settings)
    } else {
        Err(errors)
    }
}

/// Returns the stored settings for the form, or the fallback
/// values when the row is missing.
pub async fn get_settings(pool: &PgPool) -> Result<ShopSettings, AppError> {
    Ok(shop_settings_repository::find(pool)
        .await?
        .unwrap_or_else(ShopSettings::fallback))
}

/// Stores validated settings and audits the change in the same
/// transaction.
///
/// Only the changed field *names* are ever recorded — in the
/// service log and nowhere else; values, above all the contact
/// address, are never logged (the logging rules). The audit row
/// itself follows the trail's house shape: action and target
/// only. A submit that changes nothing writes nothing.
pub async fn update_settings(
    pool: &PgPool,
    admin_id: i64,
    settings: &ShopSettings,
    expected: i64,
) -> Result<(), AppError> {
    let current = get_settings(pool).await?;
    let changed = changed_fields(&current, settings);
    if changed.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    // The form-ticket staleness guard (see
    // `form_ticket_service`); either section's form posts
    // the same ticket scope, so one guard covers both.
    let stamp = shop_settings_repository::updated_at_for_update(&mut tx).await?;
    crate::services::form_ticket_service::ensure_fresh(stamp, expected)?;
    shop_settings_repository::update(&mut tx, settings, admin_id).await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "shop_settings.update",
        "shop_settings",
        Some(1),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    // Storefront reads go through the settings cache; drop it
    // so the new words are live on the next page.
    shop_settings_service::invalidate_cache();
    // Field names only — a settings value must never reach a log line.
    tracing::info!(
        admin_id,
        fields = ?changed,
        "shop_settings.update: shop settings changed"
    );
    Ok(())
}

/// Returns the names of the fields whose values differ between
/// the stored settings and the submitted ones.
fn changed_fields(current: &ShopSettings, next: &ShopSettings) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if current.shop_name != next.shop_name {
        changed.push("shop_name");
    }
    if current.tagline != next.tagline {
        changed.push("tagline");
    }
    if current.shop_description != next.shop_description {
        changed.push("shop_description");
    }
    if current.contact_email != next.contact_email {
        changed.push("contact_email");
    }
    if current.contact_url != next.contact_url {
        changed.push("contact_url");
    }
    if current.support_note != next.support_note {
        changed.push("support_note");
    }
    if current.footer_note != next.footer_note {
        changed.push("footer_note");
    }
    if current.copyright_holder != next.copyright_holder {
        changed.push("copyright_holder");
    }
    if current.top_title != next.top_title {
        changed.push("top_title");
    }
    if current.default_meta_description != next.default_meta_description {
        changed.push("default_meta_description");
    }
    if current.mail_signature != next.mail_signature {
        changed.push("mail_signature");
    }
    if current.invoice_registration_number != next.invoice_registration_number {
        changed.push("invoice_registration_number");
    }
    if current.maintenance_mode != next.maintenance_mode {
        changed.push("maintenance_mode");
    }
    if current.points_enabled != next.points_enabled {
        changed.push("points_enabled");
    }
    if current.points_earn_percent != next.points_earn_percent {
        changed.push("points_earn_percent");
    }
    if current.points_expiry_days != next.points_expiry_days {
        changed.push("points_expiry_days");
    }
    if current.maintenance_message != next.maintenance_message {
        changed.push("maintenance_message");
    }
    if current.shipping_flat_yen != next.shipping_flat_yen {
        changed.push("shipping_flat_yen");
    }
    if current.free_shipping_threshold_yen != next.free_shipping_threshold_yen {
        changed.push("free_shipping_threshold_yen");
    }
    if current.cod_fee_yen != next.cod_fee_yen {
        changed.push("cod_fee_yen");
    }
    if current.delivery_earliest_offset_days != next.delivery_earliest_offset_days {
        changed.push("delivery_earliest_offset_days");
    }
    if current.delivery_latest_offset_days != next.delivery_latest_offset_days {
        changed.push("delivery_latest_offset_days");
    }
    if current.disabled_delivery_slots != next.disabled_delivery_slots {
        changed.push("disabled_delivery_slots");
    }
    if current.disabled_carriers != next.disabled_carriers {
        changed.push("disabled_carriers");
    }
    changed
}

/// The raw shipping/delivery settings form as submitted:
/// amounts as typed (the service parses them), the disabled
/// lists as checkbox code sets.
#[derive(Clone, Default)]
pub struct ShippingSettingsInput {
    /// Flat shipping fee as typed.
    pub shipping_flat_yen: String,
    /// Free-shipping threshold as typed.
    pub free_shipping_threshold_yen: String,
    /// COD fee as typed.
    pub cod_fee_yen: String,
    /// Earliest delivery offset in days, as typed.
    pub delivery_earliest_offset_days: String,
    /// Latest delivery offset in days, as typed.
    pub delivery_latest_offset_days: String,
    /// Delivery slot codes the operator unchecked (disabled).
    pub disabled_delivery_slots: Vec<String>,
    /// Carrier codes the operator unchecked (disabled).
    pub disabled_carriers: Vec<String>,
}

impl ShippingSettingsInput {
    /// The form's initial values: the stored settings,
    /// rendered back into form shape.
    pub fn from_settings(settings: &ShopSettings) -> Self {
        Self {
            shipping_flat_yen: settings.shipping_flat_yen.to_string(),
            free_shipping_threshold_yen: settings.free_shipping_threshold_yen.to_string(),
            cod_fee_yen: settings.cod_fee_yen.to_string(),
            delivery_earliest_offset_days: settings.delivery_earliest_offset_days.to_string(),
            delivery_latest_offset_days: settings.delivery_latest_offset_days.to_string(),
            disabled_delivery_slots: settings
                .disabled_delivery_slots
                .split(',')
                .filter(|c| !c.trim().is_empty())
                .map(|c| c.trim().to_string())
                .collect(),
            disabled_carriers: settings
                .disabled_carriers
                .split(',')
                .filter(|c| !c.trim().is_empty())
                .map(|c| c.trim().to_string())
                .collect(),
        }
    }
}

/// Largest fee the form accepts, in yen: a sanity ceiling far
/// above any real fee, so a typo with too many digits reports
/// as an error instead of silently repricing the shop.
const FEE_MAX_YEN: i64 = 1_000_000;
/// Largest delivery-window offset the form accepts, in days.
const OFFSET_MAX_DAYS: i64 = 60;

/// Parses one yen amount field: a plain non-negative integer
/// up to [`FEE_MAX_YEN`]; anything else is the field's error.
fn parse_fee(errors: &mut Vec<FieldError>, field: &'static str, label: &str, raw: &str) -> i64 {
    match raw.trim().parse::<i64>() {
        Ok(v) if (0..=FEE_MAX_YEN).contains(&v) => v,
        _ => {
            errors.push(FieldError::new(
                field,
                &format!("{label}は0〜{FEE_MAX_YEN}の整数で入力してください。"),
            ));
            0
        }
    }
}

/// Parses one day-offset field: a non-negative integer up to
/// [`OFFSET_MAX_DAYS`].
fn parse_offset(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    label: &str,
    raw: &str,
) -> Option<i32> {
    match raw.trim().parse::<i64>() {
        Ok(v) if (0..=OFFSET_MAX_DAYS).contains(&v) => Some(v as i32),
        _ => {
            errors.push(FieldError::new(
                field,
                &format!("{label}は0〜{OFFSET_MAX_DAYS}の整数で入力してください。"),
            ));
            None
        }
    }
}

/// Validates the shipping/delivery form against the stored
/// row: fees and offsets parse as bounded integers, the
/// latest offset may not precede the earliest, and disabled
/// codes must come from the catalogues (`none` can never be
/// disabled — see [`ShopSettings::slot_disabled`]). Returns
/// the full settings row with only this section replaced.
pub fn validate_shipping(
    input: &ShippingSettingsInput,
    current: &ShopSettings,
) -> Result<ShopSettings, Vec<FieldError>> {
    let mut errors = Vec::new();
    let shipping_flat_yen = parse_fee(
        &mut errors,
        "shipping_flat_yen",
        "送料",
        &input.shipping_flat_yen,
    );
    let free_shipping_threshold_yen = parse_fee(
        &mut errors,
        "free_shipping_threshold_yen",
        "送料無料の基準額",
        &input.free_shipping_threshold_yen,
    );
    let cod_fee_yen = parse_fee(
        &mut errors,
        "cod_fee_yen",
        "代引き手数料",
        &input.cod_fee_yen,
    );
    let earliest = parse_offset(
        &mut errors,
        "delivery_earliest_offset_days",
        "最短お届け日（注文日から）",
        &input.delivery_earliest_offset_days,
    );
    let latest = parse_offset(
        &mut errors,
        "delivery_latest_offset_days",
        "最長お届け日（注文日から）",
        &input.delivery_latest_offset_days,
    );
    // The range check runs whenever both offsets parsed, even
    // when a fee also failed: every problem reports at once.
    if let (Some(e), Some(l)) = (earliest, latest)
        && l < e
    {
        errors.push(FieldError::new(
            "delivery_latest_offset_days",
            "最長お届け日は最短お届け日以降の日数を入力してください。",
        ));
    }
    let mut settings = current.clone();
    settings.shipping_flat_yen = shipping_flat_yen;
    settings.free_shipping_threshold_yen = free_shipping_threshold_yen;
    settings.cod_fee_yen = cod_fee_yen;
    settings.delivery_earliest_offset_days =
        earliest.unwrap_or(current.delivery_earliest_offset_days);
    settings.delivery_latest_offset_days = latest.unwrap_or(current.delivery_latest_offset_days);
    settings.disabled_delivery_slots = input
        .disabled_delivery_slots
        .iter()
        .map(|c| c.trim())
        .filter(|c| {
            crate::models::delivery::DELIVERY_SLOTS
                .iter()
                .any(|s| s.code == *c)
                && *c != "none"
        })
        .collect::<Vec<_>>()
        .join(",");
    settings.disabled_carriers = input
        .disabled_carriers
        .iter()
        .map(|c| c.trim())
        .filter(|c| {
            crate::models::carrier::CARRIERS
                .iter()
                .any(|k| k.code == *c)
        })
        .collect::<Vec<_>>()
        .join(",");
    if errors.is_empty() {
        Ok(settings)
    } else {
        Err(errors)
    }
}

/// The points section of the settings form (EC V2 core 1):
/// its own struct, its own save, like the shipping section.
#[derive(Clone, Debug)]
pub struct PointsSettingsInput {
    /// Whether the programme is live.
    pub points_enabled: bool,
    /// Earn rate as typed (percent of the earn base).
    pub points_earn_percent: String,
    /// Grant lifetime as typed, in days.
    pub points_expiry_days: String,
}

impl PointsSettingsInput {
    /// The form's initial values: the stored settings,
    /// rendered back into form shape.
    pub fn from_settings(settings: &ShopSettings) -> Self {
        Self {
            points_enabled: settings.points_enabled,
            points_earn_percent: settings.points_earn_percent.to_string(),
            points_expiry_days: settings.points_expiry_days.to_string(),
        }
    }
}

/// Largest earn percent the form accepts: points are a
/// rebate, and anything past 100% pays the customer to buy.
const EARN_PERCENT_MAX: i64 = 100;
/// Largest grant lifetime the form accepts, in days
/// (ten years: a sanity ceiling, not a policy).
const EXPIRY_DAYS_MAX: i64 = 3650;

/// Parses one bounded integer field for the points form.
fn parse_bounded(
    errors: &mut Vec<FieldError>,
    field: &'static str,
    label: &str,
    raw: &str,
    min: i64,
    max: i64,
) -> Option<i32> {
    match raw.trim().parse::<i64>() {
        Ok(v) if (min..=max).contains(&v) => Some(v as i32),
        _ => {
            errors.push(FieldError::new(
                field,
                &format!("{label}は{min}〜{max}の整数で入力してください。"),
            ));
            None
        }
    }
}

/// Validates the points form against the stored row.
/// Returns the full settings row with only this section
/// replaced (the shipping section's shape).
pub fn validate_points(
    input: &PointsSettingsInput,
    current: &ShopSettings,
) -> Result<ShopSettings, Vec<FieldError>> {
    let mut errors = Vec::new();
    let earn_percent = parse_bounded(
        &mut errors,
        "points_earn_percent",
        "付与率（%）",
        &input.points_earn_percent,
        0,
        EARN_PERCENT_MAX,
    );
    let expiry_days = parse_bounded(
        &mut errors,
        "points_expiry_days",
        "有効期限（日）",
        &input.points_expiry_days,
        1,
        EXPIRY_DAYS_MAX,
    );
    let mut settings = current.clone();
    settings.points_enabled = input.points_enabled;
    settings.points_earn_percent = earn_percent.unwrap_or(current.points_earn_percent);
    settings.points_expiry_days = expiry_days.unwrap_or(current.points_expiry_days);
    if errors.is_empty() {
        Ok(settings)
    } else {
        Err(errors)
    }
}
