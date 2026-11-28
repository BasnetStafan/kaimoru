//! Mail module persistence.
//! The only layer that talks to the mail tables: consent
//! records, templates, campaigns and the send queue. Every
//! value is a bound parameter.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::mail::{
    CampaignStats, EmailCampaign, EmailDelivery, EmailJob, EmailSubscription, EmailTemplate,
    MailRecipient,
};
use crate::pii::decrypt_field;

// ----------------------------------------------------------------
// Subscriptions and consent
// ----------------------------------------------------------------

/// Returns one member's subscription row, if they have one.
/// A member with no row is unsubscribed by definition.
pub async fn find_subscription(
    pool: &PgPool,
    user_id: i64,
) -> Result<Option<EmailSubscription>, AppError> {
    sqlx::query_as::<_, EmailSubscription>(
        "SELECT user_id, status, consented_at, source, unsubscribe_token, created_at, updated_at \
         FROM email_subscriptions WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find subscription", e))
}

/// Returns the subscription row carrying one unsubscribe
/// token. The token is a credential: it is looked up by
/// exact bound match and never logged.
pub async fn find_subscription_by_token(
    pool: &PgPool,
    token: &str,
) -> Result<Option<EmailSubscription>, AppError> {
    sqlx::query_as::<_, EmailSubscription>(
        "SELECT user_id, status, consented_at, source, unsubscribe_token, created_at, updated_at \
         FROM email_subscriptions WHERE unsubscribe_token = $1",
    )
    .bind(token)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find subscription by token", e))
}

/// Returns one member's subscription row inside a
/// transaction, locked for the consent service's
/// read-modify-write: a concurrent consent change for the
/// same member waits rather than racing the write.
pub async fn find_subscription_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<Option<EmailSubscription>, AppError> {
    sqlx::query_as::<_, EmailSubscription>(
        "SELECT user_id, status, consented_at, source, unsubscribe_token, created_at, updated_at \
         FROM email_subscriptions WHERE user_id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| AppError::db("find subscription (tx)", e))
}

/// Returns the subscription row carrying one unsubscribe
/// token, inside a transaction and locked: the unsubscribe
/// flow reads and flips the row as one unit.
pub async fn find_subscription_by_token_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    token: &str,
) -> Result<Option<EmailSubscription>, AppError> {
    sqlx::query_as::<_, EmailSubscription>(
        "SELECT user_id, status, consented_at, source, unsubscribe_token, created_at, updated_at \
         FROM email_subscriptions WHERE unsubscribe_token = $1 FOR UPDATE",
    )
    .bind(token)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| AppError::db("find subscription by token (tx)", e))
}

/// Inserts one member's first subscription row.
#[allow(clippy::too_many_arguments)]
pub async fn insert_subscription(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    status: &str,
    consented_at: Option<DateTime<Utc>>,
    source: &str,
    unsubscribe_token: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO email_subscriptions \
         (user_id, status, consented_at, source, unsubscribe_token) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(user_id)
    .bind(status)
    .bind(consented_at)
    .bind(source)
    .bind(unsubscribe_token)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert subscription", e))?;
    Ok(())
}

/// Updates one subscription row's state. `consented_at`
/// moves only when a value is passed: an opt-out keeps the
/// historical grant on record, an opt-in stamps the new one.
pub async fn update_subscription(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    status: &str,
    consented_at: Option<DateTime<Utc>>,
    source: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE email_subscriptions \
         SET status = $2, consented_at = COALESCE($3, consented_at), \
             source = $4, updated_at = now() \
         WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(status)
    .bind(consented_at)
    .bind(source)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update subscription", e))?;
    Ok(())
}

/// Appends one consent-history row: the evidence trail for
/// an opt-in or opt-out, with where it happened.
pub async fn insert_consent_log(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    action: &str,
    source: &str,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO email_consent_logs (user_id, action, source) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(action)
        .bind(source)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("insert consent log", e))?;
    Ok(())
}

/// Counts members who may receive marketing mail right now:
/// subscribed and not withdrawn. The dashboard's headline
/// number and the audience every campaign draws from.
pub async fn count_subscribers(pool: &PgPool) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM email_subscriptions s \
         JOIN users u ON u.id = s.user_id \
         WHERE s.status = 'subscribed' AND u.withdrawn_at IS NULL",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("count subscribers", e))
}

