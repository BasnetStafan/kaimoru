//! Mail module management.
//!
//! The メール配信 section: a dashboard, reusable templates,
//! and campaigns built through the guided flow — audience
//! (the live subscriber count), content (optionally from a
//! template), a test send to the operator, then the
//! schedule/send choice on a confirm screen that shows the
//! exact recipient count. Requests never send campaign
//! mail: enqueueing snapshots the recipients and the
//! background worker delivers.

use axum::{
    Form, Router,
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, pagination, record_not_found, render, set_flash};
use crate::config::CONFIG;
use crate::models::AdminIdentity;
use crate::models::notification_template::{NOTIFICATION_MAILS, NotificationMailDef, find_def};
use crate::repositories::notification_template_repository;
use crate::services::admin::mail_service::{
    self as admin_mail_service, CampaignFormInput, EnqueueTiming, TemplateFormInput,
};
use crate::services::admin::notification_template_service;
use crate::services::marketing_mail_service;
use crate::services::notification_template_service as notification_templates;
use crate::services::shop_settings_service;
use crate::session::CurrentAdmin;
use crate::validation::{FieldError, field_error};
use crate::views::admin::Page;
use crate::views::admin::mail::{
    CampaignConfirmView, CampaignFormView, CampaignListView, MailDashboardView, MailPreviewView,
    MailTagGroupView, MailTagView, NotificationFormView, NotificationRow, ScenarioListView,
    ScenarioRow, TRANSACTIONAL_MAILS, TemplateFormView, TemplateListView, TemplateOption,
    campaign_row, template_row,
};

/// Routes for the mail surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/mail", get(dashboard))
        .route("/admin/mail/templates", get(template_list))
        .route(
            "/admin/mail/templates/new",
            get(template_new).post(template_create),
        )
        .route(
            "/admin/mail/templates/{id}/edit",
            get(template_edit).post(template_update),
        )
        .route("/admin/mail/templates/{id}/delete", post(template_delete))
        .route(
            "/admin/mail/notifications/{key}/edit",
            get(notification_edit).post(notification_update),
        )
        .route(
            "/admin/mail/notifications/{key}/test-send",
            post(notification_test_send),
        )
        .route(
            "/admin/mail/notifications/{key}/reset",
            post(notification_reset),
        )
        .route("/admin/mail/campaigns", get(campaign_list))
        .route(
            "/admin/mail/campaigns/new",
            get(campaign_new).post(campaign_create),
        )
        .route(
            "/admin/mail/campaigns/{id}/edit",
            get(campaign_edit).post(campaign_update),
        )
        .route(
            "/admin/mail/campaigns/{id}/test-send",
            post(campaign_test_send),
        )
        .route("/admin/mail/scenarios", get(scenario_list))
        .route("/admin/mail/scenarios/{id}/toggle", post(scenario_toggle))
        .route("/admin/mail/campaigns/{id}/confirm", get(campaign_confirm))
        .route("/admin/mail/campaigns/{id}/enqueue", post(campaign_enqueue))
}

/// GET /admin/mail: counts, recent and scheduled campaigns,
/// and the transactional inventory.
pub async fn dashboard(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    let data = match admin_mail_service::dashboard(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("mail dashboard", e),
    };
    let mut recent = Vec::new();
    for campaign in &data.recent {
        match admin_mail_service::campaign_stats(&pool, campaign.id).await {
            Ok(stats) => recent.push(campaign_row(campaign, stats)),
            Err(e) => return db_error("mail dashboard", e),
        }
    }
    let mut scheduled = Vec::new();
    for campaign in &data.scheduled {
        match admin_mail_service::campaign_stats(&pool, campaign.id).await {
            Ok(stats) => scheduled.push(campaign_row(campaign, stats)),
            Err(e) => return db_error("mail dashboard", e),
        }
    }
    render(MailDashboardView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "mail",
        subscribers: data.subscribers,
        sent_this_month: data.sent_this_month,
        failed: data.failed,
        recent,
        scheduled,
        transactional: TRANSACTIONAL_MAILS.to_vec(),
    })
}

// ----------------------------------------------------------------
// Templates
// ----------------------------------------------------------------

/// GET /admin/mail/templates: the template list.
pub async fn template_list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    let templates = match admin_mail_service::list_templates(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("mail templates", e),
    };
    let notifications = notification_rows(&pool).await;
    render(TemplateListView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "mail_templates",
        rows: templates.iter().map(template_row).collect(),
        notifications,
    })
}

/// The template form body.
#[derive(Deserialize)]
pub struct TemplateForm {
    /// Name as submitted.
    name: String,
    /// Subject as submitted.
    subject: String,
    /// HTML body as submitted.
    body_html: String,
    /// Text body as submitted.
    body_text: String,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the form, which the
    /// edit submit treats as stale. The create submit
    /// ignores it.
    #[serde(default)]
    form_ticket: String,
}

impl From<&TemplateForm> for TemplateFormInput {
    fn from(f: &TemplateForm) -> Self {
        TemplateFormInput {
            name: f.name.clone(),
            subject: f.subject.clone(),
            body_html: f.body_html.clone(),
            body_text: f.body_text.clone(),
        }
    }
}

