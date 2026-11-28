//! Mail module views: the メール配信 section's
//! dashboards, template and campaign screens.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::messages::{
    MAIL_NAME_EMAIL_CHANGE_CODE, MAIL_NAME_INQUIRY_ACKNOWLEDGEMENT, MAIL_NAME_KONBINI_INSTRUCTIONS,
    MAIL_NAME_ORDER_CANCELLATION, MAIL_NAME_ORDER_CONFIRMATION, MAIL_NAME_PASSWORD_RESET_CODE,
    MAIL_NAME_PAYMENT_CONFIRMATION, MAIL_NAME_SHIPPING_NOTIFICATION, MAIL_NAME_VERIFICATION_CODE,
    MAIL_NAME_WITHDRAWAL_CODE, MAIL_NAME_WITHDRAWAL_COMPLETED,
};
use crate::models::mail::{CampaignStats, EmailCampaign, EmailTemplate};
use crate::views::admin::Page;

/// The transactional mails the app sends outside the mail
/// module: (mail.rs function, Japanese name). The dashboard
/// inventories them as built-in — they are sent by their
/// own flows and never consent-gated. All but the inquiry
/// reply are editable from the template menu; the catalog
/// in `crate::models::notification_template` carries their
/// defaults and tag sets. Keep this list in step with
/// `crate::mail`.
pub const TRANSACTIONAL_MAILS: [(&str, &str); 12] = [
    ("verification_code", MAIL_NAME_VERIFICATION_CODE),
    ("password_reset_code", MAIL_NAME_PASSWORD_RESET_CODE),
    ("email_change_code", MAIL_NAME_EMAIL_CHANGE_CODE),
    ("withdrawal_code", MAIL_NAME_WITHDRAWAL_CODE),
    ("order_confirmation", MAIL_NAME_ORDER_CONFIRMATION),
    ("payment_confirmation", MAIL_NAME_PAYMENT_CONFIRMATION),
    ("konbini_instructions", MAIL_NAME_KONBINI_INSTRUCTIONS),
    ("shipping_notification", MAIL_NAME_SHIPPING_NOTIFICATION),
    ("order_cancellation", MAIL_NAME_ORDER_CANCELLATION),
    ("inquiry_acknowledgement", MAIL_NAME_INQUIRY_ACKNOWLEDGEMENT),
    ("send_inquiry_reply", "お問い合わせへの返信"),
    ("withdrawal_completed", MAIL_NAME_WITHDRAWAL_COMPLETED),
];

/// One dashboard campaign row, display-ready.
pub struct CampaignRow {
    /// Campaign ID.
    pub id: i64,
    /// Campaign name.
    pub name: String,
    /// Status pill label.
    pub status_label: &'static str,
    /// Schedule label (`YYYY/MM/DD HH:mm` JST), when set.
    pub scheduled_label: Option<String>,
    /// Sent label, when the campaign has finished.
    pub sent_label: Option<String>,
    /// Delivery tallies.
    pub stats: CampaignStats,
}

/// Builds the display row for one campaign.
pub fn campaign_row(campaign: &EmailCampaign, stats: CampaignStats) -> CampaignRow {
    CampaignRow {
        id: campaign.id,
        name: campaign.name.clone(),
        status_label: campaign.status_label(),
        scheduled_label: campaign
            .scheduled_at
            .map(crate::models::jst::jst_datetime_label),
        sent_label: campaign.sent_at.map(crate::models::jst::jst_datetime_label),
        stats,
    }
}

/// The mail dashboard page.
#[derive(Template)]
#[template(path = "admin/pages/mail/dashboard.html")]
pub struct MailDashboardView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Members who may receive marketing mail now.
    pub subscribers: i64,
    /// Deliveries sent this calendar month (JST).
    pub sent_this_month: i64,
    /// Deliveries that exhausted their attempts.
    pub failed: i64,
    /// The most recent campaigns.
    pub recent: Vec<CampaignRow>,
    /// Campaigns waiting for their scheduled time.
    pub scheduled: Vec<CampaignRow>,
    /// The built-in transactional mails: (function, name).
    pub transactional: Vec<(&'static str, &'static str)>,
}

/// One template row on the list.
pub struct TemplateRow {
    /// Template ID.
    pub id: i64,
    /// Template name.
    pub name: String,
    /// Subject line.
    pub subject: String,
    /// Last-update label.
    pub updated_label: String,
}

/// Builds the display row for one template.
pub fn template_row(template: &EmailTemplate) -> TemplateRow {
    TemplateRow {
        id: template.id,
        name: template.name.clone(),
        subject: template.subject.clone(),
        updated_label: crate::models::jst::jst_datetime_label(template.updated_at),
    }
}