/// Lists the eligible recipients in member order: the
/// enqueue snapshot a campaign's deliveries are built from.
pub async fn list_recipients(pool: &PgPool) -> Result<Vec<MailRecipient>, AppError> {
    let rows = sqlx::query_as::<_, (i64, String)>(
        "SELECT u.id, u.email FROM users u \
         JOIN email_subscriptions s ON s.user_id = u.id \
         WHERE s.status = 'subscribed' AND u.withdrawn_at IS NULL \
         ORDER BY u.id",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list mail recipients", e))?;
    Ok(rows
        .into_iter()
        .map(|(user_id, email)| MailRecipient {
            user_id,
            email: decrypt_field(&email),
        })
        .collect())
}

/// The recipients of a segment-targeted campaign: the
/// segment's current members intersected with the eligible
/// subscribers — segment membership never overrides consent,
/// and send-time re-checks still apply per delivery. A
/// segment id that no longer resolves yields an empty list;
/// the service refuses to enqueue on it rather than widen
/// the audience.
pub async fn list_segment_recipients(
    pool: &PgPool,
    segment_id: i64,
) -> Result<Vec<MailRecipient>, AppError> {
    let Some(segment) =
        crate::repositories::admin::segment_repository::find_segment(pool, segment_id).await?
    else {
        return Ok(Vec::new());
    };
    let member_ids =
        crate::repositories::admin::segment_repository::matching_member_ids(pool, &segment).await?;
    if member_ids.is_empty() {
        return Ok(Vec::new());
    }
    let all = list_recipients(pool).await?;
    Ok(all
        .into_iter()
        .filter(|r| member_ids.contains(&r.user_id))
        .collect())
}

/// Whether a segment still exists (the enqueue guard).
pub async fn segment_exists(pool: &PgPool, segment_id: i64) -> Result<bool, AppError> {
    Ok(
        crate::repositories::admin::segment_repository::find_segment(pool, segment_id)
            .await?
            .is_some(),
    )
}

/// How many campaigns point at one segment (the segment
/// delete guard).
pub async fn count_campaigns_for_segment(pool: &PgPool, segment_id: i64) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM email_campaigns WHERE segment_id = $1")
        .bind(segment_id)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("count campaigns for segment", e))
}

// ----------------------------------------------------------------
// Templates
// ----------------------------------------------------------------

/// Lists every template, newest first.
pub async fn list_templates(pool: &PgPool) -> Result<Vec<EmailTemplate>, AppError> {
    sqlx::query_as::<_, EmailTemplate>(
        "SELECT id, name, subject, body_html, body_text, created_at, updated_at \
         FROM email_templates ORDER BY id DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list mail templates", e))
}

/// Returns the template's current `updated_at` with the
/// row locked: the edit form's staleness guard compares
/// its form ticket's stamp against this value inside the
/// update transaction (the product guard's shape).
pub async fn template_updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<DateTime<Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, DateTime<Utc>>(
        "SELECT updated_at FROM mail_templates WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

/// Returns one template by ID.
pub async fn find_template(pool: &PgPool, id: i64) -> Result<Option<EmailTemplate>, AppError> {
    sqlx::query_as::<_, EmailTemplate>(
        "SELECT id, name, subject, body_html, body_text, created_at, updated_at \
         FROM email_templates WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find mail template", e))
}

/// Inserts one template; returns its ID.
pub async fn insert_template(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    name: &str,
    subject: &str,
    body_html: &str,
    body_text: &str,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO email_templates (name, subject, body_html, body_text) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(name)
    .bind(subject)
    .bind(body_html)
    .bind(body_text)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert mail template", e))
}

/// Updates one template in place.
pub async fn update_template(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    name: &str,
    subject: &str,
    body_html: &str,
    body_text: &str,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE email_templates \
         SET name = $2, subject = $3, body_html = $4, body_text = $5, updated_at = now(), updated_by = $6 \
         WHERE id = $1",
    )
    .bind(id)
    .bind(name)
    .bind(subject)
    .bind(body_html)
    .bind(body_text)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update mail template", e))?;
    Ok(())
}