/// GET /admin/mail/templates/new: a blank template form.
pub async fn template_new(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    template_form_response(
        &pool,
        &admin,
        &session,
        &TemplateFormInput::default(),
        None,
        None,
        String::new(),
    )
    .await
}

/// POST /admin/mail/templates/new: creates one template.
pub async fn template_create(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<TemplateForm>,
) -> Response {
    let input = TemplateFormInput::from(&form);
    match admin_mail_service::validate_template(&input) {
        Ok(valid) => match admin_mail_service::create_template(&pool, admin.id, &valid).await {
            Ok(id) => {
                set_flash(&session, "メールテンプレートを登録しました").await;
                Redirect::to(&format!("/admin/mail/templates/{id}/edit")).into_response()
            }
            Err(e) => db_error("mail templates", e),
        },
        Err(errors) => {
            template_form_response(
                &pool,
                &admin,
                &session,
                &input,
                None,
                Some(&errors),
                String::new(),
            )
            .await
        }
    }
}

/// GET /admin/mail/templates/{id}/edit: the edit form.
pub async fn template_edit(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match admin_mail_service::find_template(&pool, id).await {
        Ok(Some(template)) => {
            let input = TemplateFormInput {
                name: template.name.clone(),
                subject: template.subject.clone(),
                body_html: template.body_html.clone(),
                body_text: template.body_text.clone(),
            };
            let form_ticket = crate::services::form_ticket_service::issue(
                &session,
                &mail_template_scope(id),
                template.updated_at.timestamp_micros(),
            )
            .await;
            template_form_response(&pool, &admin, &session, &input, Some(id), None, form_ticket)
                .await
        }
        Ok(None) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("mail templates", e),
    }
}

/// POST /admin/mail/templates/{id}/edit: updates one template.
pub async fn template_update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<TemplateForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let template = match admin_mail_service::find_template(&pool, id).await {
        Ok(Some(template)) => template,
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("mail templates", e),
    };
    // Spend the form ticket before validating: a submit
    // without a valid ticket was not rendered from the
    // current record, so it is refused as stale. A ticket
    // whose stamp has gone stale is refused by the
    // service's check instead.
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &mail_template_scope(id),
        &form.form_ticket,
    )
    .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/mail/templates/{id}/edit")).into_response();
    };
    let input = TemplateFormInput::from(&form);
    match admin_mail_service::validate_template(&input) {
        Ok(valid) => {
            match admin_mail_service::update_template(&pool, admin.id, id, &valid, expected).await {
                Ok(()) => {
                    set_flash(&session, "メールテンプレートを更新しました").await;
                    Redirect::to(&format!("/admin/mail/templates/{id}/edit")).into_response()
                }
                Err(crate::error::AppError::Conflict(message)) => {
                    set_flash(&session, &message).await;
                    Redirect::to(&format!("/admin/mail/templates/{id}/edit")).into_response()
                }
                Err(e) => db_error("mail templates", e),
            }
        }
        Err(errors) => {
            let form_ticket = crate::services::form_ticket_service::issue(
                &session,
                &mail_template_scope(id),
                template.updated_at.timestamp_micros(),
            )
            .await;
            template_form_response(
                &pool,
                &admin,
                &session,
                &input,
                Some(id),
                Some(&errors),
                form_ticket,
            )
            .await
        }
    }
}

/// POST /admin/mail/templates/{id}/delete: deletes one
/// template. The list's delete form sits behind the
/// browser's native confirmation (admin.js).
pub async fn template_delete(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match admin_mail_service::delete_template(&pool, admin.id, id).await {
        Ok(()) => {
            set_flash(&session, "メールテンプレートを削除しました").await;
            Redirect::to("/admin/mail/templates").into_response()
        }
        Err(e) => db_error("mail templates", e),
    }
}

/// Renders the shared template form.
/// The form-ticket scope of one mail template.
fn mail_template_scope(id: i64) -> String {
    format!("mail_template:{id}")
}

async fn template_form_response(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    input: &TemplateFormInput,
    id: Option<i64>,
    errors: Option<&Vec<FieldError>>,
    form_ticket: String,
) -> Response {
    let err = |field: &str| {
        errors.and_then(|list| {
            list.iter()
                .find(|e| e.field == field)
                .map(|e| e.message.clone())
        })
    };
    render(TemplateFormView {
        layout: layout_for(pool, admin, session).await,
        nav: "mail_templates",
        heading: if id.is_some() {
            "メールテンプレートを編集".to_string()
        } else {
            "メールテンプレートを登録".to_string()
        },
        action: match id {
            Some(id) => format!("/admin/mail/templates/{id}/edit"),
            None => "/admin/mail/templates/new".to_string(),
        },
        delete_action: id.map(|id| format!("/admin/mail/templates/{id}/delete")),
        name: input.name.clone(),
        subject: input.subject.clone(),
        body_html: input.body_html.clone(),
        body_text: input.body_text.clone(),
        name_error: err("name"),
        subject_error: err("subject"),
        body_html_error: err("body_html"),
        body_text_error: err("body_text"),
        form_ticket,
    })
}