/// The template list page.
#[derive(Template)]
#[template(path = "admin/pages/mail/template_list.html")]
pub struct TemplateListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The templates.
    pub rows: Vec<TemplateRow>,
    /// The notification mails and their override states.
    pub notifications: Vec<NotificationRow>,
}

/// One option of the campaign form's template picker.
pub struct TemplateOption {
    /// Template ID (empty = テンプレートを使わない).
    pub value: String,
    /// Template name.
    pub label: String,
    /// Whether this template is the selected one.
    pub selected: bool,
}

/// The template create/edit form page.
#[derive(Template)]
#[template(path = "admin/pages/mail/template_form.html")]
pub struct TemplateFormView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Page heading.
    pub heading: String,
    /// Form POST target.
    pub action: String,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`); empty on
    /// the create form, which has no record to guard.
    pub form_ticket: String,
    /// Delete POST target on the edit form.
    pub delete_action: Option<String>,
    /// Name field value.
    pub name: String,
    /// Subject field value.
    pub subject: String,
    /// HTML body field value.
    pub body_html: String,
    /// Text body field value.
    pub body_text: String,
    /// Name field error.
    pub name_error: Option<String>,
    /// Subject field error.
    pub subject_error: Option<String>,
    /// HTML body field error.
    pub body_html_error: Option<String>,
    /// Text body field error.
    pub body_text_error: Option<String>,
}

/// The campaign list page.
#[derive(Template)]
#[template(path = "admin/pages/mail/campaign_list.html")]
pub struct CampaignListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The campaigns of this page.
    pub rows: Vec<CampaignRow>,
    /// Pagination state.
    pub page: Page,
}

/// The campaign create/edit form page: the guided flow's
/// audience note, content fields, test send and schedule.
#[derive(Template)]
#[template(path = "admin/pages/mail/campaign_form.html")]
pub struct CampaignFormView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Page heading.
    pub heading: String,
    /// Form POST target.
    pub action: String,
    /// Campaign ID on the edit form (drives the test-send
    /// and confirm links).
    pub campaign_id: Option<i64>,
    /// Whether the campaign is still a draft: a sent or
    /// scheduled campaign renders read-only.
    pub editable: bool,
    /// Status pill label on the edit form.
    pub status_label: Option<&'static str>,
    /// The live eligible-recipient count (the audience).
    pub recipient_count: i64,
    /// Audience choices: all subscribers + segments.
    pub segment_options: Vec<SegmentOption>,
    /// The selected segment id as a form value ("" = all).
    pub segment_id: String,
    /// Segment field error.
    pub segment_error: Option<String>,
    /// Template picker options.
    pub template_options: Vec<TemplateOption>,
    /// Name field value.
    pub name: String,
    /// Subject field value.
    pub subject: String,
    /// HTML body field value.
    pub body_html: String,
    /// Text body field value.
    pub body_text: String,
    /// Schedule field value (`datetime-local`).
    pub scheduled_at: String,
    /// Name field error.
    pub name_error: Option<String>,
    /// Subject field error.
    pub subject_error: Option<String>,
    /// HTML body field error.
    pub body_html_error: Option<String>,
    /// Text body field error.
    pub body_text_error: Option<String>,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`); empty on
    /// the create form, which has no record to guard.
    pub form_ticket: String,
    /// Schedule field error.
    pub scheduled_at_error: Option<String>,
    /// Form-level error (e.g. a refused update).
    pub error: Option<String>,
}

/// One audience choice in the campaign form.
pub struct SegmentOption {
    /// Segment id as the submitted value; "" = all
    /// subscribers.
    pub value: String,
    /// Display label.
    pub name: String,
    /// Whether this is the current choice.
    pub selected: bool,
}

/// The campaign confirm page: the exact recipient count
/// and the enqueue choice (now or at the schedule).
#[derive(Template)]
#[template(path = "admin/pages/mail/campaign_confirm.html")]
pub struct CampaignConfirmView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Campaign ID.
    pub campaign_id: i64,
    /// Campaign name.
    pub name: String,
    /// Subject line.
    pub subject: String,
    /// Status pill label.
    pub status_label: &'static str,
    /// Whether the campaign is still a draft (enqueueable).
    pub editable: bool,
    /// The exact eligible-recipient count right now.
    pub recipient_count: i64,
    /// The audience in words: all subscribers or the
    /// segment's name.
    pub audience_label: String,
    /// Schedule label, when one is set.
    pub scheduled_label: Option<String>,
    /// Whether a schedule is set (enables the 予約する choice).
    pub has_schedule: bool,
    /// Delivery tallies so far (after an enqueue).
    pub stats: CampaignStats,
    /// The opens/clicks line (C2 tracking), present once
    /// anything has been sent.
    pub tracking_label: Option<String>,
    /// Form-level error (e.g. a refused enqueue).
    pub error: Option<String>,
}