/// Deletes one template; returns the rows removed. Campaigns
/// that referenced it keep their own content copy and lose
/// only the link (the FK has no cascade by design).
pub async fn delete_template(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<u64, AppError> {
    // Detach campaigns first: the reference is provenance,
    // not a reason to block the delete.
    sqlx::query("UPDATE email_campaigns SET template_id = NULL WHERE template_id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("detach template from campaigns", e))?;
    let result = sqlx::query("DELETE FROM email_templates WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete mail template", e))?;
    Ok(result.rows_affected())
}

// ----------------------------------------------------------------
// Campaigns
// ----------------------------------------------------------------

/// Lists one page of campaigns, newest first, with the
/// filtered total.
pub async fn list_campaigns(
    pool: &PgPool,
    limit: i64,
    offset: i64,
) -> Result<(Vec<EmailCampaign>, i64), AppError> {
    let total = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM email_campaigns")
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("count campaigns", e))?;
    let rows = sqlx::query_as::<_, EmailCampaign>(
        "SELECT id, name, template_id, subject, body_html, body_text, audience, segment_id, status, \
                scheduled_at, sent_at, created_by, created_at, updated_at \
         FROM email_campaigns ORDER BY id DESC LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list campaigns", e))?;
    Ok((rows, total))
}

/// Returns one campaign by ID.
pub async fn find_campaign(pool: &PgPool, id: i64) -> Result<Option<EmailCampaign>, AppError> {
    sqlx::query_as::<_, EmailCampaign>(
        "SELECT id, name, template_id, subject, body_html, body_text, audience, segment_id, status, \
                scheduled_at, sent_at, created_by, created_at, updated_at \
         FROM email_campaigns WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find campaign", e))
}

/// Returns one campaign by ID with its row locked, inside
/// the caller's transaction: the enqueue guard reads and
/// transitions the status atomically.
pub async fn find_campaign_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<EmailCampaign>, AppError> {
    sqlx::query_as::<_, EmailCampaign>(
        "SELECT id, name, template_id, subject, body_html, body_text, audience, segment_id, status, \
                scheduled_at, sent_at, created_by, created_at, updated_at \
         FROM email_campaigns WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| AppError::db("find campaign for update", e))
}

/// The most recent campaigns, for the dashboard.
pub async fn recent_campaigns(pool: &PgPool, limit: i64) -> Result<Vec<EmailCampaign>, AppError> {
    sqlx::query_as::<_, EmailCampaign>(
        "SELECT id, name, template_id, subject, body_html, body_text, audience, segment_id, status, \
                scheduled_at, sent_at, created_by, created_at, updated_at \
         FROM email_campaigns ORDER BY id DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("recent campaigns", e))
}

/// Campaigns waiting for their scheduled time, soonest first.
pub async fn scheduled_campaigns(pool: &PgPool) -> Result<Vec<EmailCampaign>, AppError> {
    sqlx::query_as::<_, EmailCampaign>(
        "SELECT id, name, template_id, subject, body_html, body_text, audience, segment_id, status, \
                scheduled_at, sent_at, created_by, created_at, updated_at \
         FROM email_campaigns WHERE status = 'scheduled' \
         ORDER BY scheduled_at NULLS FIRST, id LIMIT 20",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("scheduled campaigns", e))
}

/// Inserts one draft campaign; returns its ID.
#[allow(clippy::too_many_arguments)]
pub async fn insert_campaign(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    name: &str,
    template_id: Option<i64>,
    subject: &str,
    body_html: &str,
    body_text: &str,
    scheduled_at: Option<DateTime<Utc>>,
    segment_id: Option<i64>,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO email_campaigns \
         (name, template_id, subject, body_html, body_text, scheduled_at, created_by, \
          segment_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(name)
    .bind(template_id)
    .bind(subject)
    .bind(body_html)
    .bind(body_text)
    .bind(scheduled_at)
    .bind(admin_id)
    .bind(segment_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert campaign", e))
}

/// Updates one campaign's editable fields.
#[allow(clippy::too_many_arguments)]
pub async fn update_campaign(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    name: &str,
    template_id: Option<i64>,
    subject: &str,
    body_html: &str,
    body_text: &str,
    scheduled_at: Option<DateTime<Utc>>,
    segment_id: Option<i64>,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE email_campaigns \
         SET name = $2, template_id = $3, subject = $4, body_html = $5, body_text = $6, \
             scheduled_at = $7, segment_id = $8, updated_at = now(), updated_by = $9 \
         WHERE id = $1",
    )
    .bind(id)
    .bind(name)
    .bind(template_id)
    .bind(subject)
    .bind(body_html)
    .bind(body_text)
    .bind(scheduled_at)
    .bind(segment_id)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update campaign", e))?;
    Ok(())
}

/// Sets one campaign's status. Used by the enqueue (inside
/// its transaction) and by the worker's transitions.
pub async fn set_campaign_status_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    status: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE email_campaigns SET status = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(status)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("set campaign status", e))?;
    Ok(())
}

/// Marks one campaign sent and stamps the finish time
/// (worker completion).
pub async fn mark_campaign_sent(pool: &PgPool, id: i64) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE email_campaigns \
         SET status = 'sent', sent_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("mark campaign sent", e))?;
    Ok(())
}

