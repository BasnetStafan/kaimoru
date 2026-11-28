//! Shop settings.
//!
//! The operator's page for the shop's own words: name, contact,
//! footer and SEO defaults (CMS spec C-1). The rules live in
//! `crate::services::admin::shop_settings_service` — lengths,
//! the email and URL shapes, and the audited write. Money and
//! security configuration are not on this page at all.
//!
//! Success answers with a redirect back to this page (the admin
//! surface's Post/Redirect/Get pattern). A settings save rotates
//! no CSRF token: the middleware rotates only when the session's
//! principal or credential fingerprint moves, and a shop
//! settings save moves neither.

use axum::{
    Form, Router,
    extract::State,
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, render, set_flash};
use crate::services::admin::shop_settings_service::{
    self, PointsSettingsInput, ShippingSettingsInput, ShopSettingsInput,
};
use crate::session::CurrentAdmin;
use crate::validation::field_error;
use crate::views::admin::shop_settings::ShopSettingsView;

/// Routes for the shop settings surface: the words form and
/// the shipping/delivery form are separate posts on the same
/// page, each with its own save and completion message.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/settings/shop", get(page).post(update))
        .route(
            "/admin/settings/shop/shipping",
            axum::routing::post(update_shipping),
        )
        .route(
            "/admin/settings/shop/points",
            axum::routing::post(update_points),
        )
}

/// The shop settings form body: every field a string, exactly
/// as typed — trimming and validation live in the service.
#[derive(Deserialize)]
pub struct ShopSettingsForm {
    /// Shop name.
    shop_name: String,
    /// Tagline.
    tagline: String,
    /// Shop description.
    shop_description: String,
    /// Contact email.
    contact_email: String,
    /// Contact URL.
    contact_url: String,
    /// Support note.
    support_note: String,
    /// Footer note.
    footer_note: String,
    /// Copyright holder.
    copyright_holder: String,
    /// Top page title override.
    top_title: String,
    /// Default meta description.
    default_meta_description: String,
    /// Mail signature text. Optional in the form: a
    /// client that does not send the field leaves the
    /// stored signature untouched.
    mail_signature: Option<String>,
    /// Qualified-invoice registration number; a client
    /// that omits the field submits the default empty value,
    /// which the caller resolves like the signature.
    invoice_registration_number: Option<String>,
    /// Maintenance toggle: the checkbox posts `on`
    /// when ticked and nothing when not.
    maintenance_mode: Option<String>,
    /// The maintenance page's message.
    maintenance_message: Option<String>,
    /// The one-time form ticket minted when this page was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the page, which the
    /// save treats as stale.
    #[serde(default)]
    form_ticket: String,
}

impl ShopSettingsForm {
    /// Carries the submitted form into the service's input
    /// shape. The mail signature arrives resolved by the
    /// caller: a client that omits the field (an older
    /// form) passes the stored signature through, so
    /// saving the other settings never wipes it.
    fn into_input(
        self,
        mail_signature: String,
        invoice_registration_number: String,
    ) -> ShopSettingsInput {
        ShopSettingsInput {
            shop_name: self.shop_name,
            tagline: self.tagline,
            shop_description: self.shop_description,
            contact_email: self.contact_email,
            contact_url: self.contact_url,
            support_note: self.support_note,
            footer_note: self.footer_note,
            copyright_holder: self.copyright_holder,
            top_title: self.top_title,
            default_meta_description: self.default_meta_description,
            mail_signature,
            invoice_registration_number,
            maintenance_mode: self.maintenance_mode.as_deref() == Some("on"),
            maintenance_message: self.maintenance_message.clone().unwrap_or_default(),
        }
    }
}

/// The settings page, freshly rendered with the stored values.
/// The form-ticket scope of the singleton settings row.
/// Both section forms on the settings page share it: any
/// save moves the row's stamp and stales the other form.
const SETTINGS_TICKET_SCOPE: &str = "shop_settings";

/// Mints a settings ticket at the row's current stamp,
/// read from the repository (the `ShopSettings` struct
/// does not carry `updated_at`).
async fn settings_ticket(pool: &PgPool, session: &Session) -> String {
    // A failed stamp read is logged and reads as 0, a value
    // no real stamp equals, so the ticket fails closed (the
    // rule `profile_stamp` documents on the member side).
    let stamp = match crate::repositories::shop_settings_repository::updated_at(pool).await {
        Ok(stamp) => stamp.map(|t| t.timestamp_micros()).unwrap_or(0),
        Err(e) => {
            tracing::error!("shop settings stamp read failed: {e}");
            0
        }
    };
    crate::services::form_ticket_service::issue(session, SETTINGS_TICKET_SCOPE, stamp).await
}

