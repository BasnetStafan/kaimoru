//! Mail module models.
//! The marketing-mail records: consent, templates, campaigns
//! and the per-recipient send queue. Transactional mail has
//! no models — it renders from the order and account records
//! it already uses.

use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// One member's marketing-mail consent record.
///
/// The row exists for every member who has touched a consent
/// surface (sign-up, MyPage, an unsubscribe link); a member
/// with no row is unsubscribed by definition. `users.
/// mail_magazine` mirrors `status` for the older screens
/// that render the checkbox from the user record; this table
/// is the record of record, because it also carries when
/// consent was given, where, and the unsubscribe token.
#[derive(Clone, FromRow)]
pub struct EmailSubscription {
    /// The member this consent belongs to.
    pub user_id: i64,
    /// `subscribed` or `unsubscribed`.
    pub status: String,
    /// When consent was last granted; kept after an opt-out
    /// as the historical record of the grant.
    pub consented_at: Option<DateTime<Utc>>,
    /// Where the current state was set: `signup`, `mypage`
    /// or `unsubscribe_link`.
    pub source: Option<String>,
    /// The member's no-login unsubscribe credential: a random
    /// token, unique per member, printed only in marketing
    /// mail sent to that member.
    pub unsubscribe_token: String,
    /// When the row was created.
    pub created_at: DateTime<Utc>,
    /// When the row last changed.
    pub updated_at: DateTime<Utc>,
}

impl EmailSubscription {
    /// Whether marketing mail may be sent to this member.
    pub fn is_subscribed(&self) -> bool {
        self.status == "subscribed"
    }
}

/// One reusable marketing-mail body, stored sanitised.
#[derive(Clone, FromRow)]
pub struct EmailTemplate {
    /// Surrogate ID.
    pub id: i64,
    /// Operator-facing name.
    pub name: String,
    /// Subject line (may carry the renderer's variables).
    pub subject: String,
    /// HTML body, ammonia-sanitised at save time.
    pub body_html: String,
    /// Plain-text body.
    pub body_text: String,
    /// When the template was created.
    pub created_at: DateTime<Utc>,
    /// When the template last changed.
    pub updated_at: DateTime<Utc>,
}

/// One marketing campaign: a stored body plus its send state.
#[derive(Clone, FromRow)]
pub struct EmailCampaign {
    /// Surrogate ID.
    pub id: i64,
    /// Operator-facing name.
    pub name: String,
    /// The template the content came from, when it did. The
    /// campaign keeps its own copy of the content either way.
    pub template_id: Option<i64>,
    /// Subject line (may carry the renderer's variables).
    pub subject: String,
    /// HTML body, ammonia-sanitised at save time.
    pub body_html: String,
    /// Plain-text body.
    pub body_text: String,
    /// Audience key: only `subscribed` exists in 7B.
    pub audience: String,
    /// The C1 segment this campaign targets; `None`
    /// means all subscribers — the 7B audience, unchanged.
    pub segment_id: Option<i64>,
    /// `draft`, `scheduled`, `sending` or `sent`.
    pub status: String,
    /// When the worker may start sending; `None` at enqueue
    /// time means "as soon as possible".
    pub scheduled_at: Option<DateTime<Utc>>,
    /// When the campaign's job finished.
    pub sent_at: Option<DateTime<Utc>>,
    /// The admin who created the campaign.
    pub created_by: i64,
    /// When the campaign was created.
    pub created_at: DateTime<Utc>,
    /// When the campaign last changed.
    pub updated_at: DateTime<Utc>,
}

impl EmailCampaign {
    /// The status pill's Japanese label.
    pub fn status_label(&self) -> &'static str {
        match self.status.as_str() {
            "draft" => "下書き",
            "scheduled" => "配信予約済み",
            "sending" => "配信中",
            "sent" => "配信完了",
            _ => "不明",
        }
    }

    /// Whether the campaign can still be edited and enqueued:
    /// only a draft can.
    pub fn is_draft(&self) -> bool {
        self.status == "draft"
    }
}

/// One campaign send run. The worker claims queued jobs
/// whose campaign is due and walks their deliveries.
#[derive(Clone, FromRow)]
pub struct EmailJob {
    /// Surrogate ID.
    pub id: i64,
    /// The campaign being sent.
    pub campaign_id: i64,
    /// `queued`, `running`, `done` or `failed`.
    pub status: String,
    /// When the job was enqueued.
    pub created_at: DateTime<Utc>,
    /// When the worker claimed the job.
    pub started_at: Option<DateTime<Utc>>,
    /// When the job's last delivery settled.
    pub finished_at: Option<DateTime<Utc>>,
}

/// One recipient of one job: the send queue's unit of work.
/// The email is a snapshot from enqueue time, so a member
/// changing address mid-send never redirects a queued mail.
#[derive(Clone, FromRow)]
pub struct EmailDelivery {
    /// Surrogate ID.
    pub id: i64,
    /// The job this delivery belongs to.
    pub job_id: i64,
    /// The campaign being sent (denormalised for reporting).
    pub campaign_id: i64,
    /// The recipient member.
    pub user_id: i64,
    /// The recipient address, snapshotted at enqueue.
    pub email: String,
    /// `queued`, `sending` (claimed by one worker pass),
    /// `sent`, `failed` or `skipped`. `sending` is transient:
    /// a claimed row always ends in a final state or returns
    /// to `queued`.
    pub status: String,
    /// Send attempts made so far.
    pub attempts: i32,
    /// The most recent send failure's message.
    pub last_error: Option<String>,
    /// When the mail was accepted by the transport.
    pub sent_at: Option<DateTime<Utc>>,
    /// When the row was enqueued.
    pub created_at: DateTime<Utc>,
    /// The open-tracking token (a random id), assigned at
    /// first send processing; `None` until then.
    pub open_token: Option<String>,
    /// When a worker pass claimed the row for sending;
    /// `None` while it waits unclaimed.
    pub claimed_at: Option<DateTime<Utc>>,
}

/// One eligible campaign recipient at enqueue time.
pub struct MailRecipient {
    /// The member.
    pub user_id: i64,
    /// Their current address, snapshotted into the delivery.
    pub email: String,
}

/// Delivery tallies for one campaign, for the admin screens.
#[derive(Clone, Copy, Default)]
pub struct CampaignStats {
    /// Deliveries enqueued.
    pub total: i64,
    /// Deliveries accepted by the transport.
    pub sent: i64,
    /// Deliveries that exhausted their attempts.
    pub failed: i64,
    /// Deliveries skipped at send time (consent withdrawn).
    pub skipped: i64,
    /// Deliveries still waiting, claimed or retrying.
    pub queued: i64,
    /// Recipients who opened at least once (C2 tracking).
    pub opens_unique: i64,
    /// Total opens recorded (repeat opens included).
    pub opens_total: i64,
    /// Recipients who clicked at least one link.
    pub clicks_unique: i64,
    /// Total clicks recorded (repeat clicks included).
    pub clicks_total: i64,
}