// ----------------------------------------------------------------
// Jobs and deliveries
// ----------------------------------------------------------------

/// Inserts one queued job for a campaign; returns its ID.
pub async fn insert_job(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    campaign_id: i64,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>("INSERT INTO email_jobs (campaign_id) VALUES ($1) RETURNING id")
        .bind(campaign_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| AppError::db("insert mail job", e))
}

/// Inserts the delivery rows for one job: one per recipient,
/// addresses snapshotted from the recipient list.
pub async fn insert_deliveries(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job_id: i64,
    campaign_id: i64,
    recipients: &[MailRecipient],
) -> Result<(), AppError> {
    for recipient in recipients {
        sqlx::query(
            "INSERT INTO email_deliveries (job_id, campaign_id, user_id, email) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(job_id)
        .bind(campaign_id)
        .bind(recipient.user_id)
        .bind(&recipient.email)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("insert mail delivery", e))?;
    }
    Ok(())
}

/// Returns the running job, if one is mid-send: the worker
/// finishes what it started before claiming new work.
pub async fn find_running_job(pool: &PgPool) -> Result<Option<EmailJob>, AppError> {
    sqlx::query_as::<_, EmailJob>(
        "SELECT id, campaign_id, status, created_at, started_at, finished_at \
         FROM email_jobs WHERE status = 'running' ORDER BY id LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find running mail job", e))
}

/// Claims the oldest due queued job inside a transaction:
/// the campaign must be due (no schedule, or its time has
/// come). `FOR UPDATE SKIP LOCKED` keeps two workers from
/// ever claiming the same job.
pub async fn claim_next_job(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Option<EmailJob>, AppError> {
    let job = sqlx::query_as::<_, EmailJob>(
        "SELECT j.id, j.campaign_id, j.status, j.created_at, j.started_at, j.finished_at \
         FROM email_jobs j \
         JOIN email_campaigns c ON c.id = j.campaign_id \
         WHERE j.status = 'queued' \
           AND (c.scheduled_at IS NULL OR c.scheduled_at <= now()) \
         ORDER BY j.id LIMIT 1 FOR UPDATE OF j SKIP LOCKED",
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| AppError::db("claim mail job", e))?;
    let Some(job) = job else {
        return Ok(None);
    };
    sqlx::query("UPDATE email_jobs SET status = 'running', started_at = now() WHERE id = $1")
        .bind(job.id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("start mail job", e))?;
    Ok(Some(job))
}

/// Claims one batch of a job's queued deliveries: the rows
/// flip to `sending` and stamp `claimed_at` in the same
/// statement that selects them, `SKIP LOCKED` keeping two
/// worker processes from ever claiming the same row. The
/// caller sends exactly the rows it claimed.
pub async fn claim_deliveries(
    pool: &PgPool,
    job_id: i64,
    limit: i64,
) -> Result<Vec<EmailDelivery>, AppError> {
    sqlx::query_as::<_, EmailDelivery>(
        "WITH claimed AS ( \
             SELECT id FROM email_deliveries \
             WHERE job_id = $1 AND status = 'queued' \
             ORDER BY id LIMIT $2 FOR UPDATE SKIP LOCKED \
         ) \
         UPDATE email_deliveries d SET status = 'sending', claimed_at = now() \
         FROM claimed WHERE d.id = claimed.id \
         RETURNING d.id, d.job_id, d.campaign_id, d.user_id, d.email, d.status, \
                   d.attempts, d.last_error, d.sent_at, d.created_at, d.claimed_at, \
                   d.open_token",
    )
    .bind(job_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("claim deliveries", e))
}

/// Returns stale claims to the queue: a worker that died
/// mid-batch leaves its rows in `sending`, and once the
/// claim is older than the staleness window another pass
/// may pick them up. Returns the rows reclaimed.
pub async fn reclaim_stale_claims(pool: &PgPool, stale_minutes: i64) -> Result<u64, AppError> {
    let result = sqlx::query(
        "UPDATE email_deliveries SET status = 'queued', claimed_at = NULL \
         WHERE status = 'sending' \
           AND claimed_at < now() - $1 * interval '1 minute'",
    )
    .bind(stale_minutes)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("reclaim stale claims", e))?;
    Ok(result.rows_affected())
}