/// The stale-save refusal shared by both section saves:
/// the flash explains, the reloaded page shows the
/// current values.
async fn settings_stale(session: &Session) -> Response {
    set_flash(session, crate::messages::STALE_FORM_MESSAGE).await;
    Redirect::to("/admin/settings/shop").into_response()
}

pub async fn page(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    match shop_settings_service::get_settings(&pool).await {
        Ok(settings) => {
            let mut view = ShopSettingsView::new(
                layout_for(&pool, &admin, &session).await,
                ShopSettingsInput::from(&settings),
                ShippingSettingsInput::from_settings(&settings),
                PointsSettingsInput::from_settings(&settings),
                payment_methods_label(),
            );
            view.form_ticket = settings_ticket(&pool, &session).await;
            render(view)
        }
        Err(e) => db_error("shop settings", e),
    }
}

/// Handles the settings save.
pub async fn update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<ShopSettingsForm>,
) -> Response {
    // Spend the settings ticket before anything else: a
    // submit without a valid ticket was not rendered from
    // the current settings, so it is refused as stale. A
    // ticket whose stamp has gone stale is refused by the
    // service's check instead.
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        SETTINGS_TICKET_SCOPE,
        &form.form_ticket,
    )
    .await
    else {
        return settings_stale(&session).await;
    };
    // A form without the mail signature field keeps the
    // stored signature (see `into_input`).
    let mail_signature = match form.mail_signature.clone() {
        Some(value) => value,
        None => match shop_settings_service::get_settings(&pool).await {
            Ok(current) => current.mail_signature,
            Err(e) => return db_error("shop settings", e),
        },
    };
    // Same rule for the registration number: an older
    // form without the field must not clear a stored number.
    let invoice_registration_number = match form.invoice_registration_number.clone() {
        Some(value) => value,
        None => match shop_settings_service::get_settings(&pool).await {
            Ok(current) => current.invoice_registration_number,
            Err(e) => return db_error("shop settings", e),
        },
    };
    let input: ShopSettingsInput = form.into_input(mail_signature, invoice_registration_number);
    // The validator merges onto the stored row, so the
    // shipping section (not this form's concern) rides through.
    let current = match shop_settings_service::get_settings(&pool).await {
        Ok(current) => current,
        Err(e) => return db_error("shop settings", e),
    };
    // Field errors first, all at once: a refused submit shows
    // every problem together, never one per round trip.
    let settings = match shop_settings_service::validate_settings(&input, &current) {
        Ok(settings) => settings,
        Err(errors) => {
            let mut view = ShopSettingsView::new(
                layout_for(&pool, &admin, &session).await,
                input,
                ShippingSettingsInput::from_settings(&current),
                PointsSettingsInput::from_settings(&current),
                payment_methods_label(),
            );
            view.shop_name_error = field_error(&errors, "shop_name");
            view.tagline_error = field_error(&errors, "tagline");
            view.shop_description_error = field_error(&errors, "shop_description");
            view.contact_email_error = field_error(&errors, "contact_email");
            view.contact_url_error = field_error(&errors, "contact_url");
            view.support_note_error = field_error(&errors, "support_note");
            view.footer_note_error = field_error(&errors, "footer_note");
            view.copyright_holder_error = field_error(&errors, "copyright_holder");
            view.top_title_error = field_error(&errors, "top_title");
            view.default_meta_description_error = field_error(&errors, "default_meta_description");
            view.mail_signature_error = field_error(&errors, "mail_signature");
            view.invoice_number_error = field_error(&errors, "invoice_registration_number");
            view.maintenance_message_error = field_error(&errors, "maintenance_message");
            view.form_ticket = settings_ticket(&pool, &session).await;
            return render(view);
        }
    };
    match shop_settings_service::update_settings(&pool, admin.id, &settings, expected).await {
        Ok(()) => {
            set_flash(&session, "店舗設定を保存しました").await;
            Redirect::to("/admin/settings/shop").into_response()
        }
        Err(crate::error::AppError::Conflict(_)) => settings_stale(&session).await,
        Err(e) => db_error("shop settings", e),
    }
}

/// The currently offered payment methods as one label line
/// （クレジットカード・…・代金引換）, from the same derivation
/// the checkout uses — the settings page's factual hint beside
/// the meta description field.
fn payment_methods_label() -> String {
    crate::services::payment_service::offered_methods()
        .iter()
        .map(|method| method.label())
        .collect::<Vec<_>>()
        .join("・")
}