// ----------------------------------------------------------------
// Campaigns
// ----------------------------------------------------------------

/// List query parameters.
#[derive(Deserialize, Default)]
pub struct CampaignListQuery {
    /// Page number.
    page: Option<String>,
}

/// GET /admin/mail/campaigns: the campaign list.
pub async fn campaign_list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<CampaignListQuery>,
) -> Response {
    let (page_no, limit, offset) = pagination(query.page.as_deref(), CONFIG.admin_per_page);
    let (campaigns, total) = match admin_mail_service::list_campaigns(&pool, limit, offset).await {
        Ok(v) => v,
        Err(e) => return db_error("mail campaigns", e),
    };
    // One grouped stats pass for the whole page;
    // campaigns without deliveries default to zeroes.
    let ids: Vec<i64> = campaigns.iter().map(|c| c.id).collect();
    let stats_by_id = match admin_mail_service::campaign_stats_bulk(&pool, &ids).await {
        Ok(v) => v,
        Err(e) => return db_error("mail campaigns", e),
    };
    let rows: Vec<_> = campaigns
        .iter()
        .map(|campaign| {
            let stats = stats_by_id.get(&campaign.id).cloned().unwrap_or_default();
            campaign_row(campaign, stats)
        })
        .collect();
    render(CampaignListView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "mail_campaigns",
        rows,
        page: Page::new(
            page_no,
            total,
            limit,
            Page::base_with("/admin/mail/campaigns", ""),
        ),
    })
}

/// The campaign form body. `apply_template` is the picker
/// button: its presence means "load the chosen template
/// into the form" instead of saving.
#[derive(Deserialize)]
pub struct CampaignForm {
    /// Name as submitted.
    name: String,
    /// Chosen template ID as submitted (empty = none).
    #[serde(default)]
    template_id: String,
    /// Subject as submitted.
    subject: String,
    /// HTML body as submitted.
    body_html: String,
    /// Text body as submitted.
    body_text: String,
    /// Schedule as submitted (`datetime-local`).
    #[serde(default)]
    scheduled_at: String,
    /// Audience segment as submitted (empty = all
    /// subscribers), C2.
    #[serde(default)]
    segment_id: String,
    /// The テンプレートを反映 button's value, when pressed.
    #[serde(default)]
    apply_template: Option<String>,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); empty when a
    /// client posts without rendering the form, which the
    /// edit submit treats as stale. The create submit
    /// ignores it.
    #[serde(default)]
    form_ticket: String,
}

impl From<&CampaignForm> for CampaignFormInput {
    fn from(f: &CampaignForm) -> Self {
        CampaignFormInput {
            name: f.name.clone(),
            template_id: f.template_id.parse::<i64>().ok(),
            subject: f.subject.clone(),
            body_html: f.body_html.clone(),
            body_text: f.body_text.clone(),
            scheduled_at: f.scheduled_at.clone(),
            segment_id: f.segment_id.parse::<i64>().ok(),
        }
    }
}

/// GET /admin/mail/campaigns/new: a blank campaign form.
pub async fn campaign_new(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    campaign_form_response(
        &pool,
        &admin,
        &session,
        &CampaignFormInput::default(),
        None,
        true,
        None,
        None,
        None,
        String::new(),
    )
    .await
}

/// POST /admin/mail/campaigns/new: creates one draft
/// campaign and moves to its edit screen, where the test
/// send and the confirm step live.
pub async fn campaign_create(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<CampaignForm>,
) -> Response {
    if form.apply_template.is_some() {
        return apply_template_response(&pool, &admin, &session, &form, None).await;
    }
    let input = CampaignFormInput::from(&form);
    match admin_mail_service::validate_campaign(&pool, &input).await {
        Ok(valid) => match admin_mail_service::create_campaign(&pool, admin.id, &valid).await {
            Ok(id) => {
                set_flash(&session, "キャンペーンを下書きで登録しました").await;
                Redirect::to(&format!("/admin/mail/campaigns/{id}/edit")).into_response()
            }
            Err(e) => db_error("mail campaigns", e),
        },
        Err(errors) => {
            campaign_form_response(
                &pool,
                &admin,
                &session,
                &input,
                None,
                true,
                None,
                Some(&errors),
                None,
                String::new(),
            )
            .await
        }
    }
}

/// GET /admin/mail/campaigns/{id}/edit: the edit form, or
/// the read-only state of a campaign that left draft.
pub async fn campaign_edit(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match admin_mail_service::find_campaign(&pool, id).await {
        Ok(Some(campaign)) => {
            let input = CampaignFormInput {
                name: campaign.name.clone(),
                segment_id: campaign.segment_id,
                template_id: campaign.template_id,
                subject: campaign.subject.clone(),
                body_html: campaign.body_html.clone(),
                body_text: campaign.body_text.clone(),
                scheduled_at: campaign
                    .scheduled_at
                    .map(|at| {
                        at.with_timezone(&crate::models::jst::jst())
                            .format("%Y-%m-%dT%H:%M")
                            .to_string()
                    })
                    .unwrap_or_default(),
            };
            campaign_form_response(
                &pool,
                &admin,
                &session,
                &input,
                Some(id),
                campaign.is_draft(),
                Some(campaign.status_label()),
                None,
                None,
                crate::services::form_ticket_service::issue(
                    &session,
                    &mail_campaign_scope(id),
                    campaign.updated_at.timestamp_micros(),
                )
                .await,
            )
            .await
        }
        Ok(None) => record_not_found(&pool, &admin, &session).await,
        Err(e) => db_error("mail campaigns", e),
    }
}