/// Releases one claimed delivery back to the queue without
/// recording an outcome: the send was never attempted (a
/// consent re-check error, a render failure), so a later
/// pass retries it as if unclaimed.
pub async fn release_delivery(pool: &PgPool, delivery_id: i64) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE email_deliveries SET status = 'queued', claimed_at = NULL \
         WHERE id = $1 AND status = 'sending'",
    )
    .bind(delivery_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("release delivery", e))?;
    Ok(())
}

/// Counts a job's deliveries still in flight: waiting,
/// claimed by a worker pass, or retrying. A job is done
/// only when this reaches zero, so one instance can never
/// finish a job another still holds claims on.
pub async fn count_queued_deliveries(pool: &PgPool, job_id: i64) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM email_deliveries \
         WHERE job_id = $1 AND status IN ('queued', 'sending')",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("count queued deliveries", e))
}

/// Records one send attempt on a delivery and returns the
/// new attempt count: the caller compares it with the
/// attempt ceiling to decide the failure's finality.
pub async fn record_attempt(pool: &PgPool, delivery_id: i64) -> Result<i32, AppError> {
    sqlx::query_scalar::<_, i32>(
        "UPDATE email_deliveries SET attempts = attempts + 1 WHERE id = $1 RETURNING attempts",
    )
    .bind(delivery_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("record delivery attempt", e))
}

/// Marks one delivery sent.
pub async fn mark_delivery_sent(pool: &PgPool, delivery_id: i64) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE email_deliveries SET status = 'sent', sent_at = now(), last_error = NULL, \
             claimed_at = NULL \
         WHERE id = $1",
    )
    .bind(delivery_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("mark delivery sent", e))?;
    Ok(())
}

/// Records one delivery's send failure: final failures move
/// to `failed`; the rest stay queued for the next pass.
pub async fn mark_delivery_failure(
    pool: &PgPool,
    delivery_id: i64,
    error: &str,
    final_failure: bool,
) -> Result<(), AppError> {
    let status = if final_failure { "failed" } else { "queued" };
    sqlx::query(
        "UPDATE email_deliveries SET status = $2, last_error = $3, claimed_at = NULL \
         WHERE id = $1",
    )
    .bind(delivery_id)
    .bind(status)
    .bind(error)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("mark delivery failure", e))?;
    Ok(())
}

/// Marks one delivery skipped: consent was gone at send
/// time, so the mail is never attempted.
pub async fn mark_delivery_skipped(pool: &PgPool, delivery_id: i64) -> Result<(), AppError> {
    sqlx::query("UPDATE email_deliveries SET status = 'skipped', claimed_at = NULL WHERE id = $1")
        .bind(delivery_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::db("mark delivery skipped", e))?;
    Ok(())
}