/// Handles the shipping/delivery section's save. The form
/// posts raw pairs: the amount fields by name, and one
/// `enabled_slot` / `enabled_carrier` pair per checked box —
/// the disabled sets are the catalogue minus the checked
/// ones, so an unchecked box needs no hidden companion field.
pub async fn update_shipping(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    // Spend the settings ticket first (it arrives as one
    // of the raw pairs): a submit without a valid ticket
    // was not rendered from the current settings, so it is
    // refused as stale, exactly like the main form's save.
    let submitted_ticket = pairs
        .iter()
        .find(|(name, _)| name == "form_ticket")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        SETTINGS_TICKET_SCOPE,
        &submitted_ticket,
    )
    .await
    else {
        return settings_stale(&session).await;
    };
    let field = |name: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let enabled = |name: &str| {
        pairs
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .collect::<Vec<_>>()
    };
    let enabled_slots = enabled("enabled_slot");
    let enabled_carriers = enabled("enabled_carrier");
    let input = ShippingSettingsInput {
        shipping_flat_yen: field("shipping_flat_yen"),
        free_shipping_threshold_yen: field("free_shipping_threshold_yen"),
        cod_fee_yen: field("cod_fee_yen"),
        delivery_earliest_offset_days: field("delivery_earliest_offset_days"),
        delivery_latest_offset_days: field("delivery_latest_offset_days"),
        disabled_delivery_slots: crate::models::delivery::DELIVERY_SLOTS
            .iter()
            .map(|s| s.code)
            .filter(|code| *code != "none" && !enabled_slots.iter().any(|c| c == code))
            .map(str::to_string)
            .collect(),
        disabled_carriers: crate::models::carrier::CARRIERS
            .iter()
            .map(|c| c.code)
            .filter(|code| !enabled_carriers.iter().any(|c| c == code))
            .map(str::to_string)
            .collect(),
    };
    let current = match shop_settings_service::get_settings(&pool).await {
        Ok(current) => current,
        Err(e) => return db_error("shop settings", e),
    };
    let settings = match shop_settings_service::validate_shipping(&input, &current) {
        Ok(settings) => settings,
        Err(errors) => {
            let mut view = ShopSettingsView::new(
                layout_for(&pool, &admin, &session).await,
                ShopSettingsInput::from(&current),
                input,
                PointsSettingsInput::from_settings(&current),
                payment_methods_label(),
            );
            view.shipping_flat_error = field_error(&errors, "shipping_flat_yen");
            view.free_shipping_threshold_error =
                field_error(&errors, "free_shipping_threshold_yen");
            view.cod_fee_error = field_error(&errors, "cod_fee_yen");
            view.delivery_earliest_error = field_error(&errors, "delivery_earliest_offset_days");
            view.delivery_latest_error = field_error(&errors, "delivery_latest_offset_days");
            view.form_ticket = settings_ticket(&pool, &session).await;
            return render(view);
        }
    };
    match shop_settings_service::update_settings(&pool, admin.id, &settings, expected).await {
        Ok(()) => {
            set_flash(&session, "送料・配送設定を保存しました").await;
            Redirect::to("/admin/settings/shop").into_response()
        }
        Err(crate::error::AppError::Conflict(_)) => settings_stale(&session).await,
        Err(e) => db_error("shop settings", e),
    }
}

/// Handles the points section's save (EC V2 core 1): the
/// same ticket discipline as the other sections — the form
/// was rendered from the current settings, so a stale save
/// is refused rather than overwriting a newer one.
pub async fn update_points(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let submitted_ticket = pairs
        .iter()
        .find(|(name, _)| name == "form_ticket")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        SETTINGS_TICKET_SCOPE,
        &submitted_ticket,
    )
    .await
    else {
        return settings_stale(&session).await;
    };
    let field = |name: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let input = PointsSettingsInput {
        points_enabled: pairs
            .iter()
            .any(|(k, v)| k == "points_enabled" && v == "on"),
        points_earn_percent: field("points_earn_percent"),
        points_expiry_days: field("points_expiry_days"),
    };
    let current = match shop_settings_service::get_settings(&pool).await {
        Ok(current) => current,
        Err(e) => return db_error("shop settings", e),
    };
    let settings = match shop_settings_service::validate_points(&input, &current) {
        Ok(settings) => settings,
        Err(errors) => {
            let mut view = ShopSettingsView::new(
                layout_for(&pool, &admin, &session).await,
                ShopSettingsInput::from(&current),
                ShippingSettingsInput::from_settings(&current),
                input,
                payment_methods_label(),
            );
            view.points_earn_error = field_error(&errors, "points_earn_percent");
            view.points_expiry_error = field_error(&errors, "points_expiry_days");
            view.form_ticket = settings_ticket(&pool, &session).await;
            return render(view);
        }
    };
    match shop_settings_service::update_settings(&pool, admin.id, &settings, expected).await {
        Ok(()) => {
            set_flash(&session, "ポイント設定を保存しました").await;
            Redirect::to("/admin/settings/shop").into_response()
        }
        Err(crate::error::AppError::Conflict(_)) => settings_stale(&session).await,
        Err(e) => db_error("shop settings", e),
    }
}