/// POST /admin/mail/campaigns/{id}/edit: updates one draft.
pub async fn campaign_update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<CampaignForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    if form.apply_template.is_some() {
        return apply_template_response(&pool, &admin, &session, &form, Some(id)).await;
    }
    // Spend the form ticket before validating: a submit
    // without a valid ticket was not rendered from the
    // current record, so it is refused as stale. A ticket
    // whose stamp has gone stale is refused by the
    // service's check instead and lands on the same
    // re-render as the not-a-draft conflict.
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &mail_campaign_scope(id),
        &form.form_ticket,
    )
    .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/mail/campaigns/{id}/edit")).into_response();
    };
    let input = CampaignFormInput::from(&form);
    match admin_mail_service::validate_campaign(&pool, &input).await {
        Ok(valid) => {
            match admin_mail_service::update_campaign(&pool, admin.id, id, &valid, expected).await {
                Ok(()) => {
                    set_flash(&session, "キャンペーンを更新しました").await;
                    Redirect::to(&format!("/admin/mail/campaigns/{id}/edit")).into_response()
                }
                Err(crate::error::AppError::NotFound) => {
                    record_not_found(&pool, &admin, &session).await
                }
                Err(crate::error::AppError::Conflict(message)) => {
                    // The refusal means the stored campaign has
                    // left draft state: re-render with its real
                    // state, not the form's assumption.
                    match admin_mail_service::find_campaign(&pool, id).await {
                        Ok(Some(current)) => {
                            campaign_form_response(
                                &pool,
                                &admin,
                                &session,
                                &input,
                                Some(id),
                                current.is_draft(),
                                Some(current.status_label()),
                                None,
                                Some(message),
                                campaign_ticket(&pool, &session, id).await,
                            )
                            .await
                        }
                        Ok(None) => record_not_found(&pool, &admin, &session).await,
                        Err(e) => db_error("mail campaigns", e),
                    }
                }
                Err(e) => db_error("mail campaigns", e),
            }
        }
        Err(errors) => {
            campaign_form_response(
                &pool,
                &admin,
                &session,
                &input,
                Some(id),
                true,
                None,
                Some(&errors),
                None,
                campaign_ticket(&pool, &session, id).await,
            )
            .await
        }
    }
}

/// POST /admin/mail/campaigns/{id}/test-send: sends the
/// saved campaign content to the operating admin's own
/// address, immediately and outside the job queue.
pub async fn campaign_test_send(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let campaign = match admin_mail_service::find_campaign(&pool, id).await {
        Ok(Some(campaign)) => campaign,
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("mail campaigns", e),
    };
    match marketing_mail_service::send_test(&pool, &campaign, &admin.display_name, &admin.email)
        .await
    {
        Ok(()) => {
            set_flash(
                &session,
                "テストメールを送信しました（管理者のメールアドレス宛て）",
            )
            .await;
        }
        Err(message) => {
            set_flash(&session, &message).await;
        }
    }
    Redirect::to(&format!("/admin/mail/campaigns/{id}/edit")).into_response()
}

/// GET /admin/mail/campaigns/{id}/confirm: the final step —
/// the exact recipient count and the send choice.
pub async fn campaign_confirm(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let campaign = match admin_mail_service::find_campaign(&pool, id).await {
        Ok(Some(campaign)) => campaign,
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("mail campaigns", e),
    };
    confirm_response(&pool, &admin, &session, &campaign, None).await
}

/// The enqueue form body.
#[derive(Deserialize)]
pub struct EnqueueForm {
    /// `now` or `scheduled`.
    timing: String,
}

/// POST /admin/mail/campaigns/{id}/enqueue: snapshots the
/// recipients into a job. The worker sends; this request
/// only queues.
pub async fn campaign_enqueue(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
    Form(form): Form<EnqueueForm>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match admin_mail_service::find_campaign(&pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return record_not_found(&pool, &admin, &session).await,
        Err(e) => return db_error("mail campaigns", e),
    };
    let timing = if form.timing == "scheduled" {
        EnqueueTiming::Scheduled
    } else {
        EnqueueTiming::Now
    };
    match admin_mail_service::enqueue_campaign(&pool, admin.id, id, timing).await {
        Ok(count) => {
            set_flash(
                &session,
                &format!("{count}名への配信を予約しました。配信はバックグラウンドで実行されます"),
            )
            .await;
            Redirect::to("/admin/mail/campaigns").into_response()
        }
        Err(crate::error::AppError::NotFound) => record_not_found(&pool, &admin, &session).await,
        Err(crate::error::AppError::Conflict(message)) => {
            confirm_error_response(&pool, &admin, &session, id, message).await
        }
        Err(crate::error::AppError::Validation(errors)) => {
            // The failing field carries its own message (a
            // missing schedule, or one in the past now).
            let message = errors
                .first()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| {
                    "配信日時が設定されていません。編集画面で設定してください".to_string()
                });
            confirm_error_response(&pool, &admin, &session, id, message).await
        }
        Err(e) => db_error("mail campaigns", e),
    }
}