/// One notification (transactional) mail row on the
/// template list: its catalog name, whether an override
/// is stored, and when that override was saved.
pub struct NotificationRow {
    /// The mail's catalog key (the edit page's path part).
    pub key: &'static str,
    /// Japanese display name.
    pub name: &'static str,
    /// Whether the operator has customised this mail.
    pub customized: bool,
    /// Override save label (`YYYY/MM/DD HH:mm`), when set.
    pub updated_label: Option<String>,
}

/// One tag in the editor's dictionary.
pub struct MailTagView {
    /// Tag name as written in the template, braces included
    /// in the editor's chip.
    pub name: &'static str,
    /// One-line description of what the tag inserts.
    pub label: &'static str,
    /// Whether the tag expands to a generated block.
    pub block: bool,
}

/// One group of the editor's tag dictionary.
pub struct MailTagGroupView {
    /// Group heading.
    pub label: &'static str,
    /// The group's tags.
    pub tags: Vec<MailTagView>,
}

/// A rendered preview of the editor's current content,
/// produced with sample data.
pub struct MailPreviewView {
    /// The rendered subject.
    pub subject: String,
    /// The rendered body.
    pub body: String,
}

/// The notification mail edit page.
#[derive(Template)]
#[template(path = "admin/pages/mail/notification_form.html")]
pub struct NotificationFormView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The mail's catalog key.
    pub key: &'static str,
    /// The mail's Japanese name.
    pub name: &'static str,
    /// Whether a stored override exists for this mail.
    pub customized: bool,
    /// Override save label, when customised.
    pub updated_label: Option<String>,
    /// Subject field value.
    pub subject: String,
    /// Body field value.
    pub body: String,
    /// Subject field error.
    pub subject_error: Option<String>,
    /// Body field error.
    pub body_error: Option<String>,
    /// The tag dictionary this mail accepts.
    pub groups: Vec<MailTagGroupView>,
    /// The one-time form ticket minted for this render of
    /// the edit form (see `form_ticket_service`); empty on
    /// the create form, which has no record to guard.
    pub form_ticket: String,
    /// The rendered preview, when the operator asked for one.
    pub preview: Option<MailPreviewView>,
    /// A form-level notice (a failed test send).
    pub error: Option<String>,
}

/// Formats the campaign's tracking tallies for the
/// confirm/report page: unique opens and clicks with
/// their rates against the sent count, totals alongside.
/// Returns `None` until at least one mail has been sent
/// (rates would be meaningless before that).
pub fn tracking_label(stats: &CampaignStats) -> Option<String> {
    if stats.sent <= 0 {
        return None;
    }
    let open_rate = stats.opens_unique as f64 * 100.0 / stats.sent as f64;
    let click_rate = stats.clicks_unique as f64 * 100.0 / stats.sent as f64;
    Some(format!(
        "開封 {} 名（開封率 {:.1}% / 全 {} 回）・クリック {} 名（クリック率 {:.1}% / 全 {} 回）",
        stats.opens_unique,
        open_rate,
        stats.opens_total,
        stats.clicks_unique,
        click_rate,
        stats.clicks_total,
    ))
}

/// One scenario row on the step-mail list (EC V2 #2),
/// display-ready.
pub struct ScenarioRow {
    /// Scenario ID (the toggle posts here).
    pub id: i64,
    /// Operator-facing name.
    pub name: String,
    /// What the sequence is for.
    pub description: String,
    /// The trigger's Japanese label.
    pub trigger_label: String,
    /// The chain at a glance: step count and the
    /// cumulative send days (e.g. `3通（登録当日・3日後・7日後）`).
    pub steps_label: String,
    /// The admin toggle's current state.
    pub enabled: bool,
    /// Steps successfully sent, all time.
    pub sent: i64,
    /// Steps skipped for lack of consent, all time.
    pub skipped: i64,
    /// Steps that exhausted their attempts, all time.
    pub failed: i64,
}

/// Renders pages/mail/scenarios.html: the step-mail list
/// with per-scenario send totals and the enable toggle.
#[derive(Template)]
#[template(path = "admin/pages/mail/scenarios.html")]
pub struct ScenarioListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The scenarios, in seed order.
    pub rows: Vec<ScenarioRow>,
}