/// Marks one job done and stamps the finish time.
pub async fn finish_job(pool: &PgPool, job_id: i64) -> Result<(), AppError> {
    sqlx::query("UPDATE email_jobs SET status = 'done', finished_at = now() WHERE id = $1")
        .bind(job_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::db("finish mail job", e))?;
    Ok(())
}

/// Tallies one campaign's deliveries by status.
pub async fn campaign_stats(pool: &PgPool, campaign_id: i64) -> Result<CampaignStats, AppError> {
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT status, count(*) FROM email_deliveries \
         WHERE campaign_id = $1 GROUP BY status",
    )
    .bind(campaign_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("campaign delivery stats", e))?;
    let mut stats = CampaignStats::default();
    for (status, count) in rows {
        stats.total += count;
        match status.as_str() {
            "sent" => stats.sent = count,
            "failed" => stats.failed = count,
            "skipped" => stats.skipped = count,
            // `sending` is a claim in flight, not an
            // outcome: it reports with the waiting rows.
            "queued" | "sending" => stats.queued += count,
            _ => {}
        }
    }
    // C2 tracking aggregates: opens live on the delivery
    // rows, clicks on their link rows.
    let opens = sqlx::query_as::<_, (i64, i64)>(
        "SELECT count(*) FILTER (WHERE opened_at IS NOT NULL), \
                COALESCE(SUM(open_count), 0) \
         FROM email_deliveries WHERE campaign_id = $1",
    )
    .bind(campaign_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("campaign open stats", e))?;
    stats.opens_unique = opens.0;
    stats.opens_total = opens.1;
    let clicks = sqlx::query_as::<_, (i64, i64)>(
        "SELECT count(DISTINCT l.delivery_id) FILTER (WHERE l.first_clicked_at IS NOT NULL), \
                COALESCE(SUM(l.click_count), 0) \
         FROM email_track_links l \
         JOIN email_deliveries d ON d.id = l.delivery_id \
         WHERE d.campaign_id = $1",
    )
    .bind(campaign_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("campaign click stats", e))?;
    stats.clicks_unique = clicks.0;
    stats.clicks_total = clicks.1;
    Ok(stats)
}

/// Tallies deliveries for MANY campaigns in three grouped
/// queries: the campaign list used
/// to run [`campaign_stats`] once per row — 1 + N x 3
/// queries per page. Campaigns with no deliveries are
/// absent from the map; callers default them to zeroes,
/// exactly what the per-campaign tally returns for them.
pub async fn campaign_stats_bulk(
    pool: &PgPool,
    campaign_ids: &[i64],
) -> Result<std::collections::HashMap<i64, CampaignStats>, AppError> {
    let mut by_id: std::collections::HashMap<i64, CampaignStats> = std::collections::HashMap::new();
    if campaign_ids.is_empty() {
        return Ok(by_id);
    }
    let rows = sqlx::query_as::<_, (i64, String, i64)>(
        "SELECT campaign_id, status, count(*) FROM email_deliveries \
         WHERE campaign_id = ANY($1) GROUP BY campaign_id, status",
    )
    .bind(campaign_ids)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("campaign delivery stats bulk", e))?;
    for (campaign_id, status, count) in rows {
        let stats = by_id.entry(campaign_id).or_default();
        stats.total += count;
        match status.as_str() {
            "sent" => stats.sent = count,
            "failed" => stats.failed = count,
            "skipped" => stats.skipped = count,
            "queued" | "sending" => stats.queued += count,
            _ => {}
        }
    }
    let opens = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT campaign_id, count(*) FILTER (WHERE opened_at IS NOT NULL), \
                COALESCE(SUM(open_count), 0) \
         FROM email_deliveries WHERE campaign_id = ANY($1) GROUP BY campaign_id",
    )
    .bind(campaign_ids)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("campaign open stats bulk", e))?;
    for (campaign_id, unique, total) in opens {
        let stats = by_id.entry(campaign_id).or_default();
        stats.opens_unique = unique;
        stats.opens_total = total;
    }
    let clicks = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT d.campaign_id, \
                count(DISTINCT l.delivery_id) FILTER (WHERE l.first_clicked_at IS NOT NULL), \
                COALESCE(SUM(l.click_count), 0) \
         FROM email_track_links l \
         JOIN email_deliveries d ON d.id = l.delivery_id \
         WHERE d.campaign_id = ANY($1) GROUP BY d.campaign_id",
    )
    .bind(campaign_ids)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("campaign click stats bulk", e))?;
    for (campaign_id, unique, total) in clicks {
        let stats = by_id.entry(campaign_id).or_default();
        stats.clicks_unique = unique;
        stats.clicks_total = total;
    }
    Ok(by_id)
}