/// Re-renders the confirm page after a refused enqueue,
/// from a fresh load: the page shows the state that
/// actually refused (a campaign enqueued by a racing
/// request reads as scheduled, not as the draft this
/// request started from).
async fn confirm_error_response(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    id: i64,
    message: String,
) -> Response {
    match admin_mail_service::find_campaign(pool, id).await {
        Ok(Some(current)) => confirm_response(pool, admin, session, &current, Some(message)).await,
        Ok(None) => record_not_found(pool, admin, session).await,
        Err(e) => db_error("mail campaigns", e),
    }
}

/// Renders the confirm page for one campaign.
async fn confirm_response(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    campaign: &crate::models::mail::EmailCampaign,
    error: Option<String>,
) -> Response {
    let recipient_count =
        match admin_mail_service::eligible_recipient_count_for(pool, campaign.segment_id).await {
            Ok(v) => v,
            Err(e) => return db_error("mail campaigns", e),
        };
    let audience_label = match campaign.segment_id {
        Some(segment_id) => {
            match crate::repositories::admin::segment_repository::find_segment(pool, segment_id)
                .await
            {
                Ok(Some(segment)) => format!("セグメント「{}」の購読者", segment.name),
                _ => "セグメント（削除済み）".to_string(),
            }
        }
        None => "メールマガジン購読者全員".to_string(),
    };
    let stats = match admin_mail_service::campaign_stats(pool, campaign.id).await {
        Ok(v) => v,
        Err(e) => return db_error("mail campaigns", e),
    };
    render(CampaignConfirmView {
        layout: layout_for(pool, admin, session).await,
        nav: "mail_campaigns",
        campaign_id: campaign.id,
        name: campaign.name.clone(),
        subject: campaign.subject.clone(),
        status_label: campaign.status_label(),
        editable: campaign.is_draft(),
        recipient_count,
        audience_label,
        scheduled_label: campaign
            .scheduled_at
            .map(crate::models::jst::jst_datetime_label),
        has_schedule: campaign.scheduled_at.is_some(),
        tracking_label: crate::views::admin::mail::tracking_label(&stats),
        stats,
        error,
    })
}

/// Loads the chosen template into the campaign form
/// (name and schedule kept from the form as submitted).
async fn apply_template_response(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    form: &CampaignForm,
    id: Option<i64>,
) -> Response {
    let mut input = CampaignFormInput::from(form);
    if let Some(template_id) = input.template_id {
        match admin_mail_service::find_template(pool, template_id).await {
            Ok(Some(template)) => {
                input.subject = template.subject.clone();
                input.body_html = template.body_html.clone();
                input.body_text = template.body_text;
            }
            Ok(None) => {}
            Err(e) => return db_error("mail campaigns", e),
        }
    }
    let form_ticket = match id {
        Some(id) => campaign_ticket(pool, session, id).await,
        None => String::new(),
    };
    campaign_form_response(
        pool,
        admin,
        session,
        &input,
        id,
        true,
        None,
        None,
        None,
        form_ticket,
    )
    .await
}

/// Renders the shared campaign form.
#[allow(clippy::too_many_arguments)]
/// The form-ticket scope of one mail campaign.
fn mail_campaign_scope(id: i64) -> String {
    format!("mail_campaign:{id}")
}

/// Mints an edit ticket for the campaign at its current
/// stamp (read back through the service).
async fn campaign_ticket(pool: &PgPool, session: &Session, id: i64) -> String {
    // A failed stamp read is logged and reads as 0, a value
    // no real stamp equals, so the ticket fails closed (the
    // rule `profile_stamp` documents on the member side).
    let stamp = match admin_mail_service::find_campaign(pool, id).await {
        Ok(stamp) => stamp.map(|c| c.updated_at.timestamp_micros()).unwrap_or(0),
        Err(e) => {
            tracing::error!("mail campaign stamp read failed: {e}");
            0
        }
    };
    crate::services::form_ticket_service::issue(session, &mail_campaign_scope(id), stamp).await
}

