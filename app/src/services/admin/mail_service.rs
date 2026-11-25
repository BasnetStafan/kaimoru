//! Admin mail rules.
//!
//! Templates and campaigns for the メール配信 section.
//! Bodies are ammonia-sanitised at save (the CMS v2 rule),
//! campaigns move draft → scheduled → sending → sent, and
//! enqueueing snapshots the consent-filtered recipient
//! list into deliveries — the worker, never a request,
//! does the sending.

use chrono::{DateTime, Datelike, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{DATETIME_FORMAT_MESSAGE_BARE, SEGMENT_NOT_FOUND_MESSAGE};
use crate::models::mail::{CampaignStats, EmailCampaign, EmailTemplate};
use crate::repositories::mail_repository;
use crate::services::admin::audit_service;
use crate::util::html_sanitize;
use crate::validation::FieldError;

/// The raw template form as submitted.
#[derive(Clone, Default)]
pub struct TemplateFormInput {
    /// Template name as typed.
    pub name: String,
    /// Subject as typed.
    pub subject: String,
    /// HTML body as typed (sanitised by validation).
    pub body_html: String,
    /// Plain-text body as typed.
    pub body_text: String,
}

/// A template form that passed validation, ready to store.
#[derive(Debug)]
pub struct ValidatedTemplate {
    /// Trimmed name.
    pub name: String,
    /// Trimmed subject.
    pub subject: String,
    /// Sanitised HTML body.
    pub body_html: String,
    /// Trimmed plain-text body.
    pub body_text: String,
}

/// Validates the shared content fields of templates and
/// campaigns: name and subject bounded, both bodies
/// present, the HTML body sanitised here so no caller can
/// store it raw. Every error at once, each on its field.
fn validate_content(
    name: &str,
    subject: &str,
    body_html: &str,
    body_text: &str,
) -> Result<(String, String, String, String), Vec<FieldError>> {
    let mut errors = Vec::new();
    let name = name.trim().to_string();
    if name.is_empty() {
        errors.push(FieldError::new("name", "名前を入力してください"));
    } else if name.chars().count() > 100 {
        errors.push(FieldError::new(
            "name",
            "名前は100文字以内で入力してください",
        ));
    }
    let subject = subject.trim().to_string();
    if subject.is_empty() {
        errors.push(FieldError::new("subject", "件名を入力してください"));
    } else if subject.chars().count() > 200 {
        errors.push(FieldError::new(
            "subject",
            "件名は200文字以内で入力してください",
        ));
    }
    // Sanitise before the emptiness check: a body of pure
    // script is empty once cleaned, and must fail as empty.
    let body_html = html_sanitize::sanitize_html(body_html.trim());
    if body_html.trim().is_empty() {
        errors.push(FieldError::new("body_html", "HTML本文を入力してください"));
    } else if body_html.chars().count() > 60_000 {
        errors.push(FieldError::new(
            "body_html",
            "HTML本文は60000文字以内で入力してください",
        ));
    }
    let body_text = body_text.trim().to_string();
    if body_text.is_empty() {
        errors.push(FieldError::new(
            "body_text",
            "テキスト本文を入力してください",
        ));
    } else if body_text.chars().count() > 20_000 {
        errors.push(FieldError::new(
            "body_text",
            "テキスト本文は20000文字以内で入力してください",
        ));
    }
    if errors.is_empty() {
        Ok((name, subject, body_html, body_text))
    } else {
        Err(errors)
    }
}

/// Validates one template form.
pub fn validate_template(input: &TemplateFormInput) -> Result<ValidatedTemplate, Vec<FieldError>> {
    let (name, subject, body_html, body_text) = validate_content(
        &input.name,
        &input.subject,
        &input.body_html,
        &input.body_text,
    )?;
    Ok(ValidatedTemplate {
        name,
        subject,
        body_html,
        body_text,
    })
}

/// Lists every template for the admin list and the
/// campaign form's picker.
pub async fn list_templates(pool: &PgPool) -> Result<Vec<EmailTemplate>, AppError> {
    mail_repository::list_templates(pool).await
}

/// Returns one template by ID.
pub async fn find_template(pool: &PgPool, id: i64) -> Result<Option<EmailTemplate>, AppError> {
    mail_repository::find_template(pool, id).await
}

/// Creates one template with its `mail_template.create`
/// audit row.
pub async fn create_template(
    pool: &PgPool,
    admin_id: i64,
    valid: &ValidatedTemplate,
) -> Result<i64, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let id = mail_repository::insert_template(
        &mut tx,
        &valid.name,
        &valid.subject,
        &valid.body_html,
        &valid.body_text,
    )
    .await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "mail_template.create",
        "mail_template",
        Some(id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(id)
}

/// Updates one template with its `mail_template.update`
/// audit row.
pub async fn update_template(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    valid: &ValidatedTemplate,
    expected: i64,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    // The form-ticket staleness guard (see `form_ticket_service`).
    let current = mail_repository::template_updated_at_for_update(&mut tx, id).await?;
    crate::services::form_ticket_service::ensure_fresh(current, expected)?;
    mail_repository::update_template(
        &mut tx,
        id,
        &valid.name,
        &valid.subject,
        &valid.body_html,
        &valid.body_text,
        admin_id,
    )
    .await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "mail_template.update",
        "mail_template",
        Some(id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}

/// Deletes one template with its `mail_template.delete`
/// audit row, written only when a row was actually
/// removed: deletion is idempotent, as on the other
/// admin surfaces.
pub async fn delete_template(pool: &PgPool, admin_id: i64, id: i64) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let removed = mail_repository::delete_template(&mut tx, id).await?;
    if removed > 0 {
        audit_service::record(
            &mut tx,
            admin_id,
            "mail_template.delete",
            "mail_template",
            Some(id),
        )
        .await?;
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}

/// The raw campaign form as submitted.
#[derive(Clone, Default)]
pub struct CampaignFormInput {
    /// Campaign name as typed.
    pub name: String,
    /// Chosen template ID, when the content came from one.
    pub template_id: Option<i64>,
    /// Subject as typed.
    pub subject: String,
    /// HTML body as typed (sanitised by validation).
    pub body_html: String,
    /// Plain-text body as typed.
    pub body_text: String,
    /// Schedule as typed (`datetime-local`, JST); empty
    /// leaves the campaign unscheduled.
    pub scheduled_at: String,
    /// The C1 segment to target; `None` = all
    /// subscribers.
    pub segment_id: Option<i64>,
}

/// A campaign form that passed validation, ready to store.
#[derive(Debug)]
pub struct ValidatedCampaign {
    /// Trimmed name.
    pub name: String,
    /// The template the content came from, if any.
    pub template_id: Option<i64>,
    /// Trimmed subject.
    pub subject: String,
    /// Sanitised HTML body.
    pub body_html: String,
    /// Trimmed plain-text body.
    pub body_text: String,
    /// The schedule as a UTC instant; `None` is uncheduled.
    pub scheduled_at: Option<DateTime<Utc>>,
    /// The segment audience; `None` = all subscribers.
    pub segment_id: Option<i64>,
}

/// Parses a `datetime-local` value as JST wall-clock time
/// into UTC — the same conversion the product form's sale
/// windows use. Empty is `Some(None)`; a malformed value
/// pushes an error and returns `None`.
fn parse_jst_local(
    raw: &str,
    field: &'static str,
    errors: &mut Vec<FieldError>,
) -> Option<Option<DateTime<Utc>>> {
    let text = raw.trim();
    if text.is_empty() {
        return Some(None);
    }
    match chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M") {
        Ok(naive) => {
            let utc = naive - chrono::Duration::hours(9);
            Some(Some(DateTime::<Utc>::from_naive_utc_and_offset(utc, Utc)))
        }
        Err(_) => {
            errors.push(FieldError::new(field, DATETIME_FORMAT_MESSAGE_BARE));
            None
        }
    }
}

/// Validates one campaign form. The template reference is
/// checked against the database, so the signature is async;
/// a missing template is a field error here (the tag form's
/// pattern), never an FK failure at insert time.
pub async fn validate_campaign(
    pool: &PgPool,
    input: &CampaignFormInput,
) -> Result<ValidatedCampaign, Vec<FieldError>> {
    let mut errors = Vec::new();
    if let Some(template_id) = input.template_id {
        match mail_repository::find_template(pool, template_id).await {
            Ok(Some(_)) => {}
            Ok(None) => errors.push(FieldError::new(
                "template_id",
                "選択したテンプレートが存在しません",
            )),
            Err(_) => errors.push(FieldError::new(
                "template_id",
                "テンプレートを確認できませんでした",
            )),
        }
    }
    let content = validate_content(
        &input.name,
        &input.subject,
        &input.body_html,
        &input.body_text,
    );
    let scheduled_at = parse_jst_local(&input.scheduled_at, "scheduled_at", &mut errors);
    let (name, subject, body_html, body_text) = match content {
        Ok(values) => values,
        Err(mut content_errors) => {
            errors.append(&mut content_errors);
            (String::new(), String::new(), String::new(), String::new())
        }
    };
    if errors.is_empty() {
        Ok(ValidatedCampaign {
            name,
            template_id: input.template_id,
            subject,
            body_html,
            body_text,
            scheduled_at: scheduled_at.flatten(),
            segment_id: input.segment_id,
        })
    } else {
        Err(errors)
    }
}

/// Returns one page of the campaign list with its total.
pub async fn list_campaigns(
    pool: &PgPool,
    limit: i64,
    offset: i64,
) -> Result<(Vec<EmailCampaign>, i64), AppError> {
    mail_repository::list_campaigns(pool, limit, offset).await
}

/// Returns one campaign by ID.
pub async fn find_campaign(pool: &PgPool, id: i64) -> Result<Option<EmailCampaign>, AppError> {
    mail_repository::find_campaign(pool, id).await
}

/// Creates one draft campaign with its `mail_campaign.
/// create` audit row.
pub async fn create_campaign(
    pool: &PgPool,
    admin_id: i64,
    valid: &ValidatedCampaign,
) -> Result<i64, AppError> {
    if let Some(segment_id) = valid.segment_id
        && !mail_repository::segment_exists(pool, segment_id).await?
    {
        return Err(AppError::Validation(vec![FieldError::new(
            "segment_id",
            SEGMENT_NOT_FOUND_MESSAGE,
        )]));
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let id = mail_repository::insert_campaign(
        &mut tx,
        admin_id,
        &valid.name,
        valid.template_id,
        &valid.subject,
        &valid.body_html,
        &valid.body_text,
        valid.scheduled_at,
        valid.segment_id,
    )
    .await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "mail_campaign.create",
        "mail_campaign",
        Some(id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(id)
}

/// Updates one draft campaign with its `mail_campaign.
/// update` audit row. A campaign that has left draft state
/// is refused: its content is the record of what was (or
/// will be) sent.
pub async fn update_campaign(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    valid: &ValidatedCampaign,
    expected: i64,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let Some(campaign) = mail_repository::find_campaign_for_update(&mut tx, id).await? else {
        return Err(AppError::NotFound);
    };
    // The form-ticket staleness guard: the row is already
    // locked by the read above, so comparing its stamp
    // here is exact.
    crate::services::form_ticket_service::ensure_fresh(Some(campaign.updated_at), expected)?;
    if !campaign.is_draft() {
        return Err(AppError::Conflict(
            "配信済み・配信予約済みのキャンペーンは編集できません".to_string(),
        ));
    }
    if let Some(segment_id) = valid.segment_id
        && !mail_repository::segment_exists(pool, segment_id).await?
    {
        return Err(AppError::Validation(vec![FieldError::new(
            "segment_id",
            SEGMENT_NOT_FOUND_MESSAGE,
        )]));
    }
    mail_repository::update_campaign(
        &mut tx,
        id,
        &valid.name,
        valid.template_id,
        &valid.subject,
        &valid.body_html,
        &valid.body_text,
        valid.scheduled_at,
        valid.segment_id,
        admin_id,
    )
    .await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "mail_campaign.update",
        "mail_campaign",
        Some(id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}

/// How an enqueue should treat the campaign's schedule.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EnqueueTiming {
    /// Send as soon as the worker picks the job up.
    Now,
    /// Send at the campaign's stored schedule.
    Scheduled,
}

/// Enqueues one draft campaign: snapshots the currently
/// eligible (subscribed, not withdrawn) members into
/// deliveries under one job, moves the campaign to
/// `scheduled`, and writes the `mail_campaign.enqueue`
/// audit row. The job, its deliveries, the status and the
/// audit row commit in one transaction, and the campaign
/// row lock serialises enqueues — two attempts cannot both
/// pass the draft check. The recipient list itself is read
/// on the pool, outside that transaction: a consent change
/// landing between the snapshot and the send is still
/// honoured, because consent is re-checked per recipient
/// at send time regardless — the snapshot can never send
/// to someone who has since opted out. Returns the exact
/// recipient count. Sending itself is the worker's job;
/// nothing is mailed from here.
pub async fn enqueue_campaign(
    pool: &PgPool,
    admin_id: i64,
    id: i64,
    timing: EnqueueTiming,
) -> Result<i64, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let Some(campaign) = mail_repository::find_campaign_for_update(&mut tx, id).await? else {
        return Err(AppError::NotFound);
    };
    if !campaign.is_draft() {
        return Err(AppError::Conflict(
            "このキャンペーンは既に配信予約済みです".to_string(),
        ));
    }
    if timing == EnqueueTiming::Scheduled {
        match campaign.scheduled_at {
            None => {
                return Err(AppError::Validation(vec![FieldError::new(
                    "scheduled_at",
                    "配信日時を設定してください",
                )]));
            }
            // A schedule at or before now would fire at once
            // and silently; it is a form error, not a send.
            Some(at) if at <= Utc::now() => {
                return Err(AppError::Validation(vec![FieldError::new(
                    "scheduled_at",
                    "配信日時は現在時刻より後の日時を設定してください",
                )]));
            }
            _ => {}
        }
    }
    if timing == EnqueueTiming::Now && campaign.scheduled_at.is_some() {
        // "Send now" clears the schedule: the due check reads
        // the campaign row, so a stored future time would
        // otherwise hold the job back.
        mail_repository::update_campaign(
            &mut tx,
            id,
            &campaign.name,
            campaign.template_id,
            &campaign.subject,
            &campaign.body_html,
            &campaign.body_text,
            None,
            campaign.segment_id,
            admin_id,
        )
        .await?;
    }
    // The recipient snapshot, read on the pool outside
    // this transaction (see the doc comment): the send-time
    // consent re-check is what makes that safe. A segment
    // audience intersects the segment's live membership
    // with the eligible subscribers; a segment that no
    // longer resolves refuses the enqueue outright — the
    // audience must never silently widen to everyone.
    let recipients = match campaign.segment_id {
        Some(segment_id) => {
            if !mail_repository::segment_exists(pool, segment_id).await? {
                return Err(AppError::Conflict(
                    "宛先のセグメントが見つかりません。キャンペーンを編集して宛先を選び直してください"
                        .to_string(),
                ));
            }
            mail_repository::list_segment_recipients(pool, segment_id).await?
        }
        None => mail_repository::list_recipients(pool).await?,
    };
    let job_id = mail_repository::insert_job(&mut tx, id).await?;
    mail_repository::insert_deliveries(&mut tx, job_id, id, &recipients).await?;
    mail_repository::set_campaign_status_tx(&mut tx, id, "scheduled").await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "mail_campaign.enqueue",
        "mail_campaign",
        Some(id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    tracing::info!(
        campaign_id = id,
        recipients = recipients.len(),
        action = "mail_campaign_enqueued",
        "mail campaign enqueued"
    );
    Ok(recipients.len() as i64)
}

/// The exact recipient count an enqueue would snapshot
/// right now: the confirm screen's number.
pub async fn eligible_recipient_count(pool: &PgPool) -> Result<i64, AppError> {
    mail_repository::count_subscribers(pool).await
}

/// The same count for a segment audience: segment
/// members who are eligible subscribers right now.
pub async fn eligible_recipient_count_for(
    pool: &PgPool,
    segment_id: Option<i64>,
) -> Result<i64, AppError> {
    match segment_id {
        Some(id) => Ok(mail_repository::list_segment_recipients(pool, id)
            .await?
            .len() as i64),
        None => eligible_recipient_count(pool).await,
    }
}

/// One campaign's delivery tallies, for the list and
/// confirm screens.
pub async fn campaign_stats(pool: &PgPool, campaign_id: i64) -> Result<CampaignStats, AppError> {
    mail_repository::campaign_stats(pool, campaign_id).await
}

/// The campaign list's tallies for a whole page of
/// campaigns in one grouped pass.
pub async fn campaign_stats_bulk(
    pool: &PgPool,
    campaign_ids: &[i64],
) -> Result<std::collections::HashMap<i64, CampaignStats>, AppError> {
    mail_repository::campaign_stats_bulk(pool, campaign_ids).await
}

/// The dashboard's numbers and campaign lists.
pub struct DashboardData {
    /// Members who may receive marketing mail now.
    pub subscribers: i64,
    /// Deliveries sent this calendar month (JST).
    pub sent_this_month: i64,
    /// Deliveries that exhausted their attempts.
    pub failed: i64,
    /// The most recent campaigns.
    pub recent: Vec<EmailCampaign>,
    /// Campaigns waiting for their scheduled time.
    pub scheduled: Vec<EmailCampaign>,
}

/// Loads the mail dashboard.
pub async fn dashboard(pool: &PgPool) -> Result<DashboardData, AppError> {
    // The JST month start as a UTC instant: the first of
    // the month at 00:00 JST is 15:00 UTC the day before.
    let today = crate::models::jst::jst_today();
    let first = today.with_day(1).unwrap_or(today);
    let month_start = first.and_hms_opt(0, 0, 0).map(|naive| {
        DateTime::<Utc>::from_naive_utc_and_offset(naive - chrono::Duration::hours(9), Utc)
    });
    let sent_this_month = match month_start {
        Some(since) => mail_repository::count_sent_since(pool, since).await?,
        None => 0,
    };
    Ok(DashboardData {
        subscribers: mail_repository::count_subscribers(pool).await?,
        sent_this_month,
        failed: mail_repository::count_failed(pool).await?,
        recent: mail_repository::recent_campaigns(pool, 5).await?,
        scheduled: mail_repository::scheduled_campaigns(pool).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template_input(html: &str) -> TemplateFormInput {
        TemplateFormInput {
            name: "テスト".to_string(),
            subject: "件名".to_string(),
            body_html: html.to_string(),
            body_text: "本文".to_string(),
        }
    }

    #[test]
    fn script_tag_is_sanitised_out_at_validation() {
        let valid = validate_template(&template_input(
            "<p>こんにちは</p><script>alert(1)</script>",
        ))
        .expect("valid template");
        assert!(!valid.body_html.contains("<script"));
        assert!(!valid.body_html.contains("alert"));
        assert!(valid.body_html.contains("こんにちは"));
    }

    #[test]
    fn script_only_body_fails_as_empty() {
        let errors = validate_template(&template_input("<script>alert(1)</script>"))
            .expect_err("script-only body must fail");
        assert!(errors.iter().any(|e| e.field == "body_html"));
    }

    #[test]
    fn empty_fields_error_together() {
        let errors = validate_template(&TemplateFormInput::default()).expect_err("must fail");
        assert_eq!(errors.len(), 4);
    }

    #[test]
    fn jst_local_parses_as_jst_wall_clock() {
        let mut errors = Vec::new();
        let parsed = parse_jst_local("2026-12-01T10:30", "scheduled_at", &mut errors)
            .expect("parsed")
            .expect("present");
        assert_eq!(parsed.to_rfc3339(), "2026-12-01T01:30:00+00:00");
        assert!(errors.is_empty());
    }
}