/// Counts deliveries sent since an instant (the dashboard's
/// sends-this-month, with the JST month start passed in).
pub async fn count_sent_since(pool: &PgPool, since: DateTime<Utc>) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM email_deliveries WHERE status = 'sent' AND sent_at >= $1",
    )
    .bind(since)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("count sent deliveries", e))
}

/// Counts deliveries that exhausted their attempts.
pub async fn count_failed(pool: &PgPool) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM email_deliveries WHERE status = 'failed'")
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("count failed deliveries", e))
}

/// Campaign delivery backlog for the ops status panel: how
/// many deliveries are waiting (queued or claimed) and how
/// many have failed outright.
pub async fn delivery_backlog_counts(pool: &PgPool) -> Result<(i64, i64), AppError> {
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT count(*) FILTER (WHERE status IN ('queued', 'sending'))::bigint, \
                count(*) FILTER (WHERE status = 'failed')::bigint \
         FROM email_deliveries",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("delivery backlog counts", e))
}

/// Campaign jobs currently claimed by a worker (`running`):
/// a number that stays above zero between passes means a
/// stuck job.
pub async fn running_jobs_count(pool: &PgPool) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM email_jobs WHERE status = 'running'")
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("running jobs count", e))
}

/// Assigns a delivery's open-tracking token (first write
/// wins: a retry keeps the token already mailed).
pub async fn set_open_token(pool: &PgPool, delivery_id: i64, token: &str) -> Result<(), AppError> {
    sqlx::query("UPDATE email_deliveries SET open_token = $2 WHERE id = $1 AND open_token IS NULL")
        .bind(delivery_id)
        .bind(token)
        .execute(pool)
        .await
        .map_err(|e| AppError::db("set open token", e))?;
    Ok(())
}

/// The tracking token already stored for one delivery's
/// link to `url`, when a previous attempt created it.
pub async fn track_link_token(
    pool: &PgPool,
    delivery_id: i64,
    url: &str,
) -> Result<Option<String>, AppError> {
    sqlx::query_scalar("SELECT token FROM email_track_links WHERE delivery_id = $1 AND url = $2")
        .bind(delivery_id)
        .bind(url)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("track link token", e))
}

/// Stores one tracked link for a delivery.
pub async fn insert_track_link(
    pool: &PgPool,
    delivery_id: i64,
    url: &str,
    token: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO email_track_links (delivery_id, url, token) VALUES ($1, $2, $3) \
         ON CONFLICT (delivery_id, url) DO NOTHING",
    )
    .bind(delivery_id)
    .bind(url)
    .bind(token)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("insert track link", e))?;
    Ok(())
}

/// Records one open against a token: bumps the total and
/// stamps the first open. Returns whether the token
/// exists at all (an unknown token is a forgery or a
/// stale client and reports as missing).
pub async fn record_open(pool: &PgPool, token: &str) -> Result<bool, AppError> {
    let updated = sqlx::query(
        "UPDATE email_deliveries \
         SET open_count = open_count + 1, opened_at = COALESCE(opened_at, now()) \
         WHERE open_token = $1",
    )
    .bind(token)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("record open", e))?;
    Ok(updated.rows_affected() > 0)
}

/// Records one click against a token and returns the
/// stored target URL — the ONLY URL the redirect may
/// ever use. An unknown token returns `None`.
pub async fn record_click(pool: &PgPool, token: &str) -> Result<Option<String>, AppError> {
    sqlx::query_scalar(
        "UPDATE email_track_links \
         SET click_count = click_count + 1, \
             first_clicked_at = COALESCE(first_clicked_at, now()) \
         WHERE token = $1 RETURNING url",
    )
    .bind(token)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("record click", e))
}