#[allow(clippy::too_many_arguments)]
async fn campaign_form_response(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    input: &CampaignFormInput,
    id: Option<i64>,
    editable: bool,
    status_label: Option<&'static str>,
    errors: Option<&Vec<FieldError>>,
    error: Option<String>,
    form_ticket: String,
) -> Response {
    let err = |field: &str| {
        errors.and_then(|list| {
            list.iter()
                .find(|e| e.field == field)
                .map(|e| e.message.clone())
        })
    };
    let recipient_count =
        match admin_mail_service::eligible_recipient_count_for(pool, input.segment_id).await {
            Ok(v) => v,
            Err(e) => return db_error("mail campaigns", e),
        };
    let segments = match crate::repositories::admin::segment_repository::list_segments(pool).await {
        Ok(v) => v,
        Err(e) => return db_error("mail campaigns", e),
    };
    let mut segment_options = vec![crate::views::admin::mail::SegmentOption {
        value: String::new(),
        name: "すべての購読者".to_string(),
        selected: input.segment_id.is_none(),
    }];
    for segment in &segments {
        segment_options.push(crate::views::admin::mail::SegmentOption {
            value: segment.id.to_string(),
            name: segment.name.clone(),
            selected: input.segment_id == Some(segment.id),
        });
    }
    let templates = match admin_mail_service::list_templates(pool).await {
        Ok(v) => v,
        Err(e) => return db_error("mail campaigns", e),
    };
    let mut template_options = vec![TemplateOption {
        value: String::new(),
        label: "テンプレートを使わない".to_string(),
        selected: input.template_id.is_none(),
    }];
    for template in &templates {
        template_options.push(TemplateOption {
            value: template.id.to_string(),
            label: template.name.clone(),
            selected: input.template_id == Some(template.id),
        });
    }
    render(CampaignFormView {
        layout: layout_for(pool, admin, session).await,
        nav: "mail_campaigns",
        heading: if id.is_some() {
            "メールキャンペーンを編集".to_string()
        } else {
            "メールキャンペーンを登録".to_string()
        },
        action: match id {
            Some(id) => format!("/admin/mail/campaigns/{id}/edit"),
            None => "/admin/mail/campaigns/new".to_string(),
        },
        campaign_id: id,
        editable,
        status_label,
        recipient_count,
        segment_options,
        segment_id: input
            .segment_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        segment_error: err("segment_id"),
        template_options,
        name: input.name.clone(),
        subject: input.subject.clone(),
        body_html: input.body_html.clone(),
        body_text: input.body_text.clone(),
        scheduled_at: input.scheduled_at.clone(),
        name_error: err("name"),
        subject_error: err("subject"),
        body_html_error: err("body_html"),
        body_text_error: err("body_text"),
        scheduled_at_error: err("scheduled_at"),
        error,
        form_ticket,
    })
}

// ----------------------------------------------------------------
// Notification templates
// ----------------------------------------------------------------

/// Builds the template list's notification rows: every
/// catalog mail with its override state.
async fn notification_rows(pool: &PgPool) -> Vec<NotificationRow> {
    let overrides = notification_template_repository::list_overrides(pool)
        .await
        .unwrap_or_default();
    NOTIFICATION_MAILS
        .iter()
        .map(|def| {
            let row = overrides.iter().find(|r| r.mail_key == def.key);
            NotificationRow {
                key: def.key,
                name: def.name,
                customized: row.is_some(),
                updated_label: row.map(|r| crate::models::jst::jst_datetime_label(r.updated_at)),
            }
        })
        .collect()
}

/// The notification edit form body.
#[derive(Deserialize)]
pub struct NotificationForm {
    /// Subject as submitted.
    subject: String,
    /// Body as submitted.
    body: String,
    /// Which action the operator chose: `save` or `preview`.
    intent: String,
    /// The one-time form ticket minted when this form was
    /// rendered (see `form_ticket_service`); spent only by
    /// the save — a preview saves nothing and leaves the
    /// ticket valid. Empty when a client posts without
    /// rendering the form, which the save treats as stale.
    #[serde(default)]
    form_ticket: String,
}

/// The notification reset form body: only the form ticket
/// is read from it (the CSRF token is middleware-checked).
/// The reset spends the same ticket the edit form carries,
/// so a reset submitted from a stale page cannot delete an
/// override saved after that page was rendered.
#[derive(Deserialize)]
pub struct NotificationResetForm {
    /// The one-time form ticket minted when the edit form
    /// was rendered (see `form_ticket_service`).
    #[serde(default)]
    form_ticket: String,
}

/// The form-ticket scope of one notification mail.
fn notification_scope(key: &str) -> String {
    format!("notification:{key}")
}

/// GET /admin/mail/notifications/{key}/edit: one
/// notification mail's edit form, prefilled with the
/// effective template (the override, or the default).
pub async fn notification_edit(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(key): Path<String>,
) -> Response {
    let Some(def) = find_def(&key) else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let template = notification_templates::effective_template(&pool, def).await;
    notification_form_response(
        &pool,
        &admin,
        &session,
        def,
        &template.subject,
        &template.body,
        None,
        None,
        None,
    )
    .await
}

/// POST /admin/mail/notifications/{key}/edit: saves the
/// override, or renders a preview of the submitted
/// content with sample data (nothing is saved then).
pub async fn notification_update(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(key): Path<String>,
    Form(form): Form<NotificationForm>,
) -> Response {
    let Some(def) = find_def(&key) else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let (subject, body) =
        match notification_templates::validate_template(def, &form.subject, &form.body) {
            Ok(valid) => valid,
            Err(errors) => {
                return notification_form_response(
                    &pool,
                    &admin,
                    &session,
                    def,
                    &form.subject,
                    &form.body,
                    Some(&errors),
                    None,
                    None,
                )
                .await;
            }
        };
    if form.intent == "preview" {
        let settings = shop_settings_service::load(&pool).await;
        let shop_url = std::env::var("PUBLIC_BASE_URL").unwrap_or_default();
        let ctx = notification_templates::sample_context(def, &settings, &shop_url, &admin.email);
        let preview = MailPreviewView {
            subject: notification_templates::render_inline(&subject, &ctx),
            body: notification_templates::render_text(&body, &ctx),
        };
        return notification_form_response(
            &pool,
            &admin,
            &session,
            def,
            &subject,
            &body,
            None,
            Some(preview),
            None,
        )
        .await;
    }
    // Spend the form ticket at the save (the preview above
    // saved nothing and left it valid): a submit without a
    // valid ticket was not rendered from the current
    // override, so it is refused as stale. A ticket whose
    // stamp has gone stale is refused by the service's
    // check instead.
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &notification_scope(def.key),
        &form.form_ticket,
    )
    .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/mail/notifications/{}/edit", def.key))
            .into_response();
    };
    match notification_template_service::save_override(
        &pool, admin.id, def, &subject, &body, expected,
    )
    .await
    {
        Ok(()) => {
            set_flash(&session, "通知メールを保存しました").await;
            Redirect::to(&format!("/admin/mail/notifications/{}/edit", def.key)).into_response()
        }
        Err(crate::error::AppError::Conflict(message)) => {
            set_flash(&session, &message).await;
            Redirect::to(&format!("/admin/mail/notifications/{}/edit", def.key)).into_response()
        }
        Err(e) => db_error("notification templates", e),
    }
}

/// POST /admin/mail/notifications/{key}/test-send: sends
/// the SAVED template, rendered with sample data, to the
/// logged-in admin's own address.
pub async fn notification_test_send(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(key): Path<String>,
) -> Response {
    let Some(def) = find_def(&key) else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let template = notification_templates::effective_template(&pool, def).await;
    let settings = shop_settings_service::load(&pool).await;
    let shop_url = std::env::var("PUBLIC_BASE_URL").unwrap_or_default();
    let ctx = notification_templates::sample_context(def, &settings, &shop_url, &admin.email);
    let subject = format!(
        "【テスト送信】{}",
        notification_templates::render_inline(&template.subject, &ctx)
    );
    let body = notification_templates::render_text(&template.body, &ctx);
    // The mail error type is not Send, so the outcome is
    // reduced to a flag before any further await.
    let sent = crate::mail::send_rendered_plain(&admin.email, &subject, body)
        .await
        .is_ok();
    match sent {
        true => {
            set_flash(&session, "テストメールを送信しました").await;
            Redirect::to(&format!("/admin/mail/notifications/{}/edit", def.key)).into_response()
        }
        false => {
            notification_form_response(
                &pool,
                &admin,
                &session,
                def,
                &template.subject,
                &template.body,
                None,
                None,
                Some("テストメールを送信できませんでした。時間をおいてもう一度お試しください。"),
            )
            .await
        }
    }
}

/// POST /admin/mail/notifications/{key}/reset: deletes
/// the override; the mail renders from its code default
/// again （初期値に戻す).
pub async fn notification_reset(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(key): Path<String>,
    Form(form): Form<NotificationResetForm>,
) -> Response {
    let Some(def) = find_def(&key) else {
        return record_not_found(&pool, &admin, &session).await;
    };
    // The reset spends the edit form's ticket: a reset
    // submitted from a stale page is refused, so it cannot
    // delete an override saved after that page rendered.
    let Some(expected) = crate::services::form_ticket_service::consume(
        &session,
        &notification_scope(def.key),
        &form.form_ticket,
    )
    .await
    else {
        set_flash(&session, crate::messages::STALE_FORM_MESSAGE).await;
        return Redirect::to(&format!("/admin/mail/notifications/{}/edit", def.key))
            .into_response();
    };
    match notification_template_service::reset_override(&pool, admin.id, def, expected).await {
        Ok(()) => {
            set_flash(&session, "初期値に戻しました").await;
            Redirect::to(&format!("/admin/mail/notifications/{}/edit", def.key)).into_response()
        }
        Err(crate::error::AppError::Conflict(message)) => {
            set_flash(&session, &message).await;
            Redirect::to(&format!("/admin/mail/notifications/{}/edit", def.key)).into_response()
        }
        Err(e) => db_error("notification templates", e),
    }
}

/// Renders the notification edit form.
#[allow(clippy::too_many_arguments)]
async fn notification_form_response(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    def: &NotificationMailDef,
    subject: &str,
    body: &str,
    errors: Option<&[FieldError]>,
    preview: Option<MailPreviewView>,
    error: Option<&str>,
) -> Response {
    let override_row = notification_templates::load_override(pool, def.key).await;
    // The ticket binds this render to the override's
    // current stamp — 0 while no override exists (the form
    // then shows the code default).
    let form_ticket = crate::services::form_ticket_service::issue(
        session,
        &notification_scope(def.key),
        override_row
            .as_ref()
            .map(|r| r.updated_at.timestamp_micros())
            .unwrap_or(0),
    )
    .await;
    render(NotificationFormView {
        layout: layout_for(pool, admin, session).await,
        nav: "mail_templates",
        key: def.key,
        name: def.name,
        customized: override_row.is_some(),
        updated_label: override_row
            .as_ref()
            .map(|r| crate::models::jst::jst_datetime_label(r.updated_at)),
        subject: subject.to_string(),
        body: body.to_string(),
        subject_error: errors.and_then(|e| field_error(e, "subject")),
        body_error: errors.and_then(|e| field_error(e, "body")),
        groups: tag_group_views(def),
        preview,
        error: error.map(str::to_string),
        form_ticket,
    })
}

/// Builds the editor's tag dictionary for one mail.
fn tag_group_views(def: &NotificationMailDef) -> Vec<MailTagGroupView> {
    let mut groups: Vec<MailTagGroupView> = def
        .groups
        .iter()
        .map(|group| MailTagGroupView {
            label: group.label,
            tags: group
                .tags
                .iter()
                .map(|tag| MailTagView {
                    name: tag.name,
                    label: tag.label,
                    block: tag.block,
                })
                .collect(),
        })
        .collect();
    if !def.specific_tags.is_empty() {
        groups.push(MailTagGroupView {
            label: "このメール固有",
            tags: def
                .specific_tags
                .iter()
                .map(|tag| MailTagView {
                    name: tag.name,
                    label: tag.label,
                    block: tag.block,
                })
                .collect(),
        });
    }
    groups
}

// ----------------------------------------------------------------
// Step-mail scenarios (EC V2 #2)
// ----------------------------------------------------------------

/// GET /admin/mail/scenarios: the scenario list with send
/// totals and the enable toggle. Copy and delays are
/// seeded (internal step-mail design record); this surface only
/// shows state and flips the switch.
pub async fn scenario_list(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    let scenarios = match crate::repositories::scenario_repository::list_scenarios(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("mail scenarios", crate::error::AppError::Db(e)),
    };
    let counts = match crate::repositories::scenario_repository::send_counts(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("mail scenarios", crate::error::AppError::Db(e)),
    };
    let mut rows = Vec::new();
    for scenario in &scenarios {
        let steps =
            match crate::repositories::scenario_repository::steps_for(&pool, scenario.id).await {
                Ok(v) => v,
                Err(e) => return db_error("mail scenarios", crate::error::AppError::Db(e)),
            };
        // Cumulative send days, the way an operator reads
        // the chain: step delays stack (0, +3, +4 -> days
        // 0, 3, 7).
        let trigger_label = match scenario.trigger_kind.as_str() {
            "signup" => "会員登録",
            "order_delivered" => "配送完了",
            _ => "—",
        };
        let day_zero_label = match scenario.trigger_kind.as_str() {
            "signup" => "登録当日",
            "order_delivered" => "配送完了当日",
            _ => "当日",
        };
        let mut day = 0i32;
        let mut day_labels = Vec::new();
        for step in &steps {
            day += step.delay_days;
            day_labels.push(match day {
                0 => day_zero_label.to_string(),
                d => format!("{d}日後"),
            });
        }
        let counts_row = counts.iter().find(|c| c.scenario_id == scenario.id);
        rows.push(ScenarioRow {
            id: scenario.id,
            name: scenario.name.clone(),
            description: scenario.description.clone(),
            trigger_label: trigger_label.to_string(),
            steps_label: format!("{}通（{}）", steps.len(), day_labels.join("・")),
            enabled: scenario.enabled,
            sent: counts_row.map(|c| c.sent).unwrap_or(0),
            skipped: counts_row.map(|c| c.skipped).unwrap_or(0),
            failed: counts_row.map(|c| c.failed).unwrap_or(0),
        });
    }
    render(ScenarioListView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "mail_scenarios",
        rows,
    })
}

/// POST /admin/mail/scenarios/{id}/toggle: flips the
/// scenario's enabled state. A disabled scenario's due
/// enrollments wait untouched; nothing is sent or skipped
/// while it is off.
pub async fn scenario_toggle(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Path(raw_id): Path<String>,
) -> Response {
    let Ok(id) = raw_id.parse::<i64>() else {
        return record_not_found(&pool, &admin, &session).await;
    };
    let scenarios = match crate::repositories::scenario_repository::list_scenarios(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("mail scenarios", crate::error::AppError::Db(e)),
    };
    let Some(scenario) = scenarios.iter().find(|s| s.id == id) else {
        return record_not_found(&pool, &admin, &session).await;
    };
    match crate::repositories::scenario_repository::set_scenario_enabled(
        &pool,
        id,
        !scenario.enabled,
    )
    .await
    {
        Ok(()) => {
            set_flash(
                &session,
                if scenario.enabled {
                    "シナリオを無効にしました"
                } else {
                    "シナリオを有効にしました"
                },
            )
            .await;
            Redirect::to("/admin/mail/scenarios").into_response()
        }
        Err(e) => db_error("mail scenarios", crate::error::AppError::Db(e)),
    }
}
